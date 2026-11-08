//! Slim run summaries for the account-private record store.
//!
//! Every recorded build's `summary.json` is uploaded as a `run` record
//! (`upload_run`), cut to what analysis needs: home paths rewritten to `~`,
//! the first error cut to one line, the slowest units and passes only,
//! floats rounded, and `machine`, `repo`, `kind` added. `justrust export`
//! and committing `data/runs/` are retired; that directory is a frozen
//! archive of runs cited by older docs.

use anyhow::{Result, bail};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::Path;

const TOP_UNITS: usize = 6;
const TOP_PASSES: usize = 5;
const ERROR_CHARS: usize = 200;

/// `justrust export` is retired: runs go to the account record store.
pub fn command() -> Result<()> {
    bail!(
        "`justrust export` is retired. Every recorded build is uploaded automatically to the account-private record store: use `justrust records list` and `justrust records show run <id>`. data/runs/ is a frozen archive."
    )
}

/// Slim summary for the account-private record store (`upload_run`).
/// Private to the account, so names, args, and paths stay (home rewritten
/// to `~`), only size is cut like the export.
pub fn upload_run(s: &crate::summary::Summary) {
    if !crate::remote_hosted::records_enabled() || !crate::remote_hosted::signed_in() {
        return;
    }
    let Ok(v) = serde_json::to_value(s) else {
        return;
    };
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let mut public = HashSet::new();
    let git_root = v
        .get("git")
        .and_then(|g| g.get("root"))
        .and_then(Value::as_str);
    let cwd = v.get("cwd").and_then(Value::as_str).unwrap_or("");
    public.insert(repo_name(git_root, cwd));
    let data = slim(v, &home, &crate::procfs::hostname(), &public);
    crate::remote_hosted::upload_record("run", &s.id, data);
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
    if let Some(t) = obj.get_mut("tests").and_then(Value::as_object_mut) {
        t.remove("slowest");
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
    if let Some(Value::Array(slow)) = obj.get_mut("tests").and_then(|t| t.get_mut("slowest")) {
        slow.truncate(5);
    }
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
}
