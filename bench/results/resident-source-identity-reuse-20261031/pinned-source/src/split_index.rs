//! Incremental state behind `justrust split`, kept up to date by the
//! operations that already have each piece in hand, so a split
//! recommendation costs milliseconds at the end of a build instead of a
//! full re-analysis:
//!
//! - `split/edits.jsonl`: one small record per finished run (what cargo saw
//!   change, which local units rebuilt and for how long, whether the machine
//!   was quiet). Appended by every recorded run, local or remote. Run
//!   summaries stay the source of truth: a run missing here (older runs, a
//!   lost or outdated index) is read from its `summary.json` once and
//!   appended.
//! - `split/facts-<workspace>.json`: what the module-graph parser extracts
//!   from each source file (line count, `mod` items, written module paths,
//!   type definitions, inherent impls), keyed by size and mtime. Whoever
//!   builds a module graph refreshes the entries of files that changed and
//!   reuses the rest.
//! - `split/churn-<workspace>.json`: most edited items of crate root files
//!   (git hunk headers), keyed by HEAD and day.
//!
//! All three are caches: deleting `~/.justrust/split` loses nothing, the
//! next `justrust split` rebuilds them.

use crate::paths;
use crate::summary::{RebuildReason, Resources, Summary, UnitBreakdown};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Bump when `EditRecord` or `FileFacts` change meaning. Old lines and
/// cache files are ignored and rebuilt from the run summaries and sources.
const VERSION: u32 = 1;

pub fn dir() -> Result<PathBuf> {
    Ok(paths::home()?.join("split"))
}

fn edits_file() -> Result<PathBuf> {
    Ok(dir()?.join("edits.jsonl"))
}

/// Stable short name for per-workspace cache files.
fn workspace_key(root: &Path) -> String {
    let mut h = crate::depcache::H128::new();
    h.field(root.as_os_str().as_encoded_bytes());
    h.hex()[..16].to_owned()
}

// ---------------------------------------------------------------------------
// Edit records.

/// The part of a run summary `justrust split` uses.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EditRecord {
    pub v: u32,
    pub id: String,
    pub start: f64,
    pub cwd: String,
    #[serde(default)]
    pub git_root: Option<String>,
    pub subcommand: String,
    pub ncpu: usize,
    pub avg_other_cores: f64,
    #[serde(default)]
    pub reasons: Vec<RebuildReason>,
    /// Local units: (name, kind, wall share).
    #[serde(default)]
    pub units: Vec<(String, String, f64)>,
}

impl EditRecord {
    pub fn from_summary(s: &Summary) -> EditRecord {
        EditRecord {
            v: VERSION,
            id: s.id.clone(),
            start: s.start,
            cwd: s.cwd.clone(),
            git_root: s.git.root.clone(),
            subcommand: s.subcommand.clone(),
            ncpu: s.ncpu,
            avg_other_cores: s.resources.avg_other_cores,
            reasons: s.rebuild_reasons.clone(),
            units: s
                .top_units
                .iter()
                .filter(|u| u.local)
                .map(|u| (u.name.clone(), u.kind.clone(), u.wall_share))
                .collect(),
        }
    }

    /// A summary with exactly the fields `split` reads.
    pub fn to_summary(&self) -> Summary {
        let mut s = Summary {
            id: self.id.clone(),
            start: self.start,
            cwd: self.cwd.clone(),
            subcommand: self.subcommand.clone(),
            ncpu: self.ncpu,
            resources: Resources {
                avg_other_cores: self.avg_other_cores,
                ..Default::default()
            },
            rebuild_reasons: self.reasons.clone(),
            ..Default::default()
        };
        s.git.root = self.git_root.clone();
        s.top_units = self
            .units
            .iter()
            .map(|(name, kind, share)| UnitBreakdown {
                name: name.clone(),
                kind: kind.clone(),
                local: true,
                wall_share: *share,
                ..Default::default()
            })
            .collect();
        s
    }
}

/// Called when a run's summary is written. Never fails the build.
pub fn record(s: &Summary) {
    let Ok(f) = edits_file() else { return };
    let _ = std::fs::create_dir_all(f.parent().unwrap());
    if let Ok(line) = serde_json::to_string(&EditRecord::from_summary(s)) {
        let _ = paths::append_line(&f, &line);
    }
}

/// Runs of the workspace at `root` from the last `days` days, oldest first,
/// as the summaries `split` needs. Reads the index, then back-fills runs it
/// does not have from their `summary.json` (once: they are appended).
pub fn load(root: &Path, days: f64) -> Result<Vec<Summary>> {
    let cutoff = paths::now() - days * 86400.0;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Summary> = Vec::new();
    let file = edits_file()?;
    if let Ok(raw) = std::fs::read_to_string(&file) {
        for line in raw.lines() {
            let Ok(r) = serde_json::from_str::<EditRecord>(line) else {
                continue;
            };
            if r.v != VERSION || !seen.insert(r.id.clone()) {
                continue;
            }
            let s = r.to_summary();
            if s.start >= cutoff && crate::split::belongs(&s, root) {
                out.push(s);
            }
        }
    }
    // Back-fill. Run ids start with the local date (YYYYMMDD), so runs
    // older than the window are skipped without reading them.
    let oldest = chrono::DateTime::from_timestamp(cutoff as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y%m%d").to_string())
        .unwrap_or_default();
    let mut added = String::new();
    if let Ok(rd) = std::fs::read_dir(paths::runs_dir()?) {
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if seen.contains(&id) || id.get(..8).is_some_and(|d| d < oldest.as_str()) {
                continue;
            }
            let Ok(raw) = std::fs::read(e.path().join("summary.json")) else {
                continue; // still running, or not a run
            };
            let Ok(s) = serde_json::from_slice::<Summary>(&raw) else {
                continue;
            };
            let r = EditRecord::from_summary(&s);
            if let Ok(l) = serde_json::to_string(&r) {
                added.push_str(&l);
                added.push('\n');
            }
            seen.insert(id);
            let s = r.to_summary();
            if s.start >= cutoff && crate::split::belongs(&s, root) {
                out.push(s);
            }
        }
    }
    if !added.is_empty() {
        let _ = std::fs::create_dir_all(file.parent().unwrap());
        let _ = paths::append_line(&file, added.trim_end_matches('\n'));
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(out)
}

// ---------------------------------------------------------------------------
// Per-file parse facts.

/// What the module-graph parser extracts from one source file.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FileFacts {
    pub lines: usize,
    /// `mod` items: (name, candidate files, test-only).
    pub decls: Vec<(String, Vec<PathBuf>, bool)>,
    /// Module paths as written, per statement: (inside `pub use`, paths).
    /// Resolved against the module tree when the graph is built.
    pub paths: Vec<(bool, Vec<String>)>,
    pub defines: Vec<String>,
    pub impls: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    len: u64,
    mtime_ns: i128,
    root: bool,
    facts: FileFacts,
}

#[derive(Serialize, Deserialize, Default)]
struct FactsFile {
    v: u32,
    files: HashMap<PathBuf, Entry>,
}

/// File facts for one workspace, loaded once per command.
pub struct Facts {
    path: Option<PathBuf>,
    data: FactsFile,
    dirty: bool,
    pub hits: usize,
    pub misses: usize,
}

fn stamp(p: &Path) -> Option<(u64, i128)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(p).ok()?;
    Some((
        m.len(),
        m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128,
    ))
}

impl Facts {
    /// No persistence (tests, one-off graphs).
    pub fn ephemeral() -> Facts {
        Facts {
            path: None,
            data: FactsFile::default(),
            dirty: false,
            hits: 0,
            misses: 0,
        }
    }

    pub fn open(root: &Path) -> Facts {
        // Unit tests build graphs of throwaway fixtures: keep them in memory.
        if cfg!(test) {
            return Facts::ephemeral();
        }
        let path = dir()
            .ok()
            .map(|d| d.join(format!("facts-{}.json", workspace_key(root))));
        let data = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<FactsFile>(&b).ok())
            .filter(|f| f.v == VERSION)
            .unwrap_or_default();
        Facts {
            path,
            data,
            dirty: false,
            hits: 0,
            misses: 0,
        }
    }

    /// Facts for `file`, parsing it only if it changed since last time.
    pub fn get(
        &mut self,
        file: &Path,
        is_root: bool,
        parse: impl FnOnce(&str) -> FileFacts,
    ) -> Option<&FileFacts> {
        let (len, mtime_ns) = stamp(file)?;
        let fresh = self
            .data
            .files
            .get(file)
            .is_some_and(|e| e.len == len && e.mtime_ns == mtime_ns && e.root == is_root);
        if fresh {
            self.hits += 1;
        } else {
            let src = std::fs::read_to_string(file).ok()?;
            self.misses += 1;
            self.dirty = true;
            self.data.files.insert(
                file.to_path_buf(),
                Entry {
                    len,
                    mtime_ns,
                    root: is_root,
                    facts: parse(&src),
                },
            );
        }
        self.data.files.get(file).map(|e| &e.facts)
    }

    /// Write back if anything changed, dropping files that no longer exist.
    pub fn save(mut self) {
        let Some(path) = self.path.take() else { return };
        if !self.dirty {
            return;
        }
        self.data.v = VERSION;
        self.data.files.retain(|p, _| p.exists());
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if let Ok(b) = serde_json::to_vec(&self.data)
            && std::fs::write(&tmp, b).is_ok()
        {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

// ---------------------------------------------------------------------------
// Churn of crate root files.

#[derive(Serialize, Deserialize, Default)]
struct ChurnFile {
    v: u32,
    /// "<file>\0<head>\0<days>\0<date>" -> items.
    items: BTreeMap<String, Vec<(String, usize)>>,
}

/// `compute` runs `git log`; cached until HEAD or the date changes.
pub fn churn(
    root: &Path,
    file: &Path,
    days: f64,
    compute: impl FnOnce() -> Vec<(String, usize)>,
) -> Vec<(String, usize)> {
    if cfg!(test) {
        return compute();
    }
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let (Some(head), Ok(d)) = (head, dir()) else {
        return compute();
    };
    let path = d.join(format!("churn-{}.json", workspace_key(root)));
    let mut data = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<ChurnFile>(&b).ok())
        .filter(|c| c.v == VERSION)
        .unwrap_or_default();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let key = format!("{}\0{head}\0{days}\0{today}", file.display());
    if let Some(v) = data.items.get(&key) {
        return v.clone();
    }
    let v = compute();
    data.v = VERSION;
    data.items.retain(|k, _| k.ends_with(&today));
    data.items.insert(key, v.clone());
    let _ = std::fs::create_dir_all(&d);
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if let Ok(b) = serde_json::to_vec(&data)
        && std::fs::write(&tmp, b).is_ok()
    {
        let _ = std::fs::rename(&tmp, &path);
    }
    v
}

// ---------------------------------------------------------------------------
// Split hints: per edited file, computed in the background, read by builds.

/// Min seconds between background hint refreshes of one workspace.
const REFRESH_INTERVAL_SECS: f64 = 60.0;

#[derive(Serialize, Deserialize, Default)]
struct HintsFile {
    v: u32,
    root: PathBuf,
    computed_at: f64,
    runs: usize,
    hints: BTreeMap<PathBuf, crate::split::Hint>,
}

fn hints_path(d: &Path, root: &Path) -> PathBuf {
    d.join(format!("hints-{}.json", workspace_key(root)))
}

pub fn save_hints(root: &Path, hints: &BTreeMap<PathBuf, crate::split::Hint>, runs: usize) {
    if let Ok(d) = dir() {
        save_hints_in(&d, root, hints, runs);
    }
}

fn save_hints_in(
    d: &Path,
    root: &Path,
    hints: &BTreeMap<PathBuf, crate::split::Hint>,
    runs: usize,
) {
    let path = hints_path(d, root);
    let data = HintsFile {
        v: VERSION,
        root: root.to_path_buf(),
        computed_at: paths::now(),
        runs,
        hints: hints.clone(),
    };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if let Ok(b) = serde_json::to_vec(&data)
        && std::fs::write(&tmp, b).is_ok()
    {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The hints file of the workspace containing `cwd` (nearest ancestor that
/// has one). A handful of `stat`s; no cargo, no parsing of sources.
fn find_hints(d: &Path, cwd: &Path) -> Option<HintsFile> {
    for anc in cwd.ancestors() {
        if let Ok(b) = std::fs::read(hints_path(d, anc)) {
            return serde_json::from_slice::<HintsFile>(&b)
                .ok()
                .filter(|h| h.v == VERSION);
        }
    }
    None
}

/// Files cargo saw change in this run (`Dirty ...: the file X has changed`).
pub fn edited_files(s: &Summary) -> Vec<String> {
    let mut v: Vec<String> = s
        .rebuild_reasons
        .iter()
        .filter_map(|r| {
            r.reason
                .strip_prefix("the file `")
                .and_then(|x| x.strip_suffix("` has changed"))
                .map(str::to_owned)
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Attach the stored hints for the files this run edited. Called when a
/// run finishes, before its report is printed. Costs a few `stat`s and one
/// small JSON read.
pub fn attach(s: &mut Summary) {
    if let Ok(d) = dir() {
        attach_from(&d, s);
    }
}

fn attach_from(d: &Path, s: &mut Summary) {
    let files = edited_files(s);
    if files.is_empty() {
        return;
    }
    let Some(h) = find_hints(d, Path::new(&s.cwd)) else {
        return;
    };
    let mut out: Vec<crate::summary::SplitHint> = Vec::new();
    for f in files {
        let p = Path::new(&f);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            h.root.join(p)
        };
        let abs = crate::split::normalize_pub(&abs);
        if let Some(hint) = h.hints.get(&abs)
            && hint.secs >= 1.0
            && !out.iter().any(|o| o.text == hint.text)
        {
            out.push(crate::summary::SplitHint {
                file: f,
                secs: hint.secs,
                text: hint.text.clone(),
                age_secs: (paths::now() - h.computed_at).max(0.0),
            });
        }
    }
    out.sort_by(|a, b| b.secs.total_cmp(&a.secs));
    out.truncate(2);
    s.split_hints = out;
}

/// After a run that edited files: refresh this workspace's hints in a
/// detached, niced `justrust split --refresh-hints`, at most once per
/// `REFRESH_INTERVAL_SECS`. One `stat` when not due.
pub fn maybe_refresh(s: &Summary) {
    if std::env::var_os("JUSTRUST_SPLIT_HINTS").is_some_and(|v| v == "0")
        || edited_files(s).is_empty()
    {
        return;
    }
    let key_dir = s.git.root.clone().unwrap_or_else(|| s.cwd.clone());
    let Ok(d) = dir() else { return };
    let stamp = d.join(format!(
        "refresh-{}.stamp",
        workspace_key(Path::new(&key_dir))
    ));
    if let Ok(m) = std::fs::metadata(&stamp)
        && let Ok(t) = m.modified()
        && t.elapsed().map(|e| e.as_secs_f64()).unwrap_or(0.0) < REFRESH_INTERVAL_SECS
    {
        return;
    }
    let _ = std::fs::create_dir_all(&d);
    if std::fs::write(&stamp, b"").is_err() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use std::os::unix::process::CommandExt;
    let _ = std::process::Command::new(exe)
        .args(["split", "--refresh-hints"])
        .current_dir(&s.cwd)
        .env("JUSTRUST_SPLIT_HINTS", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
}

/// Held while a refresh of `root` runs, so concurrent builds do not stack
/// refreshes.
pub fn refresh_lock(root: &Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let d = dir().ok()?;
    std::fs::create_dir_all(&d).ok()?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(d.join(format!("refresh-{}.lock", workspace_key(root))))
        .ok()?;
    // SAFETY: flock on a valid fd.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trips_what_split_reads() {
        let mut s = Summary {
            id: "20261010-000000000-1".into(),
            start: 5.0,
            cwd: "/w".into(),
            subcommand: "check".into(),
            ncpu: 16,
            ..Default::default()
        };
        s.git.root = Some("/w".into());
        s.resources.avg_other_cores = 2.5;
        s.rebuild_reasons.push(RebuildReason {
            package: "a".into(),
            reason: "the file `a/src/lib.rs` has changed".into(),
        });
        s.top_units.push(UnitBreakdown {
            name: "a".into(),
            kind: "lib".into(),
            local: true,
            wall_share: 3.0,
            ..Default::default()
        });
        s.top_units.push(UnitBreakdown {
            name: "serde".into(),
            local: false,
            wall_share: 9.0,
            ..Default::default()
        });
        let line = serde_json::to_string(&EditRecord::from_summary(&s)).unwrap();
        let back = serde_json::from_str::<EditRecord>(&line)
            .unwrap()
            .to_summary();
        assert_eq!(back.id, s.id);
        assert_eq!(back.git.root.as_deref(), Some("/w"));
        assert_eq!(back.resources.avg_other_cores, 2.5);
        assert_eq!(back.rebuild_reasons.len(), 1);
        assert_eq!(back.top_units.len(), 1, "registry units are not needed");
        assert_eq!(back.top_units[0].wall_share, 3.0);
    }

    #[test]
    fn stored_hints_attach_to_runs_that_edit_those_files() {
        let home = std::env::temp_dir().join(format!("jr-hints-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let d = home.join("split");
        let root = home.join("ws");
        let file = root.join("crates/a/src/m.rs");
        let mut hints = BTreeMap::new();
        hints.insert(
            file.clone(),
            crate::split::Hint {
                secs: 4.0,
                text: "edits to a::m cost ~5.0s".into(),
            },
        );
        hints.insert(
            root.join("crates/a/src/cheap.rs"),
            crate::split::Hint {
                secs: 0.5,
                text: "too small to mention".into(),
            },
        );
        save_hints_in(&d, &root, &hints, 7);
        let mut s = Summary {
            cwd: root.join("crates/a").to_string_lossy().into_owned(),
            ..Default::default()
        };
        for f in [
            "crates/a/src/m.rs",
            "crates/a/src/cheap.rs",
            "crates/a/src/other.rs",
        ] {
            s.rebuild_reasons.push(RebuildReason {
                package: "a".into(),
                reason: format!("the file `{f}` has changed"),
            });
        }
        attach_from(&d, &mut s);
        assert_eq!(s.split_hints.len(), 1, "{:?}", s.split_hints);
        assert_eq!(s.split_hints[0].file, "crates/a/src/m.rs");
        assert_eq!(s.split_hints[0].text, "edits to a::m cost ~5.0s");
        // A run elsewhere finds nothing.
        let mut other = s.clone();
        other.split_hints.clear();
        other.cwd = "/nonexistent/elsewhere".into();
        attach_from(&d, &mut other);
        assert!(other.split_hints.is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Not a correctness test: `cargo test --release attach_cost -- --ignored --nocapture`
    /// against the real ~/.justrust state.
    #[test]
    #[ignore]
    fn attach_cost() {
        let mut s = Summary {
            cwd: "/home/jeremy/jcode".into(),
            ..Default::default()
        };
        s.rebuild_reasons.push(RebuildReason {
            package: "jcode-base".into(),
            reason: "the file `crates/jcode-base/src/config/config_file.rs` has changed".into(),
        });
        let d = dirs::home_dir().unwrap().join(".justrust/split");
        let t = std::time::Instant::now();
        for _ in 0..100 {
            s.split_hints.clear();
            attach_from(&d, &mut s);
        }
        eprintln!(
            "attach: {:?} per run, {} hints",
            t.elapsed() / 100,
            s.split_hints.len()
        );
    }

    #[test]
    fn facts_reparse_only_changed_files() {
        let d = std::env::temp_dir().join(format!("jr-facts-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("a.rs");
        std::fs::write(&f, "x\n").unwrap();
        let mut facts = Facts::ephemeral();
        let parse = |s: &str| FileFacts {
            lines: s.lines().count(),
            ..Default::default()
        };
        assert_eq!(facts.get(&f, false, parse).unwrap().lines, 1);
        assert_eq!(facts.get(&f, false, |_| unreachable!()).unwrap().lines, 1);
        assert_eq!((facts.hits, facts.misses), (1, 1));
        std::fs::write(&f, "x\ny\nz\n").unwrap();
        assert_eq!(facts.get(&f, false, parse).unwrap().lines, 3);
        // Same file as a crate root parses differently (`mod` dirs).
        assert_eq!(facts.get(&f, true, parse).unwrap().lines, 3);
        assert_eq!(facts.misses, 3);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
