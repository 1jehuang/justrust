//! `justrust split`: which code to move into its own crate to stop rebuild
//! cascades, worked out from recorded runs plus a static look at the source.
//!
//! 1. History. Every recorded run in this workspace says which files cargo
//!    saw change (`Dirty <pkg> ...: the file X has changed`) and how long each
//!    local crate took. A run's cost is the edited crates plus every dependent
//!    that rebuilt with them.
//! 2. Module graph. Every crate's module tree is read from its `mod` items
//!    (including `#[path]`), and each module's references to other modules of
//!    the same crate (`crate::`, `super::`, `self::`, child paths) become
//!    edges. Moving subtree `M` out of crate `C` also forces out every module
//!    that references it, transitively: anything left behind that still uses
//!    `M` would put the new crate upstream of `C` again, and every edit would
//!    still rebuild `C` and everything after it. The new crate depends on `C`
//!    for everything else. `pub use` re-exports are not edges, they are
//!    listed as imports to update.
//! 3. Replay. For each candidate (every ancestor of every edited module), the
//!    recorded runs whose edits in `C` all fall inside the moved set are
//!    replayed: `C` shrinks to the moved share, and dependents rebuild only if
//!    they use the moved code or depend on a crate that does.
//!
//! Edits to a crate root file cannot be split by module. For those, git
//! history says which items in the file change most, and a word search says
//! which dependent crates use each one.
//!
//! This is regex over source text, not a Rust parser: it ranks candidates and
//! names the edges to cut. It does not perform the move.

use crate::paths;
use crate::summary::Summary;
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Moving more than this share of a crate is not a split, it is a rename.
const MAX_MOVE_SHARE: f64 = 0.35;

pub fn command(days: f64, top: usize, run_ids: &[String]) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::load(&cwd)?;
    let history = load_runs(&ws.root, days)?;
    let runs = if run_ids.is_empty() {
        history.clone()
    } else {
        // Replay specific runs, for example ones made in a scratch worktree
        // of this workspace. Cargo reports edited files relative to the
        // workspace root, so they resolve against this checkout.
        let dir = paths::runs_dir()?;
        let mut v = Vec::new();
        for id in run_ids {
            let raw = std::fs::read(dir.join(id).join("summary.json"))
                .with_context(|| format!("run {id} has no summary"))?;
            v.push(serde_json::from_slice::<Summary>(&raw)?);
        }
        v
    };
    print!("{}", report_with(&ws, &runs, &history, days, top));
    Ok(())
}

// ---------------------------------------------------------------------------
// Workspace.

#[derive(Debug, Default)]
pub struct Krate {
    pub name: String,
    /// `name` with dashes replaced, as used in paths and unit names.
    pub ident: String,
    pub dir: PathBuf,
    pub root_file: Option<PathBuf>,
    /// Workspace crates this one depends on (normal and build deps; dev
    /// dependencies only affect this crate's own tests).
    pub deps: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct Workspace {
    pub root: PathBuf,
    pub crates: BTreeMap<String, Krate>,
}

impl Workspace {
    fn load(cwd: &Path) -> Result<Workspace> {
        let cargo = paths::real_cargo()?;
        let out = Command::new(cargo)
            .args([
                "metadata",
                "--no-deps",
                "--format-version",
                "1",
                "--offline",
            ])
            .current_dir(cwd)
            .stderr(Stdio::inherit())
            .output()
            .context("running cargo metadata")?;
        anyhow::ensure!(out.status.success(), "cargo metadata failed");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
        Ok(Workspace::from_metadata(&v))
    }

    pub fn from_metadata(v: &serde_json::Value) -> Workspace {
        let root = PathBuf::from(v["workspace_root"].as_str().unwrap_or("."));
        let mut crates = BTreeMap::new();
        let pkgs = v["packages"].as_array().cloned().unwrap_or_default();
        let names: BTreeSet<String> = pkgs
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_owned))
            .collect();
        for p in &pkgs {
            let name = p["name"].as_str().unwrap_or_default().to_owned();
            let manifest = PathBuf::from(p["manifest_path"].as_str().unwrap_or_default());
            let dir = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
            let targets = p["targets"].as_array().cloned().unwrap_or_default();
            let kind_is = |t: &serde_json::Value, k: &str| {
                t["kind"]
                    .as_array()
                    .is_some_and(|ks| ks.iter().any(|x| x.as_str() == Some(k)))
            };
            let root_file = targets
                .iter()
                .find(|t| kind_is(t, "lib") || kind_is(t, "proc-macro"))
                .or_else(|| targets.iter().find(|t| kind_is(t, "bin")))
                .and_then(|t| t["src_path"].as_str())
                .map(PathBuf::from);
            let deps = p["dependencies"]
                .as_array()
                .map(|ds| {
                    ds.iter()
                        .filter(|d| d["kind"].as_str() != Some("dev"))
                        .filter_map(|d| d["name"].as_str())
                        .filter(|n| names.contains(*n) && *n != name)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            crates.insert(
                name.clone(),
                Krate {
                    ident: name.replace('-', "_"),
                    name,
                    dir,
                    root_file,
                    deps,
                },
            );
        }
        Workspace { root, crates }
    }

    /// Workspace crates that depend on `name`, directly or not (not `name`).
    pub fn dependents(&self, name: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut todo = vec![name.to_owned()];
        while let Some(n) = todo.pop() {
            for (c, k) in &self.crates {
                if c != name && k.deps.contains(&n) && out.insert(c.clone()) {
                    todo.push(c.clone());
                }
            }
        }
        out
    }

    /// `names` plus everything that depends on any of them.
    fn with_dependents<'a>(&self, names: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for n in names {
            out.insert(n.clone());
            out.extend(self.dependents(n));
        }
        out
    }

    /// Package name for a unit name such as `jcode_base (test)`.
    fn package_of_unit(&self, unit: &str) -> Option<&str> {
        let base = unit.split(" (").next().unwrap_or(unit);
        self.crates
            .values()
            .find(|k| k.ident == base)
            .map(|k| k.name.as_str())
    }
}

// ---------------------------------------------------------------------------
// Module graph.

#[derive(Debug, Default, Clone)]
pub struct Node {
    /// `a::b::c`, empty for the crate root.
    pub path: String,
    pub lines: usize,
    /// Under `#[cfg(test)]` or named like a test module.
    pub test: bool,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Referenced node -> number of references (not counting re-exports).
    pub refs: BTreeMap<usize, usize>,
    /// Nodes this one re-exports from with `pub use`.
    pub reexports: BTreeSet<usize>,
    /// Types this file defines (`struct`, `enum`, `union`, `trait`, `type`).
    pub defines: BTreeSet<String>,
    /// Types this file has inherent `impl Type { .. }` blocks for.
    pub inherent_impls: BTreeSet<String>,
}

/// A crate's file-level module tree. Node 0 is the crate root.
#[derive(Debug, Default)]
pub struct ModuleGraph {
    pub nodes: Vec<Node>,
    by_path: HashMap<String, usize>,
    by_file: HashMap<PathBuf, usize>,
    pub total_lines: usize,
}

impl ModuleGraph {
    pub fn build(root_file: &Path) -> ModuleGraph {
        let mut g = ModuleGraph::default();
        let mut srcs: Vec<String> = Vec::new();
        g.add(root_file, String::new(), None, false, true, &mut srcs, 0);
        for (i, full) in srcs.iter().enumerate() {
            if g.nodes[i].test {
                continue;
            }
            let src = strip_test_tail(full);
            let (refs, reexports) = g.resolve_refs(i, src);
            g.nodes[i].refs = refs;
            g.nodes[i].reexports = reexports;
            let (defines, impls) = type_items(src);
            g.nodes[i].defines = defines;
            g.nodes[i].inherent_impls = impls;
        }
        g
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        file: &Path,
        path: String,
        parent: Option<usize>,
        test: bool,
        is_root: bool,
        srcs: &mut Vec<String>,
        depth: usize,
    ) {
        if depth > 64 || self.by_file.contains_key(file) {
            return;
        }
        let Ok(src) = std::fs::read_to_string(file) else {
            return;
        };
        let id = self.nodes.len();
        let lines = src.lines().count();
        self.total_lines += lines;
        self.nodes.push(Node {
            path: path.clone(),
            lines,
            test,
            parent,
            ..Default::default()
        });
        self.by_path.insert(path.clone(), id);
        self.by_file.insert(file.to_path_buf(), id);
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        let decls = mod_decls(&src, file, is_root);
        srcs.push(src);
        for d in decls {
            let Some(f) = d.candidates.iter().find(|c| c.is_file()) else {
                continue;
            };
            let child = if path.is_empty() {
                d.name.clone()
            } else {
                format!("{path}::{}", d.name)
            };
            self.add(f, child, Some(id), test || d.test, false, srcs, depth + 1);
        }
    }

    /// Resolve a module path relative to the crate root to the deepest node
    /// it names (`auth::cursor::Thing` -> `auth::cursor`).
    fn resolve(&self, segs: &[&str]) -> Option<usize> {
        let mut best = None;
        let mut path = String::new();
        for (i, s) in segs.iter().enumerate() {
            if i > 0 {
                path.push_str("::");
            }
            path.push_str(s);
            match self.by_path.get(&path) {
                Some(&n) => best = Some(n),
                None => break,
            }
        }
        best
    }

    fn resolve_refs(&self, i: usize, src: &str) -> (BTreeMap<usize, usize>, BTreeSet<usize>) {
        static RE: OnceLock<(Regex, Regex, Regex)> = OnceLock::new();
        let (path_re, group_re, line_re) = RE.get_or_init(|| {
            (
                // `crate::a::b`, `super::super::x`, `self::x`, `child::x`.
                Regex::new(r"(^|[^\w:])((?:crate|self|super|[a-z_][a-z0-9_]*)(?:::[A-Za-z_][A-Za-z0-9_]*)+)").unwrap(),
                // `prefix::{a, b::c}` (one level of braces).
                Regex::new(r"(^|[^\w:])((?:crate|self|super|[a-z_][a-z0-9_]*)(?:::[A-Za-z_][A-Za-z0-9_]*)*)::\{([^{}]*)\}").unwrap(),
                Regex::new(r"^\s*pub(?:\([^)]*\))?\s+use\b").unwrap(),
            )
        });
        let node = &self.nodes[i];
        let own: Vec<&str> = if node.path.is_empty() {
            Vec::new()
        } else {
            node.path.split("::").collect()
        };
        let child_names: BTreeSet<&str> = node
            .children
            .iter()
            .filter_map(|c| self.nodes[*c].path.rsplit("::").next())
            .collect();
        // Absolute segments for a path as written in module `own`.
        let absolute = |written: &str| -> Option<Vec<String>> {
            let mut segs: Vec<&str> = written.split("::").collect();
            let mut base: Vec<&str> = match segs[0] {
                "crate" => {
                    segs.remove(0);
                    Vec::new()
                }
                "self" => {
                    segs.remove(0);
                    own.clone()
                }
                "super" => {
                    let mut b = own.clone();
                    while segs.first() == Some(&"super") {
                        segs.remove(0);
                        b.pop()?;
                    }
                    b
                }
                first if child_names.contains(first) => own.clone(),
                _ => return None,
            };
            base.extend(segs);
            Some(base.into_iter().map(str::to_owned).collect())
        };
        let mut refs: BTreeMap<usize, usize> = BTreeMap::new();
        let mut reexports = BTreeSet::new();
        let mut note = |segs: Vec<String>, reexport: bool| {
            let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
            if let Some(t) = self.resolve(&segs)
                && t != i
                && t != 0
            {
                if reexport {
                    reexports.insert(t);
                } else {
                    *refs.entry(t).or_default() += 1;
                }
            }
        };
        // Statements, so a multi-line `pub use x::{...};` is one unit.
        for stmt in statements(src) {
            let reexport = line_re.is_match(stmt);
            for c in group_re.captures_iter(stmt) {
                let Some(prefix) = absolute(&format!("{}::_", &c[2])) else {
                    continue;
                };
                let prefix = &prefix[..prefix.len() - 1];
                for item in c[3].split(',') {
                    let item = item.trim();
                    let item = item.split(" as ").next().unwrap_or(item).trim();
                    if item.is_empty() || item == "self" || item == "*" {
                        if item == "self" || item == "*" {
                            note(prefix.to_vec(), reexport);
                        }
                        continue;
                    }
                    let mut segs = prefix.to_vec();
                    segs.extend(item.split("::").map(str::to_owned));
                    note(segs, reexport);
                }
            }
            for c in path_re.captures_iter(stmt) {
                if let Some(segs) = absolute(&c[2]) {
                    note(segs, reexport);
                }
            }
        }
        (refs, reexports)
    }

    pub fn node_of(&self, file: &Path) -> Option<usize> {
        self.by_file.get(file).copied()
    }

    fn subtree(&self, n: usize, out: &mut BTreeSet<usize>) {
        if out.insert(n) {
            for &c in &self.nodes[n].children {
                self.subtree(c, out);
            }
        }
    }

    /// What has to move with subtree `n`: the subtree plus the subtrees of
    /// every non-test module that references anything moved, transitively.
    /// The crate root never moves; its references are reported instead.
    pub fn closure(&self, n: usize) -> BTreeSet<usize> {
        let mut moved = BTreeSet::new();
        self.subtree(n, &mut moved);
        loop {
            let add: Vec<usize> = (1..self.nodes.len())
                .filter(|x| !moved.contains(x) && !self.nodes[*x].test)
                .filter(|x| self.nodes[*x].refs.keys().any(|t| moved.contains(t)))
                .collect();
            if add.is_empty() {
                break;
            }
            for x in add {
                self.subtree(x, &mut moved);
            }
        }
        moved
    }

    /// Modules outside `n`'s subtree referencing into it, with counts.
    pub fn direct_users(&self, n: usize) -> Vec<(String, usize)> {
        let mut sub = BTreeSet::new();
        self.subtree(n, &mut sub);
        let mut v: Vec<(String, usize)> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, x)| !sub.contains(i) && !x.test)
            .filter_map(|(_, x)| {
                let c: usize = x
                    .refs
                    .iter()
                    .filter(|(t, _)| sub.contains(t))
                    .map(|(_, c)| c)
                    .sum();
                (c > 0).then(|| (label(&x.path), c))
            })
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    fn lines_of(&self, set: &BTreeSet<usize>) -> usize {
        set.iter().map(|n| self.nodes[*n].lines).sum()
    }

    /// The minimal module paths covering `set` (subtree roots).
    fn roots(&self, set: &BTreeSet<usize>) -> Vec<String> {
        set.iter()
            .filter(|n| self.nodes[**n].parent.is_none_or(|p| !set.contains(&p)))
            .map(|n| self.nodes[*n].path.clone())
            .collect()
    }

    /// Inherent impls in `set` for types defined only outside it. Rust only
    /// allows `impl Type { .. }` in the crate that defines `Type`, so these
    /// block a move (the methods must become free functions or an extension
    /// trait first). Returns (type, defining module, impl modules).
    pub fn orphan_impls(&self, set: &BTreeSet<usize>) -> Vec<(String, String, Vec<String>)> {
        let mut out: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for &n in set {
            for ty in &self.nodes[n].inherent_impls {
                if set.iter().any(|m| self.nodes[*m].defines.contains(ty)) {
                    continue;
                }
                // Defined in another crate: already foreign, not our problem.
                let Some(def) = self
                    .nodes
                    .iter()
                    .find(|x| !x.test && x.defines.contains(ty))
                else {
                    continue;
                };
                let e = out
                    .entry(ty.clone())
                    .or_insert_with(|| (label(&def.path), Vec::new()));
                e.1.push(label(&self.nodes[n].path));
            }
        }
        out.into_iter().map(|(t, (d, v))| (t, d, v)).collect()
    }
}

/// Types defined in `src` and types it has inherent impl blocks for.
fn type_items(src: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    static RE: OnceLock<(Regex, Regex)> = OnceLock::new();
    let (def, imp) = RE.get_or_init(|| {
        (
            Regex::new(
                r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union|trait|type)\s+([A-Za-z_]\w*)",
            )
            .unwrap(),
            // `impl Foo {`, `impl<T> Foo<T> {`, not `impl Trait for Foo`.
            Regex::new(
                r"(?m)^\s*impl(?:<[^>{]*>)?\s+(?:[\w:]+::)?([A-Za-z_]\w*)(?:<[^>{]*>)?\s*(?:where[^{]*)?\{",
            )
            .unwrap(),
        )
    });
    let defines = def.captures_iter(src).map(|c| c[1].to_owned()).collect();
    let impls = imp.captures_iter(src).map(|c| c[1].to_owned()).collect();
    (defines, impls)
}

fn label(path: &str) -> String {
    if path.is_empty() {
        "crate root".into()
    } else {
        path.into()
    }
}

/// Split source into `;`/`{`/`}`-terminated statements, keeping braces inside
/// `use` groups together, so `pub use x::{\n a,\n b,\n};` is one unit.
fn statements(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_use_group = 0usize;
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if i >= 2 && &bytes[i - 2..i] == b"::" => in_use_group += 1,
            b'}' if in_use_group > 0 => in_use_group -= 1,
            b';' | b'{' | b'}' if in_use_group == 0 => {
                out.push(&src[start..=i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < src.len() {
        out.push(&src[start..]);
    }
    out
}

#[derive(Debug)]
struct ModDecl {
    name: String,
    /// Candidate files, first existing one wins.
    candidates: Vec<PathBuf>,
    test: bool,
}

/// `mod x;` declarations in `src` (file `file`), with their candidate paths.
fn mod_decls(src: &str, file: &Path, is_root: bool) -> Vec<ModDecl> {
    static RE: OnceLock<(Regex, Regex)> = OnceLock::new();
    let (decl, path_attr) = RE.get_or_init(|| {
        (
            Regex::new(r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
                .unwrap(),
            Regex::new(r#"#\[path\s*=\s*"([^"]+)"\s*\]"#).unwrap(),
        )
    });
    let dir = file.parent().unwrap_or(Path::new("."));
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let owns_dir = is_root || stem == "mod";
    let child_dir = if owns_dir {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    let mut out = Vec::new();
    let mut attrs = String::new();
    for line in src.lines() {
        let t = line.trim();
        if let Some(c) = decl.captures(line) {
            attrs.push_str(t);
            out.push(make_decl(&c[1], &attrs, path_attr, dir, &child_dir));
            attrs.clear();
        } else if t.starts_with("#[") {
            attrs.push_str(t);
        } else if !(t.is_empty() || t.starts_with("//")) {
            attrs.clear();
        }
    }
    out
}

fn make_decl(name: &str, attrs: &str, path_attr: &Regex, dir: &Path, child_dir: &Path) -> ModDecl {
    let test = attrs.contains("cfg(test)")
        || name == "tests"
        || name.ends_with("_tests")
        || name.ends_with("_test");
    let candidates = match path_attr.captures(attrs) {
        Some(p) => vec![dir.join(&p[1])],
        None => vec![
            child_dir.join(format!("{name}.rs")),
            child_dir.join(name).join("mod.rs"),
        ],
    };
    ModDecl {
        name: name.to_owned(),
        candidates,
        test,
    }
}

/// Drop a trailing `#[cfg(test)] mod tests { ... }` block (by convention the
/// last item in the file), so test-only references do not count as edges.
fn strip_test_tail(src: &str) -> &str {
    let mut offset = 0;
    let mut pending: Option<usize> = None;
    for line in src.split_inclusive('\n') {
        let t = line.trim();
        if t == "#[cfg(test)]" {
            pending = Some(offset);
        } else if let Some(start) = pending {
            if (t.starts_with("mod ") || t.starts_with("pub mod ")) && t.ends_with('{') {
                return &src[..start];
            }
            if !t.is_empty() && !t.starts_with("#[") {
                pending = None;
            }
        }
        offset += line.len();
    }
    src
}

/// All `.rs` files under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            let n = name.to_string_lossy();
            // Build output (`target`, `target-release`, ...) and hidden dirs.
            if !n.starts_with("target") && !n.starts_with('.') {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Collapse `a/b/../c` without touching the filesystem.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// History.

fn load_runs(root: &Path, days: f64) -> Result<Vec<Summary>> {
    let cutoff = paths::now() - days * 86400.0;
    let mut out = Vec::new();
    for e in std::fs::read_dir(paths::runs_dir()?)?.flatten() {
        let Ok(raw) = std::fs::read(e.path().join("summary.json")) else {
            continue;
        };
        let Ok(s) = serde_json::from_slice::<Summary>(&raw) else {
            continue;
        };
        if belongs(&s, root) && s.start >= cutoff {
            out.push(s);
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(out)
}

/// Whether a recorded run was made in the workspace at `root`. Matches path
/// components, so `/x/jcode` does not claim runs from `/x/jcode-desktop`.
fn belongs(s: &Summary, root: &Path) -> bool {
    s.git.root.as_deref().map(Path::new) == Some(root) || Path::new(&s.cwd).starts_with(root)
}

/// (package, file) pairs cargo reported as changed in a run. Files are
/// absolute and normalized; the package is the one whose fingerprint saw it
/// (for build-script inputs, the crate with the build script).
fn edited_files(s: &Summary, root: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = s
        .rebuild_reasons
        .iter()
        .filter_map(|r| {
            let f = r
                .reason
                .strip_prefix("the file `")
                .and_then(|x| x.strip_suffix("` has changed"))?;
            let p = Path::new(f);
            let abs = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            Some((r.package.clone(), normalize(&abs)))
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Where an edit landed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Site {
    /// A module file (node index in the crate's graph).
    Module { krate: String, node: usize },
    /// The crate root file.
    CrateRoot { krate: String },
    /// A non-source file a build script watches.
    BuildInput { krate: String, path: String },
}

impl Site {
    fn krate(&self) -> &str {
        match self {
            Site::Module { krate, .. }
            | Site::CrateRoot { krate }
            | Site::BuildInput { krate, .. } => krate,
        }
    }
}

/// One recorded run, reduced to what the replay needs.
struct Edit {
    sites: Vec<Site>,
    /// Seconds per local package.
    secs: HashMap<String, f64>,
    /// Other processes used under a quarter of the machine during the run.
    quiet: bool,
    /// A `check`/`clippy` run (metadata only, no codegen).
    check: bool,
}

impl Edit {
    /// Seconds the run spent on the edited crates and their dependents.
    fn cost(&self, ws: &Workspace) -> (f64, f64) {
        let edited: BTreeSet<String> = self.sites.iter().map(|s| s.krate().to_owned()).collect();
        let involved = ws.with_dependents(&edited);
        let total: f64 = involved.iter().filter_map(|p| self.secs.get(p)).sum();
        let cascade: f64 = involved
            .iter()
            .filter(|p| !edited.contains(*p))
            .filter_map(|p| self.secs.get(p))
            .sum();
        (total, cascade)
    }

    /// Crates other edits in this run rebuild anyway (outside `krate`).
    fn others(&self, ws: &Workspace, krate: &str) -> BTreeSet<String> {
        let other: BTreeSet<String> = self
            .sites
            .iter()
            .map(Site::krate)
            .filter(|k| *k != krate)
            .map(str::to_owned)
            .collect();
        ws.with_dependents(&other)
    }
}

fn to_edits(ws: &Workspace, graphs: &HashMap<String, ModuleGraph>, runs: &[Summary]) -> Vec<Edit> {
    let mut out = Vec::new();
    for s in runs {
        let mut sites = Vec::new();
        for (pkg, f) in edited_files(s, &ws.root) {
            let Some(k) = ws.crates.get(&pkg) else {
                continue;
            };
            let g = graphs.get(&pkg);
            let site = match g.and_then(|g| g.node_of(&f)) {
                Some(0) => Site::CrateRoot { krate: pkg },
                Some(node) => Site::Module { krate: pkg, node },
                None if f.extension().is_some_and(|x| x == "rs") && f.starts_with(&k.dir) => {
                    // A source file outside the module tree (build.rs, an
                    // include!): treat as the root.
                    Site::CrateRoot { krate: pkg }
                }
                None => Site::BuildInput {
                    krate: pkg,
                    path: f
                        .strip_prefix(&ws.root)
                        .unwrap_or(&f)
                        .to_string_lossy()
                        .into_owned(),
                },
            };
            if !sites.contains(&site) {
                sites.push(site);
            }
        }
        if sites.is_empty() {
            continue;
        }
        let mut secs = HashMap::new();
        // Lib/bin units of packages cargo marked dirty in this run. Test
        // units are excluded: they rebuild only for the package under test,
        // and their size (a whole test harness) would inflate the cost of
        // the library a split would spare.
        let dirty: BTreeSet<&str> = s
            .rebuild_reasons
            .iter()
            .map(|r| r.package.as_str())
            .collect();
        for u in s.top_units.iter().filter(|u| u.local && u.kind != "test") {
            if let Some(p) = ws.package_of_unit(&u.name)
                && dirty.contains(p)
            {
                *secs.entry(p.to_owned()).or_default() += u.wall_share;
            }
        }
        let quiet = s.resources.avg_other_cores < 0.25 * s.ncpu.max(1) as f64;
        let check = matches!(s.subcommand.as_str(), "check" | "c" | "clippy");
        out.push(Edit {
            sites,
            secs,
            quiet,
            check,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Candidates.

/// Source text of every workspace crate, read once.
struct Sources(HashMap<String, Vec<String>>);

impl Sources {
    fn read(ws: &Workspace) -> Sources {
        let mut m = HashMap::new();
        for k in ws.crates.values() {
            let mut files = Vec::new();
            rust_files(&k.dir, &mut files);
            // A root package's directory contains the member crates; their
            // files belong to them, not to it.
            let nested: Vec<&Path> = ws
                .crates
                .values()
                .filter(|o| o.dir != k.dir && o.dir.starts_with(&k.dir))
                .map(|o| o.dir.as_path())
                .collect();
            let texts = files
                .iter()
                .filter(|f| !nested.iter().any(|d| f.starts_with(d)))
                .filter_map(|f| std::fs::read_to_string(f).ok())
                .collect();
            m.insert(k.name.clone(), texts);
        }
        Sources(m)
    }

    fn any(&self, krate: &str, re: &Regex) -> bool {
        self.0
            .get(krate)
            .is_some_and(|ts| ts.iter().any(|t| re.is_match(t)))
    }
}

#[derive(Debug)]
struct Candidate {
    krate: String,
    moved: BTreeSet<usize>,
    roots: Vec<String>,
    moved_lines: usize,
    crate_lines: usize,
    /// Dependents that use the moved code (must depend on the new crate).
    users: BTreeSet<String>,
    /// Dependents of the old crate that would no longer rebuild.
    spared: Vec<String>,
    /// Modules outside the moved set that `pub use` from it.
    reexported_by: Vec<String>,
    /// Inherent impls of types that stay behind: (type, defined in, impl modules).
    orphan_impls: Vec<(String, String, Vec<String>)>,
    /// References from the crate root (stays behind; must be rewritten).
    root_refs: usize,
    runs: usize,
    saved: f64,
}

/// Dependents of `krate` that mention a moved module path, either through
/// the crate name or through `crate::` (glob re-exports such as
/// `pub use jcode_base::*`).
fn users_of(ws: &Workspace, src: &Sources, krate: &str, roots: &[String]) -> BTreeSet<String> {
    let crate_alts: Vec<String> = std::iter::once("crate".to_owned())
        .chain(ws.crates.values().map(|c| regex::escape(&c.ident)))
        .collect();
    let mut alts = Vec::new();
    for r in roots {
        let segs: Vec<&str> = r.split("::").collect();
        let (last, parent) = segs.split_last().unwrap();
        let parent = parent.iter().map(|s| format!("{s}::")).collect::<String>();
        alts.push(format!(r"{}\b", regex::escape(r)));
        alts.push(format!(
            r"{}\{{[^}}]*\b{}\b",
            regex::escape(&parent),
            regex::escape(last)
        ));
    }
    let pat = format!(r"\b(?:{})::(?:{})", crate_alts.join("|"), alts.join("|"));
    let Ok(re) = Regex::new(&pat) else {
        return BTreeSet::new();
    };
    ws.dependents(krate)
        .into_iter()
        .filter(|d| src.any(d, &re))
        .collect()
}

fn evaluate(
    ws: &Workspace,
    src: &Sources,
    g: &ModuleGraph,
    krate: &str,
    root: usize,
    edits: &[Edit],
    typical: &Typical,
) -> Candidate {
    let moved = g.closure(root);
    let moved_lines = g.lines_of(&moved);
    let roots = g.roots(&moved);
    let share = moved_lines as f64 / g.total_lines.max(1) as f64;
    let mut c = Candidate {
        krate: krate.to_owned(),
        roots,
        moved_lines,
        crate_lines: g.total_lines,
        users: BTreeSet::new(),
        spared: Vec::new(),
        reexported_by: Vec::new(),
        orphan_impls: g.orphan_impls(&moved),
        root_refs: g.nodes[0]
            .refs
            .iter()
            .filter(|(t, _)| moved.contains(t))
            .map(|(_, n)| n)
            .sum(),
        runs: 0,
        saved: 0.0,
        moved,
    };
    c.reexported_by = g
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| !c.moved.contains(i) && n.reexports.iter().any(|t| c.moved.contains(t)))
        .map(|(_, n)| label(&n.path))
        .collect();
    if share > MAX_MOVE_SHARE {
        return c;
    }
    c.users = users_of(ws, src, krate, &c.roots);
    let affected = ws.with_dependents(&c.users);
    c.spared = ws
        .dependents(krate)
        .into_iter()
        .filter(|d| !affected.contains(d))
        .collect();
    for e in edits {
        let mine: Vec<&Site> = e.sites.iter().filter(|s| s.krate() == krate).collect();
        let inside = !mine.is_empty()
            && mine
                .iter()
                .all(|s| matches!(s, Site::Module { node, .. } if c.moved.contains(node)));
        if !inside {
            continue;
        }
        c.runs += 1;
        let other = e.others(ws, krate);
        if !other.contains(krate) {
            c.saved += typical.get(krate) * (1.0 - share);
        }
        for d in &c.spared {
            if !other.contains(d) && e.secs.contains_key(d) {
                c.saved += typical.get(d);
            }
        }
    }
    c
}

/// Typical seconds per package over the runs that rebuilt it: the median of
/// quiet `check`/`clippy` runs (other processes under a quarter of the cores),
/// the agent's edit loop and what the estimate is calibrated against. Falls
/// back to quiet runs of any kind, then to all runs. Two things inflate a
/// naive replay: runs made while other builds held the machine (60+ foreign
/// rustc) take 5-10x longer per unit, and `test`/`build` units do codegen,
/// costing 2-3x a metadata-only check of the same crate.
pub struct Typical(HashMap<String, f64>);

impl Typical {
    fn from(edits: &[Edit]) -> Typical {
        // (all, quiet, quiet check)
        type Samples = (Vec<f64>, Vec<f64>, Vec<f64>);
        let mut all: HashMap<String, Samples> = HashMap::new();
        for e in edits {
            for (p, s) in &e.secs {
                let entry = all.entry(p.clone()).or_default();
                entry.0.push(*s);
                if e.quiet {
                    entry.1.push(*s);
                    if e.check {
                        entry.2.push(*s);
                    }
                }
            }
        }
        let median = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        Typical(
            all.into_iter()
                .map(|(p, (any, quiet, check))| {
                    let v = [check, quiet, any]
                        .into_iter()
                        .find(|v| !v.is_empty())
                        .unwrap_or_default();
                    (p, median(v))
                })
                .collect(),
        )
    }

    fn get(&self, p: &str) -> f64 {
        self.0.get(p).copied().unwrap_or(0.0)
    }
}

/// Items of `file` changed most often in the last `days` (git hunk headers).
fn churned_items(repo: &Path, file: &Path, days: f64) -> Vec<(String, usize)> {
    let Ok(out) = Command::new("git")
        .args([
            "log",
            &format!("--since={}.days", days.ceil() as u64),
            "--format=",
            "--unified=0",
            "--",
        ])
        .arg(file)
        .current_dir(repo)
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"^@@[^@]*@@\s*(?:pub(?:\([^)]*\))?\s+)?(?:impl(?:<[^>]*>)?\s+(?:[\w:<>]+\s+for\s+)?|struct\s+|enum\s+|trait\s+|fn\s+|type\s+|const\s+|static\s+|mod\s+)([A-Za-z_]\w*)",
        )
        .unwrap()
    });
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(c) = re.captures(l) {
            *counts.entry(c[1].to_owned()).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

// ---------------------------------------------------------------------------
// Report.

#[cfg(test)]
pub fn report(ws: &Workspace, runs: &[Summary], days: f64, top: usize) -> String {
    report_with(ws, runs, runs, days, top)
}

/// `runs` are the edits to rank; `history` supplies typical unit costs. They
/// differ when replaying chosen runs (`--runs`) against the workspace's
/// history, so an estimate is never calibrated on the runs it is checked
/// against.
pub fn report_with(
    ws: &Workspace,
    runs: &[Summary],
    history: &[Summary],
    days: f64,
    top: usize,
) -> String {
    let graphs: HashMap<String, ModuleGraph> = ws
        .crates
        .values()
        .filter_map(|k| Some((k.name.clone(), ModuleGraph::build(k.root_file.as_deref()?))))
        .collect();
    let edits = to_edits(ws, &graphs, runs);
    let typical = Typical::from(&to_edits(ws, &graphs, history));

    // Cost per edit site, split evenly between a run's sites.
    #[derive(Default)]
    struct St {
        runs: usize,
        secs: f64,
        cascade: f64,
    }
    let mut stats: BTreeMap<Site, St> = BTreeMap::new();
    let mut total = 0.0;
    for e in &edits {
        let (t, c) = e.cost(ws);
        total += t;
        let share = 1.0 / e.sites.len() as f64;
        for s in &e.sites {
            let st = stats.entry(s.clone()).or_default();
            st.runs += 1;
            st.secs += t * share;
            st.cascade += c * share;
        }
    }
    let site_label = |s: &Site| match s {
        Site::Module { krate, node } => format!("{krate}::{}", graphs[krate].nodes[*node].path),
        Site::CrateRoot { krate } => format!("{krate} (crate root file)"),
        Site::BuildInput { krate, path } => format!("{path} (build-script input of {krate})"),
    };

    let mut o = String::new();
    let _ = writeln!(
        o,
        "Rebuild cost by edit site in {} (last {days:.0} days): {} runs with edits, \
         {:.0}s compiling the edited crates and their dependents",
        ws.root.display(),
        edits.len(),
        total
    );
    if edits.is_empty() {
        let _ = writeln!(o, "\nNo recorded runs with file edits here yet.");
        return o;
    }
    let mut ranked: Vec<(&Site, &St)> = stats.iter().collect();
    ranked.sort_by(|a, b| b.1.secs.total_cmp(&a.1.secs));
    let _ = writeln!(o, "\n   TOTAL  CASCADE  RUNS  EDIT SITE");
    for (site, st) in ranked.iter().take(top.max(10)) {
        let _ = writeln!(
            o,
            "{:>7.0}s {:>7.0}s {:>5}  {}",
            st.secs,
            st.cascade,
            st.runs,
            site_label(site)
        );
    }

    // Module candidates: every ancestor of every edited module.
    let src = Sources::read(ws);
    let mut roots: BTreeSet<(String, usize)> = BTreeSet::new();
    for (site, _) in &ranked {
        if let Site::Module { krate, node } = site {
            let g = &graphs[krate];
            let mut n = Some(*node);
            while let Some(x) = n.filter(|x| *x != 0) {
                // Test-only modules compile into the test harness, not the
                // library dependents see: moving them spares nothing.
                if !g.nodes[x].test {
                    roots.insert((krate.clone(), x));
                }
                n = g.nodes[x].parent;
            }
        }
    }
    let mut cands: Vec<Candidate> = Vec::new();
    let mut seen: BTreeSet<(String, BTreeSet<usize>)> = BTreeSet::new();
    for (krate, n) in roots {
        let g = &graphs[&krate];
        let c = evaluate(ws, &src, g, &krate, n, &edits, &typical);
        if seen.insert((krate.clone(), c.moved.clone())) && c.saved >= 1.0 {
            cands.push(c);
        }
    }
    // Keep the best of overlapping candidates in one crate, ready moves first.
    cands.sort_by(|a, b| {
        a.orphan_impls
            .is_empty()
            .cmp(&b.orphan_impls.is_empty())
            .reverse()
            .then(b.saved.total_cmp(&a.saved))
    });
    let mut shown: Vec<Candidate> = Vec::new();
    for c in cands {
        if !shown
            .iter()
            .any(|s| s.krate == c.krate && !s.moved.is_disjoint(&c.moved))
        {
            shown.push(c);
        }
    }

    let _ = writeln!(
        o,
        "\nModules to move into their own crate, best estimated saving first"
    );
    if shown.is_empty() {
        let _ = writeln!(
            o,
            "\n  none: every edited module is referenced from too much of its crate"
        );
    }
    for (i, c) in shown.iter().take(top).enumerate() {
        let ready = c.orphan_impls.is_empty();
        let _ = writeln!(
            o,
            "\n{}. {}::{{{}}}  saved ~{:.0}s over {} recorded runs (~{:.1}s per edit){}",
            i + 1,
            c.krate,
            c.roots.join(", "),
            c.saved,
            c.runs,
            c.saved / c.runs.max(1) as f64,
            if ready { "" } else { "  (needs prep)" }
        );
        let _ = writeln!(
            o,
            "   moves {} of {} lines ({:.1}%) into a new crate that depends on {}",
            c.moved_lines,
            c.crate_lines,
            100.0 * c.moved_lines as f64 / c.crate_lines.max(1) as f64,
            c.krate
        );
        let _ = writeln!(
            o,
            "   {}",
            if c.users.is_empty() {
                "no other workspace crate uses this code".to_owned()
            } else {
                format!(
                    "{} use it, so they still rebuild on these edits and need the new crate as a \
                     dependency or through a re-export (e.g. `pub use new_crate as {}` in a crate \
                     they already use)",
                    c.users.iter().cloned().collect::<Vec<_>>().join(", "),
                    c.roots
                        .first()
                        .and_then(|r| r.rsplit("::").next())
                        .unwrap_or("module")
                )
            }
        );
        let _ = writeln!(
            o,
            "   edits there would stop rebuilding {}{}",
            c.krate,
            if c.spared.is_empty() {
                String::new()
            } else {
                format!(
                    " and {} dependents ({})",
                    c.spared.len(),
                    short_list(&c.spared, 6)
                )
            }
        );
        if !c.reexported_by.is_empty() || c.root_refs > 0 {
            let mut v = c.reexported_by.clone();
            if c.root_refs > 0 && !v.iter().any(|x| x == "crate root") {
                v.push("crate root".into());
            }
            let _ = writeln!(
                o,
                "   also update: re-exports or uses in {}",
                short_list(&v, 6)
            );
        }
        for (ty, def, at) in c.orphan_impls.iter().take(4) {
            let _ = writeln!(
                o,
                "   blocker: `impl {ty} {{ .. }}` in {} but {ty} is defined in {def}, which stays; \
                 inherent impls must live in the defining crate, so turn them into free \
                 functions or an extension trait first",
                short_list(at, 3)
            );
        }
        if c.orphan_impls.len() > 4 {
            let _ = writeln!(o, "   +{} more such impls", c.orphan_impls.len() - 4);
        }
    }

    // Hot modules no viable candidate covers: show what ties them in.
    let mut tangled = Vec::new();
    for (site, st) in &ranked {
        let Site::Module { krate, node } = site else {
            continue;
        };
        if st.cascade < 1.0
            || shown
                .iter()
                .any(|c| &c.krate == krate && c.moved.contains(node))
        {
            continue;
        }
        let g = &graphs[krate];
        // The edited file's top-level module is the natural unit.
        let mut top_node = *node;
        while let Some(p) = g.nodes[top_node].parent.filter(|p| *p != 0) {
            top_node = p;
        }
        if tangled.iter().any(|(k, n, _)| k == krate && *n == top_node) {
            continue;
        }
        tangled.push((krate.clone(), top_node, *node));
    }
    if !tangled.is_empty() {
        let _ = writeln!(o, "\nHot modules too entangled to move as is");
        for (krate, top_node, node) in tangled.iter().take(top) {
            let g = &graphs[krate];
            let moved = g.closure(*node);
            let users = g.direct_users(*node);
            let st = &stats[&Site::Module {
                krate: krate.clone(),
                node: *node,
            }];
            let _ = writeln!(
                o,
                "\n- {krate}::{} ({} runs, {:.0}s cascade; in {})",
                g.nodes[*node].path, st.runs, st.cascade, g.nodes[*top_node].path
            );
            let _ = writeln!(
                o,
                "  moving it drags along {:.0}% of {krate}; cut these references first: {}",
                100.0 * g.lines_of(&moved) as f64 / g.total_lines.max(1) as f64,
                if users.is_empty() {
                    "none found".to_owned()
                } else {
                    short_list(
                        &users
                            .iter()
                            .map(|(u, n)| format!("{u} ({n})"))
                            .collect::<Vec<_>>(),
                        6,
                    )
                }
            );
        }
    }

    // Crate root files and build inputs.
    let mut extra = String::new();
    for (site, st) in ranked.iter().filter(|(_, st)| st.cascade >= 1.0) {
        match site {
            Site::CrateRoot { krate } => {
                let k = &ws.crates[krate];
                let Some(file) = &k.root_file else { continue };
                let deps = ws.dependents(krate);
                let _ = writeln!(
                    extra,
                    "\n- {krate} root file: {} runs, {:.0}s cascade into {} dependents",
                    st.runs,
                    st.cascade,
                    deps.len()
                );
                let items = churned_items(&ws.root, file, days);
                if items.is_empty() {
                    continue;
                }
                let mut users_all: BTreeSet<String> = BTreeSet::new();
                let mut lines = Vec::new();
                for (item, n) in items.iter().take(6) {
                    let Ok(re) = Regex::new(&format!(r"\b{}\b", regex::escape(item))) else {
                        continue;
                    };
                    let users: Vec<String> =
                        deps.iter().filter(|d| src.any(d, &re)).cloned().collect();
                    users_all.extend(users.iter().cloned());
                    lines.push(format!(
                        "    {item} ({n} edits): used by {}",
                        if users.is_empty() {
                            "no dependent".to_owned()
                        } else {
                            short_list(&users, 4)
                        }
                    ));
                }
                let _ = writeln!(extra, "  most edited items (git, last {days:.0} days):");
                for l in lines {
                    let _ = writeln!(extra, "{l}");
                }
                let affected = ws.with_dependents(&users_all);
                let spared = deps.iter().filter(|d| !affected.contains(*d)).count();
                let _ = writeln!(
                    extra,
                    "  moving them to a crate only their users depend on would spare {spared} of {} \
                     dependents on those edits",
                    deps.len()
                );
            }
            Site::BuildInput { krate, path } => {
                let deps = ws.dependents(krate);
                let _ = writeln!(
                    extra,
                    "\n- {path}: {krate}'s build script reruns on change, recompiling {krate} and \
                     {} dependents ({} runs, {:.0}s). Embed it in a small leaf crate used only \
                     where it is read, or load it at runtime",
                    deps.len(),
                    st.runs,
                    st.secs
                );
            }
            Site::Module { .. } => {}
        }
    }
    if !extra.is_empty() {
        let _ = writeln!(o, "\nCrate roots and build inputs");
        o.push_str(&extra);
    }
    let _ = writeln!(
        o,
        "\nSavings count the recorded runs whose edits in that crate all fall inside the \
         moved code, priced at each crate's typical rebuild time (median of quiet `check` \
         runs): the crate rebuilds only its moved share, and dependents that do not use the \
         moved code no longer rebuild. Which crates stop rebuilding was exact in two real \
         splits; the seconds came out about 2x high (FINDINGS 15b), so use them to rank. \
         The new crate needs the dependencies and test-support features its code uses."
    );
    o
}

fn short_list(v: &[String], n: usize) -> String {
    if v.len() <= n {
        v.join(", ")
    } else {
        format!("{}, +{} more", v[..n].join(", "), v.len() - n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::{RebuildReason, UnitBreakdown};

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// base { logging, auth { store }, emails, net { http } }
    ///   auth uses logging; emails uses logging; net::http uses auth.
    /// mid depends on base and uses auth (through a glob re-export);
    /// app depends on mid and uses emails by crate path.
    fn fixture(name: &str) -> (PathBuf, Workspace) {
        let root = std::env::temp_dir().join(format!("jr-split-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base/src");
        write(
            &base.join("lib.rs"),
            "pub mod logging;\npub mod auth;\npub mod emails;\npub mod net;\n\
             pub use emails::Email;\n#[cfg(test)]\nmod lib_tests;\n",
        );
        write(&base.join("logging.rs"), &"pub fn log() {}\n".repeat(200));
        write(
            &base.join("auth.rs"),
            &format!(
                "use crate::logging;\n#[path = \"auth_store.rs\"]\nmod store;\npub use store::S;\n{}",
                "pub fn a() {}\n".repeat(100)
            ),
        );
        write(
            &base.join("auth_store.rs"),
            "pub struct S;\npub fn s() { super::super::logging::log() }\n",
        );
        write(
            &base.join("emails.rs"),
            "use crate::{\n    logging,\n};\npub struct Email;\n#[cfg(test)]\nmod tests {\n use crate::auth;\n}\n",
        );
        write(&base.join("net/mod.rs"), "pub mod http;\n");
        write(
            &base.join("net/http.rs"),
            "use crate::auth::a;\npub fn get() {}\n",
        );
        write(&base.join("lib_tests.rs"), "use crate::net;\n");
        write(
            &root.join("mid/src/lib.rs"),
            "pub use base::*;\nfn f() { crate::auth::a() }\n",
        );
        write(
            &root.join("app/src/main.rs"),
            "use base::{emails::Email};\nfn main() {}\n",
        );
        let meta = serde_json::json!({
            "workspace_root": root,
            "packages": [
                {"name": "base", "manifest_path": root.join("base/Cargo.toml"),
                 "targets": [{"kind": ["lib"], "src_path": base.join("lib.rs")}],
                 "dependencies": [{"name": "app", "kind": "dev"}]},
                {"name": "mid", "manifest_path": root.join("mid/Cargo.toml"),
                 "targets": [{"kind": ["lib"], "src_path": root.join("mid/src/lib.rs")}],
                 "dependencies": [{"name": "base", "kind": null}]},
                {"name": "app", "manifest_path": root.join("app/Cargo.toml"),
                 "targets": [{"kind": ["bin"], "src_path": root.join("app/src/main.rs")}],
                 "dependencies": [{"name": "mid", "kind": null}, {"name": "base", "kind": null}]},
            ]
        });
        (root.clone(), Workspace::from_metadata(&meta))
    }

    fn run(pkg: &str, file: &str, units: &[(&str, f64)]) -> Summary {
        let mut s = Summary {
            ncpu: 16,
            ..Default::default()
        };
        s.rebuild_reasons.push(RebuildReason {
            package: pkg.into(),
            reason: format!("the file `{file}` has changed"),
        });
        for (n, w) in units {
            let base = n.split(" (").next().unwrap();
            if base != pkg {
                s.rebuild_reasons.push(RebuildReason {
                    package: base.into(),
                    reason: format!("the dependency `{pkg}` was rebuilt"),
                });
            }
            s.top_units.push(UnitBreakdown {
                name: (*n).into(),
                kind: if n.ends_with("(test)") { "test" } else { "lib" }.into(),
                local: true,
                wall_share: *w,
                ..Default::default()
            });
        }
        s
    }

    fn graph(ws: &Workspace) -> ModuleGraph {
        ModuleGraph::build(ws.crates["base"].root_file.as_deref().unwrap())
    }

    fn id(g: &ModuleGraph, p: &str) -> usize {
        g.by_path[p]
    }

    #[test]
    fn dev_dependency_cycles_are_ignored() {
        let (root, ws) = fixture("dev");
        assert_eq!(
            ws.dependents("base"),
            BTreeSet::from(["mid".to_owned(), "app".to_owned()])
        );
        assert!(ws.dependents("app").is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn module_graph_resolves_nested_paths() {
        let (root, ws) = fixture("graph");
        let g = graph(&ws);
        let store = g.node_of(&root.join("base/src/auth_store.rs")).unwrap();
        assert_eq!(g.nodes[store].path, "auth::store");
        let (auth, logging, emails) = (id(&g, "auth"), id(&g, "logging"), id(&g, "emails"));
        // super::super::logging from auth::store.
        assert!(g.nodes[store].refs.contains_key(&logging));
        // Multi-line group import.
        assert!(g.nodes[emails].refs.contains_key(&logging));
        // Test-only reference is not an edge.
        assert!(!g.nodes[emails].refs.contains_key(&auth));
        // `pub use store::S` is a re-export, not an edge.
        assert!(g.nodes[auth].reexports.contains(&store));
        assert!(!g.nodes[auth].refs.contains_key(&store));
        // Deep reference resolves to the deepest module.
        let http = id(&g, "net::http");
        assert!(g.nodes[http].refs.contains_key(&auth));
        // The root's `pub use emails::Email` is a re-export.
        assert!(g.nodes[0].reexports.contains(&emails));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn closure_pulls_in_users() {
        let (root, ws) = fixture("closure");
        let g = graph(&ws);
        let names = |s: BTreeSet<usize>| -> BTreeSet<String> {
            s.into_iter().map(|n| g.nodes[n].path.clone()).collect()
        };
        assert_eq!(
            names(g.closure(id(&g, "emails"))),
            BTreeSet::from(["emails".to_owned()])
        );
        // auth is used by net::http, whose parent net does not reference it.
        assert_eq!(
            names(g.closure(id(&g, "auth"))),
            BTreeSet::from(["auth".into(), "auth::store".into(), "net::http".into()])
        );
        assert_eq!(g.roots(&g.closure(id(&g, "auth"))), ["auth", "net::http"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recommends_leaf_module_and_estimates_saving() {
        let (root, ws) = fixture("leaf");
        let mut busy = run(
            "base",
            "base/src/emails.rs",
            &[("base", 90.0), ("mid", 80.0), ("app", 20.0)],
        );
        busy.resources.avg_other_cores = 14.0;
        let runs = vec![
            run(
                "base",
                "base/src/emails.rs",
                &[("base", 10.0), ("mid", 8.0), ("app", 2.0)],
            ),
            // The test unit is not part of what a split spares.
            run(
                "base",
                "base/src/emails.rs",
                &[("base (test)", 6.0), ("base", 12.0), ("mid", 6.0)],
            ),
            // A run on a contended machine: excluded from the typical cost.
            busy,
        ];
        let out = report(&ws, &runs, 30.0, 5);
        assert!(out.contains("1. base::{emails}"), "{out}");
        assert!(out.contains("app use it"), "{out}");
        assert!(
            out.contains("stop rebuilding base and 1 dependents (mid)"),
            "{out}"
        );
        assert!(out.contains("re-exports or uses in crate root"), "{out}");
        // Typical quiet costs (upper median of two): base 12s, mid 8s.
        let g = graph(&ws);
        let share = g.nodes[id(&g, "emails")].lines as f64 / g.total_lines as f64;
        let per_edit = 12.0 * (1.0 - share) + 8.0;
        let total = 3.0 * per_edit;
        assert!(
            out.contains(&format!(
                "saved ~{total:.0}s over 3 recorded runs (~{per_edit:.1}s per edit)"
            )),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn typical_cost_prefers_quiet_check_runs() {
        let mk = |sub: &str, quiet: bool, secs: f64| Edit {
            sites: Vec::new(),
            secs: HashMap::from([("base".to_owned(), secs)]),
            quiet,
            check: sub == "check",
        };
        let t = Typical::from(&[
            mk("check", true, 3.0),
            mk("test", true, 9.0),
            mk("test", true, 10.0),
            mk("check", false, 30.0),
        ]);
        assert_eq!(t.get("base"), 3.0);
        let t = Typical::from(&[mk("test", true, 9.0), mk("check", false, 30.0)]);
        assert_eq!(t.get("base"), 9.0);
        let t = Typical::from(&[mk("check", false, 30.0)]);
        assert_eq!(t.get("base"), 30.0);
    }

    #[test]
    fn test_only_modules_are_not_candidates() {
        let (root, ws) = fixture("testonly");
        // lib_tests.rs is `#[cfg(test)] mod lib_tests;`: edits rebuild only
        // the crate's own test unit.
        let runs = vec![run(
            "base",
            "base/src/lib_tests.rs",
            &[("base (test)", 9.0)],
        )];
        let out = report(&ws, &runs, 30.0, 5);
        assert!(!out.contains("base::{lib_tests}"), "{out}");
        assert!(out.contains("none: "), "{out}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn entangled_module_lists_edges_to_cut() {
        let (root, ws) = fixture("tangled");
        let runs = vec![run(
            "base",
            "base/src/logging.rs",
            &[("base", 10.0), ("mid", 8.0), ("app", 2.0)],
        )];
        let out = report(&ws, &runs, 30.0, 5);
        assert!(out.contains("too entangled"), "{out}");
        assert!(out.contains("auth::store (1)"), "{out}");
        assert!(out.contains("emails (1)"), "{out}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inherent_impls_of_staying_types_block_a_move() {
        let (root, ws) = fixture("orphan");
        write(
            &root.join("base/src/emails.rs"),
            "pub struct Email;\nimpl crate::auth::Session {\n fn f(&self) {}\n}\n\
             impl Email { fn g() {} }\nimpl Clone for Email { fn clone(&self) -> Self { Email } }\n",
        );
        write(
            &root.join("base/src/auth.rs"),
            "pub struct Session;\n#[path = \"auth_store.rs\"]\nmod store;\n",
        );
        let g = graph(&ws);
        let moved = g.closure(id(&g, "emails"));
        assert_eq!(
            g.orphan_impls(&moved),
            [(
                "Session".to_owned(),
                "auth".to_owned(),
                vec!["emails".to_owned()]
            )]
        );
        let runs = vec![run(
            "base",
            "base/src/emails.rs",
            &[("base", 10.0), ("mid", 8.0)],
        )];
        let out = report(&ws, &runs, 30.0, 5);
        assert!(out.contains("(needs prep)"), "{out}");
        assert!(out.contains("blocker: `impl Session"), "{out}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn build_script_inputs_are_named() {
        let (root, ws) = fixture("buildinput");
        let s = run("mid", "mid/../docs", &[("mid", 8.0), ("app", 2.0)]);
        let out = report(&ws, &[s], 30.0, 5);
        assert!(out.contains("docs (build-script input of mid)"), "{out}");
        assert!(out.contains("build script reruns"), "{out}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runs_from_sibling_dirs_with_a_shared_prefix_do_not_belong() {
        let mut s = Summary {
            cwd: "/x/jcode-desktop".into(),
            ..Default::default()
        };
        assert!(!belongs(&s, Path::new("/x/jcode")));
        s.cwd = "/x/jcode/crates/a".into();
        assert!(belongs(&s, Path::new("/x/jcode")));
        s.cwd = "/elsewhere".into();
        s.git.root = Some("/x/jcode".into());
        assert!(belongs(&s, Path::new("/x/jcode")));
    }

    #[test]
    fn root_package_does_not_claim_member_crate_sources() {
        let root = std::env::temp_dir().join(format!("jr-split-{}-nested", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(&root.join("src/main.rs"), "fn main() {}\n");
        write(&root.join("crates/a/src/lib.rs"), "pub mod m;\n");
        write(&root.join("crates/a/src/m.rs"), "pub fn f() {}\n");
        write(&root.join("crates/b/src/lib.rs"), "fn g() { a::m::f() }\n");
        write(&root.join("target-release/x.rs"), "fn h() { a::m::f() }\n");
        let meta = serde_json::json!({
            "workspace_root": root,
            "packages": [
                {"name": "app", "manifest_path": root.join("Cargo.toml"),
                 "targets": [{"kind": ["bin"], "src_path": root.join("src/main.rs")}],
                 "dependencies": [{"name": "a"}, {"name": "b"}]},
                {"name": "a", "manifest_path": root.join("crates/a/Cargo.toml"),
                 "targets": [{"kind": ["lib"], "src_path": root.join("crates/a/src/lib.rs")}],
                 "dependencies": []},
                {"name": "b", "manifest_path": root.join("crates/b/Cargo.toml"),
                 "targets": [{"kind": ["lib"], "src_path": root.join("crates/b/src/lib.rs")}],
                 "dependencies": [{"name": "a"}]},
            ]
        });
        let ws = Workspace::from_metadata(&meta);
        let src = Sources::read(&ws);
        assert_eq!(
            users_of(&ws, &src, "a", &["m".to_owned()]),
            BTreeSet::from(["b".to_owned()])
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn strips_trailing_test_module() {
        let src = "use crate::a;\n#[cfg(test)]\nmod tests {\n use crate::b;\n}\n";
        assert_eq!(strip_test_tail(src), "use crate::a;\n");
    }

    #[test]
    fn statements_keep_use_groups_together() {
        let s = statements("pub use a::{\n b,\n c::{d},\n};\nfn f() { x::y(); }\n");
        assert_eq!(s[0], "pub use a::{\n b,\n c::{d},\n};");
        assert!(s.iter().any(|x| x.contains("x::y()")));
    }
}
