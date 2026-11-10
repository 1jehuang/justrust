//! Why a dependency unit missed the shared cache, decided at the miss.
//!
//! The base key is one hash, so a miss alone says nothing about which input
//! moved. Every stored unit therefore leaves its key parts (toolchain, flags,
//! each `--extern` artifact, native library dirs, keyed env, source path) in
//! a small per-package ledger, `<cache>/w/<package>.jsonl`. A later miss of
//! the same package is compared with the closest ledger line and named after
//! the parts that differ:
//!
//! | reason | meaning |
//! |---|---|
//! | `evicted` | this exact key was stored before, its outputs are gone |
//! | `inputs-changed` | same key, but a source file or tracked env var differs |
//! | `first-seen` | no ledger line for this package (or this target of it) |
//! | `new-version` | the package is known, this version is not |
//! | `toolchain` | another compiler |
//! | `flags` | rustc arguments differ (profile, features, emit, target) |
//! | `dependency` | an `--extern` artifact has other bytes (a cascade) |
//! | `native-lib` | a build-script library dir has other content |
//! | `env` | a keyed environment variable differs |
//! | `source-path` | the package source dir differs |
//! | `unit-hash` | only cargo's own unit hash (`-C metadata`) differs |
//! | `key-scheme` | all parts equal: the key scheme itself moved |
//! | `compile-failed` | the compile failed, nothing could be stored |
//!
//! Several parts can differ at once (`flags+dependency`), causes before
//! consequences: the first one is the category reports count. The text after
//! `: ` names the differing flags, dependencies or variables. Only misses pay
//! for this (one manifest and one ledger read), hits never do. Every error
//! means "no reason recorded".

use crate::depcache::{self, Entry, H128, Plan};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Ledger lines kept per package: enough for its versions, profiles and
/// toolchains in use, small enough to read on every miss.
const KEEP: usize = 48;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub(crate) struct Parts {
    pub(crate) base: String,
    pub(crate) version: String,
    pub(crate) crate_name: String,
    pub(crate) tc: String,
    pub(crate) cwd: String,
    pub(crate) args: Vec<String>,
    pub(crate) externs: Vec<(String, String)>,
    pub(crate) native: String,
    pub(crate) env: Vec<(String, String)>,
}

/// What the shim keeps between the miss and the store.
pub(crate) struct Ticket {
    ledger: PathBuf,
    parts: Parts,
}

fn h12(b: &[u8]) -> String {
    let mut h = H128::new();
    h.field(b);
    h.hex()[..12].to_owned()
}

fn ledger_path(root: &Path, package: &str, crate_name: &str) -> PathBuf {
    let name = if package.is_empty() {
        crate_name
    } else {
        package
    };
    let safe: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    root.join("w").join(format!("{safe}.jsonl"))
}

fn parts(plan: &Plan, real: &Path, args: &[OsString]) -> Option<Parts> {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    let mut p = Parts {
        base: plan.base.clone(),
        version: std::env::var("CARGO_PKG_VERSION").unwrap_or_default(),
        crate_name: plan.crate_name.clone(),
        tc: h12(depcache::rustc_version(real)?.as_bytes()),
        cwd: plan.norm.norm(plan.cwd.to_str()?),
        ..Default::default()
    };
    let mut native = H128::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next = args.get(i + 1).map(String::as_str);
        if a.starts_with("--diagnostic-width") {
        } else if a == "--extern" {
            let v = next?;
            match v.split_once('=') {
                Some((name, path)) => p.externs.push((
                    name.to_owned(),
                    depcache::extern_hash(&plan.cwd.join(path))?[..12].to_owned(),
                )),
                None => p.args.push(format!("--extern {v}")),
            }
            i += 1;
        } else if a == "-L" || a == "-C" {
            let v = next?;
            if a == "-L" {
                depcache::key_lib_dir(v, &plan.norm, &plan.cwd, &mut native)?;
            }
            p.args.push(format!("{a} {}", plan.norm.norm(v)));
            i += 1;
        } else {
            if let Some(v) = a.strip_prefix("-L") {
                depcache::key_lib_dir(v, &plan.norm, &plan.cwd, &mut native)?;
            }
            p.args.push(plan.norm.norm(a));
        }
        i += 1;
    }
    p.native = native.hex()[..12].to_owned();
    p.env = std::env::vars()
        .filter(|(k, _)| depcache::keyed_env(k))
        .map(|(k, v)| {
            let v = h12(plan.norm.norm(&v).as_bytes());
            (k, v)
        })
        .collect();
    p.env.sort();
    Some(p)
}

fn read_ledger(path: &Path) -> Vec<Parts> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn short(s: &str) -> String {
    match s.char_indices().nth(70) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_owned(),
    }
}

fn names(v: Vec<String>) -> String {
    let more = v.len().saturating_sub(4);
    let mut s = v.into_iter().take(4).collect::<Vec<_>>().join(", ");
    if more > 0 {
        s.push_str(&format!(" (+{more})"));
    }
    s
}

/// Pairs present or different in `new` relative to `old`, by name.
fn changed(new: &[(String, String)], old: &[(String, String)]) -> Vec<String> {
    let mut v: Vec<String> = new
        .iter()
        .filter(|n| !old.contains(n))
        .map(|(k, _)| k.clone())
        .chain(
            old.iter()
                .filter(|(k, _)| !new.iter().any(|(n, _)| n == k))
                .map(|(k, _)| format!("-{k}")),
        )
        .collect();
    v.dedup();
    v
}

/// The reason from the ledger alone (the manifest was already ruled out).
pub(crate) fn classify(now: &Parts, ledger: &[Parts]) -> String {
    if ledger.is_empty() {
        return "first-seen: package".into();
    }
    let same: Vec<&Parts> = ledger.iter().filter(|l| l.version == now.version).collect();
    if same.is_empty() {
        let mut had: Vec<String> = ledger.iter().map(|l| l.version.clone()).collect();
        had.sort();
        had.dedup();
        return format!("new-version: had {}", names(had));
    }
    let same: Vec<&Parts> = same
        .into_iter()
        .filter(|l| l.crate_name == now.crate_name)
        .collect();
    if same.is_empty() {
        return "first-seen: target".into();
    }
    if same.iter().any(|l| l.base == now.base) {
        return "evicted".into();
    }
    let score = |l: &Parts| {
        [
            l.tc == now.tc,
            l.args == now.args,
            l.externs == now.externs,
            l.native == now.native,
            l.env == now.env,
            l.cwd == now.cwd,
        ]
        .iter()
        .filter(|b| **b)
        .count()
    };
    // Latest line wins ties.
    let Some(best) = same.iter().rev().max_by_key(|l| score(l)) else {
        return String::new();
    };
    // Cargo derives `-C metadata`, `-C extra-filename`, the build dir names
    // and `OUT_DIR` from the unit's other inputs: they follow a cause and
    // are named only when nothing else moved.
    let derived = |a: &str| {
        a.starts_with("-C metadata=")
            || a.starts_with("-C extra-filename=")
            || a.contains("@@JUSTRUST_PROFILE_DIR@@")
    };
    let causal =
        |v: &[String]| -> Vec<String> { v.iter().filter(|a| !derived(a)).cloned().collect() };
    let causal_env = |v: &[(String, String)]| -> Vec<(String, String)> {
        v.iter().filter(|(k, _)| k != "OUT_DIR").cloned().collect()
    };
    let (args_now, args_old) = (causal(&now.args), causal(&best.args));
    let (env_now, env_old) = (causal_env(&now.env), causal_env(&best.env));
    // Causes first, consequences last: the first label is the category.
    let mut labels = Vec::new();
    let mut detail = Vec::new();
    if best.tc != now.tc {
        labels.push("toolchain");
    }
    if args_old != args_now {
        labels.push("flags");
        let added: Vec<String> = args_now
            .iter()
            .filter(|a| !args_old.contains(a))
            .map(|a| short(a))
            .chain(
                args_old
                    .iter()
                    .filter(|a| !args_now.contains(a))
                    .map(|a| format!("-{}", short(a))),
            )
            .collect();
        detail.push(names(added));
    }
    if env_old != env_now {
        labels.push("env");
        detail.push(names(changed(&env_now, &env_old)));
    }
    if best.cwd != now.cwd {
        labels.push("source-path");
    }
    if best.externs != now.externs {
        labels.push("dependency");
        detail.push(names(changed(&now.externs, &best.externs)));
    }
    if best.native != now.native {
        labels.push("native-lib");
    }
    if labels.is_empty() {
        // Only cargo's own unit hash moved (resolver, source id, profile
        // setting that reaches no rustc flag), or nothing visible at all.
        return match best.args != now.args || best.env != now.env {
            true => "unit-hash".into(),
            false => "key-scheme".into(),
        };
    }
    detail.retain(|d| !d.is_empty());
    match detail.is_empty() {
        true => labels.join("+"),
        false => format!("{}: {}", labels.join("+"), detail.join("; ")),
    }
}

/// The reason when the base key already has manifest lines.
fn from_manifest(plan: &Plan) -> Option<String> {
    let text = std::fs::read_to_string(plan.manifest()).ok()?;
    let mut memo = HashMap::new();
    let entries: Vec<Entry> = text
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let newest = entries.first()?;
    if entries.iter().any(|e| plan.matches(e, &mut memo)) {
        return Some("evicted".into());
    }
    let env = newest
        .env
        .iter()
        .find(|(k, v)| std::env::var(k).ok().map(|x| plan.norm.norm(&x)).as_ref() != v.as_ref());
    if let Some((k, _)) = env {
        return Some(format!("inputs-changed: env {k}"));
    }
    let file = newest.inputs.iter().find(|(p, hash)| {
        depcache::file_hash(Path::new(&plan.norm.denorm(p)), false).as_deref()
            != Some(hash.as_str())
    });
    Some(match file {
        Some((p, _)) => format!(
            "inputs-changed: {}",
            p.rsplit('/').next().unwrap_or(p.as_str())
        ),
        None => "inputs-changed".into(),
    })
}

/// Called after every tier missed and before the compile. Returns the reason
/// (empty when it cannot be determined) and the ticket to `remember`.
pub(crate) fn diagnose(plan: &Plan, real: &Path, args: &[OsString]) -> (String, Option<Ticket>) {
    let Some(parts) = parts(plan, real, args) else {
        return (from_manifest(plan).unwrap_or_default(), None);
    };
    let package = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
    let ledger = ledger_path(&plan.root, &package, &plan.crate_name);
    let why = from_manifest(plan).unwrap_or_else(|| classify(&parts, &read_ledger(&ledger)));
    (why, Some(Ticket { ledger, parts }))
}

/// The unit was stored: leave its key parts for later misses.
pub(crate) fn remember(t: &Ticket) {
    let known = read_ledger(&t.ledger);
    if known.iter().any(|l| l.base == t.parts.base) {
        return;
    }
    let Ok(line) = serde_json::to_string(&t.parts) else {
        return;
    };
    if let Some(dir) = t.ledger.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if known.len() < 2 * KEEP {
        let _ = paths::append_line(&t.ledger, &line);
        return;
    }
    // Compact: the newest lines, rewritten atomically. A line appended by a
    // concurrent shim in between can be lost, which costs one vaguer reason.
    let mut body: Vec<String> = known[known.len() - KEEP..]
        .iter()
        .filter_map(|p| serde_json::to_string(p).ok())
        .collect();
    body.push(line);
    let tmp = t
        .ledger
        .with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, body.join("\n") + "\n").is_err()
        || std::fs::rename(&tmp, &t.ledger).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The reason without its detail: the unit of aggregation. With several
/// moved parts it is the first, the likeliest cause (`flags+dependency`
/// counts as `flags`: the dependency was rebuilt with those flags too).
pub(crate) fn category(why: &str) -> &str {
    let head = why.split_once(':').map_or(why, |(c, _)| c);
    match head.split_once('+') {
        Some((first, _)) => first,
        None if head.is_empty() => "unrecorded",
        None => head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(base: &str) -> Parts {
        Parts {
            base: base.into(),
            version: "1.0.0".into(),
            crate_name: "x".into(),
            tc: "t1".into(),
            cwd: "/src/x".into(),
            args: vec!["--edition=2021".into(), "-C opt-level=0".into()],
            externs: vec![("a".into(), "h1".into()), ("b".into(), "h2".into())],
            native: "n".into(),
            env: vec![("SDKROOT".into(), "e1".into())],
        }
    }

    #[test]
    fn names_the_part_that_moved() {
        let old = p("k1");
        assert_eq!(classify(&p("k2"), &[]), "first-seen: package");
        assert_eq!(classify(&old, std::slice::from_ref(&old)), "evicted");
        assert_eq!(classify(&p("k2"), std::slice::from_ref(&old)), "key-scheme");

        let mut v = p("k2");
        v.version = "1.0.1".into();
        assert_eq!(
            classify(&v, std::slice::from_ref(&old)),
            "new-version: had 1.0.0"
        );

        let mut t = p("k2");
        t.crate_name = "build_script_build".into();
        assert_eq!(
            classify(&t, std::slice::from_ref(&old)),
            "first-seen: target"
        );

        let mut d = p("k2");
        d.externs[1].1 = "other".into();
        assert_eq!(classify(&d, std::slice::from_ref(&old)), "dependency: b");

        let mut f = p("k2");
        f.args[1] = "-C opt-level=3".into();
        f.tc = "t2".into();
        assert_eq!(
            classify(&f, std::slice::from_ref(&old)),
            "toolchain+flags: -C opt-level=3, --C opt-level=0"
        );

        let mut e = p("k2");
        e.env.clear();
        assert_eq!(classify(&e, std::slice::from_ref(&old)), "env: -SDKROOT");
    }

    #[test]
    fn compares_with_the_closest_line() {
        // A release line and a debug line: a debug miss whose dependency
        // moved is a dependency miss, not a flags miss.
        let debug = p("k1");
        let mut release = p("k2");
        release.args[1] = "-C opt-level=3".into();
        let mut now = p("k3");
        now.externs[0].1 = "new".into();
        assert_eq!(classify(&now, &[debug, release]), "dependency: a");
        assert_eq!(category("dependency: a"), "dependency");
        assert_eq!(category(""), "unrecorded");
        assert_eq!(category("evicted"), "evicted");
        assert_eq!(category("flags+dependency: -C opt-level=3; a"), "flags");
    }

    #[test]
    fn derived_parts_are_not_causes() {
        let mut old = p("k1");
        old.args.push("-C metadata=aaaa".into());
        old.env.push(("OUT_DIR".into(), "o1".into()));
        let mut now = p("k2");
        now.args.push("-C metadata=bbbb".into());
        now.env.push(("OUT_DIR".into(), "o2".into()));
        assert_eq!(classify(&now, std::slice::from_ref(&old)), "unit-hash");
        now.externs[0].1 = "new".into();
        assert_eq!(classify(&now, std::slice::from_ref(&old)), "dependency: a");
    }

    #[test]
    fn ledger_is_bounded_and_deduplicated() {
        let dir = std::env::temp_dir().join(format!("jr-why-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ledger = ledger_path(&dir, "se/rde", "x");
        assert!(ledger.ends_with("w/se_rde.jsonl"));
        for i in 0..(3 * KEEP) {
            let t = Ticket {
                ledger: ledger.clone(),
                parts: p(&format!("k{i}")),
            };
            remember(&t);
            remember(&t);
        }
        let got = read_ledger(&ledger);
        assert!(got.len() <= 2 * KEEP, "{}", got.len());
        assert_eq!(got.last().unwrap().base, format!("k{}", 3 * KEEP - 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
