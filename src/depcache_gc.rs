//! Size cap and LRU eviction for the shared dependency cache, and
//! `justrust cache`.
//!
//! Recency: a hit touches `o/<key>/meta.json` (see `depcache::touch`), a store
//! creates it fresh, so its mtime is the entry's last use.
//!
//! Eviction runs in a detached background process after a recorded run, at
//! most once per [`GC_INTERVAL_SECS`], so builds never wait for it. Only one
//! collector runs at a time (`gc.lock` flock). When the cache exceeds the cap
//! (default 30 GB, `JUSTRUST_DEPCACHE_MAX`, e.g. `50G`, `800M`, `0` = no cap),
//! the least recently used entries are removed until it is under 90% of it.
//!
//! Safety with concurrent builds:
//! - An entry is first renamed into `trash/` (atomic), then deleted. A restore
//!   that already opened a file keeps reading it, one that has not yet fails to
//!   open it and compiles normally (restore fails open and rustc overwrites
//!   any partially restored outputs).
//! - Entries used within [`GRACE_SECS`] are never evicted, and an entry whose
//!   `meta.json` was touched between the scan and the rename is put back.
//! - Manifest lines pointing at evicted entries simply miss.

use crate::depcache;
use crate::paths;
use anyhow::{Context, Result};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_BYTES: u64 = 30 * 1024 * 1024 * 1024;
const GC_INTERVAL_SECS: f64 = 600.0;
/// Entries used this recently are never evicted (`JUSTRUST_DEPCACHE_GRACE`
/// overrides it, for stress tests).
const GRACE_SECS: f64 = 600.0;
const LOW_WATER: f64 = 0.9;

/// Parse `30G`, `512M`, `1.5T`, `123456` (bytes). `0` means no cap.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((i, _)) => {
            let unit = s[i..].to_ascii_uppercase();
            let unit = unit.trim_end_matches('B').trim_end_matches('I');
            let m: u64 = match unit {
                "" => 1,
                "K" => 1 << 10,
                "M" => 1 << 20,
                "G" => 1 << 30,
                "T" => 1 << 40,
                _ => return None,
            };
            (&s[..i], m)
        }
        None => (s, 1),
    };
    let v: f64 = num.trim().parse().ok()?;
    (v >= 0.0).then_some((v * mult as f64) as u64)
}

pub fn max_bytes() -> u64 {
    std::env::var("JUSTRUST_DEPCACHE_MAX")
        .ok()
        .and_then(|v| parse_size(&v))
        .unwrap_or(DEFAULT_MAX_BYTES)
}

#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    bytes: u64,
    last_used: f64,
}

#[derive(Debug, Default)]
pub struct Scan {
    entries: Vec<Entry>,
    pub bytes: u64,
    pub manifests: usize,
    pub manifest_bytes: u64,
}

fn mtime(m: &std::fs::Metadata) -> f64 {
    m.mtime() as f64 + m.mtime_nsec() as f64 / 1e9
}

/// Disk blocks actually allocated (reflinked extents still count in full).
fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.blocks() * 512)
        .sum()
}

pub fn scan(root: &Path) -> Scan {
    let mut s = Scan::default();
    for e in std::fs::read_dir(root.join("o"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let path = e.path();
        if name.starts_with('.') {
            // Half-written store from a crashed process: collect once stale.
            if let Ok(m) = e.metadata()
                && paths::now() - mtime(&m) > 3600.0
            {
                let _ = std::fs::remove_dir_all(&path);
            }
            continue;
        }
        let last_used = std::fs::metadata(path.join("meta.json"))
            .or_else(|_| e.metadata())
            .map(|m| mtime(&m))
            .unwrap_or(0.0);
        let bytes = dir_bytes(&path);
        s.bytes += bytes;
        s.entries.push(Entry {
            path,
            bytes,
            last_used,
        });
    }
    for e in std::fs::read_dir(root.join("m"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(m) = e.metadata() {
            s.manifests += 1;
            s.manifest_bytes += m.blocks() * 512;
        }
    }
    s.bytes += s.manifest_bytes;
    s
}

#[derive(Debug, Default, PartialEq)]
pub struct Pruned {
    pub entries: usize,
    pub bytes: u64,
}

/// Evict least recently used entries until the cache is under `LOW_WATER`
/// of `max` (or everything not in use, when `max` is 0 and `all` is set).
pub fn prune(root: &Path, max: u64, all: bool) -> Pruned {
    let mut s = scan(root);
    let mut out = Pruned::default();
    if !all && (max == 0 || s.bytes <= max) {
        clear_trash(root);
        return out;
    }
    let target = if all {
        0
    } else {
        (max as f64 * LOW_WATER) as u64
    };
    s.entries
        .sort_by(|a, b| a.last_used.total_cmp(&b.last_used));
    let trash = root.join("trash");
    let _ = std::fs::create_dir_all(&trash);
    let now = paths::now();
    let grace = std::env::var("JUSTRUST_DEPCACHE_GRACE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(GRACE_SECS);
    for e in &s.entries {
        if s.bytes <= target {
            break;
        }
        if now - e.last_used < grace {
            // Everything after this was used even more recently.
            break;
        }
        let Some(name) = e.path.file_name() else {
            continue;
        };
        let dst = trash.join(format!("{}.{}", name.to_string_lossy(), std::process::id()));
        if std::fs::rename(&e.path, &dst).is_err() {
            continue;
        }
        // Hit between scan and rename: put it back.
        let touched = std::fs::metadata(dst.join("meta.json"))
            .map(|m| mtime(&m) > e.last_used + 0.5)
            .unwrap_or(false);
        if touched && std::fs::rename(&dst, &e.path).is_ok() {
            continue;
        }
        s.bytes = s.bytes.saturating_sub(e.bytes);
        out.entries += 1;
        out.bytes += e.bytes;
    }
    clear_trash(root);
    drop_dead_manifest_lines(root);
    out
}

fn clear_trash(root: &Path) {
    for e in std::fs::read_dir(root.join("trash"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let _ = std::fs::remove_dir_all(e.path());
    }
}

/// Rewrite manifests without lines whose outputs were evicted. A line
/// appended concurrently between read and rename can be lost, which only
/// costs a future miss.
fn drop_dead_manifest_lines(root: &Path) {
    let o = root.join("o");
    for e in std::fs::read_dir(root.join("m"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = e.path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let keep: Vec<&str> = text
            .lines()
            .filter(|l| {
                depcache::entry_out(l).is_some_and(|out| o.join(out).join("meta.json").exists())
            })
            .collect();
        if keep.len() == text.lines().count() {
            continue;
        }
        if keep.is_empty() {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        let mut body = keep.join("\n");
        body.push('\n');
        if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn try_lock(path: &Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    // SAFETY: flock on a valid fd.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

/// Called after every recorded run. Costs one `stat` unless a collection is
/// due, then spawns a detached `justrust cache --auto` and returns.
pub fn maybe_spawn() {
    if !depcache::enabled() {
        return;
    }
    let Some(root) = depcache::cache_dir() else {
        return;
    };
    let stamp = root.join("gc.stamp");
    match std::fs::metadata(&stamp) {
        Ok(m) if paths::now() - mtime(&m) < GC_INTERVAL_SECS => return,
        Err(_) if !root.join("o").is_dir() => return,
        _ => {}
    }
    if std::fs::write(&stamp, b"").is_err() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use std::os::unix::process::CommandExt;
    let _ = std::process::Command::new(exe)
        .args(["cache", "--auto"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
}

/// Background collection: nice, lock, prune to the cap, log what happened.
fn auto(root: &Path) {
    // SAFETY: plain syscalls on the current process.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
    let Some(_lock) = try_lock(&root.join("gc.lock")) else {
        return;
    };
    let start = paths::now();
    let p = prune(root, max_bytes(), false);
    if p.entries > 0 {
        let line = serde_json::json!({
            "t": start,
            "evicted": p.entries,
            "bytes": p.bytes,
            "secs": paths::now() - start,
        });
        let _ = paths::append_line(&root.join("gc.jsonl"), &line.to_string());
    }
}

fn gb(b: u64) -> String {
    format!("{:.2} GB", b as f64 / (1u64 << 30) as f64)
}

/// Hit/miss totals over recorded runs in the last `days`.
fn hit_rate(days: f64) -> (usize, usize, f64, usize) {
    let cutoff = paths::now() - days * 86400.0;
    let Ok(runs) = paths::runs_dir() else {
        return (0, 0, 0.0, 0);
    };
    let (mut hits, mut misses, mut saved, mut n) = (0, 0, 0.0, 0);
    for e in crate::runs::load_index() {
        if e.start < cutoff {
            continue;
        }
        let Ok(b) = std::fs::read(runs.join(&e.id).join("summary.json")) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) else {
            continue;
        };
        let u = &v["units"];
        let h = u["depcache_hits"].as_u64().unwrap_or(0) as usize;
        let m = u["depcache_misses"].as_u64().unwrap_or(0) as usize;
        if h + m > 0 {
            n += 1;
        }
        hits += h;
        misses += m;
        saved += u["depcache_saved_secs"].as_f64().unwrap_or(0.0);
    }
    (hits, misses, saved, n)
}

/// `justrust cache [--prune] [--clear]`.
pub fn command(prune_now: bool, clear: bool, auto_mode: bool) -> Result<()> {
    let root = depcache::cache_dir().context("no cache directory")?;
    if auto_mode {
        auto(&root);
        return Ok(());
    }
    let max = max_bytes();
    if prune_now || clear {
        let _lock = try_lock(&root.join("gc.lock"))
            .context("another eviction is running, try again shortly")?;
        let p = prune(&root, max, clear);
        println!("evicted {} entries, {}", p.entries, gb(p.bytes));
    }
    let s = scan(&root);
    let cap = if max == 0 { "no cap".into() } else { gb(max) };
    println!("depcache  {}", root.display());
    println!(
        "size      {} of {} cap ({} entries, {} manifests)",
        gb(s.bytes),
        cap,
        s.entries.len(),
        s.manifests
    );
    if !depcache::enabled() {
        println!("status    disabled by JUSTRUST_DEPCACHE");
    }
    if let (Some(old), Some(new)) = (
        s.entries.iter().map(|e| e.last_used).reduce(f64::min),
        s.entries.iter().map(|e| e.last_used).reduce(f64::max),
    ) {
        let now = paths::now();
        println!(
            "used      oldest entry last used {:.1}h ago, newest {:.0}s ago",
            (now - old) / 3600.0,
            now - new
        );
    }
    let (h, m, saved, n) = hit_rate(7.0);
    if h + m > 0 {
        println!(
            "hit rate  {:.1}% over the last 7 days ({h} hit / {m} miss in {n} runs, ~{:.0} min of rustc saved)",
            100.0 * h as f64 / (h + m) as f64,
            saved / 60.0
        );
    }
    let log = std::fs::read_to_string(root.join("gc.jsonl")).unwrap_or_default();
    if let Some(last) = log
        .lines()
        .last()
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
    {
        println!(
            "evicted   {} entries ({}) {:.1}h ago in the background",
            last["evicted"],
            gb(last["bytes"].as_u64().unwrap_or(0)),
            (paths::now() - last["t"].as_f64().unwrap_or(0.0)) / 3600.0
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("30G"), Some(30 << 30));
        assert_eq!(parse_size("512m"), Some(512 << 20));
        assert_eq!(parse_size("1.5GiB"), Some(3 << 29));
        assert_eq!(parse_size("1234"), Some(1234));
        assert_eq!(parse_size("0"), Some(0));
        assert_eq!(parse_size("lots"), None);
        assert_eq!(parse_size("5X"), None);
    }

    fn entry(root: &Path, key: &str, kb: usize, age_secs: f64) {
        let d = root.join("o").join(key);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("libx.rlib"), vec![7u8; kb * 1024]).unwrap();
        std::fs::write(d.join("meta.json"), b"{}").unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs_f64(age_secs);
        std::fs::File::options()
            .append(true)
            .open(d.join("meta.json"))
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn evicts_least_recently_used_first_and_spares_recent() {
        let root = std::env::temp_dir().join(format!("jr-depcache-gc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        entry(&root, "old", 100, 86400.0);
        entry(&root, "mid", 100, 7200.0);
        entry(&root, "new", 100, 3600.0);
        entry(&root, "hot", 100, 1.0);
        std::fs::create_dir_all(root.join("m")).unwrap();
        let line = |out: &str| format!(r#"{{"inputs":[],"env":[],"out":"{out}"}}"#);
        std::fs::write(
            root.join("m/k.jsonl"),
            format!("{}\n{}\n", line("old"), line("new")),
        )
        .unwrap();

        let total = scan(&root).bytes;
        assert!(total >= 400 * 1024);
        // Under the cap: nothing happens.
        assert_eq!(prune(&root, total * 2, false), Pruned::default());
        // Cap of ~2.5 entries: low water 90% means two oldest go.
        let p = prune(&root, 250 * 1024, false);
        assert_eq!(p.entries, 2, "{p:?}");
        assert!(!root.join("o/old").exists() && !root.join("o/mid").exists());
        assert!(root.join("o/new").exists() && root.join("o/hot").exists());
        // Manifest lines for evicted entries are dropped.
        assert_eq!(
            std::fs::read_to_string(root.join("m/k.jsonl")).unwrap(),
            format!("{}\n", line("new"))
        );
        // Tiny cap: the recently used entry survives the grace period.
        prune(&root, 1, false);
        assert!(!root.join("o/new").exists());
        assert!(root.join("o/hot").exists());
        assert!(!root.join("m/k.jsonl").exists());
        assert_eq!(std::fs::read_dir(root.join("trash")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
