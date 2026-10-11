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
//! or an absolute path). Unset or `0` means off. Lookups happen after a
//! local miss and before the account cache. The index, when the store has
//! one, is fetched once per 10 minutes and answers most misses without a
//! request: a cold build of 900 units makes one request per unit the store
//! actually has. A missing manifest (404) is a plain miss; any other error
//! trips a 10-minute breaker, so a dead store costs one timeout, not one per
//! unit. Every downloaded bundle is verified (`unpack`: names, file list,
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
use crate::depcache_remote::{pack, unpack};
use crate::paths;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BREAKER_SECS: f64 = 600.0;
const INDEX_SECS: f64 = 600.0;
const MAX_INDEX: u64 = 64 * 1024 * 1024;

/// The store base, without a trailing slash, or `None` when off.
pub fn base() -> Option<String> {
    if !depcache::enabled() {
        return None;
    }
    parse_base(&std::env::var("JUSTRUST_PREBUILT").ok()?)
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

enum Got {
    Ok,
    Missing,
    Failed(String),
}

/// Fetch `<base>/v1/<rel>` into `dst`.
fn get(base: &str, rel: &str, dst: &Path, max_time: u32, max_bytes: u64) -> Got {
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
    let out = Command::new("curl")
        .args([
            "-sS",
            "--proto",
            "=https,http",
            "--max-redirs",
            "2",
            "--connect-timeout",
            "3",
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

fn breaker(root: &Path) -> PathBuf {
    root.join("prebuilt.down")
}

fn fresh_file(p: &Path, secs: f64) -> bool {
    std::fs::metadata(p)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|e| e.as_secs_f64() < secs)
}

fn tmp(root: &Path, tag: &str) -> PathBuf {
    let d = root.join("tmp");
    let _ = std::fs::create_dir_all(&d);
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    d.join(format!(
        "pb-{tag}.{}.{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

/// Local copy of the store's index, one hash of the base URL per store.
fn index_path(root: &Path, base: &str) -> PathBuf {
    let mut h = depcache::H128::new();
    h.field(base.as_bytes());
    root.join(format!("prebuilt-index-{}", &h.hex()[..12]))
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
        // One fetcher refreshes; concurrent shims use the old copy or ask.
        let t = tmp(root, "idx");
        match get(base, "index", &t, 20, MAX_INDEX) {
            Got::Ok => {
                let _ = std::fs::rename(&t, &idx);
            }
            Got::Missing => {
                let _ = std::fs::write(&none, b"");
                return true;
            }
            Got::Failed(why) => {
                let _ = std::fs::remove_file(&t);
                let _ = std::fs::write(breaker(root), format!("index: {why}"));
                return false;
            }
        }
    }
    match std::fs::read(&idx) {
        Ok(b) => index_contains(&b, key),
        Err(_) => true,
    }
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
    if fresh_file(&breaker(root), BREAKER_SECS) || !may_have(root, base, &plan.base) {
        return None;
    }
    let m = tmp(root, "m");
    let got = get(base, &format!("m/{}.jsonl", plan.base), &m, 8, 1 << 20);
    let text = std::fs::read_to_string(&m).unwrap_or_default();
    let _ = std::fs::remove_file(&m);
    match got {
        Got::Ok => {}
        Got::Missing => return None,
        Got::Failed(why) => {
            let _ = std::fs::write(breaker(root), format!("manifest: {why}"));
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
        let ok = matches!(
            get(base, &format!("o/{out}.jrdc"), &bundle, 120, 95 << 20),
            Got::Ok
        ) && unpack(&bundle, &stage).is_some();
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

fn valid_key(k: &str) -> bool {
    k.len() == 32
        && k.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Debug, Default, PartialEq)]
pub struct Published {
    pub bundles: usize,
    pub lines: usize,
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
            // Build-script run entries share the directory and line shape,
            // but their meta.json is not a rustc `Outputs`: `pack` rejects
            // them, so they are skipped (their keys include a host package
            // database stamp and would not match elsewhere anyway).
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
    Ok(p)
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
        None => "prebuilt  off (set JUSTRUST_PREBUILT to a store URL)".into(),
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

    #[test]
    fn publish_then_fetch_into_an_empty_cache() {
        use crate::depcache::Norm;
        let dir = std::env::temp_dir().join(format!("jr-prebuilt-rt-{}", std::process::id()));
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
        std::fs::write(deps1.join("libx-abc.rmeta"), b"rmeta").unwrap();
        p1.store(0.0, 2.5, &[]).unwrap();
        let store = dir.join("store");
        let first = publish(&p1.root, &store).unwrap();
        assert_eq!((first.bundles, first.lines), (1, 1));
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
        assert_eq!(hit.saved_secs, 2.5);
        assert_eq!(
            std::fs::read(p2.out_dir.join("libx-abc.rmeta")).unwrap(),
            b"rmeta"
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
        assert_eq!(parse_base(""), None);
    }
}
