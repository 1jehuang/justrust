//! Prebuilt dependency store: a static, read-only, anonymous tier of the
//! dependency cache, filled by justrust's prebuild fleet with popular
//! crates.io dependencies compiled by the pinned toolchain.
//!
//! Unlike the account cache (`depcache_remote.rs`), the store needs no
//! server logic and no sign-in: it is plain files under one base URL, so an
//! S3 bucket, a CDN, `python -m http.server`, or a local directory serves
//! it. Cold builds on any machine download instead of compiling, because
//! the pinned toolchain and the portable key scheme (cargo home and target
//! dir remapped, scheme 6) give every machine the same keys.
//!
//! ```text
//! <base>/v1/index              base keys present, one per line (optional)
//! <base>/v1/m/<base>.jsonl     manifest lines (depcache `Entry` JSON)
//! <base>/v1/o/<out>.jrdc       bundle (`depcache_remote::pack` format)
//! ```
//!
//! `<base>` is `JUSTRUST_PREBUILT` (`https://...`, `http://...`, `file:///dir`
//! or an absolute path), or `default`/unset for justrust's public store
//! ([`DEFAULT_STORE`], CloudFront + S3). `0`/`off` means off. Lookups happen after a
//! local miss and before the account cache. The index, when the store has
//! one, is fetched once per 10 minutes and answers most misses without a
//! request: a cold build of 900 units makes one request per unit the store
//! actually has. A missing manifest (404) is a plain miss. Failed requests
//! are retried once, and five in a row trip a 10-minute breaker, so a dead
//! store costs a few timeouts, not one per unit. Every downloaded bundle is
//! verified (`unpack`: names, file list,
//! content hash of every binary output) before it enters the local cache.
//!
//! Trust: the store holds only what our fleet compiled from crates.io
//! sources (`justrust cache --publish`), served over HTTPS. Users never
//! write it.
//!
//! Publish: `justrust cache --publish <dir>` copies every rustc entry of
//! the local cache into a store directory (idempotent, append-only), which
//! the fleet then syncs to the bucket.

use crate::depcache::{self, Entry, Plan};
use crate::depcache_remote::{PUBLIC_MAX_BUNDLE, pack_public as pack, unpack_public as unpack};
use crate::paths;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) const BREAKER_SECS: f64 = 600.0;
const INDEX_SECS: f64 = 600.0;
const MAX_INDEX: u64 = 64 * 1024 * 1024;
/// Consecutive failed requests (across this machine's shims) before the
/// store is paused for `BREAKER_SECS`. One timeout in a cold build, where
/// dozens of shims connect at once, used to pause it for the whole build
/// (18 of 313 units fetched from a healthy S3 store).
const BREAKER_TRIPS: usize = 5;

/// Record a failed store request; pause the store once `BREAKER_TRIPS`
/// failures arrived without a success in between.
pub(crate) fn note_failure(root: &Path, why: &str) {
    let strikes = root.join("prebuilt.strikes");
    let _ = paths::append_line(&strikes, why);
    let n = std::fs::read_to_string(&strikes)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    if n >= BREAKER_TRIPS {
        let _ = std::fs::write(breaker(root), format!("{n} failures in a row, last: {why}"));
        let _ = std::fs::remove_file(&strikes);
    }
}

/// A request succeeded: forget earlier failures.
pub(crate) fn note_success(root: &Path) {
    let strikes = root.join("prebuilt.strikes");
    if strikes.exists() {
        let _ = std::fs::remove_file(strikes);
    }
}

/// justrust's public prebuilt store: CloudFront in front of the S3 bucket
/// (justrust-cloud `infra/prebuilt-store.yaml`, filled daily by the prebuild
/// fleet). Used when `JUSTRUST_PREBUILT` is unset; `off` disables it.
/// Only units whose download beats compiling them here are fetched
/// (`worth_fetching`): fetching everything was 2-3x slower than compiling
/// on a quiet laptop, fetching selectively 19-26 s vs 34-35 s (FINDINGS 43).
pub const DEFAULT_STORE: &str = "https://d1j555vqgf2xkj.cloudfront.net";

/// The bucket behind [`DEFAULT_STORE`] (us-east-1, `StoreUrl` of
/// justrust-cloud `infra/prebuilt-store.yaml`): the same public, read-only
/// objects without the CDN. A machine that keeps connections open (the
/// remote agent's proxy) reads a key the CDN edge does not hold faster from
/// here, because the edge has to come here first: 0.10 s against 0.27 s per
/// manifest from a us-west-2 build host.
#[cfg_attr(not(unix), allow(dead_code))]
pub const DEFAULT_STORE_ORIGIN: &str =
    "https://jcode-justrust-prebuilt-production-280196252242.s3.us-east-1.amazonaws.com";

/// Read replicas of that bucket in other regions (justrust-cloud
/// `infra/prebuilt-store-replica.yaml`), filled by S3 replication, so they
/// can lag it: a key missing in one is asked for again from the store. From
/// a build host in the same region a request takes about 4 ms.
#[cfg_attr(not(unix), allow(dead_code))]
pub const DEFAULT_STORE_REPLICAS: &[&str] = &[
    "https://jcode-justrust-prebuilt-production-280196252242-us-west-2.s3.us-west-2.amazonaws.com",
];

/// Every place the remote agent's proxy can read the store at `base` from,
/// which it ranks by measured latency (`prebuilt_proxy`). The public store
/// adds its bucket and replicas. `JUSTRUST_PREBUILT_REPLICAS` (https URLs
/// separated by spaces or commas) names replicas of any store.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn proxy_sources(base: &str) -> Vec<crate::prebuilt_proxy::Source> {
    proxy_sources_with(
        base,
        std::env::var("JUSTRUST_PREBUILT_REPLICAS").ok().as_deref(),
    )
}

#[cfg_attr(not(unix), allow(dead_code))]
fn proxy_sources_with(base: &str, replicas: Option<&str>) -> Vec<crate::prebuilt_proxy::Source> {
    use crate::prebuilt_proxy::Source;
    let base = base.trim_end_matches('/');
    let mut v = vec![Source {
        base: base.to_owned(),
        authoritative: true,
    }];
    let replica = |url: &str, v: &mut Vec<Source>| {
        let url = url.trim().trim_end_matches('/');
        if url.starts_with("https://") && v.iter().all(|s| s.base != url) {
            v.push(Source {
                base: url.to_owned(),
                authoritative: false,
            });
        }
    };
    if base == DEFAULT_STORE {
        v.push(Source {
            base: DEFAULT_STORE_ORIGIN.to_owned(),
            authoritative: true,
        });
        for r in DEFAULT_STORE_REPLICAS {
            replica(r, &mut v);
        }
    }
    for r in replicas
        .unwrap_or_default()
        .split([' ', ',', '\n', '\t'])
        .filter(|r| !r.is_empty())
    {
        replica(r, &mut v);
    }
    v
}

/// The toolchain the public store is compiled with (justrust-cloud
/// `prebuild/run.sh` default). Keys include rustc `-vV`, so only builds
/// with a toolchain the fleet compiles for can hit the store. Remote build
/// machines fall back to it when [`HOSTED_TOOLCHAIN`] cannot serve a build
/// (`remote_agent::store_toolchain`).
#[cfg_attr(windows, allow(dead_code))] // the remote agent runs on Linux and macOS only
pub const STORE_TOOLCHAIN: &str = "nightly-2026-10-08";

/// justrust's own compiler (jr-rustc with jr-cargo), the default on remote
/// build machines for projects that pin nothing. It must be published on
/// the toolchain store (`v1/toolchains/<host>/<id>/manifest.json`) and be
/// in the prebuild fleet's matrix (justrust-cloud
/// `infra/prebuild-fleet.yaml`, `PrebuildToolchains`), or hosted builds
/// lose every prebuilt hit.
#[cfg_attr(windows, allow(dead_code))]
pub const HOSTED_TOOLCHAIN: &str = "jr-ac20de5e7921";

/// The store base, without a trailing slash, or `None` when off.
pub fn base() -> Option<String> {
    if !depcache::enabled() {
        return None;
    }
    let Ok(v) = std::env::var("JUSTRUST_PREBUILT") else {
        return Some(DEFAULT_STORE.to_owned());
    };
    if matches!(v.trim(), "default" | "1" | "on" | "public") {
        return Some(DEFAULT_STORE.to_owned());
    }
    parse_base(&v)
}

fn parse_base(v: &str) -> Option<String> {
    let v = v.trim().trim_end_matches('/');
    if v.is_empty() || matches!(v, "0" | "off" | "false" | "no") {
        return None;
    }
    if v.starts_with("https://") || v.starts_with("http://") || v.starts_with("file:///") {
        return Some(v.to_owned());
    }
    v.starts_with('/').then(|| format!("file://{v}"))
}

pub(crate) enum Got {
    Ok,
    Missing,
    Failed(String),
}

/// Fetch `<base>/v1/<rel>` into `dst`.
pub(crate) fn get(base: &str, rel: &str, dst: &Path, max_time: u32, max_bytes: u64) -> Got {
    match get_once(base, rel, dst, max_time, max_bytes) {
        // A connection that did not finish (code 0) or a 5xx: once more.
        Got::Failed(_) if !base.starts_with("file://") => {
            get_once(base, rel, dst, max_time, max_bytes)
        }
        g => g,
    }
}

fn get_once(base: &str, rel: &str, dst: &Path, max_time: u32, max_bytes: u64) -> Got {
    if let Some(dir) = base.strip_prefix("file://") {
        let src = Path::new(dir).join("v1").join(rel);
        return match std::fs::metadata(&src) {
            Ok(m) if m.len() > max_bytes => Got::Failed("too large".into()),
            Ok(_) => match depcache::reflink_or_copy(&src, dst) {
                true => Got::Ok,
                false => Got::Failed(format!("copy {}", src.display())),
            },
            Err(_) => Got::Missing,
        };
    }
    if let Some(addr) = base.strip_prefix("http://127.0.0.1:")
        && !addr.contains('/')
    {
        return get_loopback(&format!("127.0.0.1:{addr}"), rel, dst, max_time, max_bytes);
    }
    let out = Command::new("curl")
        .args([
            "-sS",
            "--proto",
            "=https,http",
            "--max-redirs",
            "2",
            "--connect-timeout",
            "10",
        ])
        .args(["--max-time", &max_time.to_string()])
        .args(["--max-filesize", &max_bytes.to_string()])
        .arg("-o")
        .arg(dst)
        .args(["-w", "%{http_code}"])
        .arg(format!("{base}/v1/{rel}"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let code: u16 = out
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0);
    match code {
        200 => Got::Ok,
        // S3 answers 403 for a missing key when listing is not public.
        404 | 403 => {
            let _ = std::fs::remove_file(dst);
            Got::Missing
        }
        c => {
            let _ = std::fs::remove_file(dst);
            Got::Failed(format!("HTTP {c}"))
        }
    }
}

/// Direct HTTP/1.1 GET against the local loopback prebuilt proxy, avoiding
/// per-request `curl` fork+exec overhead (~0.2ms vs ~15ms).
fn get_loopback(addr: &str, rel: &str, dst: &Path, max_time: u32, max_bytes: u64) -> Got {
    use std::io::{BufRead, BufReader, Read};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let Ok(sock_addr) = addr.parse::<SocketAddr>() else {
        return Got::Failed("invalid loopback addr".into());
    };
    let timeout = Duration::from_secs(u64::from(max_time.max(1)));
    let Ok(mut stream) = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5)) else {
        return Got::Failed("connect failed".into());
    };
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let req = format!("GET /v1/{rel} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).is_err() || stream.flush().is_err() {
        return Got::Failed("write failed".into());
    }
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    if reader.read_line(&mut status_line).unwrap_or(0) == 0 {
        return Got::Failed("empty response".into());
    }
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut content_len: Option<u64> = None;
    loop {
        let mut hdr = String::new();
        if reader.read_line(&mut hdr).unwrap_or(0) == 0 || hdr == "\r\n" || hdr == "\n" {
            break;
        }
        if let Some((k, v)) = hdr.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            content_len = v.trim().parse().ok();
        }
    }
    match code {
        200 => {
            if content_len.is_some_and(|n| n > max_bytes) {
                return Got::Failed("too large".into());
            }
            let Ok(mut f) = std::fs::File::create(dst) else {
                return Got::Failed("create dst failed".into());
            };
            let mut limited = reader.take(max_bytes + 1);
            match std::io::copy(&mut limited, &mut f) {
                Ok(n) if n <= max_bytes && content_len.is_none_or(|want| n == want) => Got::Ok,
                Ok(_) => {
                    let _ = std::fs::remove_file(dst);
                    Got::Failed("truncated or too large".into())
                }
                Err(_) => {
                    let _ = std::fs::remove_file(dst);
                    Got::Failed("read body failed".into())
                }
            }
        }
        404 | 403 => Got::Missing,
        c => Got::Failed(format!("HTTP {c}")),
    }
}

pub(crate) fn breaker(root: &Path) -> PathBuf {
    root.join("prebuilt.down")
}

pub(crate) fn fresh_file(p: &Path, secs: f64) -> bool {
    std::fs::metadata(p)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|e| e.as_secs_f64() < secs)
}

pub(crate) fn tmp(root: &Path, tag: &str) -> PathBuf {
    let d = root.join("tmp");
    let _ = std::fs::create_dir_all(&d);
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    d.join(format!(
        "pb-{tag}.{}.{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

/// Names the store behind `JUSTRUST_PREBUILT` when that is a loopback proxy
/// (`http://127.0.0.1:<port>`, what the remote agent gives its builds).
pub(crate) const STORE_ENV: &str = "JUSTRUST_PREBUILT_STORE";

/// Local copy of the store's index, one hash of the store's URL per store.
///
/// Behind the agent's proxy the base URL is `http://127.0.0.1:<port>` with a
/// port the kernel picked for that agent, so naming the file after the base
/// gave every session its own index of the same store: its first build
/// fetched it again, and the old files were never read again. The agent
/// names the store it proxies in [`STORE_ENV`], and the file is named after
/// that. One index covers every toolchain (the keys in it differ), so the
/// toolchain is not part of the name. A different store is a different name.
fn index_path(root: &Path, base: &str) -> PathBuf {
    index_path_for(root, base, std::env::var(STORE_ENV).ok().as_deref())
}

fn index_path_for(root: &Path, base: &str, store: Option<&str>) -> PathBuf {
    let mut h = depcache::H128::new();
    h.field(index_identity(base, store).as_bytes());
    root.join(format!("prebuilt-index-{}", &h.hex()[..12]))
}

/// Remove other stores' index copies nobody has refreshed for a day.
///
/// An index is used for [`INDEX_SECS`] and then fetched again, so one this
/// old is only disk: a build host had 50 of them (206 MB), one per agent
/// session, from when they were named after the proxy's port. Runs after a
/// refresh, so at most once per [`INDEX_SECS`] per store. Lock files (empty)
/// stay: a lock must keep its inode while anyone may hold it.
fn sweep_old_indexes(root: &Path, keep: &Path) {
    const OLD_SECS: f64 = 24.0 * 3600.0;
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let Some(hex) = name
            .to_str()
            .and_then(|n| n.strip_prefix("prebuilt-index-"))
        else {
            continue;
        };
        let is_index = hex.len() == 12 && hex.bytes().all(|b| b.is_ascii_hexdigit());
        if is_index && p != keep && !fresh_file(&p, OLD_SECS) {
            let _ = std::fs::remove_file(&p);
            let _ = std::fs::remove_file(p.with_extension("costs"));
            let _ = std::fs::remove_file(p.with_extension("none"));
        }
    }
}

/// What an index file is named after: the proxied store for a loopback base
/// that names one, else the base itself. A store name next to a real store
/// URL is ignored, so it cannot point one store's lookups at another's index.
fn index_identity<'a>(base: &'a str, store: Option<&'a str>) -> &'a str {
    let loopback = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .any(|p| base.starts_with(p));
    match store.map(|s| s.trim().trim_end_matches('/')) {
        Some(s) if loopback && !s.is_empty() => s,
        _ => base,
    }
}

/// Whether downloading the unit should beat compiling it here, from the
/// store's `v1/costs` (fleet compile time, bundle size) and this machine's
/// measured download rate. `true` when the store has no costs file or the
/// key is not priced (old behavior: always fetch). Measured on an idle
/// 16-core laptop at ~10 MB/s: median fetch 1.2 s vs median compile 0.18 s,
/// and fetching all 615 hits made a cold check 2-3x slower (FINDINGS 43).
fn worth_fetching(root: &Path, base: &str, key: &str) -> bool {
    worth_fetching_with_override(
        root,
        base,
        key,
        std::env::var("JUSTRUST_PREBUILT_ALWAYS").is_ok_and(|v| v == "1"),
    )
}

// Explicit policy input keeps cost-model tests independent of the environment
// inherited by real hosted runs, whose agent defaults the override to true.
fn worth_fetching_with_override(root: &Path, base: &str, key: &str, always: bool) -> bool {
    if always {
        return true;
    }
    let Some((compile_ms, bytes)) = cost_of(root, base, key) else {
        // Unpriced: a private store (`JUSTRUST_PREBUILT=<url>`) is fetched as
        // before; the public default store is not, since fetching everything
        // is what made it slower than compiling (FINDINGS 43).
        return base != DEFAULT_STORE;
    };
    let rate = download_rate(root);
    // Per-unit fixed cost: two requests (manifest, bundle), measured on this
    // machine, plus curl and unpack.
    let fetch_ms = fetch_overhead_ms(root) + bytes as f64 * 1000.0 / rate;
    // The fleet machine compiles ~1.5x faster per thread than a laptop under
    // agent load; leaving that out errs toward compiling, which is safe.
    fetch_ms < compile_ms as f64
}

/// Fixed time one store hit costs beyond its transfer, until this machine has
/// measured its own request latency. 700 ms was a laptop over home Wi-Fi; a
/// build host next to CloudFront measured 20 ms per request (FINDINGS 67).
const FETCH_OVERHEAD_MS: f64 = 700.0;

fn latency_path(root: &Path) -> PathBuf {
    root.join("prebuilt.latency")
}

/// Fixed cost of one hit: two store requests at this machine's measured
/// latency (EWMA of manifest requests) plus process and unpack overhead.
fn fetch_overhead_ms(root: &Path) -> f64 {
    std::fs::read_to_string(latency_path(root))
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|l| l.is_finite() && *l > 0.0)
        .map(|l| (2.0 * l * 1000.0 + 30.0).clamp(30.0, 3000.0))
        .unwrap_or(FETCH_OVERHEAD_MS)
}

/// Fold one manifest request (small, so all latency) into the estimate.
fn note_latency(root: &Path, secs: f64) {
    if !(secs.is_finite() && secs > 0.0) {
        return;
    }
    let prev = std::fs::read_to_string(latency_path(root))
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|l| l.is_finite() && *l > 0.0);
    let l = match prev {
        Some(p) => 0.8 * p + 0.2 * secs,
        None => secs,
    };
    let _ = std::fs::write(latency_path(root), format!("{l:.4}"));
}

fn costs_path(root: &Path, base: &str) -> PathBuf {
    index_path(root, base).with_extension("costs")
}

/// `(compile ms, bundle bytes)` of `key`, from a local copy of `v1/costs`
/// refreshed every 10 minutes.
fn cost_of(root: &Path, base: &str, key: &str) -> Option<(u64, u64)> {
    let p = costs_path(root, base);
    let none = p.with_extension("costs-none");
    if fresh_file(&none, INDEX_SECS) {
        return None;
    }
    if !fresh_file(&p, INDEX_SECS) {
        let t = tmp(root, "costs");
        match get(base, "costs", &t, 20, MAX_INDEX) {
            Got::Ok => {
                let _ = std::fs::rename(&t, &p);
            }
            _ => {
                let _ = std::fs::remove_file(&t);
                let _ = std::fs::write(&none, b"");
                return None;
            }
        }
    }
    let text = std::fs::read_to_string(&p).ok()?;
    let line = cost_line(&text, key)?;
    let mut it = line.split(' ').skip(1);
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

// Search for a row boundary and prefix together. Do not assume the table is
// sorted or unique. Keep whole-file UTF-8 validation in the caller.
fn cost_line<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    if text.len() < 4096 || key.is_empty() || key.contains(['\r', '\n']) {
        return text.lines().find(|line| line.starts_with(key));
    }
    let start = if text.starts_with(key) {
        0
    } else {
        let prefix = format!("\n{key}");
        memchr::memmem::find(text.as_bytes(), prefix.as_bytes())? + 1
    };
    text[start..].lines().next()
}

fn rate_path(root: &Path) -> PathBuf {
    root.join("prebuilt.rate")
}

/// Bytes per second this machine downloads bundles at (EWMA), 5 MB/s until
/// measured.
fn download_rate(root: &Path) -> f64 {
    std::fs::read_to_string(rate_path(root))
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|r| *r > 1000.0)
        .unwrap_or(5e6)
}

/// Fold one bundle download into the rate estimate. Bundles under 256 KB
/// say more about latency than bandwidth and are skipped.
fn note_download(root: &Path, bytes: u64, secs: f64) {
    if bytes < 256 * 1024 || secs <= 0.0 {
        return;
    }
    let sample = bytes as f64 / secs;
    let r = 0.8 * download_rate(root) + 0.2 * sample;
    let _ = std::fs::write(rate_path(root), format!("{r:.0}"));
}

/// Whether the store may have `key`. `true` when it has no index (every
/// lookup asks the store) or the index cannot be read.
fn may_have(root: &Path, base: &str, key: &str) -> bool {
    let idx = index_path(root, base);
    let none = idx.with_extension("none");
    if fresh_file(&none, INDEX_SECS) {
        return true;
    }
    if !fresh_file(&idx, INDEX_SECS) {
        // Keep a stable lock inode even when the index is atomically replaced.
        // Process exit releases the OS lock. Never unlink the lock file.
        let Ok(lock) = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(idx.with_extension("refresh-lock"))
        else {
            return true;
        };
        if lock.try_lock().is_err() && !wait_for_refresh(&lock, &idx, &none) {
            // Still busy or unsupported locking: a stale negative could hide
            // a new key. Probe the manifest normally, including all validation.
            return true;
        }
        if fresh_file(&none, INDEX_SECS) {
            return true;
        }
        if fresh_file(&idx, INDEX_SECS) {
            return index_file_contains(&idx, key).unwrap_or(true);
        }
        let t = tmp(root, "idx");
        match get(base, "index", &t, 20, MAX_INDEX) {
            Got::Ok => {
                if std::fs::rename(&t, &idx).is_err() {
                    let _ = std::fs::remove_file(&t);
                    return true;
                }
                sweep_old_indexes(root, &idx);
            }
            Got::Missing => {
                let _ = std::fs::write(&none, b"");
                return true;
            }
            Got::Failed(why) => {
                let _ = std::fs::remove_file(&t);
                note_failure(root, &format!("index: {why}"));
                return false;
            }
        }
    }
    index_file_contains(&idx, key).unwrap_or(true)
}

/// How long a shim waits for another shim's index refresh before it probes
/// the store itself. One upstream manifest probe costs about this much.
const REFRESH_WAIT: std::time::Duration = std::time::Duration::from_millis(300);

/// Another shim is refreshing the index: wait briefly for it instead of
/// asking the store about this key. A build starts one shim per job at once
/// and all of them find the index stale. Only one can refresh it, and the
/// rest each paid an upstream manifest request (measured on a build host:
/// 0.75s for each of the first 13 units, 9.4s of job time, while the index
/// itself arrived from the loopback proxy in 2ms). Returns true holding the
/// lock, or with a fresh index or a fresh no-index marker in place. The
/// answer still comes from a fresh index only.
fn wait_for_refresh(lock: &std::fs::File, idx: &Path, none: &Path) -> bool {
    let t = std::time::Instant::now();
    while t.elapsed() < REFRESH_WAIT {
        std::thread::sleep(std::time::Duration::from_millis(2));
        if fresh_file(idx, INDEX_SECS) || fresh_file(none, INDEX_SECS) || lock.try_lock().is_ok() {
            return true;
        }
    }
    false
}

fn index_file_contains(path: &Path, key: &str) -> std::io::Result<bool> {
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    index_snapshot_contains(file, meta, key)
}

fn index_snapshot_contains(
    mut file: std::fs::File,
    meta: std::fs::Metadata,
    key: &str,
) -> std::io::Result<bool> {
    use std::io::Read;
    let len = meta.len();
    // Pin the descriptor across all probes: an atomic refresh cannot mix generations.
    #[cfg(unix)]
    if meta.is_file() && len > 65536 && len.is_multiple_of(33) {
        use std::os::unix::fs::FileExt;
        if key.len() != 32 {
            return Ok(false);
        }
        let (mut lo, mut hi) = (0, len / 33);
        let mut probe = [0; 32];
        while hi - lo > 256 {
            let mid = lo + (hi - lo) / 2;
            file.read_exact_at(&mut probe, mid * 33)?;
            match probe.as_slice().cmp(key.as_bytes()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(true),
            }
        }
        let mut window = [0; 256 * 33];
        let window = &mut window[..((hi - lo) * 33) as usize];
        file.read_exact_at(window, lo * 33)?;
        return Ok(index_contains(window, key) || file.metadata()?.len() != len);
    }
    // Small files, streams, non-Unix platforms and variable-width indexes keep
    // the buffered parser. Do not size the allocation from unbounded metadata.
    let mut bytes = Vec::with_capacity(len.min(MAX_INDEX) as usize);
    file.read_to_end(&mut bytes)?;
    Ok((meta.is_file() && bytes.len() as u64 != len) || index_contains(&bytes, key))
}

/// The index is sorted, so binary search over fixed-width lines.
fn index_contains(b: &[u8], key: &str) -> bool {
    let k = key.as_bytes();
    if k.len() != 32 {
        return false;
    }
    if b.len().is_multiple_of(33) {
        let (mut lo, mut hi) = (0, b.len() / 33);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match b[mid * 33..mid * 33 + 32].cmp(k) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return true,
            }
        }
        return false;
    }
    b.split(|c| *c == b'\n').any(|l| l == k)
}

/// After a local miss: find the unit in the store, verify and install it
/// into the local cache, then restore it locally. `None` on any failure.
pub fn fetch(plan: &Plan, err: &mut dyn Write) -> Option<depcache::Hit> {
    fetch_from(&base()?, plan, err)
}

fn fetch_from(base: &str, plan: &Plan, err: &mut dyn Write) -> Option<depcache::Hit> {
    let root = &plan.root;
    if fresh_file(&breaker(root), BREAKER_SECS)
        || !may_have(root, base, &plan.base)
        || !worth_fetching(root, base, &plan.base)
    {
        return None;
    }
    let m = tmp(root, "m");
    let t_m = std::time::Instant::now();
    let got = get(base, &format!("m/{}.jsonl", plan.base), &m, 8, 1 << 20);
    let m_secs = t_m.elapsed().as_secs_f64();
    let text = std::fs::read_to_string(&m).unwrap_or_default();
    let _ = std::fs::remove_file(&m);
    match got {
        Got::Ok => {
            note_success(root);
            note_latency(root, m_secs);
        }
        Got::Missing => {
            note_latency(root, m_secs);
            return None;
        }
        Got::Failed(why) => {
            note_failure(root, &format!("manifest: {why}"));
            return None;
        }
    }
    let mut memo = std::collections::HashMap::new();
    let (out, line) = text.lines().rev().find_map(|line| {
        let e: Entry = serde_json::from_str(line).ok()?;
        (valid_key(&e.out) && plan.matches(&e, &mut memo)).then(|| (e.out, line.to_owned()))
    })?;
    let final_dir = root.join("o").join(&out);
    if !final_dir.join("meta.json").is_file() {
        let bundle = tmp(root, "b");
        let stage = root
            .join("o")
            .join(format!(".{out}.pb{}", std::process::id()));
        let t0 = std::time::Instant::now();
        let ok = matches!(
            get(
                base,
                &format!("o/{out}.jrdc"),
                &bundle,
                120,
                PUBLIC_MAX_BUNDLE
            ),
            Got::Ok
        ) && {
            let n = std::fs::metadata(&bundle).map(|m| m.len()).unwrap_or(0);
            note_download(root, n, t0.elapsed().as_secs_f64());
            unpack(&bundle, &stage).is_some()
        };
        let _ = std::fs::remove_file(&bundle);
        if !ok || std::fs::rename(&stage, &final_dir).is_err() {
            let _ = std::fs::remove_dir_all(&stage);
            if !final_dir.join("meta.json").is_file() {
                return None;
            }
        }
    }
    let known = std::fs::read_to_string(plan.manifest()).unwrap_or_default();
    if !known.lines().any(|l| l == line) {
        std::fs::create_dir_all(root.join("m")).ok()?;
        paths::append_line(&plan.manifest(), &line).ok()?;
    }
    plan.restore(err)
}

pub(crate) fn valid_key(k: &str) -> bool {
    k.len() == 32
        && k.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Debug, Default, PartialEq)]
pub struct Published {
    pub bundles: usize,
    pub lines: usize,
    /// Hermetic build-script runs published (`v1/s/`).
    pub scripts: usize,
    pub skipped: usize,
    pub bytes: u64,
}

/// Copy every rustc entry of the local cache at `root` into the store
/// directory `dst` (`dst/v1/...`). Idempotent: existing bundles are kept,
/// manifest lines are appended once. Rewrites the sorted index.
pub fn publish(root: &Path, dst: &Path) -> anyhow::Result<Published> {
    let v1 = dst.join("v1");
    std::fs::create_dir_all(v1.join("m"))?;
    std::fs::create_dir_all(v1.join("o"))?;
    let mut p = Published::default();
    let mut manifests: Vec<_> = std::fs::read_dir(root.join("m"))?.flatten().collect();
    manifests.sort_by_key(|e| e.file_name());
    for m in manifests {
        let name = m.file_name().to_string_lossy().into_owned();
        if !name.strip_suffix(".jsonl").is_some_and(valid_key) {
            continue;
        }
        let text = std::fs::read_to_string(m.path()).unwrap_or_default();
        let dst_m = v1.join("m").join(&name);
        let mut have = std::fs::read_to_string(&dst_m).unwrap_or_default();
        for line in text.lines() {
            let Some(out) = serde_json::from_str::<Entry>(line)
                .ok()
                .map(|e| e.out)
                .filter(|o| valid_key(o))
            else {
                p.skipped += 1;
                continue;
            };
            if have.lines().any(|l| l == line) {
                continue;
            }
            // Build-script run entries share the cache dir and line shape
            // (their meta.json has `stdout`). Hermetic ones, keyed by the C
            // toolchain, go to `v1/s/`; the rest are host-specific.
            let o = root.join("o").join(&out);
            let is_script = serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(o.join("meta.json")).unwrap_or_default(),
            )
            .is_ok_and(|v| v.get("stdout").is_some());
            if is_script {
                match crate::buildscript::publish_entry(root, line, &v1) {
                    Some(n) => {
                        let sm = v1.join("s/m").join(&name);
                        let have_s = std::fs::read_to_string(&sm).unwrap_or_default();
                        if !have_s.lines().any(|l| l == line) {
                            std::fs::create_dir_all(v1.join("s/m"))?;
                            paths::append_line(&sm, line)?;
                            p.scripts += 1;
                            p.bytes += n;
                        }
                    }
                    None => p.skipped += 1,
                }
                continue;
            }
            let bundle = v1.join("o").join(format!("{out}.jrdc"));
            if !bundle.is_file() {
                let t = v1.join("o").join(format!(".{out}.{}", std::process::id()));
                if pack(&root.join("o").join(&out), &t).is_none() {
                    let _ = std::fs::remove_file(&t);
                    p.skipped += 1;
                    continue;
                }
                p.bytes += std::fs::metadata(&t).map(|m| m.len()).unwrap_or(0);
                std::fs::rename(&t, &bundle)?;
                p.bundles += 1;
            }
            paths::append_line(&dst_m, line)?;
            have.push_str(line);
            have.push('\n');
            p.lines += 1;
        }
    }
    write_index(&v1)?;
    write_costs(root, &v1)?;
    if v1.join("s/m").is_dir() {
        write_index(&v1.join("s"))?;
    }
    Ok(p)
}

/// `v1/costs`: one line per base key, `<key> <compile ms> <bundle bytes>`,
/// for the newest entry of each manifest, so a client can decide before any
/// request whether downloading a unit beats compiling it. Lines for keys
/// this publisher cannot price (bundles published earlier from another
/// machine) are kept from the existing file.
fn write_costs(root: &Path, v1: &Path) -> anyhow::Result<()> {
    let mut costs: std::collections::BTreeMap<String, String> =
        std::fs::read_to_string(v1.join("costs"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| Some((l.split(' ').next()?.to_owned(), l.to_owned())))
            .collect();
    for m in std::fs::read_dir(v1.join("m"))?.flatten() {
        let name = m.file_name().to_string_lossy().into_owned();
        let Some(key) = name.strip_suffix(".jsonl").filter(|k| valid_key(k)) else {
            continue;
        };
        let text = std::fs::read_to_string(m.path()).unwrap_or_default();
        let outs: Vec<String> = text
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<Entry>(l).ok().map(|e| e.out))
            .collect();
        // Size of what a client would download (newest line), compile time
        // from any line this publisher built itself (a fresh fleet machine
        // compiles everything, but its outputs need not hash the same).
        // When the newest bundle was published earlier and is only in the
        // bucket, pack our own output to measure it: same format and
        // compression, so the same size up to a few bytes.
        let packed_size = |o: &str| -> Option<u64> {
            let t = v1.join(format!(".size.{}", std::process::id()));
            let n = pack(&root.join("o").join(o), &t).and_then(|_| std::fs::metadata(&t).ok());
            let _ = std::fs::remove_file(&t);
            n.map(|m| m.len())
        };
        let Some(bytes) = outs
            .first()
            .and_then(|o| {
                std::fs::metadata(v1.join("o").join(format!("{o}.jrdc")))
                    .ok()
                    .map(|m| m.len())
            })
            .or_else(|| outs.iter().find_map(|o| packed_size(o)))
        else {
            continue;
        };
        let Some(wall) = outs.iter().find_map(|o| {
            let b = std::fs::read(root.join("o").join(o).join("meta.json")).ok()?;
            serde_json::from_slice::<serde_json::Value>(&b)
                .ok()?
                .get("wall")?
                .as_f64()
        }) else {
            continue;
        };
        let ms = (wall * 1000.0).round() as u64;
        costs.insert(key.to_owned(), format!("{key} {ms} {bytes}"));
    }
    let mut s = String::new();
    for line in costs.values() {
        s.push_str(line);
        s.push('\n');
    }
    let t = v1.join(format!(".costs.{}", std::process::id()));
    std::fs::write(&t, s)?;
    std::fs::rename(&t, v1.join("costs"))?;
    Ok(())
}

/// `v1/index`: sorted base keys that have a manifest, 33 bytes per line.
fn write_index(v1: &Path) -> anyhow::Result<()> {
    let mut keys: Vec<String> = std::fs::read_dir(v1.join("m"))?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_suffix(".jsonl")
                .filter(|b| valid_key(b))
                .map(str::to_owned)
        })
        .collect();
    keys.sort();
    let mut s = String::with_capacity(keys.len() * 33);
    for k in keys {
        s.push_str(&k);
        s.push('\n');
    }
    let t = v1.join(format!(".index.{}", std::process::id()));
    std::fs::write(&t, s)?;
    std::fs::rename(&t, v1.join("index"))?;
    Ok(())
}

/// One status line for `justrust cache`.
pub fn status_line(root: &Path) -> String {
    match base() {
        None => "prebuilt  off (JUSTRUST_PREBUILT=off or the depcache is off)".into(),
        Some(b) if fresh_file(&breaker(root), BREAKER_SECS) => format!(
            "prebuilt  {b}: paused after an error ({}), retrying within 10 min",
            std::fs::read_to_string(breaker(root))
                .unwrap_or_default()
                .trim()
        ),
        Some(b) => format!("prebuilt  {b}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_reads_the_public_store_from_its_bucket_and_replicas() {
        let v = proxy_sources_with(DEFAULT_STORE, None);
        assert_eq!(v[0].base, DEFAULT_STORE);
        assert_eq!(v[1].base, DEFAULT_STORE_ORIGIN);
        assert!(v[0].authoritative && v[1].authoritative);
        assert_eq!(v.len(), 2 + DEFAULT_STORE_REPLICAS.len());
        // A replica can lag: its "missing" is never final.
        assert!(v[2..].iter().all(|s| !s.authoritative));
        for s in &v {
            assert!(s.base.starts_with("https://") && !s.base.ends_with('/'));
        }

        // A private store is read from where the user said, plus replicas
        // they name. Nothing of the public store is mixed in.
        let v = proxy_sources_with(
            "https://store.example/",
            Some("https://r1.example/, http://plain.example https://r1.example https://r2.example"),
        );
        let bases: Vec<&str> = v.iter().map(|s| s.base.as_str()).collect();
        assert_eq!(
            bases,
            [
                "https://store.example",
                "https://r1.example",
                "https://r2.example"
            ]
        );
        assert!(v[0].authoritative && !v[1].authoritative && !v[2].authoritative);
    }

    #[test]
    fn index_file_is_named_after_the_store_not_the_proxy_port() {
        let root = Path::new("/r");
        let store = "https://prebuilt.example";
        let a = index_path_for(root, "http://127.0.0.1:40001", Some(store));
        // Another session: another port, the same store, the same file.
        assert_eq!(
            a,
            index_path_for(
                root,
                "http://127.0.0.1:51877",
                Some("https://prebuilt.example/")
            )
        );
        // And the file a direct client of that store uses.
        assert_eq!(a, index_path_for(root, store, None));
        // Another store behind the same port is another file.
        assert_ne!(
            a,
            index_path_for(
                root,
                "http://127.0.0.1:40001",
                Some("https://other.example")
            )
        );
        // A proxy that names no store keeps its own file, as before.
        let unnamed = index_path_for(root, "http://127.0.0.1:40001", None);
        assert_ne!(a, unnamed);
        assert_eq!(
            unnamed,
            index_path_for(root, "http://127.0.0.1:40001", Some(" "))
        );
        // A real store URL is never renamed by the variable.
        assert_eq!(
            index_path_for(root, "https://other.example", Some(store)),
            index_path_for(root, "https://other.example", None)
        );
        assert_eq!(
            index_path_for(root, "file:///srv/store", Some(store)),
            index_path_for(root, "file:///srv/store", None)
        );
        // Costs, the no-index marker and the refresh lock follow the index.
        assert_eq!(
            costs_path(root, store),
            index_path_for(root, store, None).with_extension("costs")
        );
    }

    #[test]
    fn only_day_old_copies_of_other_indexes_are_swept() {
        let root = std::env::temp_dir().join(format!("jr-idx-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let age = |name: &str, secs: u64| {
            let p = root.join(name);
            std::fs::write(&p, b"k\n").unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs))
                .unwrap();
            p
        };
        let day = 24 * 3600;
        let ours = age("prebuilt-index-aaaaaaaaaaaa", 2 * day);
        let old = age("prebuilt-index-bbbbbbbbbbbb", 2 * day);
        let old_costs = age("prebuilt-index-bbbbbbbbbbbb.costs", 2 * day);
        let old_lock = age("prebuilt-index-bbbbbbbbbbbb.refresh-lock", 2 * day);
        let recent = age("prebuilt-index-cccccccccccc", day / 2);
        let other = age("prebuilt-index-notanindex", 2 * day);
        sweep_old_indexes(&root, &ours);
        assert!(ours.exists(), "the index just refreshed");
        assert!(!old.exists() && !old_costs.exists());
        assert!(old_lock.exists(), "locks keep their inode");
        assert!(recent.exists(), "another store in use today");
        assert!(other.exists(), "not a file this module named");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn index_lookup_sorted_and_fallback() {
        let keys = ["0".repeat(32), "5".repeat(32), "a".repeat(32)];
        let b = keys.iter().map(|k| format!("{k}\n")).collect::<String>();
        for k in &keys {
            assert!(index_contains(b.as_bytes(), k));
        }
        assert!(!index_contains(b.as_bytes(), &"3".repeat(32)));
        assert!(!index_contains(b.as_bytes(), &"f".repeat(32)));
        assert!(!index_contains(b.as_bytes(), "short"));
        // Not fixed width: linear scan still answers.
        let odd = format!("{}\n{}", "1".repeat(32), "2".repeat(32));
        assert!(index_contains(odd.as_bytes(), &"2".repeat(32)));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_index_uses_open_snapshot_and_original_search_order() {
        let dir = std::env::temp_dir().join(format!("jr-index-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("index");
        let mut records = (0..3000)
            .map(|i| format!("{:032x}\n", i / 2))
            .collect::<String>()
            .into_bytes();
        // Malformed but divisible: the original parser ignores terminators and
        // does not repair out-of-order records. Compare against that parser.
        records[32] = b'x';
        records[33 * 7..33 * 7 + 32].fill(b'z');
        std::fs::write(&path, &records).unwrap();
        for i in 0..1503 {
            let key = format!("{i:032x}");
            assert_eq!(
                index_file_contains(&path, &key).unwrap(),
                index_contains(&records, &key)
            );
        }
        let held = std::fs::File::open(&path).unwrap();
        let meta = held.metadata().unwrap();
        let replacement = path.with_extension("new");
        std::fs::write(&replacement, &records).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let truncated = std::fs::File::open(&path).unwrap();
        let before = truncated.metadata().unwrap();
        std::fs::write(&path, []).unwrap();
        let key = format!("{:032x}", 1499);
        assert!(index_snapshot_contains(held, meta, &key).unwrap());
        assert!(index_snapshot_contains(truncated, before, &key).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn publish_then_fetch_into_an_empty_cache() {
        publish_fetch_round_trip(false, None);
    }

    #[test]
    fn public_publish_fetch_allows_bundle_above_worker_cap() {
        publish_fetch_round_trip(true, None);
    }

    #[test]
    #[ignore = "requires an existing real artifact via JUSTRUST_TEST_PUBLIC_ARTIFACT"]
    fn public_real_artifact_round_trip() {
        let artifact = PathBuf::from(
            std::env::var_os("JUSTRUST_TEST_PUBLIC_ARTIFACT").expect("real artifact path"),
        );
        publish_fetch_round_trip(false, Some(&artifact));
    }

    fn publish_fetch_round_trip(large: bool, real_artifact: Option<&Path>) {
        use crate::depcache::Norm;
        let dir = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "jr-prebuilt-rt-{large}-{}-{}",
                real_artifact.is_some(),
                std::process::id()
            ));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src.rs");
        std::fs::create_dir_all(dir.join("t1/debug/deps")).unwrap();
        std::fs::create_dir_all(dir.join("t2/debug/deps")).unwrap();
        std::fs::write(&src, "pub fn f() {}").unwrap();
        let mk = |t: &str, cache: &str| {
            let out_dir = dir.join(t).join("debug/deps");
            Plan {
                base: "b".repeat(32),
                out_dir: out_dir.clone(),
                extra: "-abc".into(),
                crate_name: "x".into(),
                cwd: dir.clone(),
                norm: Norm::new(out_dir.parent().unwrap()).unwrap(),
                root: dir.join(cache),
                extra_args: Vec::new(),
                unkeyed_env: Vec::new(),
            }
        };
        // Publisher: a compile stored in its own cache, then published.
        let p1 = mk("t1", "cache1");
        let deps1 = &p1.out_dir;
        std::fs::write(
            deps1.join("x-abc.d"),
            format!("{0}/x-abc.d: {1}\n\n{1}:\n", deps1.display(), src.display()),
        )
        .unwrap();
        let artifact_name =
            if real_artifact.is_some_and(|p| p.extension().is_some_and(|e| e == "rlib")) {
                "libx-abc.rlib"
            } else {
                "libx-abc.rmeta"
            };
        let artifact = deps1.join(artifact_name);
        if let Some(src) = real_artifact {
            std::fs::copy(src, &artifact).unwrap();
        } else if large {
            // Incompressible deterministic bytes exercise the actual compressed cap,
            // not just a tiny gzip expanding to a large zero-filled file.
            let mut file = std::fs::File::create(&artifact).unwrap();
            let mut state = 0x123456789abcdefu64;
            let mut chunk = [0u8; 64 * 1024];
            for _ in 0..1536 {
                // 96 MiB
                for bytes in chunk.as_chunks_mut::<8>().0 {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    bytes.copy_from_slice(&state.to_le_bytes());
                }
                file.write_all(&chunk).unwrap();
            }
        } else {
            std::fs::write(&artifact, b"rmeta").unwrap();
        }
        p1.store(0.0, 2500.0, &[]).unwrap();
        let store = dir.join("store");
        let first = publish(&p1.root, &store).unwrap();
        assert_eq!((first.bundles, first.lines), (1, 1));
        if real_artifact.is_some() || large {
            let bundle = std::fs::read_dir(store.join("v1/o"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            eprintln!(
                "public artifact (real={}): raw={} gzip={}",
                real_artifact.is_some(),
                std::fs::metadata(&artifact).unwrap().len(),
                std::fs::metadata(bundle).unwrap().len()
            );
        }
        if large {
            let bundle = std::fs::read_dir(store.join("v1/o"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            assert!(std::fs::metadata(&bundle).unwrap().len() > 95 << 20);
            assert!(
                crate::depcache_remote::unpack(&bundle, &dir.join("private-rejected")).is_none()
            );
            assert!(!dir.join("private-rejected").exists());
        }
        // Idempotent.
        let again = publish(&p1.root, &store).unwrap();
        assert_eq!((again.bundles, again.lines), (0, 0));
        assert_eq!(
            std::fs::read_to_string(store.join("v1/index")).unwrap(),
            format!("{}\n", "b".repeat(32))
        );

        // Consumer: empty cache, other target dir.
        let base = format!("file://{}", store.display());
        let p2 = mk("t2", "cache2");
        assert!(p2.restore(&mut Vec::new()).is_none());
        let hit = fetch_from(&base, &p2, &mut Vec::new()).expect("prebuilt hit");
        assert_eq!(hit.saved_secs, 2500.0);
        // Published with its price: compile ms and bundle bytes.
        let costs = std::fs::read_to_string(store.join("v1/costs")).unwrap();
        assert!(
            costs.starts_with(&format!("{} 2500000 ", "b".repeat(32))),
            "{costs}"
        );
        assert_eq!(
            depcache::hash_file_content(&p2.out_dir.join(artifact_name)),
            depcache::hash_file_content(&artifact)
        );
        // Installed locally: the next lookup is a local hit.
        assert!(p2.restore(&mut Vec::new()).is_some());

        // Not in the index: no request, a miss.
        let mut p3 = mk("t2", "cache3");
        p3.base = "c".repeat(32);
        assert!(fetch_from(&base, &p3, &mut Vec::new()).is_none());
        // Changed source: the manifest line does not match.
        std::fs::write(&src, "pub fn g() {}").unwrap();
        let p4 = mk("t2", "cache4");
        assert!(fetch_from(&base, &p4, &mut Vec::new()).is_none());
        std::fs::write(&src, "pub fn f() {}").unwrap();
        // A tampered bundle is rejected and nothing is installed.
        let bundle = std::fs::read_dir(store.join("v1/o"))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        std::fs::write(&bundle, b"not a bundle").unwrap();
        let p5 = mk("t2", "cache5");
        assert!(fetch_from(&base, &p5, &mut Vec::new()).is_none());
        assert!(p5.restore(&mut Vec::new()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cost_lookup_matches_first_line_prefix() {
        for pad in [0, 5000] {
            let text = format!(
                "z 3 4\n{}xkey 4 5\nkeylong bad 9\r\nkey 7 8\nλ 1 2",
                "x".repeat(pad)
            );
            for key in ["z", "key", "keylong", "λ", "", "\n", "9\r", "missing"] {
                assert_eq!(
                    cost_line(&text, key),
                    text.lines().find(|l| l.starts_with(key))
                );
            }
        }
    }

    #[test]
    fn fetches_only_units_that_cost_more_to_compile() {
        let root = std::env::temp_dir().join(format!("jr-worth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = root.join("store");
        std::fs::create_dir_all(store.join("v1")).unwrap();
        let (cheap, slow, big, unpriced) = (
            "1".repeat(32),
            "2".repeat(32),
            "3".repeat(32),
            "4".repeat(32),
        );
        std::fs::write(
            store.join("v1/costs"),
            format!("{cheap} 180 50000\n{slow} 9000 400000\n{big} 3000 200000000\n"),
        )
        .unwrap();
        let base = format!("file://{}", store.display());
        let priced = |key: &str| worth_fetching_with_override(&root, &base, key, false);
        std::fs::write(rate_path(&root), "10000000").unwrap(); // 10 MB/s
        // 180 ms to compile vs 0.7 s fixed fetch cost: compile.
        assert!(!priced(&cheap));
        // 9 s to compile vs 0.7 s + 40 ms transfer: fetch.
        assert!(priced(&slow));
        // 3 s to compile vs 20 s of transfer: compile.
        assert!(!priced(&big));
        // Not priced by the store: fetch, as before costs existed.
        assert!(priced(&unpriced));
        // The hosted override still fetches even when compiling would win.
        assert!(worth_fetching_with_override(&root, &base, &cheap, true));
        assert!(worth_fetching_with_override(&root, &base, &big, true));
        // Measured downloads move the rate.
        note_download(&root, 4_000_000, 4.0);
        assert!(download_rate(&root) < 10_000_000.0);
        // A build host next to the store (20 ms per request): the 180 ms
        // unit is now worth fetching, the 20 s transfer still is not.
        std::fs::write(rate_path(&root), "10000000").unwrap();
        note_latency(&root, 0.02);
        assert!((fetch_overhead_ms(&root) - 70.0).abs() < 1.0);
        assert!(priced(&cheap));
        assert!(!priced(&big));
        // A slow request pulls the estimate back up, bounded.
        for _ in 0..50 {
            note_latency(&root, 10.0);
        }
        assert_eq!(fetch_overhead_ms(&root), 3000.0);
        assert!(!priced(&cheap));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn breaker_needs_consecutive_failures() {
        let root = std::env::temp_dir().join(format!("jr-breaker-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paused = || fresh_file(&breaker(&root), BREAKER_SECS);
        for _ in 0..BREAKER_TRIPS - 1 {
            note_failure(&root, "HTTP 0");
        }
        assert!(
            !paused(),
            "a few timeouts in a cold build must not pause the store"
        );
        note_success(&root);
        for _ in 0..BREAKER_TRIPS - 1 {
            note_failure(&root, "HTTP 0");
        }
        assert!(!paused(), "a success resets the count");
        note_failure(&root, "HTTP 0");
        assert!(paused(), "a store that keeps failing is paused");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn base_accepts_urls_and_absolute_paths_only() {
        assert_eq!(
            parse_base("https://p.example/store/").as_deref(),
            Some("https://p.example/store")
        );
        assert_eq!(
            parse_base("/srv/store").as_deref(),
            Some("file:///srv/store")
        );
        assert_eq!(parse_base("relative/dir"), None);
        assert_eq!(parse_base("ftp://x"), None);
        assert_eq!(parse_base("0"), None);
        assert_eq!(parse_base("off"), None);
        assert_eq!(parse_base(""), None);
        // The default store must survive its own parser (https, no slash).
        assert_eq!(parse_base(DEFAULT_STORE).as_deref(), Some(DEFAULT_STORE));
    }
}
