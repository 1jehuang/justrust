//! Read-only loopback HTTP proxy and local disk mirror for the public prebuilt
//! dependency store (`DEFAULT_STORE`).
//!
//! Hosted build machines run dozens of `rustc` shims in parallel during a cold
//! build. Without a proxy, each shim spawns two cold `curl` processes (manifest
//! and bundle) with fresh TCP+TLS handshakes to CloudFront (~0.5-0.8s per
//! crate, adding 30-50s of wall time on large cold builds).
//!
//! This proxy runs inside `justrust remote-agent` on `127.0.0.1:<ephemeral>`,
//! pools persistent upstream TLS/HTTP connections via a single `ureq::Agent`,
//! and caches verified read-only responses from the public prebuilt store on
//! local NVMe. It is strictly read-only (only `GET /v1/<validated-path>`):
//! tenant builds can never write to it or poison it.
//!
//! The store can be read from several places (`Source`): the CDN, the bucket
//! behind it, and read replicas of that bucket in other regions. Each request
//! goes to the one that has been answering fastest from this machine. A key a
//! machine has never fetched is almost never in the CDN edge next to it, and
//! the edge then fetches it across the country: measured from a us-west-2
//! build host, 0.27 s per manifest and 0.49 s per bundle through the CDN
//! against 0.10 s and 0.13 s straight from the us-east-1 bucket. That is per
//! dependency and serial along the dependency chain, so it set the wall time
//! of every first build after a toolchain change (674 units, 51 s).

use crate::depcache_prebuilt::{self, fresh_file};
use crate::depcache_remote::PUBLIC_MAX_BUNDLE;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

const WORKER_THREADS: usize = 64;
const REFRESH_THREADS: usize = 4;
const REFRESH_QUEUE_CAP: usize = 1024;
const PREFETCH_THREADS: usize = 16;
const PREFETCH_QUEUE_CAP: usize = 2048;
const GROUP_QUEUE_CAP: usize = 32;
const INDEX_TTL_SECS: f64 = 600.0;
/// Oldest index the mirror serves while it refreshes in the background. The
/// shim treats the copy it gets as current for another `INDEX_TTL_SECS`, and
/// a key the index lacks is never asked for. The mirror outlives sessions,
/// so without a bound the first build after an idle night or a toolchain
/// release answered from an index as old as the previous session and missed
/// everything the fleet had published since.
const INDEX_MAX_STALE_SECS: f64 = 2.0 * INDEX_TTL_SECS;
/// How often the mirror checks `HEAD /v1/index` for a new ETag so later
/// checkpoints of the same toolchain become visible within ~1 min without
/// shortening the TTL or polling per build.
#[cfg(not(test))]
const INDEX_POLL_INTERVAL: Duration = Duration::from_secs(60);
const MANIFEST_TTL_SECS: f64 = 3600.0;
const NEGATIVE_TTL: Duration = Duration::from_secs(60);
const MAX_NEGATIVE_ENTRIES: usize = 16_384;
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// Failures or stale answers in a row before an upstream is set aside.
const UPSTREAM_STRIKES: u64 = 3;
const UPSTREAM_BENCH: Duration = Duration::from_secs(300);
/// One request in this many goes to the runner-up, so its latency estimate
/// stays current (an edge that has warmed up, a replica that caught up).
const EXPLORE_EVERY: u64 = 64;

/// One place the store can be read from.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// Base URL, the part before `/v1/`.
    pub base: String,
    /// Whether a missing key here is final. A replica can lag the bucket it
    /// copies, so its "missing" is asked again from an authoritative source.
    pub authoritative: bool,
}

struct Upstream {
    source: Source,
    /// EWMA of the time to response headers of successful requests, in
    /// microseconds. 0 until measured.
    ttfb_us: AtomicU64,
    strikes: AtomicU64,
    benched: Mutex<Option<Instant>>,
    hits: AtomicU64,
    misses: AtomicU64,
    errors: AtomicU64,
    /// Answered "missing" for a key an authoritative source had.
    stale: AtomicU64,
}

impl Upstream {
    fn new(source: Source) -> Self {
        Self {
            source: Source {
                base: source.base.trim_end_matches('/').to_owned(),
                authoritative: source.authoritative,
            },
            ttfb_us: AtomicU64::new(0),
            strikes: AtomicU64::new(0),
            benched: Mutex::new(None),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            stale: AtomicU64::new(0),
        }
    }

    fn note_ok(&self, ttfb: Duration) {
        let sample = (ttfb.as_micros() as u64).max(1);
        let prev = self.ttfb_us.load(Ordering::Relaxed);
        let next = if prev == 0 {
            sample
        } else {
            (prev * 3 + sample) / 4
        };
        self.ttfb_us.store(next, Ordering::Relaxed);
        self.strikes.store(0, Ordering::Relaxed);
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn strike(&self) {
        if self.strikes.fetch_add(1, Ordering::Relaxed) + 1 >= UPSTREAM_STRIKES
            && let Ok(mut b) = self.benched.lock()
        {
            *b = Some(Instant::now() + UPSTREAM_BENCH);
            self.strikes.store(0, Ordering::Relaxed);
        }
    }

    fn is_benched(&self) -> bool {
        let Ok(mut b) = self.benched.lock() else {
            return false;
        };
        match *b {
            Some(until) if Instant::now() < until => true,
            Some(_) => {
                *b = None;
                false
            }
            None => false,
        }
    }
}

/// The order to try the upstreams in for request number `n`: unmeasured ones
/// first (rotated by `n`, so a burst of first requests measures all of them
/// at once), then by measured latency. Benched ones last: they are still
/// tried when everything else fails.
fn upstream_order(ups: &[Upstream], n: u64) -> Vec<usize> {
    let mut fresh = Vec::new();
    let mut measured = Vec::new();
    let mut benched = Vec::new();
    for (i, u) in ups.iter().enumerate() {
        if u.is_benched() {
            benched.push(i);
        } else {
            match u.ttfb_us.load(Ordering::Relaxed) {
                0 => fresh.push(i),
                t => measured.push((t, i)),
            }
        }
    }
    if !fresh.is_empty() {
        let by = (n % fresh.len() as u64) as usize;
        fresh.rotate_left(by);
    }
    measured.sort();
    if fresh.is_empty() && measured.len() > 1 && (n + 1).is_multiple_of(EXPLORE_EVERY) {
        measured.swap(0, 1);
    }
    fresh
        .into_iter()
        .chain(measured.into_iter().map(|(_, i)| i))
        .chain(benched)
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Mutable { max_bytes: u64, ttl_secs: f64 },
    Immutable { max_bytes: u64 },
}

/// Validate a `/v1/<rel>` request path. Only exact public store shapes are
/// accepted; everything else is rejected with 400.
fn validate_rel(rel: &str) -> Option<Kind> {
    if rel == "index" || rel == "costs" || rel == "g/index" {
        return Some(Kind::Mutable {
            max_bytes: MAX_INDEX_BYTES,
            ttl_secs: INDEX_TTL_SECS,
        });
    }
    if let Some(rest) = rel.strip_prefix("g/")
        && let Some(gid) = rest.strip_suffix(".txt")
        && valid_group_id(gid)
    {
        return Some(Kind::Immutable {
            max_bytes: MAX_INDEX_BYTES,
        });
    }
    if let Some(rest) = rel.strip_prefix("m/")
        && let Some(key) = rest.strip_suffix(".jsonl")
        && depcache_prebuilt::valid_key(key)
    {
        return Some(Kind::Mutable {
            max_bytes: MAX_MANIFEST_BYTES,
            ttl_secs: MANIFEST_TTL_SECS,
        });
    }
    if let Some(rest) = rel.strip_prefix("o/")
        && let Some(key) = rest.strip_suffix(".jrdc")
        && depcache_prebuilt::valid_key(key)
    {
        return Some(Kind::Immutable {
            max_bytes: PUBLIC_MAX_BUNDLE,
        });
    }
    if let Some(rest) = rel.strip_prefix("s/")
        && let Some(key) = rest.strip_suffix(".jrbs")
        && depcache_prebuilt::valid_key(key)
    {
        return Some(Kind::Immutable {
            max_bytes: PUBLIC_MAX_BUNDLE,
        });
    }
    None
}

fn valid_group_id(gid: &str) -> bool {
    gid.len() == 16
        && gid
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

type InflightGate = Arc<(Mutex<bool>, Condvar)>;

/// The mirror directory of the proxy this process started, for
/// [`note_toolchain`].
static MIRROR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
/// Compilers noted before the proxy started (an agent's first build picks
/// its compiler before it starts the proxy).
static NOTED_EARLY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Tell the proxy which compiler the next build runs. The first time this
/// machine sees a compiler, its copy of the index predates every unit the
/// fleet built with it, however recently it was refreshed: mark it too old
/// to serve, so the next request refetches it before answering. After a
/// toolchain release this replaces up to `INDEX_TTL_SECS` of builds that
/// compile dependencies the store already has. One refetch per compiler,
/// not a shorter TTL for everyone.
pub fn note_toolchain(id: &str) {
    match MIRROR.get() {
        Some(dir) => {
            note_toolchain_in(dir, id);
        }
        None => {
            if let Ok(mut early) = NOTED_EARLY.lock()
                && early.len() < 16
            {
                early.push(id.to_owned());
            }
        }
    }
}

fn note_toolchain_in(dir: &Path, id: &str) -> bool {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'));
    if !ok {
        return false;
    }
    let seen = dir.join("toolchains-seen");
    let text = std::fs::read_to_string(&seen).unwrap_or_default();
    if text.lines().any(|l| l == id) {
        return false;
    }
    if crate::paths::append_line(&seen, id).is_err() {
        return false;
    }
    // Aged, not removed: it is still the answer if no upstream responds.
    if let Ok(f) = std::fs::File::options()
        .write(true)
        .open(dir.join("v1/index"))
    {
        let _ = f.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1));
    }
    true
}

struct State {
    upstreams: Vec<Upstream>,
    cache_dir: PathBuf,
    agent: ureq::Agent,
    inflight: Mutex<HashMap<String, InflightGate>>,
    missing: Mutex<HashMap<String, Instant>>,
    refreshing: Mutex<HashSet<String>>,
    refresh_tx: mpsc::SyncSender<(String, Kind, PathBuf)>,
    group_tx: mpsc::SyncSender<String>,
    prefetch_tx: mpsc::SyncSender<(String, Kind)>,
    prefetched_groups: Mutex<HashSet<String>>,
    group_index: Mutex<Option<(Instant, HashMap<String, String>)>>,
    seq: AtomicU64,
    /// Upstream fetches started, for the order they try the upstreams in.
    requests: AtomicU64,
}

/// Start the loopback read-only prebuilt proxy in front of one store URL.
/// Returns its base URL (`http://127.0.0.1:<port>`) or `None` if binding
/// fails.
#[cfg(test)]
pub fn start(upstream: String, cache_dir: PathBuf) -> Option<String> {
    start_with(
        vec![Source {
            base: upstream,
            authoritative: true,
        }],
        cache_dir,
    )
}

/// Start the proxy in front of every place the store can be read from. At
/// least one source must be authoritative.
pub fn start_with(sources: Vec<Source>, cache_dir: PathBuf) -> Option<String> {
    if !sources.iter().any(|s| s.authoritative) {
        return None;
    }
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let addr = listener.local_addr().ok()?;
    let _ = std::fs::create_dir_all(cache_dir.join("v1/m"));
    let _ = std::fs::create_dir_all(cache_dir.join("v1/o"));
    let _ = std::fs::create_dir_all(cache_dir.join("v1/s"));
    let _ = std::fs::create_dir_all(cache_dir.join("v1/g"));
    #[cfg(not(test))]
    if MIRROR.set(cache_dir.clone()).is_ok()
        && let Ok(mut early) = NOTED_EARLY.lock()
    {
        for id in early.drain(..) {
            note_toolchain_in(&cache_dir, &id);
        }
    }

    let config = ureq::Agent::config_builder()
        .max_idle_connections(WORKER_THREADS)
        .max_idle_connections_per_host(WORKER_THREADS)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);

    let (refresh_tx, refresh_rx) = mpsc::sync_channel::<(String, Kind, PathBuf)>(REFRESH_QUEUE_CAP);
    let (group_tx, group_rx) = mpsc::sync_channel::<String>(GROUP_QUEUE_CAP);
    let (prefetch_tx, prefetch_rx) = mpsc::sync_channel::<(String, Kind)>(PREFETCH_QUEUE_CAP);
    let state = Arc::new(State {
        upstreams: sources.into_iter().map(Upstream::new).collect(),
        cache_dir,
        agent,
        inflight: Mutex::new(HashMap::new()),
        missing: Mutex::new(HashMap::new()),
        refreshing: Mutex::new(HashSet::new()),
        refresh_tx,
        group_tx,
        prefetch_tx,
        prefetched_groups: Mutex::new(HashSet::new()),
        group_index: Mutex::new(None),
        seq: AtomicU64::new(0),
        requests: AtomicU64::new(0),
    });

    let refresh_rx = Arc::new(Mutex::new(refresh_rx));
    for _ in 0..REFRESH_THREADS {
        let rx = Arc::clone(&refresh_rx);
        let state = Arc::clone(&state);
        let _ = std::thread::Builder::new()
            .name("prebuilt-proxy-r".into())
            .spawn(move || {
                while let Ok((rel, kind, dst)) =
                    rx.lock().ok().and_then(|g| g.recv().ok()).ok_or(())
                {
                    let _ = fetch_upstream(&state, &rel, kind, &dst);
                    if let Ok(mut set) = state.refreshing.lock() {
                        set.remove(&rel);
                    }
                }
            });
    }

    {
        let state = Arc::clone(&state);
        let _ = std::thread::Builder::new()
            .name("prebuilt-proxy-g".into())
            .spawn(move || {
                while let Ok(base) = group_rx.recv() {
                    prefetch_group_for(&state, &base);
                }
            });
    }
    let prefetch_rx = Arc::new(Mutex::new(prefetch_rx));
    for _ in 0..PREFETCH_THREADS {
        let rx = Arc::clone(&prefetch_rx);
        let state = Arc::clone(&state);
        let _ = std::thread::Builder::new()
            .name("prebuilt-proxy-p".into())
            .spawn(move || {
                while let Ok((rel, kind)) = rx.lock().ok().and_then(|g| g.recv().ok()).ok_or(()) {
                    let _ = fetch_cached(&state, &rel, kind);
                }
            });
    }

    #[cfg(not(test))]
    {
        let state = Arc::clone(&state);
        let _ = std::thread::Builder::new()
            .name("prebuilt-proxy-poll".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(INDEX_POLL_INTERVAL);
                    poll_index_once(&state);
                }
            });
    }

    let (tx, rx) = mpsc::channel::<TcpStream>();
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..WORKER_THREADS {
        let rx = Arc::clone(&rx);
        let state = Arc::clone(&state);
        let _ = std::thread::Builder::new()
            .name("prebuilt-proxy-w".into())
            .spawn(move || {
                while let Ok(stream) = rx.lock().ok().and_then(|g| g.recv().ok()).ok_or(()) {
                    handle_conn(stream, &state);
                }
            });
    }

    let _ = std::thread::Builder::new()
        .name("prebuilt-proxy-a".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let _ = conn.set_nodelay(true);
                if tx.send(conn).is_err() {
                    break;
                }
            }
        });

    Some(format!("http://{addr}"))
}

fn handle_conn(mut stream: TcpStream, state: &State) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut req_line = String::new();
    if reader.read_line(&mut req_line).unwrap_or(0) == 0 {
        return;
    }
    // Drain request headers.
    let mut hdr = String::new();
    loop {
        hdr.clear();
        if reader.read_line(&mut hdr).unwrap_or(0) == 0 || hdr == "\r\n" || hdr == "\n" {
            break;
        }
    }
    let mut parts = req_line.split_whitespace();
    let Some(method) = parts.next() else {
        let _ = write_status(&mut stream, 400, "Bad Request");
        return;
    };
    if method != "GET" {
        let _ = write_status(&mut stream, 405, "Method Not Allowed");
        return;
    }
    let Some(path) = parts.next() else {
        let _ = write_status(&mut stream, 400, "Bad Request");
        return;
    };
    let Some(rel) = path.strip_prefix("/v1/") else {
        let _ = write_status(&mut stream, 400, "Bad Request");
        return;
    };
    let Some(kind) = validate_rel(rel) else {
        let _ = write_status(&mut stream, 400, "Bad Request");
        return;
    };

    match fetch_cached(state, rel, kind) {
        FetchResult::Hit(path) => {
            let _ = write_file(&mut stream, &path);
        }
        FetchResult::Missing => {
            let _ = write_status(&mut stream, 404, "Not Found");
        }
        FetchResult::Error => {
            let _ = write_status(&mut stream, 502, "Bad Gateway");
        }
    }
}

enum FetchResult {
    Hit(PathBuf),
    Missing,
    Error,
}

fn is_fresh_cached(path: &Path, kind: Kind) -> bool {
    match kind {
        Kind::Immutable { .. } => path.is_file(),
        Kind::Mutable { ttl_secs, .. } => fresh_file(path, ttl_secs),
    }
}

fn schedule_refresh(state: &State, rel: &str, kind: Kind, dst: &Path) {
    let mut set = match state.refreshing.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if !set.insert(rel.to_owned()) {
        return;
    }
    drop(set);
    if state
        .refresh_tx
        .try_send((rel.to_owned(), kind, dst.to_path_buf()))
        .is_err()
        && let Ok(mut set) = state.refreshing.lock()
    {
        set.remove(rel);
    }
}

fn is_neg_cached(state: &State, rel: &str) -> bool {
    if let Ok(mut map) = state.missing.lock()
        && let Some(&at) = map.get(rel)
    {
        if at.elapsed() < NEGATIVE_TTL {
            return true;
        }
        map.remove(rel);
    }
    false
}

fn note_missing(state: &State, rel: &str) {
    if let Ok(mut map) = state.missing.lock() {
        if map.len() >= MAX_NEGATIVE_ENTRIES {
            map.retain(|_, at| at.elapsed() < NEGATIVE_TTL);
            if map.len() >= MAX_NEGATIVE_ENTRIES {
                map.clear();
            }
        }
        map.insert(rel.to_owned(), Instant::now());
    }
}

fn fetch_cached(state: &State, rel: &str, kind: Kind) -> FetchResult {
    let dst = state.cache_dir.join("v1").join(rel);
    if is_fresh_cached(&dst, kind) {
        return FetchResult::Hit(dst);
    }
    // Stale-while-revalidate for cached mutable metadata (manifests, index,
    // costs): serve the existing local NVMe copy immediately so cold builds
    // never block on hundreds of serial manifest revalidations when a TTL
    // expires, and refresh asynchronously in the bounded background pool.
    // Bundles referenced by manifests are content-addressed and verified on
    // unpack, so serving a slightly stale append-only manifest is always safe.
    // The index is the exception: it decides which keys are asked for at
    // all, so a copy past its bound is refetched first (and still served if
    // no upstream answers).
    let too_stale = rel == "index" && !fresh_file(&dst, INDEX_MAX_STALE_SECS);
    if matches!(kind, Kind::Mutable { .. }) && dst.is_file() && !too_stale {
        schedule_refresh(state, rel, kind, &dst);
        return FetchResult::Hit(dst);
    }
    if is_neg_cached(state, rel) {
        if too_stale && dst.is_file() {
            return FetchResult::Hit(dst);
        }
        return FetchResult::Missing;
    }

    // Coalesce concurrent requests for the same key.
    let (gate, leader) = {
        let mut map = state.inflight.lock().unwrap();
        if let Some(existing) = map.get(rel) {
            (Arc::clone(existing), false)
        } else {
            let g = Arc::new((Mutex::new(false), Condvar::new()));
            map.insert(rel.to_owned(), Arc::clone(&g));
            (g, true)
        }
    };

    if !leader {
        let (lock, cv) = &*gate;
        let mut done = lock.lock().unwrap();
        while !*done {
            done = cv.wait(done).unwrap();
        }
        if is_fresh_cached(&dst, kind) {
            return FetchResult::Hit(dst);
        }
        if is_neg_cached(state, rel) {
            return FetchResult::Missing;
        }
        if too_stale && dst.is_file() {
            return FetchResult::Hit(dst);
        }
        return FetchResult::Error;
    }

    let result = fetch_upstream(state, rel, kind, &dst);
    if matches!(result, FetchResult::Hit(_))
        && let Some(key) = rel
            .strip_prefix("m/")
            .and_then(|r| r.strip_suffix(".jsonl"))
    {
        let _ = state.group_tx.try_send(key.to_owned());
    }

    {
        let mut map = state.inflight.lock().unwrap();
        map.remove(rel);
    }
    {
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }

    match result {
        // Fail open: an old index beats none when the store is unreachable.
        FetchResult::Error if too_stale && dst.is_file() => FetchResult::Hit(dst),
        r => r,
    }
}

enum One {
    Hit,
    Missing,
    Error,
}

/// Fetch from the upstreams in latency order until one has the object.
fn fetch_upstream(state: &State, rel: &str, kind: Kind, dst: &Path) -> FetchResult {
    let max_bytes = match kind {
        Kind::Mutable { max_bytes, .. } | Kind::Immutable { max_bytes } => max_bytes,
    };
    let n = state.requests.fetch_add(1, Ordering::Relaxed);
    let result = fetch_ranked(state, rel, max_bytes, dst, n);
    // Often enough to describe a short build, rarely enough to cost nothing.
    if (n + 1).is_power_of_two() || (n + 1).is_multiple_of(EXPLORE_EVERY) {
        write_stats(state);
    }
    result
}

fn fetch_ranked(state: &State, rel: &str, max_bytes: u64, dst: &Path, n: u64) -> FetchResult {
    // Replicas that said "missing" before another source had the object.
    let mut said_missing: Vec<&Upstream> = Vec::new();
    for i in upstream_order(&state.upstreams, n) {
        let up = &state.upstreams[i];
        match fetch_one(state, up, rel, max_bytes, dst) {
            One::Hit => {
                for stale in said_missing {
                    stale.stale.fetch_add(1, Ordering::Relaxed);
                    stale.strike();
                }
                return FetchResult::Hit(dst.to_path_buf());
            }
            One::Missing if up.source.authoritative => {
                note_missing(state, rel);
                return FetchResult::Missing;
            }
            One::Missing => said_missing.push(up),
            One::Error => up.strike(),
        }
    }
    // No authoritative source answered: not known to be missing.
    FetchResult::Error
}

fn fetch_one(state: &State, up: &Upstream, rel: &str, max_bytes: u64, dst: &Path) -> One {
    let url = format!("{}/v1/{rel}", up.source.base);
    let t0 = Instant::now();
    let resp = match state.agent.get(&url).call() {
        Ok(r) => r,
        Err(_) => {
            up.errors.fetch_add(1, Ordering::Relaxed);
            return One::Error;
        }
    };
    let ttfb = t0.elapsed();
    let status = resp.status().as_u16();
    if status == 404 || status == 403 {
        up.misses.fetch_add(1, Ordering::Relaxed);
        return One::Missing;
    }
    if status != 200 {
        up.errors.fetch_add(1, Ordering::Relaxed);
        return One::Error;
    }
    let etag = (rel == "index")
        .then(|| {
            resp.headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .flatten();

    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let seq = state.seq.fetch_add(1, Ordering::Relaxed);
    let tmp = dst.with_extension(format!("tmp.{}.{seq}", std::process::id()));
    let write_ok = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        let mut reader = resp.into_body().into_reader().take(max_bytes + 1);
        let copied = std::io::copy(&mut reader, &mut f)?;
        if copied > max_bytes {
            return Err(std::io::Error::other("response exceeds max_bytes"));
        }
        f.flush()?;
        Ok(())
    })();
    if write_ok.is_err() {
        let _ = std::fs::remove_file(&tmp);
        up.errors.fetch_add(1, Ordering::Relaxed);
        return One::Error;
    }
    if std::fs::rename(&tmp, dst).is_err() {
        let _ = std::fs::remove_file(&tmp);
        if !dst.is_file() {
            return One::Error;
        }
    }
    if let Some(etag) = etag {
        let _ = std::fs::write(dst.with_extension("etag"), etag);
    }
    up.note_ok(ttfb);
    One::Hit
}

/// Send one `HEAD /v1/index` to the fastest healthy upstream. If its `ETag`
/// differs from the mirror's recorded `v1/index.etag`, schedule a background
/// refresh of `v1/index` and return `true`.
fn poll_index_once(state: &State) -> bool {
    let Some(&idx) = upstream_order(&state.upstreams, 0).first() else {
        return false;
    };
    let up = &state.upstreams[idx];
    let url = format!("{}/v1/index", up.source.base);
    let Ok(resp) = state.agent.head(&url).call() else {
        return false;
    };
    if resp.status().as_u16() != 200 {
        return false;
    }
    let Some(etag) = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    let dst = state.cache_dir.join("v1/index");
    let prev = std::fs::read_to_string(dst.with_extension("etag")).unwrap_or_default();
    if dst.is_file() && prev == etag {
        return false;
    }
    if let Some(kind) = validate_rel("index") {
        schedule_refresh(state, "index", kind, &dst);
        return true;
    }
    false
}

/// Look up `base` in `v1/g/index`; if it belongs to a group not yet prefetched
/// by this proxy, fetch `v1/g/<gid>.txt` and enqueue its missing manifests and
/// bundles onto the background prefetch pool. Fail-open on any error.
fn prefetch_group_for(state: &State, base: &str) {
    let gid = {
        let mut guard = match state.group_index.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let stale = guard
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed().as_secs_f64() > INDEX_TTL_SECS);
        if stale {
            drop(guard);
            let Some(kind) = validate_rel("g/index") else {
                return;
            };
            let FetchResult::Hit(path) = fetch_cached(state, "g/index", kind) else {
                return;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                return;
            };
            let mut map = HashMap::new();
            for line in text.lines() {
                let mut parts = line.split_whitespace();
                if let (Some(k), Some(g), None) = (parts.next(), parts.next(), parts.next())
                    && depcache_prebuilt::valid_key(k)
                    && valid_group_id(g)
                {
                    map.insert(k.to_owned(), g.to_owned());
                }
            }
            guard = match state.group_index.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            *guard = Some((Instant::now(), map));
        }
        guard.as_ref().and_then(|(_, m)| m.get(base).cloned())
    };
    let Some(gid) = gid else {
        return;
    };
    {
        let mut seen = match state.prefetched_groups.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if !seen.insert(gid.clone()) {
            return;
        }
    }
    let rel = format!("g/{gid}.txt");
    let Some(kind) = validate_rel(&rel) else {
        return;
    };
    let FetchResult::Hit(path) = fetch_cached(state, &rel, kind) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(k), Some(out), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if !depcache_prebuilt::valid_key(k) || !depcache_prebuilt::valid_key(out) {
            continue;
        }
        let m_rel = format!("m/{k}.jsonl");
        if !state.cache_dir.join("v1").join(&m_rel).is_file()
            && let Some(mk) = validate_rel(&m_rel)
        {
            let _ = state.prefetch_tx.try_send((m_rel, mk));
        }
        let o_rel = format!("o/{out}.jrdc");
        if !state.cache_dir.join("v1").join(&o_rel).is_file()
            && let Some(ok) = validate_rel(&o_rel)
        {
            let _ = state.prefetch_tx.try_send((o_rel, ok));
        }
    }
}

/// What each upstream has been doing, for `cat <mirror>/upstreams`: one line
/// per source with its latency estimate and counts. Diagnosis only.
fn write_stats(state: &State) {
    let mut text = String::new();
    for u in &state.upstreams {
        text.push_str(&format!(
            "{} ttfb_ms={:.1} hits={} misses={} errors={} stale={} benched={}\n",
            u.source.base,
            u.ttfb_us.load(Ordering::Relaxed) as f64 / 1000.0,
            u.hits.load(Ordering::Relaxed),
            u.misses.load(Ordering::Relaxed),
            u.errors.load(Ordering::Relaxed),
            u.stale.load(Ordering::Relaxed),
            u.is_benched(),
        ));
    }
    let dst = state.cache_dir.join("upstreams");
    let tmp = state
        .cache_dir
        .join(format!("upstreams.tmp.{}", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &dst).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn write_status(stream: &mut TcpStream, code: u16, reason: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()
}

fn write_file(stream: &mut TcpStream, path: &Path) -> std::io::Result<()> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
    )?;
    std::io::copy(&mut f, stream)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_only_public_store_paths() {
        let k = "0123456789abcdef0123456789abcdef";
        assert!(validate_rel("index").is_some());
        assert!(validate_rel("costs").is_some());
        assert!(validate_rel(&format!("m/{k}.jsonl")).is_some());
        assert!(validate_rel(&format!("o/{k}.jrdc")).is_some());
        assert!(validate_rel(&format!("s/{k}.jrbs")).is_some());
        assert!(validate_rel("../etc/passwd").is_none());
        assert!(validate_rel("m/not-hex.jsonl").is_none());
        assert!(validate_rel(&format!("o/{k}.jrdc?x=1")).is_none());
        assert!(validate_rel("acct/123/o/foo").is_none());
    }

    /// A store on loopback that has `have` (path suffix, body) and answers
    /// 404 for everything else, each request after `delay`.
    fn fake_store(have: Vec<(String, Vec<u8>)>, delay: Duration) -> (String, Arc<AtomicU64>) {
        let hits = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&hits);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let mut stream = conn;
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = r.read_line(&mut line);
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
                        break;
                    }
                }
                std::thread::sleep(delay);
                counter.fetch_add(1, Ordering::Relaxed);
                match have.iter().find(|(path, _)| line.contains(path.as_str())) {
                    Some((_, body)) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    }
                }
                let _ = stream.flush();
            }
        });
        (format!("http://{addr}"), hits)
    }

    fn test_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "justrust-prebuilt-proxy-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn key(i: usize) -> String {
        format!("{i:032x}")
    }

    /// Measurement against the real public store, from wherever this runs
    /// (`justrust test --lib -- --ignored live_store_latency --nocapture`).
    /// Two fresh mirrors, so every key is one this "machine" never fetched:
    /// one proxy reads the CDN only (the behavior before sources were
    /// ranked), the other every source. Disjoint keys, so neither warms the
    /// CDN edge for the other.
    #[test]
    #[ignore = "real network: measures the public prebuilt store"]
    fn live_store_latency() {
        fn unit(base: &str, tmp: &Path, k: &str) -> Option<(f64, f64, u64)> {
            let m = tmp.join(format!("m-{k}"));
            let t0 = Instant::now();
            if !matches!(
                depcache_prebuilt::get(base, &format!("m/{k}.jsonl"), &m, 30, 1 << 20),
                depcache_prebuilt::Got::Ok
            ) {
                return None;
            }
            let m_secs = t0.elapsed().as_secs_f64();
            let text = std::fs::read_to_string(&m).ok()?;
            let out = text.lines().rev().find_map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).ok()?;
                v.get("out")?.as_str().map(str::to_owned)
            })?;
            let b = tmp.join(format!("o-{out}"));
            let t1 = Instant::now();
            if !matches!(
                depcache_prebuilt::get(base, &format!("o/{out}.jrdc"), &b, 120, PUBLIC_MAX_BUNDLE),
                depcache_prebuilt::Got::Ok
            ) {
                return None;
            }
            let bytes = std::fs::metadata(&b).map(|m| m.len()).unwrap_or(0);
            Some((m_secs, t1.elapsed().as_secs_f64(), bytes))
        }
        fn median(mut v: Vec<f64>) -> f64 {
            v.sort_by(|a, b| a.total_cmp(b));
            v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
        }
        let per_side: usize = std::env::var("JUSTRUST_LIVE_STORE_KEYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let store = depcache_prebuilt::DEFAULT_STORE;
        let cdn_dir = test_dir("live-cdn");
        let all_dir = test_dir("live-all");
        let cdn = start(store.to_owned(), cdn_dir.clone()).unwrap();
        let all = start_with(depcache_prebuilt::proxy_sources(store), all_dir.clone()).unwrap();

        let idx = cdn_dir.join("index");
        assert!(matches!(
            depcache_prebuilt::get(&all, "index", &idx, 60, MAX_INDEX_BYTES),
            depcache_prebuilt::Got::Ok
        ));
        let index = std::fs::read_to_string(&idx).unwrap();
        let keys: Vec<&str> = index.lines().collect();
        // Spread over the whole index, seeded by the clock so a rerun takes
        // keys the CDN edge has not been asked for.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let step = (keys.len() / (2 * per_side)).max(1);
        let picked: Vec<&str> = (0..2 * per_side)
            .map(|i| keys[(seed % step + i * step) % keys.len()])
            .collect();
        println!(
            "store index: {} keys, measuring {} per side",
            keys.len(),
            per_side
        );
        for (name, base, dir, mine) in [
            ("cdn only ", &cdn, &cdn_dir, &picked[..per_side]),
            ("ranked   ", &all, &all_dir, &picked[per_side..]),
        ] {
            let t0 = Instant::now();
            let got: Vec<(f64, f64, u64)> =
                mine.iter().filter_map(|k| unit(base, dir, k)).collect();
            let total = t0.elapsed().as_secs_f64();
            println!(
                "{name} {} of {} units in {total:.2}s serial: manifest median {:.3}s, bundle median {:.3}s, per unit {:.3}s, {} bytes",
                got.len(),
                mine.len(),
                median(got.iter().map(|g| g.0).collect()),
                median(got.iter().map(|g| g.1).collect()),
                total / mine.len() as f64,
                got.iter().map(|g| g.2).sum::<u64>(),
            );
            assert!(got.len() * 10 >= mine.len() * 9, "store answered too few");
        }
        println!(
            "{}",
            std::fs::read_to_string(all_dir.join("upstreams")).unwrap_or_default()
        );
        let _ = std::fs::remove_dir_all(&cdn_dir);
        let _ = std::fs::remove_dir_all(&all_dir);
    }

    #[test]
    fn order_measures_every_upstream_then_prefers_the_fastest() {
        let ups: Vec<Upstream> = ["http://a", "http://b", "http://c"]
            .iter()
            .map(|b| {
                Upstream::new(Source {
                    base: (*b).to_owned(),
                    authoritative: true,
                })
            })
            .collect();
        // Nothing measured: a burst of first requests is spread over all.
        assert_eq!(upstream_order(&ups, 0), [0, 1, 2]);
        assert_eq!(upstream_order(&ups, 1), [1, 2, 0]);
        assert_eq!(upstream_order(&ups, 2), [2, 0, 1]);
        ups[0].note_ok(Duration::from_millis(270));
        // An unmeasured upstream is tried before a measured one.
        assert_eq!(upstream_order(&ups, 0)[2], 0);
        ups[1].note_ok(Duration::from_millis(100));
        ups[2].note_ok(Duration::from_millis(4));
        assert_eq!(upstream_order(&ups, 0), [2, 1, 0]);
        // Now and then the runner-up goes first, so its estimate stays current.
        assert_eq!(upstream_order(&ups, EXPLORE_EVERY - 1), [1, 2, 0]);
        // Three failures in a row set an upstream aside: last resort only.
        for _ in 0..UPSTREAM_STRIKES {
            ups[2].strike();
        }
        assert_eq!(upstream_order(&ups, 0), [1, 0, 2]);
        // A success in between forgives earlier failures.
        ups[1].strike();
        ups[1].strike();
        ups[1].note_ok(Duration::from_millis(100));
        ups[1].strike();
        assert!(!ups[1].is_benched());
    }

    /// A proxy state without the listener and worker threads, so a test can
    /// read the latency estimates the routing decisions were made from.
    fn state_for(sources: Vec<Source>, cache_dir: PathBuf) -> State {
        // Nothing here goes stale, so nothing is ever queued for refresh.
        let (refresh_tx, _) = mpsc::sync_channel(1);
        let (group_tx, _) = mpsc::sync_channel(1);
        let (prefetch_tx, _) = mpsc::sync_channel(1);
        State {
            upstreams: sources.into_iter().map(Upstream::new).collect(),
            cache_dir,
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .build(),
            ),
            inflight: Mutex::new(HashMap::new()),
            missing: Mutex::new(HashMap::new()),
            refreshing: Mutex::new(HashSet::new()),
            refresh_tx,
            group_tx,
            prefetch_tx,
            prefetched_groups: Mutex::new(HashSet::new()),
            group_index: Mutex::new(None),
            seq: AtomicU64::new(0),
            requests: AtomicU64::new(0),
        }
    }

    /// Each request goes to the upstream the proxy has measured as fastest.
    /// The claim is checked against the proxy's own estimates, not against a
    /// fixed split of the requests: this used to assert "1 to the slow store,
    /// 11 to the fast one", which holds only while the fast store's measured
    /// time stays under the slow one's 60 ms. With the whole suite running, a
    /// first sample of the loopback store can take longer than that (connect
    /// plus a descheduled thread), and the proxy then correctly prefers the
    /// other store until the next exploration.
    #[test]
    fn requests_move_to_the_faster_upstream() {
        const SLOW: Duration = Duration::from_millis(60);
        let objects: Vec<(String, Vec<u8>)> = (0..12)
            .map(|i| (format!("/v1/o/{}.jrdc", key(i)), vec![i as u8; 8]))
            .collect();
        let (slow, slow_hits) = fake_store(objects.clone(), SLOW);
        let (fast, fast_hits) = fake_store(objects, Duration::ZERO);
        let tmp = test_dir("rank");
        let state = state_for(
            vec![
                Source {
                    base: slow,
                    authoritative: true,
                },
                Source {
                    base: fast,
                    authoritative: true,
                },
            ],
            tmp.clone(),
        );
        let served = || {
            [
                slow_hits.load(Ordering::Relaxed),
                fast_hits.load(Ordering::Relaxed),
            ]
        };
        let estimate = |i: usize| state.upstreams[i].ttfb_us.load(Ordering::Relaxed);
        let mut routed = Vec::new();
        for i in 0..12 {
            let rel = format!("o/{}.jrdc", key(i));
            let before = served();
            let estimates = [estimate(0), estimate(1)];
            let FetchResult::Hit(path) = fetch_cached(&state, &rel, validate_rel(&rel).unwrap())
            else {
                panic!("object {i} was not served");
            };
            assert_eq!(std::fs::read(&path).unwrap(), vec![i as u8; 8]);
            let after = served();
            let to = match [after[0] - before[0], after[1] - before[1]] {
                [1, 0] => 0,
                [0, 1] => 1,
                other => panic!("request {i} reached the stores {other:?} times"),
            };
            match i {
                // Unmeasured upstreams are measured first, one request each.
                0 => assert_eq!(to, 0),
                1 => assert_eq!(to, 1),
                // From then on, the one measured faster when the request left
                // (a tie goes to the first, as in `upstream_order`).
                _ => assert_eq!(
                    to,
                    if estimates[0] <= estimates[1] { 0 } else { 1 },
                    "request {i} with estimates {estimates:?} us, routed so far {routed:?}"
                ),
            }
            routed.push(to);
        }
        // The slow store cannot have been measured faster than it answers.
        assert!(estimate(0) >= SLOW.as_micros() as u64, "{}", estimate(0));
        assert!(estimate(1) > 0);
        // The proxy's own counts agree with what the stores saw.
        let [slow_n, fast_n] = served();
        assert_eq!(slow_n + fast_n, 12);
        assert_eq!(state.upstreams[0].hits.load(Ordering::Relaxed), slow_n);
        assert_eq!(state.upstreams[1].hits.load(Ordering::Relaxed), fast_n);
        let stats = std::fs::read_to_string(tmp.join("upstreams")).unwrap();
        assert_eq!(stats.lines().count(), 2, "{stats}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_lagging_replica_is_rechecked_then_set_aside() {
        let objects: Vec<(String, Vec<u8>)> = (0..8)
            .map(|i| (format!("/v1/m/{}.jsonl", key(i)), b"{}\n".to_vec()))
            .collect();
        // Isolate manifest routing from unrelated group prefetch and index traffic.
        let (store, store_hits) = fake_store(objects, Duration::ZERO);
        let (replica, replica_hits) = fake_store(Vec::new(), Duration::ZERO);
        let tmp = test_dir("replica");
        let state = state_for(
            vec![
                Source {
                    base: store,
                    authoritative: true,
                },
                Source {
                    base: replica,
                    authoritative: false,
                },
            ],
            tmp.clone(),
        );
        for i in 0..8 {
            // Fix the latency ranking, not scheduler-dependent loopback timings.
            state.upstreams[0].ttfb_us.store(100, Ordering::Relaxed);
            state.upstreams[1].ttfb_us.store(1, Ordering::Relaxed);
            let rel = format!("m/{}.jsonl", key(i));
            // The replica's "missing" never reaches the build.
            assert!(matches!(
                fetch_cached(&state, &rel, validate_rel(&rel).unwrap()),
                FetchResult::Hit(_)
            ));
        }
        assert_eq!(store_hits.load(Ordering::Relaxed), 8);
        // Asked until it was wrong three times in a row, then left alone.
        assert_eq!(replica_hits.load(Ordering::Relaxed), UPSTREAM_STRIKES);

        // Missing in the store is final, and is remembered.
        let gone = format!("m/{}.jsonl", key(99));
        assert!(matches!(
            fetch_cached(&state, &gone, validate_rel(&gone).unwrap()),
            FetchResult::Missing
        ));
        assert!(matches!(
            fetch_cached(&state, &gone, validate_rel(&gone).unwrap()),
            FetchResult::Missing
        ));
        assert_eq!(store_hits.load(Ordering::Relaxed), 9);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn an_unreachable_upstream_falls_back_to_the_next() {
        let k = key(7);
        let (store, _) = fake_store(
            vec![(format!("/v1/o/{k}.jrdc"), b"bundle".to_vec())],
            Duration::ZERO,
        );
        // A port nothing listens on.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let tmp = test_dir("dead");
        let base = start_with(
            vec![
                Source {
                    base: dead,
                    authoritative: true,
                },
                Source {
                    base: store,
                    authoritative: true,
                },
            ],
            tmp.clone(),
        )
        .unwrap();
        let dst = tmp.join("out");
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("o/{k}.jrdc"), &dst, 5, 1024),
            depcache_prebuilt::Got::Ok
        ));
        assert_eq!(std::fs::read(&dst).unwrap(), b"bundle");
        // No authoritative source: refuse to start rather than report
        // replication lag as "not in the store".
        assert!(
            start_with(
                vec![Source {
                    base: "http://127.0.0.1:1".into(),
                    authoritative: false,
                }],
                tmp.clone(),
            )
            .is_none()
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn proxy_caches_and_serves_immutable_bundles() {
        let upstream_hits = Arc::new(AtomicU64::new(0));
        let hits = Arc::clone(&upstream_hits);
        let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let key = "0123456789abcdef0123456789abcdef";
        let body = b"hello-prebuilt-bundle".to_vec();
        let body_clone = body.clone();
        std::thread::spawn(move || {
            for conn in upstream_listener.incoming().flatten() {
                let mut stream = conn;
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = r.read_line(&mut line);
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
                        break;
                    }
                }
                hits.fetch_add(1, Ordering::Relaxed);
                if line.contains(&format!("/v1/o/{key}.jrdc")) {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body_clone.len()
                    );
                    let _ = stream.write_all(&body_clone);
                } else {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                }
                let _ = stream.flush();
            }
        });

        let tmp = std::env::temp_dir().join(format!(
            "justrust-prebuilt-proxy-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let base = start(format!("http://{upstream_addr}"), tmp.clone()).unwrap();

        let dst1 = tmp.join("out1");
        let dst2 = tmp.join("out2");
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("o/{key}.jrdc"), &dst1, 5, 1024),
            depcache_prebuilt::Got::Ok
        ));
        assert_eq!(std::fs::read(&dst1).unwrap(), body);
        assert_eq!(upstream_hits.load(Ordering::Relaxed), 1);

        // Second request must be served from disk without hitting upstream.
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("o/{key}.jrdc"), &dst2, 5, 1024),
            depcache_prebuilt::Got::Ok
        ));
        assert_eq!(std::fs::read(&dst2).unwrap(), body);
        assert_eq!(upstream_hits.load(Ordering::Relaxed), 1);

        // Missing key returns Missing and is negatively cached.
        let miss = "fedcba9876543210fedcba9876543210";
        let dst3 = tmp.join("out3");
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("o/{miss}.jrdc"), &dst3, 5, 1024),
            depcache_prebuilt::Got::Missing
        ));
        assert_eq!(upstream_hits.load(Ordering::Relaxed), 2);
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("o/{miss}.jrdc"), &dst3, 5, 1024),
            depcache_prebuilt::Got::Missing
        ));
        assert_eq!(upstream_hits.load(Ordering::Relaxed), 2);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn stale_manifest_serves_immediately_and_refreshes_in_background() {
        use std::time::SystemTime;

        let upstream_hits = Arc::new(AtomicU64::new(0));
        let hits = Arc::clone(&upstream_hits);
        // Upstream answers only once the test opens this gate, so "served
        // without waiting for upstream" is a fact here, not a time limit a
        // loaded machine can miss.
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let open = Arc::clone(&gate);
        let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let key = "11223344556677889900aabbccddeeff";
        let fresh_body = b"{\"out\":\"fresh\"}\n".to_vec();
        let fresh_clone = fresh_body.clone();
        std::thread::spawn(move || {
            for conn in upstream_listener.incoming().flatten() {
                let mut stream = conn;
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = r.read_line(&mut line);
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
                        break;
                    }
                }
                // A fetch that blocks on upstream cannot return before this.
                while !open.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                hits.fetch_add(1, Ordering::Relaxed);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    fresh_clone.len()
                );
                let _ = stream.write_all(&fresh_clone);
                let _ = stream.flush();
            }
        });

        let tmp = std::env::temp_dir().join(format!(
            "justrust-prebuilt-proxy-swr-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let manifest_path = tmp.join(format!("v1/m/{key}.jsonl"));
        std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        std::fs::write(&manifest_path, b"{\"out\":\"stale\"}\n").unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(&manifest_path)
            .unwrap()
            .set_modified(old)
            .unwrap();

        let base = start(format!("http://{upstream_addr}"), tmp.clone()).unwrap();
        let dst1 = tmp.join("out1");
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("m/{key}.jsonl"), &dst1, 5, 1024),
            depcache_prebuilt::Got::Ok
        ));
        // Upstream has not answered: the stale copy was served without it.
        assert_eq!(upstream_hits.load(Ordering::Relaxed), 0);
        assert_eq!(std::fs::read(&dst1).unwrap(), b"{\"out\":\"stale\"}\n");
        gate.store(true, Ordering::Relaxed);

        // Wait for the background refresh to atomically replace the cached manifest.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if std::fs::read(&manifest_path).ok().as_deref() == Some(fresh_body.as_slice()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(std::fs::read(&manifest_path).unwrap(), fresh_body);
        assert_eq!(upstream_hits.load(Ordering::Relaxed), 1);

        let dst2 = tmp.join("out2");
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("m/{key}.jsonl"), &dst2, 5, 1024),
            depcache_prebuilt::Got::Ok
        ));
        assert_eq!(std::fs::read(&dst2).unwrap(), fresh_body);
        assert_eq!(upstream_hits.load(Ordering::Relaxed), 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Write `body` as the mirror's index, last refreshed `age` ago.
    fn old_index(dir: &Path, body: &[u8], age: Duration) {
        let p = dir.join("v1/index");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - age)
            .unwrap();
    }

    #[test]
    fn index_left_by_an_earlier_session_is_refetched_before_it_is_served() {
        let new = format!("{}\n{}\n", key(1), key(2)).into_bytes();
        let (store, hits) = fake_store(vec![("/v1/index".into(), new.clone())], Duration::ZERO);
        let dir = test_dir("old-index");
        // Yesterday's index lacks the key the fleet published since.
        old_index(
            &dir,
            format!("{}\n", key(1)).as_bytes(),
            Duration::from_secs(86_400),
        );
        let base = start(store, dir.clone()).unwrap();
        let dst = dir.join("got");
        assert!(matches!(
            depcache_prebuilt::get(&base, "index", &dst, 5, 1 << 20),
            depcache_prebuilt::Got::Ok
        ));
        assert_eq!(std::fs::read(&dst).unwrap(), new);
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        let _ = std::fs::remove_dir_all(&dir);

        // Within the bound it is still served at once and refreshed behind.
        let (store, _) = fake_store(
            vec![("/v1/index".into(), new.clone())],
            Duration::from_millis(300),
        );
        let dir = test_dir("recent-index");
        let recent = format!("{}\n", key(1)).into_bytes();
        old_index(
            &dir,
            &recent,
            Duration::from_secs(INDEX_TTL_SECS as u64 + 60),
        );
        let base = start(store, dir.clone()).unwrap();
        let dst = dir.join("got");
        assert!(matches!(
            depcache_prebuilt::get(&base, "index", &dst, 5, 1 << 20),
            depcache_prebuilt::Got::Ok
        ));
        assert_eq!(std::fs::read(&dst).unwrap(), recent);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_index_is_served_when_no_upstream_answers() {
        offline_index_is_served("old-index-offline", |dir| {
            old_index(
                dir,
                &format!("{}\n", key(1)).into_bytes(),
                Duration::from_secs(86_400),
            );
        });
    }

    #[test]
    fn a_compiler_seen_for_the_first_time_refetches_a_fresh_index_once() {
        let new = format!("{}\n{}\n", key(1), key(2)).into_bytes();
        let (store, hits) = fake_store(vec![("/v1/index".into(), new.clone())], Duration::ZERO);
        let dir = test_dir("new-compiler");
        // Refreshed a minute ago, before the fleet published for jr-new.
        let before = format!("{}\n", key(1)).into_bytes();
        old_index(&dir, &before, Duration::from_secs(60));
        let base = start(store, dir.clone()).unwrap();
        let get = |name: &str| {
            let dst = dir.join(name);
            assert!(matches!(
                depcache_prebuilt::get(&base, "index", &dst, 5, 1 << 20),
                depcache_prebuilt::Got::Ok
            ));
            std::fs::read(&dst).unwrap()
        };
        // A known situation first: fresh copy, no upstream request.
        assert_eq!(get("a"), before);
        assert_eq!(hits.load(Ordering::Relaxed), 0);

        assert!(note_toolchain_in(&dir, "jr-new"));
        assert_eq!(get("b"), new);
        assert_eq!(hits.load(Ordering::Relaxed), 1);

        // The same compiler again changes nothing, and neither does a name
        // that could not be a toolchain id.
        assert!(!note_toolchain_in(&dir, "jr-new"));
        assert!(!note_toolchain_in(&dir, "../escape"));
        assert!(!note_toolchain_in(&dir, "two\nlines"));
        assert_eq!(get("c"), new);
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("toolchains-seen")).unwrap(),
            "jr-new\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_aged_for_a_new_compiler_is_served_when_no_upstream_answers() {
        offline_index_is_served("new-compiler-offline", |dir| {
            old_index(
                dir,
                &format!("{}\n", key(1)).into_bytes(),
                Duration::from_secs(60),
            );
            assert!(note_toolchain_in(dir, "jr-new"));
        });
    }

    fn offline_index_is_served(tag: &str, prepare: impl Fn(&Path)) {
        // Bound and immediately closed: connections are refused.
        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let store = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        let dir = test_dir(tag);
        let body = format!("{}\n", key(1)).into_bytes();
        prepare(&dir);
        let base = start(store, dir.clone()).unwrap();
        let dst = dir.join("got");
        assert!(matches!(
            depcache_prebuilt::get(&base, "index", &dst, 5, 1 << 20),
            depcache_prebuilt::Got::Ok
        ));
        assert_eq!(std::fs::read(&dst).unwrap(), body);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_etag_poll_refreshes_when_upstream_changes() {
        let etag = Arc::new(Mutex::new("\"v1\"".to_owned()));
        let etag_srv = Arc::clone(&etag);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let tag = etag_srv.lock().unwrap().clone();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nETag: {tag}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let dir = test_dir("etag-poll");
        let dst = dir.join("v1/index");
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::write(&dst, b"k1\n").unwrap();
        std::fs::write(dst.with_extension("etag"), "\"v1\"").unwrap();

        let (refresh_tx, refresh_rx) = mpsc::sync_channel(4);
        let (group_tx, _group_rx) = mpsc::sync_channel(4);
        let (prefetch_tx, _prefetch_rx) = mpsc::sync_channel(4);
        let state = State {
            upstreams: vec![Upstream::new(Source {
                base: format!("http://{addr}"),
                authoritative: true,
            })],
            cache_dir: dir.clone(),
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .build(),
            ),
            inflight: Mutex::new(HashMap::new()),
            missing: Mutex::new(HashMap::new()),
            refreshing: Mutex::new(HashSet::new()),
            refresh_tx,
            group_tx,
            prefetch_tx,
            prefetched_groups: Mutex::new(HashSet::new()),
            group_index: Mutex::new(None),
            seq: AtomicU64::new(0),
            requests: AtomicU64::new(0),
        };

        // Unchanged ETag: no refresh scheduled.
        assert!(!poll_index_once(&state));
        assert!(refresh_rx.try_recv().is_err());

        // New ETag from a later checkpoint: refresh is scheduled once.
        *etag.lock().unwrap() = "\"v2\"".to_owned();
        assert!(poll_index_once(&state));
        let (rel, _, _) = refresh_rx.try_recv().expect("refresh scheduled");
        assert_eq!(rel, "index");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn group_prefetch_warms_peer_units_on_first_manifest_miss() {
        let k1 = key(1);
        let k2 = key(2);
        let o1 = key(11);
        let o2 = key(12);
        let gid = "0123456789abcdef";
        let (store, _hits) = fake_store(
            vec![
                (
                    format!("/v1/m/{k1}.jsonl"),
                    format!("{{\"out\":\"{o1}\"}}\n").into_bytes(),
                ),
                (
                    format!("/v1/m/{k2}.jsonl"),
                    format!("{{\"out\":\"{o2}\"}}\n").into_bytes(),
                ),
                (format!("/v1/o/{o1}.jrdc"), b"bundle-one".to_vec()),
                (format!("/v1/o/{o2}.jrdc"), b"bundle-two".to_vec()),
                (
                    "/v1/g/index".into(),
                    format!("{k1} {gid}\n{k2} {gid}\n").into_bytes(),
                ),
                (
                    format!("/v1/g/{gid}.txt"),
                    format!("{k1} {o1}\n{k2} {o2}\n").into_bytes(),
                ),
            ],
            Duration::ZERO,
        );
        let dir = test_dir("group-prefetch");
        let base = start(store, dir.clone()).unwrap();
        let dst = dir.join("got-m1");
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("m/{k1}.jsonl"), &dst, 5, 1 << 20),
            depcache_prebuilt::Got::Ok
        ));
        // Wait briefly for the background prefetch pool to warm k2 and o1/o2 on NVMe.
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if dir.join(format!("v1/m/{k2}.jsonl")).is_file()
                && dir.join(format!("v1/o/{o1}.jrdc")).is_file()
                && dir.join(format!("v1/o/{o2}.jrdc")).is_file()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(dir.join(format!("v1/m/{k2}.jsonl")).is_file());
        assert!(dir.join(format!("v1/o/{o1}.jrdc")).is_file());
        assert!(dir.join(format!("v1/o/{o2}.jrdc")).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
