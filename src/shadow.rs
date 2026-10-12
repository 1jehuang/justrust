//! Shadow leaf crates (`JUSTRUST_SHADOW=1`, opt-in, FINDINGS 47).
//!
//! `justrust test -p <big> --lib -- <m>` where `<m>` is a top-level module
//! of a big crate compiles and links the whole crate's test binary to run a
//! handful of tests. When `<m>` is a true leaf (its file reaches nothing
//! else in the crate) and only that file changed since the last good build
//! of the big crate, the same tests run in a tiny crate instead:
//!
//! ```text
//! ~/.justrust/shadow/<workspace hash>/
//!   Cargo.toml                       workspace: one member, the user's
//!                                    [patch] and [profile], target-dir = the
//!                                    real one (dependencies stay warm)
//!   Cargo.lock                       copied from the workspace
//!   .cargo/config.toml               the workspace's, relative paths made absolute
//!   jr-shadow-<m>/Cargo.toml         every dependency of <big>, resolved
//!   jr-shadow-<m>/src/lib.rs         #[path = "<real file>"] pub mod <m>;
//!   <big>.snap                       <big>'s other sources at its last good build
//! ```
//!
//! The leaf includes the user's file in place (no copy, edits are live) and
//! test names stay `<m>::tests::*`. The user's tree is never written.
//! Anything unexpected falls back to the normal build: a command of another
//! shape, a filter that could match tests outside the leaf, a module that
//! uses `crate::`, `super::` at its top level, child modules, macros, build
//! script env, or features. Caveat: the big crate's own uses of the leaf
//! are only rechecked by the next normal build.

use crate::paths;
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ENV: &str = "JUSTRUST_SHADOW";

pub fn enabled() -> bool {
    std::env::var(ENV).is_ok_and(|v| matches!(v.as_str(), "1" | "on" | "true"))
}

/// The routable command: `test -p <pkg> --lib -- <filter> [libtest flags]`.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub package: String,
    pub module: String,
    pub filter: String,
    /// Cargo flags kept as given (`--locked`, `--offline`, `-q`).
    pub cargo_flags: Vec<String>,
    /// Everything after `--`.
    pub test_args: Vec<String>,
}

pub fn parse_request(full: &[OsString]) -> Option<Request> {
    let args: Vec<String> = full
        .iter()
        .map(|a| a.to_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    let (sub, rest) = args.split_first()?;
    if sub != "test" && sub != "t" {
        return None;
    }
    let dd = rest.iter().position(|a| a == "--")?;
    let (before, after) = (&rest[..dd], &rest[dd + 1..]);
    let mut package = None;
    let mut lib = false;
    let mut cargo_flags = Vec::new();
    let mut it = before.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" | "--package" => package = Some(it.next()?.clone()),
            "--lib" => lib = true,
            "--locked" | "--offline" | "--frozen" | "-q" | "--quiet" => cargo_flags.push(a.clone()),
            s if s.starts_with("--package=") => package = Some(s[10..].to_owned()),
            s if s.starts_with("-p") && s.len() > 2 => package = Some(s[2..].to_owned()),
            _ => return None,
        }
    }
    if !lib {
        return None;
    }
    let package = package?;
    let mut filter = None;
    for a in after {
        let ok_flag = matches!(
            a.as_str(),
            "--nocapture" | "--show-output" | "-q" | "--quiet" | "--color=never" | "--color=always"
        ) || a.starts_with("--test-threads=");
        if ok_flag {
            continue;
        }
        if a.starts_with('-') || filter.is_some() {
            return None;
        }
        filter = Some(a.clone());
    }
    let filter = filter?;
    let module = filter.split("::").next()?.to_owned();
    let ident_ok = !module.is_empty()
        && module
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !module.starts_with(|c: char| c.is_ascii_digit());
    if !ident_ok || module == "tests" {
        return None;
    }
    Some(Request {
        package,
        module,
        filter,
        cargo_flags,
        test_args: after.to_vec(),
    })
}

/// Why a module file is not a self-contained leaf, or `None` if it is.
pub fn leaf_blocker(src: &str) -> Option<String> {
    let mut depth_mods: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    for (i, raw) in src.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("");
        let n = i + 1;
        let t = line.trim();
        for bad in [
            "crate::",
            "$crate",
            "super::super",
            "#[path",
            "include!",
            "env!(",
            "cfg(feature",
            "macro_rules!",
            "#[macro_use",
        ] {
            if line.contains(bad) {
                return Some(format!("line {n} uses `{bad}`"));
            }
        }
        if (t.starts_with("mod ") || t.contains(" mod ")) && t.ends_with(';') {
            return Some(format!("line {n} declares a child module file"));
        }
        let opens_mod = t.starts_with("mod ") || t.contains(" mod ");
        if line.contains("super::") && depth_mods.is_empty() {
            return Some(format!("line {n} reaches the parent with `super::`"));
        }
        for c in line.chars() {
            match c {
                '{' => {
                    depth += 1;
                    if opens_mod && depth_mods.last() != Some(&depth) {
                        depth_mods.push(depth);
                    }
                }
                '}' => {
                    if depth_mods.last() == Some(&depth) {
                        depth_mods.pop();
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
    }
    None
}

/// The root file declares `mod <m>;` (any visibility, no inline body).
pub fn declares_module(root_src: &str, m: &str) -> bool {
    root_src.lines().any(|l| {
        let t = l.split("//").next().unwrap_or("").trim();
        let Some(rest) = t.strip_suffix(';') else {
            return false;
        };
        let rest = rest.trim_end();
        let words: Vec<&str> = rest.split_whitespace().collect();
        words.len() >= 2 && words[words.len() - 2] == "mod" && words[words.len() - 1] == m
    })
}

/// Could `filter` match a test outside the leaf? True when another source
/// file names a function or module containing it.
pub fn filter_escapes(other_src: &str, filter: &str, module: &str) -> bool {
    let tail = filter.rsplit("::").next().unwrap_or(filter);
    other_src.lines().any(|l| {
        let t = l.split("//").next().unwrap_or("");
        if declares_module(t, module) {
            return false;
        }
        t.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| {
                (w[0] == "fn" || w[0] == "mod") && (w[1].contains(tail) || w[1].contains(module))
            })
    })
}

pub fn leaf_package(m: &str) -> String {
    format!("jr-shadow-{}", m.replace('_', "-"))
}

pub fn lib_rs(m: &str, real_file: &Path) -> String {
    format!(
        "//! Generated by justrust (JUSTRUST_SHADOW). Do not edit.\n\
         #![allow(dead_code, unused_imports, unreachable_pub)]\n\
         #[path = {:?}]\npub mod {m};\n",
        real_file.to_string_lossy()
    )
}

/// One dependency entry from `cargo metadata` as a TOML inline table.
fn dep_table(d: &serde_json::Value) -> Option<(String, toml_edit::InlineTable)> {
    let name = d["name"].as_str()?;
    let mut t = toml_edit::InlineTable::new();
    let key = match d["rename"].as_str() {
        Some(r) => {
            t.insert("package", name.into());
            r.to_owned()
        }
        None => name.to_owned(),
    };
    if let Some(p) = d["path"].as_str() {
        t.insert("path", p.into());
    } else {
        let src = d["source"].as_str()?;
        if let Some(git) = src.strip_prefix("git+") {
            let (url, q) = git.split_once('?').unwrap_or((git, ""));
            t.insert("git", url.split('#').next().unwrap_or(url).into());
            for kv in q.split('&') {
                if let Some((k, v)) = kv.split_once('=')
                    && matches!(k, "rev" | "branch" | "tag")
                {
                    t.insert(k, v.split('#').next().unwrap_or(v).into());
                }
            }
        } else if !src.starts_with("registry+https://github.com/rust-lang/crates.io-index")
            && !src.starts_with("sparse+https://index.crates.io")
        {
            return None;
        }
        if !src.starts_with("git+") {
            t.insert("version", d["req"].as_str().unwrap_or("*").into());
        }
    }
    if d["uses_default_features"].as_bool() == Some(false) {
        t.insert("default-features", false.into());
    }
    let feats: toml_edit::Array = d["features"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_str())
        .collect();
    if !feats.is_empty() {
        t.insert("features", toml_edit::Value::Array(feats));
    }
    Some((key, t))
}

/// The leaf crate's manifest: every non-optional normal and dev dependency
/// of the big package, so feature resolution (and so the dependency
/// artifacts) match `test -p <big>`.
pub fn leaf_manifest(pkg: &serde_json::Value, m: &str) -> Result<String> {
    let mut doc = toml_edit::DocumentMut::new();
    let mut package = toml_edit::Table::new();
    package["name"] = toml_edit::value(leaf_package(m));
    package["version"] = toml_edit::value("0.0.0");
    package["edition"] = toml_edit::value(pkg["edition"].as_str().unwrap_or("2021"));
    package["publish"] = toml_edit::value(false);
    doc["package"] = toml_edit::Item::Table(package);
    let mut lib = toml_edit::Table::new();
    lib["path"] = toml_edit::value("src/lib.rs");
    lib["doctest"] = toml_edit::value(false);
    doc["lib"] = toml_edit::Item::Table(lib);
    let mut sections: BTreeMap<(Option<String>, &str), toml_edit::Table> = BTreeMap::new();
    for d in pkg["dependencies"].as_array().into_iter().flatten() {
        if d["optional"].as_bool() == Some(true) {
            continue;
        }
        let kind = match d["kind"].as_str() {
            None => "dependencies",
            Some("dev") => "dev-dependencies",
            Some(_) => continue,
        };
        let Some((key, t)) = dep_table(d) else {
            bail!("unsupported dependency source for {}", d["name"]);
        };
        let target = d["target"].as_str().map(str::to_owned);
        let sec = sections.entry((target, kind)).or_default();
        if !sec.contains_key(&key) {
            sec.insert(&key, toml_edit::value(t));
        }
    }
    for ((target, kind), t) in sections {
        match target {
            None => doc[kind] = toml_edit::Item::Table(t),
            Some(cfg) => {
                if !doc.contains_key("target") {
                    let mut tt = toml_edit::Table::new();
                    tt.set_implicit(true);
                    doc["target"] = toml_edit::Item::Table(tt);
                }
                if doc["target"].get(&cfg).is_none() {
                    let mut ct = toml_edit::Table::new();
                    ct.set_implicit(true);
                    doc["target"][&cfg] = toml_edit::Item::Table(ct);
                }
                doc["target"][&cfg][kind] = toml_edit::Item::Table(t);
            }
        }
    }
    Ok(doc.to_string())
}

/// Make relative path strings absolute against `base` when they exist there.
fn absolutize(item: &mut toml_edit::Item, base: &Path) {
    fn walk_value(v: &mut toml_edit::Value, base: &Path) {
        match v {
            toml_edit::Value::String(s) => {
                let val = s.value().clone();
                if !val.is_empty() && !Path::new(&val).is_absolute() && base.join(&val).exists() {
                    *v = toml_edit::Value::from(base.join(&val).to_string_lossy().into_owned());
                }
            }
            toml_edit::Value::Array(a) => a.iter_mut().for_each(|x| walk_value(x, base)),
            toml_edit::Value::InlineTable(t) => t.iter_mut().for_each(|(_, x)| walk_value(x, base)),
            _ => {}
        }
    }
    match item {
        toml_edit::Item::Value(v) => walk_value(v, base),
        toml_edit::Item::Table(t) => t.iter_mut().for_each(|(_, x)| absolutize(x, base)),
        toml_edit::Item::ArrayOfTables(a) => a
            .iter_mut()
            .for_each(|t| t.iter_mut().for_each(|(_, x)| absolutize(x, base))),
        toml_edit::Item::None => {}
    }
}

/// The shadow workspace manifest: the user's `[patch]` (paths made
/// absolute) and `[profile]`, one member.
pub fn root_manifest(user_root_manifest: &str, user_root: &Path, member: &str) -> Result<String> {
    let user: toml_edit::DocumentMut = user_root_manifest
        .parse()
        .context("parsing root Cargo.toml")?;
    let mut doc = toml_edit::DocumentMut::new();
    let mut ws = toml_edit::Table::new();
    ws["members"] = toml_edit::value(toml_edit::Array::from_iter([member]));
    ws["resolver"] = toml_edit::value(
        user.get("workspace")
            .and_then(|w| w.get("resolver"))
            .and_then(|r| r.as_str())
            .unwrap_or("2"),
    );
    doc["workspace"] = toml_edit::Item::Table(ws);
    for key in ["patch", "profile"] {
        if let Some(it) = user.get(key) {
            let mut it = it.clone();
            absolutize(&mut it, user_root);
            doc[key] = it;
        }
    }
    Ok(doc.to_string())
}

/// The workspace's `.cargo/config.toml` with relative paths made absolute
/// and the target dir pinned to the real one.
pub fn cargo_config(
    user_config: Option<&str>,
    user_root: &Path,
    target_dir: &Path,
) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = user_config
        .unwrap_or("")
        .parse()
        .context("parsing .cargo/config.toml")?;
    let keys: Vec<String> = doc.iter().map(|(k, _)| k.to_owned()).collect();
    for k in keys {
        absolutize(&mut doc[&k], user_root);
    }
    if !doc.contains_key("build") {
        doc["build"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["build"]["target-dir"] = toml_edit::value(target_dir.to_string_lossy().into_owned());
    Ok(doc.to_string())
}

fn ws_key(root: &Path) -> String {
    let mut h = crate::depcache::H128::new();
    h.field(root.as_os_str().as_encoded_bytes());
    h.hex()[..16].to_owned()
}

pub fn shadow_dir(root: &Path) -> Result<PathBuf> {
    Ok(paths::home()?.join("shadow").join(ws_key(root)))
}

/// (path, len, mtime ns) of every file under the package except the leaf,
/// skipping `target`.
pub fn snapshot(pkg_dir: &Path, leaf: &Path) -> String {
    fn walk(d: &Path, leaf: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut es: Vec<_> = rd.flatten().collect();
        es.sort_by_key(|e| e.path());
        for e in es {
            let p = e.path();
            let name = e.file_name();
            if name == "target" || name.to_string_lossy().starts_with('.') || p == leaf {
                continue;
            }
            let Ok(md) = std::fs::metadata(&p) else {
                continue;
            };
            if md.is_dir() {
                walk(&p, leaf, out);
            } else {
                let mt = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                out.push(format!("{}\t{}\t{mt}", p.display(), md.len()));
            }
        }
    }
    let mut v = Vec::new();
    walk(pkg_dir, leaf, &mut v);
    v.join("\n")
}

fn read_other_sources(dir: &Path, leaf: &Path, out: &mut String) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            read_other_sources(&p, leaf, out);
        } else if p != leaf
            && p.extension().is_some_and(|x| x == "rs")
            && let Ok(s) = std::fs::read_to_string(&p)
        {
            out.push_str(&s);
            out.push('\n');
        }
    }
}

/// Everything known about one routable request.
pub struct Plan {
    pub req: Request,
    pub user_root: PathBuf,
    pub leaf_file: PathBuf,
    pub shadow: PathBuf,
    pub snap_file: PathBuf,
    pub snap_now: String,
    pkg_meta: serde_json::Value,
    target_dir: PathBuf,
}

/// Check the request against the workspace. `Err` explains a fallback.
pub fn plan(cwd: &Path, req: Request) -> Result<Plan> {
    let out = Command::new(paths::real_cargo()?)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .current_dir(cwd)
        .stderr(std::process::Stdio::null())
        .output()
        .context("cargo metadata")?;
    anyhow::ensure!(out.status.success(), "cargo metadata failed");
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let user_root = PathBuf::from(
        meta["workspace_root"]
            .as_str()
            .context("no workspace_root")?,
    );
    let target_dir = PathBuf::from(
        meta["target_directory"]
            .as_str()
            .context("no target_directory")?,
    );
    let pkg = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"].as_str() == Some(&req.package))
        .with_context(|| format!("package {} not in workspace", req.package))?
        .clone();
    let lib = pkg["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| {
            t["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|x| x == "lib" || x == "rlib" || x == "cdylib"))
        })
        .context("package has no lib target")?;
    let root_file = PathBuf::from(lib["src_path"].as_str().context("no src_path")?);
    let src_dir = root_file
        .parent()
        .context("root file has no dir")?
        .to_path_buf();
    let pkg_dir = PathBuf::from(pkg["manifest_path"].as_str().unwrap_or_default())
        .parent()
        .context("manifest has no dir")?
        .to_path_buf();
    let leaf_file = src_dir.join(format!("{}.rs", req.module));
    let root_src = std::fs::read_to_string(&root_file)?;
    if !declares_module(&root_src, &req.module) {
        bail!(
            "`mod {};` is not declared in {}",
            req.module,
            root_file.display()
        );
    }
    let leaf_src = std::fs::read_to_string(&leaf_file)
        .with_context(|| format!("{} not found", leaf_file.display()))?;
    if let Some(why) = leaf_blocker(&leaf_src) {
        bail!("{} is not a leaf: {why}", leaf_file.display());
    }
    let mut others = String::new();
    read_other_sources(&src_dir, &leaf_file, &mut others);
    if filter_escapes(&others, &req.filter, &req.module) {
        bail!(
            "filter `{}` may match tests outside {}",
            req.filter,
            req.module
        );
    }
    let shadow = shadow_dir(&user_root)?;
    let snap_file = shadow.join(format!("{}.snap", req.package));
    let snap_now = snapshot(&pkg_dir, &leaf_file);
    Ok(Plan {
        req,
        user_root,
        leaf_file,
        shadow,
        snap_file,
        snap_now,
        pkg_meta: pkg,
        target_dir,
    })
}

fn write_if_changed(p: &Path, content: &str) -> Result<()> {
    if std::fs::read_to_string(p).ok().as_deref() == Some(content) {
        return Ok(());
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, content)?;
    Ok(())
}

impl Plan {
    pub fn up_to_date(&self) -> bool {
        std::fs::read_to_string(&self.snap_file).ok().as_deref() == Some(self.snap_now.as_str())
    }

    pub fn save_snapshot(&self) {
        let _ = write_if_changed(&self.snap_file, &self.snap_now);
    }

    /// Write (or refresh) the shadow workspace. Never touches the user's tree.
    pub fn materialize(&self) -> Result<()> {
        let m = &self.req.module;
        let member = leaf_package(m);
        let root_toml = std::fs::read_to_string(self.user_root.join("Cargo.toml"))?;
        write_if_changed(
            &self.shadow.join("Cargo.toml"),
            &root_manifest(&root_toml, &self.user_root, &member)?,
        )?;
        let cfg = std::fs::read_to_string(self.user_root.join(".cargo/config.toml")).ok();
        write_if_changed(
            &self.shadow.join(".cargo/config.toml"),
            &cargo_config(cfg.as_deref(), &self.user_root, &self.target_dir)?,
        )?;
        if let Ok(lock) = std::fs::read_to_string(self.user_root.join("Cargo.lock"))
            && !self.shadow.join("Cargo.lock").exists()
        {
            write_if_changed(&self.shadow.join("Cargo.lock"), &lock)?;
        }
        write_if_changed(
            &self.shadow.join(&member).join("Cargo.toml"),
            &leaf_manifest(&self.pkg_meta, m)?,
        )?;
        write_if_changed(
            &self.shadow.join(&member).join("src/lib.rs"),
            &lib_rs(m, &self.leaf_file),
        )?;
        Ok(())
    }

    pub fn shadow_args(&self) -> Vec<OsString> {
        let mut v: Vec<OsString> = vec![
            "test".into(),
            "-p".into(),
            leaf_package(&self.req.module).into(),
            "--lib".into(),
        ];
        v.extend(self.req.cargo_flags.iter().map(OsString::from));
        v.push("--".into());
        v.extend(self.req.test_args.iter().map(OsString::from));
        v
    }
}

/// Called from the agent entry point. Returns when the normal build should
/// run; otherwise runs the shadow (or a snapshotting normal build) and exits.
pub fn maybe_route(full: &[OsString]) {
    if !enabled() || std::env::var_os("JUSTRUST_SHADOW_CHILD").is_some() {
        return;
    }
    let Some(req) = parse_request(full) else {
        return;
    };
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let plan = match plan(&cwd, req) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("justrust: shadow: normal build ({e:#})");
            return;
        }
    };
    if plan.up_to_date() {
        match plan.materialize() {
            Ok(()) => {
                eprintln!(
                    "justrust: shadow: only {} changed since the last good `{}` build; running its tests in leaf crate {} (JUSTRUST_SHADOW=0 to disable)",
                    plan.leaf_file
                        .strip_prefix(&plan.user_root)
                        .unwrap_or(&plan.leaf_file)
                        .display(),
                    plan.req.package,
                    leaf_package(&plan.req.module),
                );
                if std::env::set_current_dir(&plan.shadow).is_ok() {
                    crate::record::run(
                        plan.shadow_args(),
                        crate::record::Options {
                            agent: true,
                            max_warnings: crate::agent_output::DEFAULT_MAX_WARNINGS,
                        },
                    );
                }
            }
            Err(e) => eprintln!("justrust: shadow: normal build (writing shadow failed: {e:#})"),
        }
        return;
    }
    // Other files of the big crate changed (or first use): the normal build,
    // as a child so a success can record the snapshot it was built from.
    eprintln!(
        "justrust: shadow: {} changed outside {}; normal build",
        plan.req.package,
        plan.leaf_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let mut cmd = Command::new(me);
    cmd.args(full).env("JUSTRUST_SHADOW_CHILD", "1");
    if let Ok(st) = cmd.status() {
        if st.success() {
            plan.save_snapshot();
        }
        std::process::exit(st.code().unwrap_or(1));
    }
}

/// `justrust shadow`: which modules would route, ranked by recorded edits.
pub fn candidates_command(days: f64, top: usize) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = crate::split::Workspace::load(&cwd)?;
    let history = crate::split_index::load(&ws.root, days)?;
    let mut counts: BTreeMap<(String, PathBuf), usize> = BTreeMap::new();
    for s in &history {
        for (pkg, f) in crate::split::edited_files_pub(s, &ws.root) {
            *counts.entry((pkg, f)).or_default() += 1;
        }
    }
    let rows = rank_candidates(&ws, counts);
    if rows.is_empty() {
        println!("no recorded edits in the last {days} days");
        return Ok(());
    }
    println!("Shadow candidates (JUSTRUST_SHADOW=1), most-edited first, last {days} days:");
    for r in rows.iter().take(top) {
        match &r.blocker {
            None => println!(
                "  {:>4} edits  {}::{}  leaf: `justrust test -p {} --lib -- {}` routes to {}",
                r.edits,
                r.package,
                r.module,
                r.package,
                r.module,
                leaf_package(&r.module)
            ),
            Some(b) => println!(
                "  {:>4} edits  {}::{}  not a leaf: {b}",
                r.edits, r.package, r.module
            ),
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Candidate {
    pub package: String,
    pub module: String,
    pub edits: usize,
    pub blocker: Option<String>,
}

/// Top-level module files of workspace crates, by edit count, with why each
/// does or does not qualify.
pub fn rank_candidates(
    ws: &crate::split::Workspace,
    counts: BTreeMap<(String, PathBuf), usize>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for ((pkg, f), edits) in counts {
        let Some(k) = ws.crates.get(&pkg) else {
            continue;
        };
        let Some(root) = &k.root_file else { continue };
        let Some(src_dir) = root.parent() else {
            continue;
        };
        if f.parent() != Some(src_dir) || f == *root || f.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let module = f
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let root_src = std::fs::read_to_string(root).unwrap_or_default();
        let blocker = if !declares_module(&root_src, &module) {
            Some("not declared as `mod m;` in the crate root".to_owned())
        } else {
            match std::fs::read_to_string(&f) {
                Ok(s) => leaf_blocker(&s),
                Err(_) => Some("file gone".to_owned()),
            }
        };
        out.push(Candidate {
            package: pkg,
            module,
            edits,
            blocker,
        });
    }
    out.sort_by(|a, b| b.edits.cmp(&a.edits).then(a.module.cmp(&b.module)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_reference_command() {
        let r = parse_request(&os(&[
            "test",
            "-p",
            "jcode-desktop-ui",
            "--lib",
            "--",
            "fps_counter",
        ]))
        .unwrap();
        assert_eq!(r.package, "jcode-desktop-ui");
        assert_eq!(r.module, "fps_counter");
        let r = parse_request(&os(&[
            "test",
            "-pui",
            "--lib",
            "--locked",
            "--",
            "m::tests::x",
            "--nocapture",
        ]))
        .unwrap();
        assert_eq!((r.package.as_str(), r.module.as_str()), ("ui", "m"));
        assert_eq!(r.cargo_flags, vec!["--locked"]);
    }

    #[test]
    fn rejects_other_shapes() {
        for v in [
            &["check", "-p", "a", "--lib", "--", "m"][..],
            &["test", "-p", "a", "--", "m"],
            &["test", "--lib", "--", "m"],
            &["test", "-p", "a", "--lib", "--release", "--", "m"],
            &["test", "-p", "a", "--lib", "--", "m", "n"],
            &["test", "-p", "a", "--lib", "--", "--ignored", "m"],
            &["test", "-p", "a", "--lib", "--", "tests"],
            &["test", "-p", "a", "--lib"],
        ] {
            assert_eq!(parse_request(&os(v)), None, "{v:?}");
        }
    }

    #[test]
    fn leaf_detection() {
        let fps = "use std::time::Instant;\nuse gpui::x::Y;\npub(crate) struct F;\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn t() {}\n}\n";
        assert_eq!(leaf_blocker(fps), None);
        assert!(
            leaf_blocker("use crate::other::X;\n")
                .unwrap()
                .contains("crate::")
        );
        assert!(leaf_blocker("use super::X;\n").unwrap().contains("super::"));
        assert!(
            leaf_blocker("mod child;\n")
                .unwrap()
                .contains("child module")
        );
        assert!(leaf_blocker("#[path = \"x.rs\"]\nmod x;\n").is_some());
        assert!(leaf_blocker("macro_rules! m { () => {} }\n").is_some());
        assert!(leaf_blocker("const V: &str = env!(\"X\");\n").is_some());
        assert!(leaf_blocker("mod t { fn f() { super::super::g() } }\n").is_some());
        // A comment mentioning crate:: is fine.
        assert_eq!(leaf_blocker("fn f() {} // see crate::x\n"), None);
    }

    #[test]
    fn module_declarations() {
        assert!(declares_module(
            "mod a;\npub(crate) mod fps_counter;\n",
            "fps_counter"
        ));
        assert!(declares_module("pub mod fps_counter;", "fps_counter"));
        assert!(!declares_module(
            "mod fps_counter_x;\nmod fps_counter { }\n",
            "fps_counter"
        ));
    }

    #[test]
    fn filter_escape_detection() {
        assert!(!filter_escapes(
            "mod fps_counter;\nfn render() {}\n",
            "fps_counter",
            "fps_counter"
        ));
        assert!(filter_escapes(
            "fn fps_counter_label() {}\n",
            "fps_counter",
            "fps_counter"
        ));
        assert!(filter_escapes(
            "mod fps_counter_tests {}\n",
            "fps_counter",
            "fps_counter"
        ));
    }

    #[test]
    fn generated_files() {
        let lib = lib_rs("fps_counter", Path::new("/w/crates/ui/src/fps_counter.rs"));
        assert!(
            lib.contains("#[path = \"/w/crates/ui/src/fps_counter.rs\"]\npub mod fps_counter;")
        );
        let pkg = serde_json::json!({
            "edition": "2024",
            "dependencies": [
                {"name": "gpui", "source": "git+https://github.com/1jehuang/gpui?rev=abc", "req": "*", "kind": null, "rename": null, "optional": false, "uses_default_features": true, "features": ["profiler"], "target": null, "path": null},
                {"name": "jcode-core", "source": null, "req": "*", "kind": null, "rename": "jcode_core", "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/j/crates/jcode-core"},
                {"name": "image", "source": "registry+https://github.com/rust-lang/crates.io-index", "req": "^0.25", "kind": null, "rename": null, "optional": false, "uses_default_features": false, "features": ["png"], "target": null, "path": null},
                {"name": "tempfile", "source": "registry+https://github.com/rust-lang/crates.io-index", "req": "^3", "kind": "dev", "rename": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": null},
                {"name": "winapi", "source": "registry+https://github.com/rust-lang/crates.io-index", "req": "^0.3", "kind": null, "rename": null, "optional": false, "uses_default_features": true, "features": [], "target": "cfg(windows)", "path": null},
                {"name": "cc", "source": "registry+https://github.com/rust-lang/crates.io-index", "req": "^1", "kind": "build", "rename": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": null},
                {"name": "opt", "source": "registry+https://github.com/rust-lang/crates.io-index", "req": "^1", "kind": null, "rename": null, "optional": true, "uses_default_features": true, "features": [], "target": null, "path": null}
            ]
        });
        let m = leaf_manifest(&pkg, "fps_counter").unwrap();
        let doc: toml::Table = toml::from_str(&m).unwrap();
        assert_eq!(
            doc["package"]["name"].as_str(),
            Some("jr-shadow-fps-counter")
        );
        assert_eq!(doc["package"]["edition"].as_str(), Some("2024"));
        let deps = doc["dependencies"].as_table().unwrap();
        assert_eq!(
            deps["gpui"]["git"].as_str(),
            Some("https://github.com/1jehuang/gpui")
        );
        assert_eq!(deps["gpui"]["rev"].as_str(), Some("abc"));
        assert_eq!(deps["jcode_core"]["package"].as_str(), Some("jcode-core"));
        assert_eq!(
            deps["jcode_core"]["path"].as_str(),
            Some("/j/crates/jcode-core")
        );
        assert_eq!(deps["image"]["default-features"].as_bool(), Some(false));
        assert!(!deps.contains_key("cc") && !deps.contains_key("opt"));
        assert!(
            doc["dev-dependencies"]
                .as_table()
                .unwrap()
                .contains_key("tempfile")
        );
        assert!(
            doc["target"]["cfg(windows)"]["dependencies"]
                .as_table()
                .unwrap()
                .contains_key("winapi")
        );
    }

    #[test]
    fn root_manifest_and_config_absolutize_paths() {
        let dir = std::env::temp_dir().join(format!("jr-shadow-unit-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("vendor/x")).unwrap();
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/wrap"), "").unwrap();
        let root = "[workspace]\nmembers = [\"a\"]\nresolver = \"3\"\n[patch.crates-io]\nx = { path = \"vendor/x\" }\n[profile.dev]\nopt-level = 1\n";
        let m = root_manifest(root, &dir, "jr-shadow-m").unwrap();
        let doc: toml::Table = toml::from_str(&m).unwrap();
        assert_eq!(doc["workspace"]["resolver"].as_str(), Some("3"));
        assert_eq!(doc["workspace"]["members"][0].as_str(), Some("jr-shadow-m"));
        assert_eq!(
            doc["patch"]["crates-io"]["x"]["path"].as_str(),
            Some(dir.join("vendor/x").to_str().unwrap())
        );
        assert_eq!(doc["profile"]["dev"]["opt-level"].as_integer(), Some(1));
        let c = cargo_config(
            Some("[build]\nrustc-wrapper = \"scripts/wrap\"\n"),
            &dir,
            Path::new("/t"),
        )
        .unwrap();
        let doc: toml::Table = toml::from_str(&c).unwrap();
        assert_eq!(
            doc["build"]["rustc-wrapper"].as_str(),
            Some(dir.join("scripts/wrap").to_str().unwrap())
        );
        assert_eq!(doc["build"]["target-dir"].as_str(), Some("/t"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
