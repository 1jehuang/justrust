//! `justrust export`: copy compact run summaries into the repo's `data/runs/`.
//!
//! Raw run directories (`~/.justrust/runs/<id>/`) are hundreds of MB and hold
//! full compiler output. The export keeps one slim JSON line per finished run
//! so findings can cite run ids that exist in the repository and benchmarks
//! can replay real agent workloads:
//!
//! ```text
//! data/runs/<machine>/<YYYY-MM>.jsonl   one line per run, sorted by id
//! ```
//!
//! Each line is the run's `summary.json` with:
//! - every absolute path under the home directory rewritten to `~/...`
//! - the first compile error cut to its first line (no source snippets)
//! - `top_units` cut to the slowest 6, each with its 5 slowest passes
//! - `split_hints` dropped (derived text, regenerated on demand)
//! - floats rounded to milliseconds
//! - `machine`, `repo`, and `kind` (`agent`, `manual`, `bench`, `scratch`) added
//!
//! The repository is public, so only runs whose `repo` is listed in
//! `<out>/public-repos.txt` keep crate names, arguments, paths, and error
//! text. Every other run (private repos, scratch copies of unknown origin) is
//! exported with timings and counts only: `repo` becomes `private`, unit names
//! become `crate0..`, errors keep only their code, and paths, arguments, and
//! rebuild reasons are dropped. A missing allowlist redacts everything.
//!
//! The export is incremental: ids already present in the machine directory
//! are skipped, so running it again only appends new runs.

use crate::paths;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const TOP_UNITS: usize = 6;
const TOP_PASSES: usize = 5;
const ERROR_CHARS: usize = 200;

pub fn command(out: Option<PathBuf>, machine: Option<String>, dry_run: bool) -> Result<()> {
    let root = match out {
        Some(p) => p,
        None => default_out()?,
    };
    let machine = match machine {
        Some(m) => m,
        None => hostname(),
    };
    if machine.is_empty() || machine.contains(['/', '\\']) || machine.starts_with('.') {
        bail!("invalid machine name {machine:?}, pass --machine NAME");
    }
    let dir = root.join(&machine);
    let home = dirs::home_dir().context("no home directory")?;
    let runs_dir = paths::runs_dir()?;

    let existing = existing_ids(&dir)?;
    let public = load_public_repos(&root);
    let mut ids: Vec<String> = std::fs::read_dir(&runs_dir)
        .with_context(|| format!("reading {}", runs_dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|id| !existing.contains(id))
        .collect();
    ids.sort();

    let mut shards: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unfinished = 0usize;
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut redacted = 0usize;
    for id in &ids {
        let path = runs_dir.join(id).join("summary.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            unfinished += 1;
            continue;
        };
        let Ok(summary) = serde_json::from_str::<Value>(&text) else {
            unfinished += 1;
            continue;
        };
        let slim = slim(summary, &home, &machine, &public);
        let kind = slim["kind"].as_str().unwrap_or("?").to_string();
        *kinds.entry(kind).or_default() += 1;
        if slim["repo"] == PRIVATE {
            redacted += 1;
        }
        shards
            .entry(shard_name(id))
            .or_default()
            .push(serde_json::to_string(&slim)?);
    }

    let added: usize = shards.values().map(Vec::len).sum();
    if !dry_run && added > 0 {
        std::fs::create_dir_all(&dir)?;
        for (shard, lines) in &shards {
            let path = dir.join(format!("{shard}.jsonl"));
            let mut text = std::fs::read_to_string(&path).unwrap_or_default();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            for l in lines {
                text.push_str(l);
                text.push('\n');
            }
            let tmp = path.with_extension("jsonl.tmp");
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, &path)?;
        }
    }
    let by_kind: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} {k}")).collect();
    println!(
        "justrust export: {} {added} new runs to {} ({}; {redacted} redacted as private), {} already exported, {unfinished} without a summary",
        if dry_run { "would add" } else { "added" },
        dir.display(),
        if by_kind.is_empty() {
            "none".to_string()
        } else {
            by_kind.join(", ")
        },
        existing.len(),
    );
    Ok(())
}

/// `./data/runs` when run from the justrust repository root.
fn default_out() -> Result<PathBuf> {
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap_or_default();
    let is_repo = manifest
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| {
            t.get("package")?
                .get("name")?
                .as_str()
                .map(|n| n == "justrust")
        })
        .unwrap_or(false);
    if !is_repo {
        bail!("run `justrust export` from the justrust repository root, or pass --out DIR");
    }
    Ok(PathBuf::from("data/runs"))
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// `20261009-154349337-1890849` -> `2026-10`.
fn shard_name(id: &str) -> String {
    match (id.get(0..4), id.get(4..6)) {
        (Some(y), Some(m)) if id.as_bytes()[..6].iter().all(u8::is_ascii_digit) => {
            format!("{y}-{m}")
        }
        _ => "unknown".to_string(),
    }
}

fn existing_ids(dir: &Path) -> Result<HashSet<String>> {
    let mut ids = HashSet::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(ids);
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "jsonl") {
            continue;
        }
        for line in std::fs::read_to_string(&p)?.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line)
                && let Some(id) = v.get("id").and_then(Value::as_str)
            {
                ids.insert(id.to_string());
            }
        }
    }
    Ok(ids)
}

/// Repo names allowed to keep details, one per line, `#` comments.
fn load_public_repos(root: &Path) -> HashSet<String> {
    std::fs::read_to_string(root.join("public-repos.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

const PRIVATE: &str = "private";

/// Keep only numbers and non-identifying structure.
fn redact(obj: &mut Map<String, Value>) {
    for k in ["cwd", "git", "slot", "remote"] {
        obj.remove(k);
    }
    let sub = obj
        .get("subcommand")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    obj.insert("args".into(), Value::Array(vec![Value::String(sub)]));
    let reasons = obj
        .remove("rebuild_reasons")
        .and_then(|r| r.as_array().map(Vec::len))
        .unwrap_or(0);
    obj.insert("rebuild_reasons_count".into(), reasons.into());
    if let Some(Value::Array(units)) = obj.get_mut("top_units") {
        for (i, u) in units.iter_mut().enumerate() {
            if let Some(u) = u.as_object_mut() {
                let suffix = u
                    .get("name")
                    .and_then(Value::as_str)
                    .and_then(|n| n.find(" (").map(|p| n[p..].to_string()))
                    .unwrap_or_default();
                u.insert("name".into(), format!("crate{i}{suffix}").into());
            }
        }
    }
    if let Some(Value::Array(bins)) = obj.get_mut("tests").and_then(|t| t.get_mut("binaries")) {
        for (i, b) in bins.iter_mut().enumerate() {
            if let Some(b) = b.as_object_mut() {
                b.insert("name".into(), format!("bin{i}").into());
            }
        }
    }
    if let Some(e) = obj
        .get_mut("diagnostics")
        .and_then(|d| d.get_mut("first_error"))
        && let Value::String(s) = e
    {
        // `error[E0308]: ...` -> `error[E0308]`, anything else -> `error`.
        let code = s.find(']').map(|i| &s[..=i]).unwrap_or("error");
        *e = Value::String(code.to_string());
    }
}

/// Turn a full `summary.json` into the exported line.
pub fn slim(mut v: Value, home: &Path, machine: &str, public: &HashSet<String>) -> Value {
    let home = home.to_string_lossy().trim_end_matches('/').to_string();
    let Some(obj) = v.as_object_mut() else {
        return v;
    };
    obj.remove("split_hints");
    if let Some(Value::Array(units)) = obj.get_mut("top_units") {
        units.truncate(TOP_UNITS);
        for u in units.iter_mut() {
            if let Some(Value::Array(passes)) = u.get_mut("top_passes") {
                passes.truncate(TOP_PASSES);
            }
        }
    }
    if let Some(Value::String(e)) = obj
        .get_mut("diagnostics")
        .and_then(|d| d.get_mut("first_error"))
    {
        let first = e.lines().next().unwrap_or("");
        let cut = first.floor_char_boundary(ERROR_CHARS.min(first.len()));
        *e = first[..cut].to_string();
    }
    let cwd = obj
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let git_root = obj
        .get("git")
        .and_then(|g| g.get("root"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let has_session = obj
        .get("agent_session")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    let kind = classify(&cwd, &home, has_session);
    let mut repo = repo_name(git_root.as_deref(), &cwd);
    if !public.contains(&repo) {
        redact(obj);
        repo = PRIVATE.to_string();
    }

    rewrite(&mut v, &home);
    let obj = v.as_object_mut().expect("still an object");
    obj.insert("machine".into(), Value::String(machine.to_string()));
    obj.insert("repo".into(), Value::String(repo));
    obj.insert("kind".into(), Value::String(kind.to_string()));
    v
}

/// `bench`: justrust benchmark copies. `scratch`: throwaway copies and temp
/// dirs. `agent`: an agent session in a real checkout. `manual`: the rest.
fn classify(cwd: &str, home: &str, has_session: bool) -> &'static str {
    let rel = cwd.strip_prefix(home).unwrap_or(cwd);
    if rel.starts_with("/.justrust/bench") {
        "bench"
    } else if rel.starts_with("/.jcode/scratch")
        || rel.starts_with("/.justrust/")
        || cwd.starts_with("/tmp/")
        || rel
            .trim_start_matches('/')
            .split('/')
            .next()
            .is_some_and(|top| top.starts_with("justrust-"))
    {
        "scratch"
    } else if has_session {
        "agent"
    } else {
        "manual"
    }
}

/// The checkout's directory name: the git root when known, else the cwd.
fn repo_name(git_root: Option<&str>, cwd: &str) -> String {
    let base = git_root.unwrap_or(cwd).trim_end_matches('/');
    let name = base.rsplit('/').next().unwrap_or("");
    // A crate directory inside a workspace says little; prefer the workspace.
    if git_root.is_none()
        && let Some(i) = cwd.find("/crates/")
    {
        return cwd[..i].rsplit('/').next().unwrap_or(name).to_string();
    }
    name.to_string()
}

/// Rewrite home-directory paths to `~` and round floats to milliseconds.
fn rewrite(v: &mut Value, home: &str) {
    match v {
        Value::String(s) => {
            if !home.is_empty() && s.contains(home) {
                *s = s.replace(&format!("{home}/"), "~/").replace(home, "~");
            }
        }
        Value::Number(n) => {
            if n.is_f64()
                && let Some(f) = n.as_f64()
            {
                let r = (f * 1000.0).round() / 1000.0;
                let r = if r == 0.0 { 0.0 } else { r };
                if let Some(num) = serde_json::Number::from_f64(r) {
                    *n = num;
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| rewrite(x, home)),
        Value::Object(o) => {
            let o: &mut Map<String, Value> = o;
            o.values_mut().for_each(|x| rewrite(x, home));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "id": "20261009-154349337-1890849",
            "cwd": "/home/u/jcode-desktop/crates/ui",
            "args": ["test", "--manifest-path", "/home/u/jcode-desktop/Cargo.toml"],
            "git": {"root": "/home/u/jcode-desktop", "head": "abc", "dirty_files": 2},
            "agent_session": "session_x",
            "wall": 9.123456789,
            "exit": 0,
            "top_units": (0..10).map(|i| json!({
                "name": format!("u{i}"),
                "top_passes": (0..8).map(|p| json!({"name": format!("p{p}"), "secs": 0.1})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "diagnostics": {"first_error": "error[E0308]: mismatched types\n --> src/a.rs:1:1\n  | secret code"},
            "rebuild_reasons": [{"package": "x", "reason": "the file `/home/u/jcode/src/lib.rs` has changed"}],
            "slot": {"target_dir": "/home/u/jcode-desktop/target/justrust-slots/0"},
            "split_hints": [{"text": "move things"}],
        })
    }

    fn public() -> HashSet<String> {
        ["jcode-desktop".to_string()].into_iter().collect()
    }

    #[test]
    fn slims_and_redacts() {
        let v = slim(sample(), Path::new("/home/u"), "box", &public());
        let s = serde_json::to_string(&v).unwrap();
        assert!(!s.contains("/home/u"), "{s}");
        assert!(!s.contains("secret code"));
        assert_eq!(
            v["diagnostics"]["first_error"],
            "error[E0308]: mismatched types"
        );
        assert_eq!(v["cwd"], "~/jcode-desktop/crates/ui");
        assert_eq!(
            v["rebuild_reasons"][0]["reason"],
            "the file `~/jcode/src/lib.rs` has changed"
        );
        assert_eq!(v["top_units"].as_array().unwrap().len(), TOP_UNITS);
        assert_eq!(
            v["top_units"][0]["top_passes"].as_array().unwrap().len(),
            TOP_PASSES
        );
        assert!(v.get("split_hints").is_none());
        assert_eq!(v["wall"], 9.123);
        assert_eq!(v["exit"], 0);
        assert_eq!(v["machine"], "box");
        assert_eq!(v["repo"], "jcode-desktop");
        assert_eq!(v["kind"], "agent");
    }

    #[test]
    fn private_repos_keep_only_numbers() {
        let v = slim(sample(), Path::new("/home/u"), "box", &HashSet::new());
        let s = serde_json::to_string(&v).unwrap();
        for leak in [
            "jcode",
            "/home",
            "~/",
            "mismatched",
            "manifest-path",
            "lib.rs",
        ] {
            assert!(!s.contains(leak), "{leak} in {s}");
        }
        assert_eq!(v["repo"], PRIVATE);
        assert_eq!(v["args"], json!([""]));
        assert_eq!(v["diagnostics"]["first_error"], "error[E0308]");
        assert_eq!(v["top_units"][0]["name"], "crate0");
        assert_eq!(v["rebuild_reasons_count"], 1);
        assert_eq!(v["wall"], 9.123);
        assert_eq!(v["kind"], "agent");
    }

    #[test]
    fn allowlist_parsing() {
        let dir = std::env::temp_dir().join(format!("jr-export-allow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("public-repos.txt"),
            "# public\njcode\n  gpui  # fork\n\n",
        )
        .unwrap();
        let p = load_public_repos(&dir);
        assert_eq!(p.len(), 2);
        assert!(p.contains("jcode") && p.contains("gpui"));
        assert!(load_public_repos(&dir.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classifies_runs() {
        let h = "/home/u";
        assert_eq!(
            classify("/home/u/.justrust/bench/work/desktop/x", h, true),
            "bench"
        );
        assert_eq!(
            classify("/home/u/.jcode/scratch/tp/proj", h, true),
            "scratch"
        );
        assert_eq!(classify("/home/u/justrust-wyvern", h, true), "scratch");
        assert_eq!(classify("/tmp/x", h, false), "scratch");
        assert_eq!(classify("/home/u/jcode", h, true), "agent");
        assert_eq!(classify("/home/u/jcode", h, false), "manual");
    }

    #[test]
    fn repo_names() {
        assert_eq!(
            repo_name(Some("/home/u/jcode"), "/home/u/jcode/crates/a"),
            "jcode"
        );
        assert_eq!(
            repo_name(None, "/home/u/jcode-desktop/crates/ui"),
            "jcode-desktop"
        );
        assert_eq!(
            repo_name(None, "/home/u/.justrust/bench/work/d/jcode-desktop"),
            "jcode-desktop"
        );
    }

    #[test]
    fn shards_by_month() {
        assert_eq!(shard_name("20261009-154349337-1890849"), "2026-10");
        assert_eq!(shard_name("bogus"), "unknown");
    }

    #[test]
    fn export_is_incremental() {
        let dir = std::env::temp_dir().join(format!("jr-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("2026-10.jsonl"),
            "{\"id\":\"a\"}\n{\"id\":\"b\"}\nnot json\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "{\"id\":\"c\"}\n").unwrap();
        let ids = existing_ids(&dir).unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("a") && ids.contains("b"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
