//! Splices of edits into the executable on disk, off the way of the edits that follow.
//!
//! An edit an editor sent is in the running program once [`live::publish`] has left the patch
//! for it. The splice into the executable on disk is for the next start only, and is the
//! longest step of an edit of a large program on macOS: the copy of the executable, the
//! signature's hash of each page appended, and the write of all of it, tens of milliseconds
//! for a 20 MB debug build. So the splice waits until the edits stop for [`QUIET`], is done on
//! a thread of its own with the newest patch, and is given up on when an edit comes that wants
//! the executable: typing is never kept waiting by the disk write of a keystroke before. An
//! edit that comes while a splice runs is published with a view of the executable as it was
//! ([`Baseline::view`]), gives the splice up and does not wait for it to stop. A request for
//! the executable itself (`cargo ra-jit build`) has the splices done first
//! ([`flush`]).

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, LazyLock, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Context as _;
use rustc_hash::FxHashMap;

use crate::{
    Executable, LINKED, live, lock,
    splice::{Baseline, GaveUp, Spliced},
};

/// How long the edits have to stop for the splice to start: longer than the time between two
/// keys typed.
const QUIET: Duration = Duration::from_millis(150);

struct Pending {
    patch: Vec<u8>,
    due: Instant,
}

#[derive(Default)]
struct State {
    /// The newest patch of each executable that is not spliced yet.
    pending: FxHashMap<PathBuf, Pending>,
    /// The splice under way.
    busy: Option<Busy>,
    /// The executables a splice failed for, which are linked in full when next built.
    failed: Vec<PathBuf>,
    /// Whether the thread that splices is there.
    thread: bool,
}

struct Busy {
    binary: PathBuf,
    /// The word to give it up.
    give_up: Arc<AtomicBool>,
    /// What an edit of the executable is published with meanwhile, once the splice has the
    /// baseline.
    view: Option<Baseline>,
}

static STATE: LazyLock<(Mutex<State>, Condvar)> = LazyLock::new(Default::default);

fn wait<'a>(
    changed: &Condvar,
    state: std::sync::MutexGuard<'a, State>,
) -> std::sync::MutexGuard<'a, State> {
    changed.wait(state).unwrap_or_else(PoisonError::into_inner)
}

/// Has `patch` spliced into the executable at `binary` once the edits stop, in place of any
/// older patch that waits. Its baseline is in [`LINKED`].
pub(crate) fn defer(binary: &Path, patch: Vec<u8>) {
    let (state, changed) = &*STATE;
    let mut state = lock(state);
    state.pending.insert(binary.to_owned(), Pending { patch, due: Instant::now() + QUIET });
    if !state.thread {
        state.thread = true;
        std::thread::Builder::new()
            .name("ra-jit-splice".to_owned())
            .spawn(splice_when_quiet)
            .expect("starting the thread that splices");
    }
    changed.notify_all();
}

/// Gives up on a splice under way of the executable at `binary`, which waits again for the
/// edits to stop. With `wait`, or when the splice has no view yet, waits for it to stop, and
/// the baseline is back in [`LINKED`] for the caller; otherwise says what to publish an edit
/// with meanwhile, and the caller defers its patch.
pub(crate) fn claim(binary: &Path, wait: bool) -> Option<Baseline> {
    let (state, changed) = &*STATE;
    let mut state = lock(state);
    while let Some(busy) = &state.busy
        && busy.binary == binary
    {
        busy.give_up.store(true, Ordering::Relaxed);
        if !wait && let Some(view) = &busy.view {
            return Some(view.view());
        }
        state = self::wait(changed, state);
    }
    None
}

/// Drops the splice that waits of the executable at `binary`, which the caller links or
/// splices with a newer patch.
pub(crate) fn forget(binary: &Path) {
    lock(&STATE.0).pending.remove(binary);
}

/// Does the splices that wait now, and waits for them: the executables on disk are then as
/// the last edits. Says which a splice failed for, which have to be linked again.
pub(crate) fn flush() -> Vec<PathBuf> {
    let (state, changed) = &*STATE;
    let mut state = lock(state);
    if !state.pending.is_empty() {
        let now = Instant::now();
        state.pending.values_mut().for_each(|it| it.due = now);
        changed.notify_all();
    }
    while !state.pending.is_empty() || state.busy.is_some() {
        state = wait(changed, state);
    }
    std::mem::take(&mut state.failed)
}

fn splice_when_quiet() {
    let (state, changed) = &*STATE;
    let mut state = lock(state);
    loop {
        let now = Instant::now();
        let Some(due) = state.pending.values().map(|it| it.due).min() else {
            state.thread = false;
            return;
        };
        if due > now {
            state =
                changed.wait_timeout(state, due - now).unwrap_or_else(PoisonError::into_inner).0;
            continue;
        }
        let Some(binary) = state.pending.iter().find(|it| it.1.due <= now).map(|it| it.0.clone())
        else {
            continue;
        };
        let Some(Pending { patch, .. }) = state.pending.remove(&binary) else { continue };
        let give_up = Arc::new(AtomicBool::new(false));
        state.busy = Some(Busy { binary: binary.clone(), give_up: give_up.clone(), view: None });
        // Taken while `claim` can't look: it finds the splice under way, or the baseline.
        let known = lock(&LINKED).remove(&binary);
        drop(state);
        let outcome = match ready(&binary, known) {
            Ok(baseline) => {
                let view = baseline.view();
                if let Some(busy) = &mut lock(&STATE.0).busy {
                    busy.view = Some(view);
                }
                changed.notify_all();
                splice(&binary, baseline, &patch, &give_up)
            }
            Err(why) => {
                eprintln!("splice: {}: {why:#}; the next build links it in full", binary.display());
                Outcome::Failed
            }
        };
        state = lock(&STATE.0);
        match outcome {
            Outcome::Done => {}
            // An edit that is not spliced in its place leaves this one to be.
            Outcome::GaveUp if !state.pending.contains_key(&binary) => {
                state.pending.insert(binary, Pending { patch, due: Instant::now() + QUIET });
            }
            Outcome::GaveUp => {}
            Outcome::Failed => state.failed.push(binary),
        }
        state.busy = None;
        changed.notify_all();
    }
}

enum Outcome {
    Done,
    GaveUp,
    /// The executable is no longer known, and the next build of it links in full.
    Failed,
}

/// The baseline of the executable known as `known`, with the signature anyone gave it since:
/// the splice then leaves its image as the views have it.
fn ready(binary: &Path, known: Option<Executable>) -> anyhow::Result<Baseline> {
    let known = known.context("no executable to add to")?;
    let mut baseline = known.baseline()?;
    anyhow::ensure!(baseline.is_of(binary), "another executable is asked for");
    baseline.read_again_if_signed()?;
    Ok(baseline)
}

/// Splices `patch` into the executable at `binary`.
fn splice(binary: &Path, mut baseline: Baseline, patch: &[u8], give_up: &AtomicBool) -> Outcome {
    let start = Instant::now();
    let shown = binary.display();
    let spliced = baseline.splice(patch, give_up);
    let took = start.elapsed();
    match spliced {
        Ok(Spliced { redirected, appended, .. }) => {
            lock(&LINKED).insert(binary.to_owned(), Executable::Read(Box::new(baseline)));
            eprintln!(
                "link: {took:.1?} (spliced once the edits stopped: {redirected} functions redirected, {appended} bytes)"
            );
            Outcome::Done
        }
        Err(why) if why.is::<GaveUp>() => {
            lock(&LINKED).insert(binary.to_owned(), Executable::Read(Box::new(baseline)));
            eprintln!("splice: given up after {took:.1?} for a newer edit");
            Outcome::GaveUp
        }
        Err(why) => {
            if let Some(uuid) = baseline.uuid() {
                live::superseded(uuid, format!("the program was linked again: {why:#}"));
            }
            eprintln!("splice: {shown}: {why:#}; the next build links it in full");
            Outcome::Failed
        }
    }
}
