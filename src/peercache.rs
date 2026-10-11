//! Peer dependency cache: the local machine and the remote build machine
//! share their depcache entries over the sync daemon's ssh session.
//!
//! The HTTP tier (`depcache_remote.rs`) serves account-wide and shared
//! entries from build.jcode.sh. This tier needs no server: it uses the link
//! the remote build path already keeps open, so a laptop and its own build
//! machine (aws, ssh, or hosted) each reuse what the other compiled.
//!
//! ```text
//!  laptop                                       build machine
//!  rustc shim --unix--> daemon ==ssh frames==> agent <--unix-- rustc shim
//!               peer.sock   (Relay)     (Relay)   peer.sock
//! ```
//!
//! Both ends run a [`Relay`]: it answers the other side's requests from its
//! own cache (`serve`) and forwards its own shims' requests across the link.
//!
//! When it helps, in each direction:
//! - **Pull on miss.** A shim that misses locally (and in the HTTP tier)
//!   asks its relay for the base key. The relay answers instantly from a
//!   copy of the peer's key set when the peer has nothing, else asks the
//!   peer for its manifest lines. The shim picks the newest line whose
//!   inputs match here, then fetches only if the predicted transfer
//!   (bundle size over measured link throughput, plus queueing behind
//!   bundles already in flight) beats the entry's recorded compile time by
//!   a margin. A cold laptop build next to a warm build machine (or the
//!   reverse) restores most dependencies instead of compiling them.
//! - **Push after a build.** After a local build that compiled dependency
//!   units, the daemon offers the new entries to the machine, which takes
//!   the ones it lacks for compilers it has used, so the next remote build
//!   of this workspace hits immediately. The reverse runs too: the daemon
//!   pulls entries the machine stored, for compilers this laptop has used,
//!   so builds the router later keeps local hit. Both run in the
//!   background, off any build's critical path, and only the most
//!   valuable entries (compile seconds per byte) go first.
//!
//! Safety: identical to a local hit. Keys and manifest lines are portable
//! (cargo home and target dirs are placeholders), an entry is restored only
//! if every input hashes the same here, and `unpack` verifies every binary
//! output against its recorded content hash. A peer never writes outside
//! `o/<32 hex>` and `m/<32 hex>.jsonl`.
//!
//! Fail open: every error is a miss. Disable with `JUSTRUST_PEER_CACHE=0`.

use crate::depcache::{self, Plan};
use crate::paths;
use crate::remote_proto::{Msg, OfferItem, OutInfo, read_frame, write_frame};
use std::collections::{HashMap, HashSet};
use std::io::{BufReader, BufWriter, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Bundle chunk size on the link: keeps other frames (build output,
/// pushes) flowing between chunks of a large bundle.
const CHUNK: usize = 256 << 10;
/// How long a shim waits for the peer's manifest lines.
const QUERY_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long a shim query waits for a link that is not up yet.
const LINK_WAIT: Duration = Duration::from_millis(3000);
/// Throughput assumed before any bundle was measured (compressed bytes/s).
/// Conservative: a slow home uplink.
const DEFAULT_BW: f64 = 4e6;
/// Bundles compress about this well (rlibs measured at 4 to 6x with gzip -1,
/// the 1.5x margin covers incompressible proc macros).
const COMPRESSION: f64 = 3.0;
/// Fetch only if predicted transfer time is below this share of the
/// compile time it saves, and saves at least `MIN_GAIN_SECS`.
const FETCH_FACTOR: f64 = 0.7;
const MIN_GAIN_SECS: f64 = 0.15;
/// Fixed cost of a fetch (round trip, unpack) in seconds.
const FETCH_FIXED_SECS: f64 = 0.08;
/// Key set refreshed at most this often.
const KEYSET_TTL: Duration = Duration::from_secs(5);
/// Most entries one background push or pull moves.
const SYNC_MAX_ITEMS: usize = 400;

pub fn enabled() -> bool {
    depcache::enabled()
        && !matches!(
            std::env::var("JUSTRUST_PEER_CACHE").as_deref(),
            Ok("0") | Ok("off") | Ok("false") | Ok("no")
        )
}

/// Socket the local relay listens on for this machine's rustc shims.
pub fn sock_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("JUSTRUST_PEER_SOCK") {
        return Some(PathBuf::from(p));
    }
    paths::home().ok().map(|h| h.join("peer.sock"))
}

fn valid_key(k: &str) -> bool {
    k.len() == 32
        && k.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn root() -> Option<PathBuf> {
    depcache::cache_dir()
}

// ------------------------------------------------------------------- serve

/// Raw size and compile time of a stored entry, `None` if absent.
pub fn out_info(root: &Path, out: &str) -> Option<OutInfo> {
    if !valid_key(out) {
        return None;
    }
    let dir = root.join("o").join(out);
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
    let mut bytes = 0;
    for f in meta["files"].as_array()? {
        bytes += std::fs::metadata(dir.join(f["name"].as_str()?)).ok()?.len();
    }
    Some(OutInfo {
        out: out.to_string(),
        bytes,
        wall: meta["wall"].as_f64().unwrap_or(0.0),
    })
}

/// Manifest lines for `base` and info on each entry present here.
pub fn lines_for(root: &Path, base: &str) -> (Vec<String>, Vec<OutInfo>) {
    if !valid_key(base) {
        return Default::default();
    }
    let text =
        std::fs::read_to_string(root.join("m").join(format!("{base}.jsonl"))).unwrap_or_default();
    let mut lines = Vec::new();
    let mut outs: Vec<OutInfo> = Vec::new();
    for l in text.lines() {
        let Some(out) = depcache::entry_out(l) else {
            continue;
        };
        if let Some(i) = out_info(root, &out) {
            if !outs.iter().any(|o| o.out == out) {
                outs.push(i);
            }
            lines.push(l.to_string());
        }
    }
    (lines, outs)
}

/// Every base key with a manifest here.
pub fn key_set(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root.join("m")) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let n = e.file_name();
            let k = n.to_str()?.strip_suffix(".jsonl")?;
            valid_key(k).then(|| k.to_string())
        })
        .collect()
}

/// Pack entry `out` into a gzip bundle in memory.
pub fn bundle(root: &Path, out: &str) -> Option<Vec<u8>> {
    if !valid_key(out) {
        return None;
    }
    let dir = root.join("o").join(out);
    let tmp = scratch(root, "pb");
    let r = crate::depcache_remote::pack(&dir, &tmp).and_then(|_| std::fs::read(&tmp).ok());
    let _ = std::fs::remove_file(&tmp);
    if r.is_some() {
        depcache::touch(&dir.join("meta.json"));
    }
    r
}

fn scratch(root: &Path, tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = root.join("tmp");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!(
        "{tag}.{}.{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Install a bundle received from a peer as entry `out` with manifest
/// `line` under `base`. Verified by `unpack` (names, file list, binary
/// hashes). True if the entry is present afterwards.
pub fn install(root: &Path, base: &str, line: &str, out: &str, bundle: &[u8]) -> bool {
    if !valid_key(base) || !valid_key(out) || depcache::entry_out(line).as_deref() != Some(out) {
        return false;
    }
    let final_dir = root.join("o").join(out);
    if !final_dir.join("meta.json").is_file() {
        let tmp = scratch(root, "pi");
        let stage = root
            .join("o")
            .join(format!(".{out}.peer{}", std::process::id()));
        let _ = std::fs::create_dir_all(root.join("o"));
        let ok = std::fs::write(&tmp, bundle).is_ok()
            && crate::depcache_remote::unpack(&tmp, &stage).is_some();
        let _ = std::fs::remove_file(&tmp);
        if !ok || std::fs::rename(&stage, &final_dir).is_err() {
            let _ = std::fs::remove_dir_all(&stage);
            if !final_dir.join("meta.json").is_file() {
                return false;
            }
        }
    }
    add_line(root, base, line)
}

fn add_line(root: &Path, base: &str, line: &str) -> bool {
    let m = root.join("m").join(format!("{base}.jsonl"));
    let known = std::fs::read_to_string(&m).unwrap_or_default();
    if known.lines().any(|l| l == line) {
        return true;
    }
    std::fs::create_dir_all(root.join("m")).is_ok() && paths::append_line(&m, line).is_ok()
}

// ----------------------------------------------------------------- journal

/// Entries stored by builds on this machine, for push and pull.
fn journal(root: &Path) -> PathBuf {
    root.join("peer-journal.jsonl")
}

/// `release` and `commit-hash` of a `rustc -vV`: names the compiler.
pub fn toolchain_tag(vv: &str) -> String {
    let field = |k: &str| {
        vv.lines()
            .find_map(|l| l.strip_prefix(k))
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    format!("{} {}", field("release:"), field("commit-hash:"))
}

fn toolchains_file(root: &Path) -> PathBuf {
    root.join("peer-toolchains")
}

/// Compilers this machine has built with (one tag per line).
pub fn known_toolchains(root: &Path) -> HashSet<String> {
    std::fs::read_to_string(toolchains_file(root))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// After a successful store by the shim: remember it for push and pull.
pub fn record_store(plan: &Plan, line: &str, wall: f64, tc: &str) {
    if !enabled() {
        return;
    }
    let root = &plan.root;
    let Some(out) = depcache::entry_out(line) else {
        return;
    };
    let bytes = out_info(root, &out).map_or(0, |i| i.bytes);
    let item = OfferItem {
        base: plan.base.clone(),
        line: line.to_string(),
        out,
        bytes,
        wall,
        tc: tc.to_string(),
    };
    if let Ok(s) = serde_json::to_string(&item) {
        let _ = paths::append_line(&journal(root), &s);
    }
    note_toolchain(root, tc);
}

/// Remember that this machine builds with `tc`.
pub fn note_toolchain(root: &Path, tc: &str) {
    if tc.trim().is_empty() || known_toolchains(root).contains(tc) {
        return;
    }
    let _ = paths::append_line(&toolchains_file(root), tc);
}

/// Journal items from byte offset `since`, and the next offset.
pub fn journal_since(root: &Path, since: u64) -> (Vec<OfferItem>, u64) {
    let Ok(text) = std::fs::read(journal(root)) else {
        return (Vec::new(), 0);
    };
    let len = text.len() as u64;
    // `u64::MAX`: only learn where the journal ends now.
    if since == u64::MAX {
        return (Vec::new(), len);
    }
    // A truncated or replaced journal starts over.
    let since = if since > len { 0 } else { since };
    let tail = &text[since as usize..];
    // Only whole lines: a writer may be mid-append.
    let end = tail.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let items = String::from_utf8_lossy(&tail[..end])
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    (items, since + end as u64)
}

/// What to send, most valuable first: entries present here, deduplicated,
/// ranked by compile seconds saved per byte moved.
pub fn rank(mut items: Vec<OfferItem>, max: usize) -> Vec<OfferItem> {
    let mut seen = HashSet::new();
    items.reverse();
    items.retain(|i| valid_key(&i.base) && valid_key(&i.out) && seen.insert(i.out.clone()));
    items.sort_by(|a, b| {
        let v = |i: &OfferItem| i.wall / (i.bytes.max(1) as f64);
        v(b).total_cmp(&v(a))
    });
    items.truncate(max);
    items
}

/// Which offered entries this machine should take: absent here, built by a
/// compiler this machine uses.
pub fn wanted(root: &Path, items: &[OfferItem]) -> Vec<String> {
    let tcs = known_toolchains(root);
    items
        .iter()
        .filter(|i| valid_key(&i.out) && valid_key(&i.base))
        .filter(|i| tcs.contains(&i.tc))
        .filter(|i| !root.join("o").join(&i.out).join("meta.json").is_file())
        .map(|i| i.out.clone())
        .collect()
}

// --------------------------------------------------------------- cost model

/// Should a shim fetch an entry of `bytes` raw bytes that took `wall`
/// seconds to compile, given `busy` bundles in flight on a link measured at
/// `bw` compressed bytes/s (0 = unknown)? Returns the predicted seconds.
pub fn worth_fetching(bytes: u64, wall: f64, busy: u32, bw: f64) -> Option<f64> {
    let bw = if bw > 0.0 { bw } else { DEFAULT_BW };
    let wire = bytes as f64 / COMPRESSION;
    // Queued bundles share the link: assume each is about as large as this.
    let secs = FETCH_FIXED_SECS + wire * (1.0 + busy as f64) / bw;
    (secs < wall * FETCH_FACTOR && wall - secs >= MIN_GAIN_SECS).then_some(secs)
}

// -------------------------------------------------------------- shim side

/// After a local miss: ask this machine's relay whether the peer has a
/// matching entry, and fetch and restore it when that beats compiling.
pub fn fetch(plan: &Plan, err: &mut dyn Write) -> Option<depcache::Hit> {
    if !enabled() {
        return None;
    }
    let sock = sock_path()?;
    let Ok(s) = UnixStream::connect(&sock) else {
        note(plan, "no relay (sync daemon not running)");
        return None;
    };
    s.set_read_timeout(Some(QUERY_TIMEOUT)).ok()?;
    let mut w = BufWriter::new(s.try_clone().ok()?);
    let mut r = BufReader::new(s.try_clone().ok()?);
    write_frame(
        &mut w,
        &Msg::CacheQuery {
            id: 1,
            base: plan.base.clone(),
        },
        &[],
    )
    .ok()?;
    let reply = match read_frame(&mut r) {
        Ok(Some(f)) => f,
        _ => {
            note(plan, "relay did not answer in time");
            return None;
        }
    };
    let (lines, outs, busy, bw) = match reply {
        (
            Msg::CacheLines {
                lines,
                outs,
                busy,
                bw,
                ..
            },
            _,
        ) => (lines, outs, busy, bw),
        _ => return None,
    };
    if lines.is_empty() {
        note(plan, "absent");
        return None;
    }
    let mut memo = HashMap::new();
    let Some((line, info)) = lines.iter().rev().find_map(|l| {
        let e: depcache::Entry = serde_json::from_str(l).ok()?;
        let info = outs.iter().find(|o| o.out == e.out)?;
        plan.matches(&e, &mut memo)
            .then(|| (l.clone(), info.clone()))
    }) else {
        note(plan, "inputs differ");
        return None;
    };
    if worth_fetching(info.bytes, info.wall, busy, bw).is_none() {
        note(
            plan,
            &format!(
                "not worth it: {} KB, {:.2}s compile, {busy} in flight, {:.1} MB/s",
                info.bytes / 1024,
                info.wall,
                bw / 1e6
            ),
        );
        return None;
    }
    // The transfer itself may take a while on a slow link.
    s.set_read_timeout(Some(Duration::from_secs(120))).ok()?;
    write_frame(
        &mut w,
        &Msg::CacheFetch {
            id: 2,
            out: info.out.clone(),
        },
        &[],
    )
    .ok()?;
    let mut data = Vec::new();
    loop {
        match read_frame(&mut r).ok()?? {
            (Msg::CacheData { ok: false, .. }, _) => return None,
            (Msg::CacheData { last, .. }, p) => {
                data.extend_from_slice(&p);
                if last {
                    break;
                }
            }
            _ => return None,
        }
    }
    if !install(&plan.root, &plan.base, &line, &info.out, &data) {
        return None;
    }
    plan.restore(err)
}

/// Why a peer lookup did not fetch, for `JUSTRUST_PEER_EXPLAIN=1`.
fn note(plan: &Plan, why: &str) {
    if std::env::var_os("JUSTRUST_PEER_EXPLAIN").is_some() {
        eprintln!("justrust peer: {} {}: {why}", plan.crate_name, plan.base);
    }
}

// ------------------------------------------------------------------ relay

type Pending = Sender<(Msg, Vec<u8>)>;
/// Sends one frame to the peer; false when the link is gone.
type SendFn = Box<dyn Fn(&Msg, &[u8]) -> bool + Send + Sync>;

/// One side of the peer cache on a link (the daemon's or the agent's).
pub struct Relay {
    root: Option<PathBuf>,
    /// Sends one frame to the peer.
    send: SendFn,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
    /// Bundles from the peer in flight now.
    busy: AtomicU32,
    /// Measured throughput from the peer (compressed bytes/s), EWMA.
    bw: Mutex<f64>,
    /// The peer's key set and when it was fetched.
    keys: Mutex<Option<(Instant, Arc<HashSet<String>>)>>,
    keys_loading: std::sync::atomic::AtomicBool,
    /// Incoming `CachePut` bundles being assembled, by the sender's id.
    puts: Mutex<HashMap<u64, Vec<u8>>>,
    /// Whether the peer speaks the cache frames.
    pub capable: bool,
}

impl Relay {
    pub fn new(
        capable: bool,
        send: impl Fn(&Msg, &[u8]) -> bool + Send + Sync + 'static,
    ) -> Arc<Relay> {
        Arc::new(Relay {
            root: root(),
            send: Box::new(send),
            next: AtomicU64::new(1),
            pending: Mutex::default(),
            busy: AtomicU32::new(0),
            bw: Mutex::new(0.0),
            keys: Mutex::default(),
            keys_loading: Default::default(),
            puts: Mutex::default(),
            capable: capable && enabled(),
        })
    }

    /// Handle a cache frame from the peer. Returns false if `msg` is not a
    /// cache frame (the caller handles it).
    pub fn on_frame(self: &Arc<Self>, msg: &Msg, payload: &[u8]) -> bool {
        match msg {
            Msg::CacheLines { id, .. }
            | Msg::CacheData { id, .. }
            | Msg::CacheKeySet { id }
            | Msg::CacheJournal { id, .. }
            | Msg::CacheWant { id, .. } => {
                let done = !matches!(
                    msg,
                    Msg::CacheData {
                        last: false,
                        ok: true,
                        ..
                    }
                );
                let mut p = self.pending.lock().unwrap();
                if let Some(tx) = p.get(id) {
                    let _ = tx.send((msg.clone(), payload.to_vec()));
                }
                if done {
                    p.remove(id);
                }
                true
            }
            Msg::CacheQuery { .. }
            | Msg::CacheFetch { .. }
            | Msg::CacheKeys { .. }
            | Msg::CacheList { .. }
            | Msg::CacheOffer { .. }
            | Msg::CachePut { .. } => {
                // Off the reader thread: serving reads and packs files.
                let me = self.clone();
                let msg = msg.clone();
                let payload = payload.to_vec();
                std::thread::spawn(move || me.serve(msg, payload));
                true
            }
            _ => false,
        }
    }

    /// The link closed: fail every waiter.
    pub fn close(&self) {
        self.pending.lock().unwrap().clear();
    }

    fn reply(&self, m: &Msg, p: &[u8]) {
        let _ = (self.send)(m, p);
    }

    /// Answer one request from the peer from this machine's cache.
    fn serve(&self, msg: Msg, payload: Vec<u8>) {
        let Some(root) = self.root.clone() else {
            return self.refuse(&msg);
        };
        match msg {
            Msg::CacheQuery { id, base } => {
                let (lines, outs) = lines_for(&root, &base);
                self.reply(
                    &Msg::CacheLines {
                        id,
                        lines,
                        outs,
                        busy: 0,
                        bw: 0.0,
                    },
                    &[],
                );
            }
            Msg::CacheFetch { id, out } => match bundle(&root, &out) {
                Some(b) => self.send_chunks(&b, |last| Msg::CacheData { id, last, ok: true }),
                None => self.reply(
                    &Msg::CacheData {
                        id,
                        last: true,
                        ok: false,
                    },
                    &[],
                ),
            },
            Msg::CacheKeys { id } => {
                let keys = key_set(&root).join("\n");
                self.reply(&Msg::CacheKeySet { id }, keys.as_bytes());
            }
            Msg::CacheList { id, since } => {
                let (items, next) = journal_since(&root, since);
                self.reply(&Msg::CacheJournal { id, items, next }, &[]);
            }
            Msg::CacheOffer { id, items } => {
                let outs = wanted(&root, &items);
                self.reply(&Msg::CacheWant { id, outs }, &[]);
            }
            Msg::CachePut {
                id,
                base,
                line,
                out,
                last,
            } => {
                let data = {
                    let mut puts = self.puts.lock().unwrap();
                    let buf = puts.entry(id).or_default();
                    buf.extend_from_slice(&payload);
                    if !last {
                        return;
                    }
                    puts.remove(&id).unwrap_or_default()
                };
                let _ = install(&root, &base, &line, &out, &data);
            }
            _ => {}
        }
    }

    fn refuse(&self, msg: &Msg) {
        match msg {
            Msg::CacheQuery { id, .. } => self.reply(
                &Msg::CacheLines {
                    id: *id,
                    lines: vec![],
                    outs: vec![],
                    busy: 0,
                    bw: 0.0,
                },
                &[],
            ),
            Msg::CacheFetch { id, .. } => self.reply(
                &Msg::CacheData {
                    id: *id,
                    last: true,
                    ok: false,
                },
                &[],
            ),
            Msg::CacheKeys { id } => self.reply(&Msg::CacheKeySet { id: *id }, &[]),
            Msg::CacheList { id, .. } => self.reply(
                &Msg::CacheJournal {
                    id: *id,
                    items: vec![],
                    next: 0,
                },
                &[],
            ),
            Msg::CacheOffer { id, .. } => self.reply(
                &Msg::CacheWant {
                    id: *id,
                    outs: vec![],
                },
                &[],
            ),
            _ => {}
        }
    }

    fn send_chunks(&self, data: &[u8], mk: impl Fn(bool) -> Msg) {
        let mut chunks = data.chunks(CHUNK).peekable();
        if chunks.peek().is_none() {
            self.reply(&mk(true), &[]);
            return;
        }
        while let Some(c) = chunks.next() {
            if !(self.send)(&mk(chunks.peek().is_none()), c) {
                return;
            }
        }
    }

    /// Send a request to the peer and return the channel its replies come on.
    fn request(&self, mk: impl FnOnce(u64) -> Msg) -> Option<Receiver<(Msg, Vec<u8>)>> {
        if !self.capable {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);
        if !(self.send)(&mk(id), &[]) {
            self.pending.lock().unwrap().remove(&id);
            return None;
        }
        Some(rx)
    }

    /// The peer's key set if it is fresh enough to rule a key out, else
    /// `None` (ask the peer). A stale or missing set is refreshed in the
    /// background. The peer gains keys all the time (its builds, our
    /// pushes), so an old set must never turn a hit into a miss.
    fn peer_keys(self: &Arc<Self>) -> Option<Arc<HashSet<String>>> {
        let cur = self.keys.lock().unwrap().clone();
        let stale = cur.as_ref().is_none_or(|(t, _)| t.elapsed() > KEYSET_TTL);
        if stale && !self.keys_loading.swap(true, Ordering::SeqCst) {
            let me = self.clone();
            std::thread::spawn(move || {
                if let Some(rx) = me.request(|id| Msg::CacheKeys { id })
                    && let Ok((_, p)) = rx.recv_timeout(Duration::from_secs(30))
                {
                    let set: HashSet<String> = String::from_utf8_lossy(&p)
                        .lines()
                        .map(str::to_string)
                        .collect();
                    *me.keys.lock().unwrap() = Some((Instant::now(), Arc::new(set)));
                }
                me.keys_loading.store(false, Ordering::SeqCst);
            });
        }
        cur.filter(|_| !stale).map(|(_, k)| k)
    }

    /// Forget the peer's key set (it just gained entries).
    pub fn invalidate_keys(&self) {
        *self.keys.lock().unwrap() = None;
    }

    /// Start loading the peer's key set now (on connect).
    pub fn prefetch_keys(self: &Arc<Self>) {
        if self.capable {
            let _ = self.peer_keys();
        }
    }

    /// A shim on this machine asks for `base`.
    pub fn query(self: &Arc<Self>, base: &str) -> (Vec<String>, Vec<OutInfo>) {
        if !self.capable || !valid_key(base) {
            return Default::default();
        }
        if let Some(keys) = self.peer_keys()
            && !keys.contains(base)
        {
            return Default::default();
        }
        let Some(rx) = self.request(|id| Msg::CacheQuery {
            id,
            base: base.to_string(),
        }) else {
            return Default::default();
        };
        match rx.recv_timeout(QUERY_TIMEOUT) {
            Ok((Msg::CacheLines { lines, outs, .. }, _)) => (lines, outs),
            _ => Default::default(),
        }
    }

    pub fn busy(&self) -> u32 {
        self.busy.load(Ordering::Relaxed)
    }

    pub fn bandwidth(&self) -> f64 {
        *self.bw.lock().unwrap()
    }

    fn note_transfer(&self, bytes: usize, secs: f64) {
        // Small bundles are dominated by latency, not throughput.
        if bytes < 64 << 10 || secs <= 0.0 {
            return;
        }
        let sample = bytes as f64 / secs;
        let mut bw = self.bw.lock().unwrap();
        *bw = if *bw == 0.0 {
            sample
        } else {
            0.7 * *bw + 0.3 * sample
        };
    }

    /// Fetch bundle `out` from the peer. Calls `chunk` for each piece.
    pub fn fetch(&self, out: &str, mut chunk: impl FnMut(&[u8], bool) -> bool) -> bool {
        let Some(rx) = self.request(|id| Msg::CacheFetch {
            id,
            out: out.to_string(),
        }) else {
            return false;
        };
        self.busy.fetch_add(1, Ordering::Relaxed);
        let t = Instant::now();
        let mut total = 0;
        let ok = loop {
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok((Msg::CacheData { ok: true, last, .. }, p)) => {
                    total += p.len();
                    if !chunk(&p, last) {
                        break false;
                    }
                    if last {
                        break true;
                    }
                }
                _ => break false,
            }
        };
        self.busy.fetch_sub(1, Ordering::Relaxed);
        if ok {
            self.note_transfer(total, t.elapsed().as_secs_f64());
        }
        ok
    }

    /// Ask the peer for entries it stored since `since`.
    pub fn list(&self, since: u64) -> Option<(Vec<OfferItem>, u64)> {
        let rx = self.request(|id| Msg::CacheList { id, since })?;
        match rx.recv_timeout(Duration::from_secs(30)).ok()? {
            (Msg::CacheJournal { items, next, .. }, _) => Some((items, next)),
            _ => None,
        }
    }

    /// Offer entries to the peer, returning the ones it wants.
    pub fn offer(&self, items: Vec<OfferItem>) -> Vec<String> {
        let Some(rx) = self.request(|id| Msg::CacheOffer { id, items }) else {
            return Vec::new();
        };
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok((Msg::CacheWant { outs, .. }, _)) => outs,
            _ => Vec::new(),
        }
    }

    /// Send entry `out` to the peer.
    pub fn put(&self, root: &Path, item: &OfferItem) -> bool {
        let Some(b) = bundle(root, &item.out) else {
            return false;
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut ok = true;
        self.send_chunks_checked(&b, &mut ok, |last| Msg::CachePut {
            id,
            base: item.base.clone(),
            line: item.line.clone(),
            out: item.out.clone(),
            last,
        });
        ok
    }

    fn send_chunks_checked(&self, data: &[u8], ok: &mut bool, mk: impl Fn(bool) -> Msg) {
        let mut chunks = data.chunks(CHUNK).peekable();
        while let Some(c) = chunks.next() {
            if !(self.send)(&mk(chunks.peek().is_none()), c) {
                *ok = false;
                return;
            }
        }
    }

    /// Push entries stored on this machine since `since` that the peer
    /// lacks. Returns (entries sent, bytes, next offset).
    pub fn push_new(&self, since: u64) -> (usize, u64, u64) {
        let Some(root) = self.root.clone() else {
            return (0, 0, since);
        };
        let (items, next) = journal_since(&root, since);
        let items: Vec<OfferItem> = rank(items, SYNC_MAX_ITEMS)
            .into_iter()
            .filter(|i| root.join("o").join(&i.out).join("meta.json").is_file())
            .collect();
        if items.is_empty() {
            return (0, 0, next);
        }
        let want: HashSet<String> = self.offer(items.clone()).into_iter().collect();
        let (mut n, mut bytes) = (0, 0);
        for i in items.iter().filter(|i| want.contains(&i.out)) {
            if !self.put(&root, i) {
                break;
            }
            n += 1;
            bytes += i.bytes;
        }
        if n > 0 {
            self.invalidate_keys();
        }
        (n, bytes, next)
    }

    /// Pull entries the peer stored since `since` that this machine wants.
    /// Returns (entries installed, bytes, next offset).
    pub fn pull_new(&self, since: u64) -> (usize, u64, u64) {
        let Some(root) = self.root.clone() else {
            return (0, 0, since);
        };
        let Some((items, next)) = self.list(since) else {
            return (0, 0, since);
        };
        let items = rank(items, SYNC_MAX_ITEMS);
        let want: HashSet<String> = wanted(&root, &items).into_iter().collect();
        let (mut n, mut bytes) = (0, 0);
        for i in items.iter().filter(|i| want.contains(&i.out)) {
            let mut buf = Vec::new();
            let ok = self.fetch(&i.out, |c, _| {
                buf.extend_from_slice(c);
                true
            });
            if ok && install(&root, &i.base, &i.line, &i.out, &buf) {
                n += 1;
                bytes += i.bytes;
            }
        }
        (n, bytes, next)
    }
}

/// The relay of the link that is up now, if any. The shim socket outlives
/// reconnects, so it asks this on every request.
pub type Current = Arc<Mutex<Option<Arc<Relay>>>>;

/// Set while the daemon is connecting on a build's behalf (`Warm`): shim
/// queries then wait briefly for the link instead of missing at once.
pub static CONNECTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Serve this machine's rustc shims on `sock` (until the process exits),
/// through whichever relay is current.
pub fn listen(sock: &Path, current: Current) -> std::io::Result<()> {
    if let Some(d) = sock.parent() {
        std::fs::create_dir_all(d)?;
    }
    let _ = std::fs::remove_file(sock);
    let l = UnixListener::bind(sock)?;
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let current = current.clone();
            std::thread::spawn(move || serve_shim(&current, s));
        }
    });
    Ok(())
}

fn serve_shim(current: &Current, s: UnixStream) {
    let (Ok(r), Ok(w)) = (s.try_clone(), s.try_clone()) else {
        return;
    };
    let mut r = BufReader::new(r);
    let mut w = BufWriter::new(w);
    while let Ok(Some((msg, _))) = read_frame(&mut r) {
        // The daemon may still be connecting (a build asked it to warm up a
        // moment ago): give the link a short while rather than missing.
        let mut relay = current.lock().ok().and_then(|c| c.clone());
        let t = Instant::now();
        while relay.is_none() && CONNECTING.load(Ordering::SeqCst) && t.elapsed() < LINK_WAIT {
            std::thread::sleep(Duration::from_millis(50));
            relay = current.lock().ok().and_then(|c| c.clone());
        }
        match msg {
            Msg::CacheQuery { id, base } => {
                let (lines, outs) = relay.as_ref().map(|r| r.query(&base)).unwrap_or_default();
                let reply = Msg::CacheLines {
                    id,
                    lines,
                    outs,
                    busy: relay.as_ref().map_or(0, |r| r.busy()),
                    bw: relay.as_ref().map_or(0.0, |r| r.bandwidth()),
                };
                if write_frame(&mut w, &reply, &[]).is_err() {
                    return;
                }
            }
            Msg::CacheFetch { id, out } => {
                let mut failed = false;
                let ok = relay.is_some_and(|relay| {
                    relay.fetch(&out, |c, last| {
                        let m = Msg::CacheData { id, last, ok: true };
                        failed = write_frame(&mut w, &m, c).is_err();
                        !failed
                    })
                });
                if !ok && !failed {
                    let _ = write_frame(
                        &mut w,
                        &Msg::CacheData {
                            id,
                            last: true,
                            ok: false,
                        },
                        &[],
                    );
                }
            }
            _ => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_model() {
        // 1 MB rlib that took 5 s: fetch.
        assert!(worth_fetching(1 << 20, 5.0, 0, 4e6).is_some());
        // 50 MB proc macro that took 1 s over a slow link: compile.
        assert!(worth_fetching(50 << 20, 1.0, 0, 2e6).is_none());
        // Trivial unit: never worth the round trip.
        assert!(worth_fetching(10_000, 0.1, 0, 20e6).is_none());
        // A busy link pushes a borderline fetch to compiling.
        let b = 8 << 20;
        assert!(worth_fetching(b, 2.0, 0, 10e6).is_some());
        assert!(worth_fetching(b, 2.0, 8, 10e6).is_none());
        // Unknown bandwidth uses the conservative default.
        assert!(worth_fetching(1 << 20, 3.0, 0, 0.0).is_some());
    }

    #[test]
    fn toolchain_tag_names_the_compiler() {
        let vv = "rustc 1.99.0-nightly (abc 2026-10-08)\nbinary: rustc\ncommit-hash: abc123\nhost: x86_64-unknown-linux-gnu\nrelease: 1.99.0-nightly\n";
        assert_eq!(toolchain_tag(vv), "1.99.0-nightly abc123");
    }

    #[test]
    fn rank_dedups_and_prefers_value_per_byte() {
        let it = |out: char, bytes, wall| OfferItem {
            base: "b".repeat(32),
            line: String::new(),
            out: out.to_string().repeat(32),
            bytes,
            wall,
            tc: String::new(),
        };
        let r = rank(
            vec![
                it('a', 1000, 1.0),
                it('b', 10, 1.0),
                it('a', 1000, 1.0),
                it('c', 100, 0.0),
            ],
            2,
        );
        let outs: Vec<char> = r.iter().map(|i| i.out.chars().next().unwrap()).collect();
        assert_eq!(outs, vec!['b', 'a']);
    }

    #[test]
    fn journal_reads_whole_lines_only() {
        let d = std::env::temp_dir().join(format!("jr-peer-j-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let it = OfferItem {
            base: "b".repeat(32),
            out: "a".repeat(32),
            ..Default::default()
        };
        let line = serde_json::to_string(&it).unwrap();
        std::fs::write(journal(&d), format!("{line}\n{{\"base\":")).unwrap();
        let (items, next) = journal_since(&d, 0);
        assert_eq!(items.len(), 1);
        assert_eq!(next, line.len() as u64 + 1);
        let (items, next2) = journal_since(&d, next);
        assert!(items.is_empty());
        assert_eq!(next2, next);
        // A journal that shrank starts over.
        let (items, _) = journal_since(&d, 1 << 30);
        assert_eq!(items.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wanted_needs_known_compiler_and_absence() {
        let d = std::env::temp_dir().join(format!("jr-peer-w-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("o").join("c".repeat(32))).unwrap();
        std::fs::write(d.join("o").join("c".repeat(32)).join("meta.json"), "{}").unwrap();
        note_toolchain(&d, "1.99 abc");
        note_toolchain(&d, "1.99 abc");
        assert_eq!(known_toolchains(&d).len(), 1);
        let it = |out: char, tc: &str| OfferItem {
            base: "b".repeat(32),
            out: out.to_string().repeat(32),
            tc: tc.into(),
            ..Default::default()
        };
        let w = wanted(
            &d,
            &[
                it('a', "1.99 abc"),
                it('e', "1.98 zzz"),
                it('c', "1.99 abc"),
            ],
        );
        assert_eq!(w, vec!["a".repeat(32)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn install_rejects_bad_keys_and_mismatched_lines() {
        let d = std::env::temp_dir().join(format!("jr-peer-i-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let line = format!(r#"{{"inputs":[],"env":[],"out":"{}"}}"#, "a".repeat(32));
        assert!(!install(&d, "../x", &line, &"a".repeat(32), b""));
        assert!(!install(&d, &"b".repeat(32), &line, &"c".repeat(32), b""));
        assert!(!install(
            &d,
            &"b".repeat(32),
            &line,
            &"a".repeat(32),
            b"junk"
        ));
        assert!(
            !d.join("m")
                .join(format!("{}.jsonl", "b".repeat(32)))
                .exists()
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod serve_tests {
    use super::*;

    #[test]
    fn lines_for_reads_entries() {
        let d = std::env::temp_dir().join(format!("jr-peer-l-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let out = "0123456789abcdef0123456789abcdea";
        let dir = d.join("o").join(out);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(d.join("m")).unwrap();
        std::fs::write(dir.join("x-abc.d"), "x\n").unwrap();
        std::fs::write(
            dir.join("meta.json"),
            r#"{"files":[{"name":"x-abc.d","hash":"h","text":true}],"stderr":[],"wall":4.5}"#,
        )
        .unwrap();
        let line = format!(r#"{{"inputs":[],"env":[],"out":"{out}"}}"#);
        std::fs::write(
            d.join("m").join(format!("{}.jsonl", "a".repeat(32))),
            format!("{line}\n"),
        )
        .unwrap();
        assert_eq!(depcache::entry_out(&line).as_deref(), Some(out));
        assert!(out_info(&d, out).is_some());
        let (lines, outs) = lines_for(&d, &"a".repeat(32));
        assert_eq!(lines, vec![line]);
        assert_eq!(outs.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod relay_tests {
    use super::*;

    /// A relay whose peer answers `CacheKeys` with `keys` and every
    /// `CacheQuery` with one line, counting the queries it sees.
    fn relay_with_peer(keys: &'static str) -> (Arc<Relay>, Arc<AtomicU32>) {
        let queries = Arc::new(AtomicU32::new(0));
        let slot: Arc<Mutex<Option<Arc<Relay>>>> = Arc::default();
        let (s2, q2) = (slot.clone(), queries.clone());
        let relay = Relay::new(true, move |m, _| {
            let me = s2.lock().unwrap().clone().unwrap();
            let reply = match m {
                Msg::CacheKeys { id } => {
                    Some((Msg::CacheKeySet { id: *id }, keys.as_bytes().to_vec()))
                }
                Msg::CacheQuery { id, .. } => {
                    q2.fetch_add(1, Ordering::SeqCst);
                    Some((
                        Msg::CacheLines {
                            id: *id,
                            lines: vec!["l".into()],
                            outs: vec![],
                            busy: 0,
                            bw: 0.0,
                        },
                        vec![],
                    ))
                }
                _ => None,
            };
            if let Some((m, p)) = reply {
                std::thread::spawn(move || {
                    me.on_frame(&m, &p);
                });
            }
            true
        });
        *slot.lock().unwrap() = Some(relay.clone());
        (relay, queries)
    }

    #[test]
    fn fresh_key_set_rules_out_misses_but_stale_one_never_does() {
        let (r, q) = relay_with_peer("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let other = "b".repeat(32);
        // No key set yet: ask the peer.
        assert_eq!(r.query(&other).0.len(), 1);
        for _ in 0..100 {
            if r.keys.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Fresh set without the key: answered locally, no round trip.
        let before = q.load(Ordering::SeqCst);
        assert!(r.query(&other).0.is_empty());
        assert_eq!(q.load(Ordering::SeqCst), before);
        // Stale set (the peer may have gained the key since): ask again.
        r.keys.lock().unwrap().as_mut().unwrap().0 -= KEYSET_TTL * 2;
        assert_eq!(r.query(&other).0.len(), 1);
        // Invalidated after a push: ask again.
        r.invalidate_keys();
        assert_eq!(r.query(&other).0.len(), 1);
    }
}
