//! Edits applied to a program while it runs. The program, through `ra-jit-live`, says
//! where it is and where it has room; each patch that is spliced into its executable is
//! also laid out for that room and left for it to take. See `crates/ra-jit-live`.

use std::{
    sync::{LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, ensure};
use ra_jit_live::format::{self, Body, Patch, Process};
use rustc_hash::FxHashMap;

use crate::{
    lock,
    splice::{Baseline, Lasting},
};

/// What was last laid out for the program that runs each executable, by its UUID.
static PUBLISHED: LazyLock<Mutex<FxHashMap<[u8; 16], Published>>> = LazyLock::new(Default::default);

#[derive(Clone, Copy)]
struct Published {
    pid: u32,
    /// Where the patch ended: what was laid out may not be applied yet, and the next goes
    /// behind it.
    end: u64,
    sequence: u64,
}

/// Where the program that runs each executable, by its UUID, has the statics that patches
/// defined anew, with its pid and when it started: the next patch refers to those
/// ([`Lasting`]).
static LASTING: LazyLock<Mutex<FxHashMap<[u8; 16], ((u32, u64), Lasting)>>> =
    LazyLock::new(Default::default);

/// The program that runs the executable with `uuid` and takes edits, when one does.
pub(crate) fn running(uuid: &[u8; 16]) -> Option<Process> {
    let text = std::fs::read_to_string(format::dir(uuid)?.join("process")).ok()?;
    Process::decode(&text).filter(|it| it.uuid == *uuid && alive(it.pid))
}

/// Whether any program that takes edits runs: a worker stopped while one does starts again
/// from the store, and its first build links the program again, which ends the program's
/// edits.
pub(crate) fn any_running() -> bool {
    let Some(root) = format::root() else { return false };
    let Ok(dirs) = std::fs::read_dir(root) else { return false };
    dirs.flatten().any(|dir| {
        let text = std::fs::read_to_string(dir.path().join("process")).unwrap_or_default();
        Process::decode(&text).is_some_and(|it| alive(it.pid))
    })
}

#[cfg(unix)]
pub(crate) fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    // SAFETY: a signal of 0 is only a question.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(windows)]
pub(crate) fn alive(pid: u32) -> bool {
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::WAIT_TIMEOUT,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: it takes no pointer, and the handle is closed by what owns it from here.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return false;
    }
    // SAFETY: the handle is open and nothing else owns it.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    // SAFETY: the handle is open; a wait of no time only asks whether the process ended.
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0) == WAIT_TIMEOUT }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn alive(_: u32) -> bool {
    false
}

/// Lays `patch` out for the program that runs `baseline`'s executable, if one does, and
/// leaves it for the program. What cannot be laid out leaves the program word to restart.
pub(crate) fn publish(baseline: &Baseline, patch: &std::path::Path) {
    let start = std::time::Instant::now();
    let Some(uuid) = baseline.uuid() else { return };
    let Some(process) = running(&uuid) else { return };
    let mut published = lock(&PUBLISHED);
    let last = published.get(&uuid).copied().filter(|it| it.pid == process.pid);
    let base = last.map_or(process.next, |it| it.end.max(process.next));
    let laid = (|| -> anyhow::Result<(Body, u64)> {
        if let Some((at, why)) = &*lock(&TYPES)
            && *at > process.started
        {
            return Ok((Body::Restart(why.clone()), base));
        }
        let mut all = lock(&LASTING);
        let lasting = match all.entry(uuid).or_default() {
            (of, lasting) if *of == (process.pid, process.started) => lasting,
            // A program that took no patch yet has those of the splice it was started from,
            // and the room's end for the rest.
            known => {
                let of = (process.pid, process.started);
                let lasting = baseline.spliced_lasting(process.started);
                *known = (of, lasting.with_room(Some(process.end)));
                &mut known.1
            }
        };
        let (_, code, moved) = baseline.live(&std::fs::read(patch)?, base, lasting)?;
        let end = base + (code.text.len() + code.data.len()).next_multiple_of(0x4000) as u64;
        // The statics new since the executable take the room from its end down.
        let room = lasting.low().map_or(process.end, |it| it & !0x3fff);
        ensure!(end <= room, "the room the program set aside for edits is used up");
        crate::tier::placed(uuid, process.pid, moved, base + code.text.len() as u64);
        Ok((Body::Code(code), end))
    })();
    let (body, end) = match laid {
        Ok(laid) => laid,
        Err(why) => (Body::Restart(format!("{why:#}")), base),
    };
    let shown = match &body {
        Body::Code(code) if code.lasting.is_empty() => {
            format!("{} functions redirected", code.redirected)
        }
        Body::Code(code) => {
            format!("{} functions redirected, {} new statics", code.redirected, code.lasting.len())
        }
        Body::Restart(why) => format!("restart needed: {why}"),
    };
    let sequence = next_sequence(last);
    match write(&uuid, &Patch { uuid, sequence, body }) {
        Ok(()) => {
            eprintln!(
                "live: edit {sequence} for process {}: {shown} at_us={} took_us={}",
                process.pid,
                epoch_us(),
                start.elapsed().as_micros()
            );
            published.insert(uuid, Published { pid: process.pid, end, sequence });
        }
        Err(why) => eprintln!("live: cannot leave the edit for process {}: {why:#}", process.pid),
    }
}

/// When an edit last changed a type, and which: a program started before has values
/// laid out as they were, which code written for the new layout misreads.
static TYPES: Mutex<Option<(u64, String)>> = Mutex::new(None);

pub(crate) fn types_changed(names: &[String]) {
    let shown = match names {
        [one] => format!("`{one}`"),
        [first, rest @ ..] => format!("`{first}` and {} other types", rest.len()),
        [] => return,
    };
    let why = format!(
        "the edit changes the definition of {shown}, which the running program may have values \
         of laid out as before (a type added under a name of its own, with the code moved to it, \
         goes out as a patch)"
    );
    *lock(&TYPES) = Some((now(), why));
}

/// The executables that were linked again, with why, whose programs are told to restart
/// at the next edit: the edit that was answered with the new link may still be on its way
/// to them, in the file the word to restart would take the place of.
static SUPERSEDED: Mutex<Vec<([u8; 16], String)>> = Mutex::new(Vec::new());

pub(crate) fn superseded(uuid: [u8; 16], why: String) {
    lock(&SUPERSEDED).push((uuid, why));
}

/// Tells the programs that run an executable that was linked again to restart.
pub(crate) fn tell_superseded() {
    let superseded = std::mem::take(&mut *lock(&SUPERSEDED));
    for (uuid, why) in superseded {
        tell_restart(uuid, &why);
    }
}

/// Tells the program that runs `baseline`'s executable, if one does, that it has to be
/// started again to see the edit.
pub(crate) fn restart(baseline: &Baseline, why: &str) {
    if let Some(uuid) = baseline.uuid() {
        tell_restart(uuid, why);
    }
}

fn tell_restart(uuid: [u8; 16], why: &str) {
    let Some(process) = running(&uuid) else { return };
    let mut published = lock(&PUBLISHED);
    let last = published.get(&uuid).copied().filter(|it| it.pid == process.pid);
    let sequence = next_sequence(last);
    let patch = Patch { uuid, sequence, body: Body::Restart(why.to_owned()) };
    if write(&uuid, &patch).is_ok() {
        eprintln!("live: process {} has to be restarted: {why}", process.pid);
        let end = last.map_or(process.next, |it| it.end);
        published.insert(uuid, Published { pid: process.pid, end, sequence });
    }
}

/// Microseconds since the epoch, for lining up a worker's log with the program's.
pub(crate) fn epoch_us() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |it| it.as_micros())
}

/// A number above any the program has seen, also from a daemon that ran before this one.
fn next_sequence(last: Option<Published>) -> u64 {
    now().max(last.map_or(0, |it| it.sequence + 1))
}

/// Milliseconds since the epoch, which the program counts in too.
pub(crate) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |it| it.as_millis() as u64)
}

fn write(uuid: &[u8; 16], patch: &Patch) -> anyhow::Result<()> {
    let dir = format::dir(uuid).context("no home directory")?;
    let partial = dir.join(format!("patch.{}", std::process::id()));
    std::fs::write(&partial, patch.encode())?;
    std::fs::rename(&partial, dir.join("patch"))?;
    wake(&dir);
    Ok(())
}

/// Tells the program that a patch is there, for it to read it at once: one that does not
/// listen, as one of an older `ra-jit-live`, finds it when it next looks.
#[cfg(unix)]
fn wake(dir: &std::path::Path) {
    let Ok(socket) = std::os::unix::net::UnixDatagram::unbound() else { return };
    if socket.set_nonblocking(true).is_ok() {
        _ = socket.send_to(&[1], dir.join(format::WAKE));
    }
}

/// The same on Windows, through the event the program waits on ([`format::wake_event`]).
#[cfg(windows)]
fn wake(dir: &std::path::Path) {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent},
    };
    let name: Vec<u16> = format::wake_event(dir).encode_utf16().chain([0]).collect();
    // SAFETY: the name ends with a zero; the handle is closed below.
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if event.is_null() {
        return;
    }
    // SAFETY: the handle is open and this one's.
    unsafe {
        SetEvent(event);
        CloseHandle(event);
    }
}

#[cfg(not(any(unix, windows)))]
fn wake(_: &std::path::Path) {}
