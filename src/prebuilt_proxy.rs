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
    upstream: String,
    cache_dir: PathBuf,
    agent: ureq::Agent,
    inflight: Mutex<HashMap<String, InflightGate>>,
    missing: Mutex<HashMap<String, Instant>>,
    refreshing: Mutex<HashSet<String>>,
    refresh_tx: mpsc::SyncSender<(String, Kind, PathBuf)>,
    seq: AtomicU64,
}

/// Start the loopback read-only prebuilt proxy. Returns its base URL
/// (`http://127.0.0.1:<port>`) or `None` if binding fails.
pub fn start(upstream: String, cache_dir: PathBuf) -> Option<String> {
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
        upstream: upstream.trim_end_matches('/').to_owned(),
        cache_dir,
        agent,
        inflight: Mutex::new(HashMap::new()),
        missing: Mutex::new(HashMap::new()),
        refreshing: Mutex::new(HashSet::new()),
        refresh_tx,
        seq: AtomicU64::new(0),
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

fn fetch_upstream(state: &State, rel: &str, kind: Kind, dst: &Path) -> FetchResult {
    let max_bytes = match kind {
        Kind::Mutable { max_bytes, .. } | Kind::Immutable { max_bytes } => max_bytes,
    };
    let url = format!("{}/v1/{rel}", state.upstream);
    let resp = match state.agent.get(&url).call() {
        Ok(r) => r,
        Err(_) => return FetchResult::Error,
    };
    let status = resp.status().as_u16();
    if status == 404 || status == 403 {
        note_missing(state, rel);
        return FetchResult::Missing;
    }
    if status != 200 {
        return FetchResult::Error;
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
        return FetchResult::Error;
    }
    if std::fs::rename(&tmp, dst).is_err() {
        let _ = std::fs::remove_file(&tmp);
        if !dst.is_file() {
            return FetchResult::Error;
        }
    }
    FetchResult::Hit(dst.to_path_buf())
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
