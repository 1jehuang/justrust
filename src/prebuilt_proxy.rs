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
const INDEX_TTL_SECS: f64 = 600.0;
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
    if rel == "index" || rel == "costs" {
        return Some(Kind::Mutable {
            max_bytes: MAX_INDEX_BYTES,
            ttl_secs: INDEX_TTL_SECS,
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

type InflightGate = Arc<(Mutex<bool>, Condvar)>;

struct State {
    upstreams: Vec<Upstream>,
    cache_dir: PathBuf,
    agent: ureq::Agent,
    inflight: Mutex<HashMap<String, InflightGate>>,
    missing: Mutex<HashMap<String, Instant>>,
    refreshing: Mutex<HashSet<String>>,
    refresh_tx: mpsc::SyncSender<(String, Kind, PathBuf)>,
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

    let config = ureq::Agent::config_builder()
        .max_idle_connections(WORKER_THREADS)
        .max_idle_connections_per_host(WORKER_THREADS)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);

    let (refresh_tx, refresh_rx) = mpsc::sync_channel::<(String, Kind, PathBuf)>(REFRESH_QUEUE_CAP);
    let state = Arc::new(State {
        upstreams: sources.into_iter().map(Upstream::new).collect(),
        cache_dir,
        agent,
        inflight: Mutex::new(HashMap::new()),
        missing: Mutex::new(HashMap::new()),
        refreshing: Mutex::new(HashSet::new()),
        refresh_tx,
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
    if matches!(kind, Kind::Mutable { .. }) && dst.is_file() {
        schedule_refresh(state, rel, kind, &dst);
        return FetchResult::Hit(dst);
    }
    if is_neg_cached(state, rel) {
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
        return FetchResult::Error;
    }

    let result = fetch_upstream(state, rel, kind, &dst);

    {
        let mut map = state.inflight.lock().unwrap();
        map.remove(rel);
    }
    {
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }

    result
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
    up.note_ok(ttfb);
    One::Hit
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

    #[test]
    fn requests_move_to_the_faster_upstream() {
        let objects: Vec<(String, Vec<u8>)> = (0..12)
            .map(|i| (format!("/v1/o/{}.jrdc", key(i)), vec![i as u8; 8]))
            .collect();
        let (slow, slow_hits) = fake_store(objects.clone(), Duration::from_millis(60));
        let (fast, fast_hits) = fake_store(objects, Duration::ZERO);
        let tmp = test_dir("rank");
        let base = start_with(
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
        )
        .unwrap();
        for i in 0..12 {
            let dst = tmp.join(format!("out{i}"));
            assert!(matches!(
                depcache_prebuilt::get(&base, &format!("o/{}.jrdc", key(i)), &dst, 5, 1024),
                depcache_prebuilt::Got::Ok
            ));
            assert_eq!(std::fs::read(&dst).unwrap(), vec![i as u8; 8]);
        }
        // One request to measure the slow one, the rest to the fast one.
        assert_eq!(slow_hits.load(Ordering::Relaxed), 1);
        assert_eq!(fast_hits.load(Ordering::Relaxed), 11);
        let stats = std::fs::read_to_string(tmp.join("upstreams")).unwrap();
        assert_eq!(stats.lines().count(), 2, "{stats}");
        assert!(stats.contains("hits=1 "), "{stats}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_lagging_replica_is_rechecked_then_set_aside() {
        let objects: Vec<(String, Vec<u8>)> = (0..8)
            .map(|i| (format!("/v1/m/{}.jsonl", key(i)), b"{}\n".to_vec()))
            .collect();
        // The store answers slowly, the replica at once but has nothing.
        let (store, store_hits) = fake_store(objects, Duration::from_millis(30));
        let (replica, replica_hits) = fake_store(Vec::new(), Duration::ZERO);
        let tmp = test_dir("replica");
        let base = start_with(
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
        )
        .unwrap();
        for i in 0..8 {
            let dst = tmp.join(format!("out{i}"));
            // The replica's "missing" never reaches the build.
            assert!(matches!(
                depcache_prebuilt::get(&base, &format!("m/{}.jsonl", key(i)), &dst, 5, 1024),
                depcache_prebuilt::Got::Ok
            ));
        }
        assert_eq!(store_hits.load(Ordering::Relaxed), 8);
        // Asked until it was wrong three times in a row, then left alone.
        assert_eq!(replica_hits.load(Ordering::Relaxed), UPSTREAM_STRIKES);

        // Missing in the store is final, and is remembered.
        let gone = format!("m/{}.jsonl", key(99));
        let dst = tmp.join("gone");
        assert!(matches!(
            depcache_prebuilt::get(&base, &gone, &dst, 5, 1024),
            depcache_prebuilt::Got::Missing
        ));
        assert!(matches!(
            depcache_prebuilt::get(&base, &gone, &dst, 5, 1024),
            depcache_prebuilt::Got::Missing
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
                // Delay upstream so a blocking fetch would fail the latency assertion.
                std::thread::sleep(Duration::from_millis(120));
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
        let t0 = Instant::now();
        assert!(matches!(
            depcache_prebuilt::get(&base, &format!("m/{key}.jsonl"), &dst1, 5, 1024),
            depcache_prebuilt::Got::Ok
        ));
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_millis(80),
            "expected immediate stale serve, took {elapsed:?}"
        );
        assert_eq!(std::fs::read(&dst1).unwrap(), b"{\"out\":\"stale\"}\n");

        // Wait for the background refresh to atomically replace the cached manifest.
        let deadline = Instant::now() + Duration::from_secs(2);
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
}
