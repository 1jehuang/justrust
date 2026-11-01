//! `justrust split --apply <crate>::<module>`: move a module into a new crate,
//! in the working tree the user is in, with a commit on each side.
//!
//! Contract:
//! - Refuses unless the git tree is completely clean (nothing staged,
//!   modified or untracked). So HEAD is the exact pre-split state, and the
//!   split is one `git revert` (or `git reset --hard <pre>`) away.
//! - Refuses candidates the report would not call ready: the module must be
//!   movable on its own (nothing left in the crate references it), with no
//!   inherent impls of types that stay, no feature-gated code, and no
//!   manifest-relative macros (`include_str!`, `env!("CARGO_MANIFEST_DIR")`).
//! - Rewrites deterministically (no AI): moves the files, makes the new
//!   crate depend on the old one, rewrites paths in the moved code and in
//!   user crates, and writes manifests by copying the old crate's
//!   dependency specs.
//! - Verifies with cargo: `check` of the new crate (with a small set of
//!   rule-based fixes for missing dependencies), `check --workspace
//!   --all-targets`, and `test` of the new crate, which must run the same
//!   number of tests the moved code had before.
//! - On success: one commit with the user's git identity whose message
//!   names the pre-split commit. On failure: every touched path is restored
//!   from HEAD and the new crate directory removed, so the tree is exactly
//!   the pre-split state again.

use crate::paths;
use crate::split::{ModuleGraph, Workspace};
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What the split will do, computed before touching anything.
#[derive(Debug)]
pub struct Plan {
    pub old: String,
    pub old_ident: String,
    pub module: String,
    pub new: String,
    pub new_ident: String,
    pub new_dir: PathBuf,
    /// (from, to, module depth below the moved root) for every moved file;
    /// `to` relative to the new crate dir.
    pub files: Vec<(PathBuf, PathBuf, usize)>,
    /// The file declaring `mod <leaf>;` and the module's parent path.
    pub parent_file: PathBuf,
    pub parent_path: String,
    pub leaf: String,
    /// Workspace crates whose source mentions the module.
    pub users: BTreeSet<String>,
    /// Users that glob re-export the old crate (`pub use old::*`) and so
    /// resolve `crate::<module>` through it.
    pub glob_reexporters: BTreeSet<String>,
    /// (name, path in module) the old root re-exported from the module.
    pub reexports: Vec<(String, String)>,
    /// Tests in the moved files, to compare with what `cargo test` runs.
    pub test_count: usize,
    /// Behavior that changes without a compile error, printed with the plan.
    pub warnings: Vec<String>,
}

pub fn command(target: &str, new_name: Option<&str>, dry_run: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::load(&cwd)?;
    let root = ws.root.clone();
    let pre = if dry_run {
        None
    } else {
        Some(require_clean(&root)?)
    };
    let plan = plan(&ws, target, new_name)?;
    print_plan(&ws, &plan);
    if dry_run {
        println!("\ndry run: nothing changed");
        return Ok(());
    }
    let pre = pre.expect("checked above");
    let mut touched = Touched::default();
    match apply(&ws, &plan, &mut touched).and_then(|()| verify(&ws, &plan, &mut touched)) {
        Ok(()) => {
            let sha = commit(&root, &plan, &pre, &touched)?;
            println!(
                "\nsplit committed as {sha} (pre-split {pre}). Undo with `git revert {sha}` \
                 or `git reset --hard {pre}`."
            );
            Ok(())
        }
        Err(e) => {
            touched.restore(&root);
            Err(e.context(format!(
                "split failed; restored the working tree to {pre} (nothing was committed)"
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Preconditions.

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git")
        .args(args)
        .current_dir(root)
        .stderr(Stdio::piped())
        .output()
        .context("running git")?;
    if !o.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim_end().to_owned())
}

/// The HEAD commit, if the tree is clean. Untracked files count: they could
/// be swept into the split commit or collide with the new crate.
fn require_clean(root: &Path) -> Result<String> {
    let status = git(root, &["status", "--porcelain", "--untracked-files=normal"])?;
    if !status.is_empty() {
        let n = status.lines().count();
        bail!(
            "the working tree has {n} uncommitted or untracked paths. Commit (or stash) \
             everything first, so the pre-split state is a commit you can return to:\n{}",
            status.lines().take(10).collect::<Vec<_>>().join("\n")
        );
    }
    git(root, &["rev-parse", "--short=12", "HEAD"])
}

// ---------------------------------------------------------------------------
// Planning.

pub fn plan(ws: &Workspace, target: &str, new_name: Option<&str>) -> Result<Plan> {
    let (old, module) = target
        .split_once("::")
        .with_context(|| format!("expected <crate>::<module path>, got {target}"))?;
    let k = ws
        .crates
        .get(old)
        .or_else(|| ws.crates.values().find(|k| k.ident == old))
        .with_context(|| format!("no workspace crate named {old}"))?;
    let root_file = k.root_file.as_deref().context("crate has no lib target")?;
    if k.root_file
        .as_deref()
        .and_then(Path::file_name)
        .is_some_and(|n| n == "main.rs")
    {
        bail!("{} is a binary crate; nothing can depend on it", k.name);
    }
    let g = ModuleGraph::build(root_file);
    let n = g
        .node_at(module)
        .with_context(|| format!("{} has no file module {module}", k.name))?;
    if g.nodes[n].test {
        bail!("{module} is test-only; moving it would not spare any rebuild");
    }
    let moved = g.closure(n);
    let sub = g.subtree_of(n);
    if moved != sub {
        let extra: Vec<String> = moved
            .difference(&sub)
            .map(|x| g.nodes[*x].path.clone())
            .collect();
        bail!(
            "{module} is referenced by other modules of {} ({}), which would have to move \
             too. Cut those references first (`justrust split` lists them)",
            k.name,
            extra.join(", ")
        );
    }
    let orphans = g.orphan_impls(&moved);
    if let Some((ty, def, _)) = orphans.first() {
        bail!(
            "{module} has `impl {ty} {{ .. }}` for a type defined in {def}, which stays in {}. \
             Inherent impls must live in the defining crate; turn them into free functions or \
             an extension trait first",
            k.name
        );
    }
    // The crate root may only declare or re-export it, never use it.
    let root_uses: usize = g.nodes[0]
        .refs
        .iter()
        .filter(|(t, _)| moved.contains(t))
        .map(|(_, c)| c)
        .sum();
    if root_uses > 0 {
        bail!(
            "the crate root of {} uses {module} directly; move those uses into a module \
             first",
            k.name
        );
    }
    let crate_dir = root_file.parent().context("root file has no dir")?;
    let mut files = Vec::new();
    let mut test_count = 0;
    let mut warnings = Vec::new();
    static TEST_ATTR: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let test_attr =
        TEST_ATTR.get_or_init(|| Regex::new(r"#\[(?:[a-z_]+::)?test(?:\([^)\]]*\))?\]").unwrap());
    let top_file = &g.nodes[n].file;
    let log_macro = Regex::new(
        r"\b(?:log|tracing)::(?:trace|debug|info|warn|error|event|span)!|#\[(?:tracing::)?instrument",
    )?;
    let top_dir = top_file.parent().unwrap_or(crate_dir);
    // A `foo.rs` with children in `foo/`, or a `foo/mod.rs`.
    let top_is_mod_rs = top_file.file_name().is_some_and(|f| f == "mod.rs");
    for x in &moved {
        let f = &g.nodes[*x].file;
        let src = std::fs::read_to_string(f)?;
        test_count += test_attr.find_iter(&src).count();
        for bad in [
            "include_str!",
            "include_bytes!",
            "include!(",
            "CARGO_MANIFEST_DIR",
            "CARGO_PKG_NAME",
            "CARGO_CRATE_NAME",
            "module_path!",
        ] {
            if src.contains(bad) {
                bail!(
                    "{} uses {bad}, whose value changes with the crate it is compiled in \
                     (paths, names, log targets); move it by hand",
                    f.display()
                );
            }
        }
        if log_macro.is_match(&src) {
            warnings.push(format!(
                "{} logs through log/tracing macros; their target changes from {}::{} to the \
                 new crate's name, so target-based filters need updating",
                f.display(),
                k.ident,
                module
            ));
        }
        if src.contains("cfg(feature") || src.contains("cfg_attr(feature") {
            bail!(
                "{} has feature-gated code; the new crate would need matching features. \
                 Move it by hand",
                f.display()
            );
        }
        let to = if *x == n {
            PathBuf::from("src/lib.rs")
        } else if top_is_mod_rs {
            PathBuf::from("src").join(
                f.strip_prefix(top_dir)
                    .with_context(|| format!("{} is outside {}", f.display(), top_dir.display()))?,
            )
        } else {
            // Children of `foo.rs` live in `foo/...` or are `#[path]`ed next
            // to it; keep their position relative to `foo.rs`'s directory.
            PathBuf::from("src").join(f.strip_prefix(top_dir).with_context(|| {
                format!(
                    "{} is outside {}; move it by hand",
                    f.display(),
                    top_dir.display()
                )
            })?)
        };
        let depth = g.nodes[*x].path.matches("::").count() - g.nodes[n].path.matches("::").count();
        files.push((f.clone(), to, depth));
    }
    // Children of a non-mod.rs top file are looked up in `src/<stem>/`, but
    // as lib.rs they are looked up in `src/`. Strip the stem directory.
    if !top_is_mod_rs {
        let stem = top_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        for (from, to, _) in files.iter_mut() {
            if from == top_file {
                continue;
            }
            if let Ok(rest) = to.strip_prefix(Path::new("src").join(&stem)) {
                *to = Path::new("src").join(rest);
            } else if has_path_attr_child(top_file)? {
                bail!(
                    "{} uses #[path] children; move it by hand",
                    top_file.display()
                );
            }
        }
    }
    let leaf = module.rsplit("::").next().unwrap_or(module).to_owned();
    let parent_path = module
        .rsplit_once("::")
        .map(|(p, _)| p.to_owned())
        .unwrap_or_default();
    let parent = g.nodes[n].parent.context("module has no parent")?;
    let parent_file = g.nodes[parent].file.clone();
    let new = match new_name {
        Some(n) => n.to_owned(),
        None => format!("{}-{}", k.name, module.replace("::", "-").replace('_', "-")),
    };
    if ws.crates.contains_key(&new) {
        bail!("a crate named {new} already exists");
    }
    let new_dir = k
        .dir
        .parent()
        .context("crate dir has no parent")?
        .join(&new);
    if new_dir.exists() {
        bail!("{} already exists", new_dir.display());
    }
    let src = crate::split::Sources::read(ws);
    let mut users = crate::split::users_of(ws, &src, &k.name, &[module.to_owned()]);
    // Items the old root re-exports from the module (`pub use m::Email;`)
    // stop existing there; users reaching them as `old::Email` (or through a
    // glob re-export as `crate::Email`) are users too.
    let root_src = std::fs::read_to_string(root_file)?;
    let reexports = if parent_path.is_empty() {
        root_reexports(&root_src, &leaf)
    } else {
        Vec::new()
    };
    if reexports.iter().any(|(n, _)| n == "*") {
        bail!(
            "{} glob re-exports {module} (`pub use {module}::*`); users cannot be found \
             reliably. Replace it with named re-exports first",
            k.name
        );
    }
    if !reexports.is_empty() {
        let names: Vec<String> = reexports.iter().map(|(n, _)| regex::escape(n)).collect();
        let re = Regex::new(&format!(
            r"\b(?:crate|{})::(?:\{{[^}}]*\b)?(?:{})\b",
            regex::escape(&k.ident),
            names.join("|")
        ))?;
        users.extend(
            ws.dependents(&k.name)
                .into_iter()
                .filter(|d| src.any(d, &re)),
        );
    }
    let glob_re = Regex::new(&format!(
        r"(?m)^\s*pub\s+use\s+{}::\*\s*;",
        regex::escape(&k.ident)
    ))?;
    let glob_reexporters: BTreeSet<String> = ws
        .dependents(&k.name)
        .into_iter()
        .filter(|d| src.any(d, &glob_re))
        .collect();
    // A glob re-exporter hands the old root's names on to its own
    // dependents (`mid::Email`); they would need rewriting too.
    if !reexports.is_empty() {
        for g in &glob_reexporters {
            let gi = regex::escape(&ws.crates[g].ident);
            let names: Vec<String> = reexports.iter().map(|(n, _)| regex::escape(n)).collect();
            let re = Regex::new(&format!(r"\b{gi}::(?:{})\b", names.join("|")))?;
            if let Some(d) = ws.dependents(g).into_iter().find(|d| src.any(d, &re)) {
                bail!(
                    "{d} uses {} names re-exported from {} through {g}'s glob re-export; \
                     update those by hand first",
                    module,
                    k.name
                );
            }
        }
    }
    Ok(Plan {
        old: k.name.clone(),
        old_ident: k.ident.clone(),
        module: module.to_owned(),
        new_ident: new.replace('-', "_"),
        new,
        new_dir,
        files,
        parent_file,
        parent_path,
        leaf,
        users,
        glob_reexporters,
        reexports,
        test_count,
        warnings,
    })
}

fn has_path_attr_child(f: &Path) -> Result<bool> {
    Ok(std::fs::read_to_string(f)?.contains("#[path"))
}

fn print_plan(ws: &Workspace, p: &Plan) {
    let rel = |x: &Path| x.strip_prefix(&ws.root).unwrap_or(x).display().to_string();
    println!(
        "split {}::{} into new crate {} ({})",
        p.old,
        p.module,
        p.new,
        rel(&p.new_dir)
    );
    for (from, to, _) in &p.files {
        println!("  move {} -> {}", rel(from), rel(&p.new_dir.join(to)));
    }
    println!("  remove `mod {};` from {}", p.leaf, rel(&p.parent_file));
    if !p.users.is_empty() {
        println!(
            "  users to update: {}",
            p.users.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    if !p.glob_reexporters.is_empty() {
        println!(
            "  keep `crate::{}` working via `pub use {} as {};` in {}",
            p.module,
            p.new_ident,
            p.leaf,
            p.glob_reexporters
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("  tests moved: {}", p.test_count);
    for w in &p.warnings {
        println!("  warning: {w}");
    }
}

// ---------------------------------------------------------------------------
// Applying.

/// Every path the split wrote, created or removed, for restore on failure.
#[derive(Default)]
struct Touched {
    modified: BTreeSet<PathBuf>,
    created_dirs: Vec<PathBuf>,
}

impl Touched {
    fn write(&mut self, p: &Path, s: &str) -> Result<()> {
        self.modified.insert(p.to_path_buf());
        std::fs::write(p, s).with_context(|| format!("writing {}", p.display()))
    }

    fn restore(&self, root: &Path) {
        for d in self.created_dirs.iter().rev() {
            let _ = std::fs::remove_dir_all(d);
        }
        // Tracked files come back from HEAD; files that did not exist there
        // (only possible inside created dirs) are already gone.
        let tracked: Vec<String> = self
            .modified
            .iter()
            .filter_map(|p| p.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        for chunk in tracked.chunks(100) {
            let mut args = vec!["checkout", "HEAD", "--"];
            args.extend(chunk.iter().map(String::as_str));
            // Paths that are not in HEAD make checkout fail as a whole, so
            // retry one by one.
            if git(root, &args).is_err() {
                for p in chunk {
                    let _ = git(root, &["checkout", "HEAD", "--", p]);
                }
            }
        }
    }
}

fn apply(ws: &Workspace, p: &Plan, t: &mut Touched) -> Result<()> {
    let k = &ws.crates[&p.old];
    // Cargo rewrites the lockfile when the new member appears; restore it
    // with everything else on failure.
    let lock = ws.root.join("Cargo.lock");
    if lock.exists() {
        t.modified.insert(lock);
    }
    std::fs::create_dir_all(p.new_dir.join("src"))?;
    t.created_dirs.push(p.new_dir.clone());

    // 1. Moved files: rewrite paths into the old crate, then move.
    for (from, to, depth) in &p.files {
        let src = std::fs::read_to_string(from)?;
        let is_top = *depth == 0;
        let out = rewrite_moved(&src, &p.old_ident, &p.parent_path, is_top, *depth);
        let dest = p.new_dir.join(to);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&dest, out)?;
        std::fs::remove_file(from)?;
        t.modified.insert(from.clone());
    }
    // Remove module directories of the old crate the move emptied.
    let old_src = k
        .root_file
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = p
        .files
        .iter()
        .filter_map(|(from, _, _)| from.parent().map(Path::to_path_buf))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for d in dirs {
        let mut d = d;
        while d.starts_with(&old_src)
            && d != old_src
            && d.is_dir()
            && std::fs::read_dir(&d)?.next().is_none()
        {
            std::fs::remove_dir(&d)?;
            match d.parent() {
                Some(p) => d = p.to_path_buf(),
                None => break,
            }
        }
    }

    // 2. The old crate stops declaring it; re-exports of it from the old
    //    crate are impossible (the new crate depends on the old), so they go
    //    to the glob re-exporters instead.
    let parent_src = std::fs::read_to_string(&p.parent_file)?;
    let parent_out = remove_mod_decl(&parent_src, &p.leaf)?;
    t.write(&p.parent_file, &parent_out)?;
    if p.parent_path.is_empty() {
        let root_file = k.root_file.as_deref().unwrap();
        let s = std::fs::read_to_string(root_file)?;
        let s2 = remove_reexports(&s, &p.leaf);
        if s2 != s {
            t.write(root_file, &s2)?;
        }
    }

    // 3. New manifest.
    let manifest = new_manifest(ws, p)?;
    std::fs::write(p.new_dir.join("Cargo.toml"), manifest)?;

    // 4. Workspace manifest: member (unless a glob covers it) and, when the
    //    workspace declares the old crate in [workspace.dependencies], the
    //    new one too.
    let ws_manifest = ws.root.join("Cargo.toml");
    let s = std::fs::read_to_string(&ws_manifest)?;
    let rel_new = p
        .new_dir
        .strip_prefix(&ws.root)
        .unwrap_or(&p.new_dir)
        .to_string_lossy()
        .into_owned();
    let rel_old = k
        .dir
        .strip_prefix(&ws.root)
        .unwrap_or(&k.dir)
        .to_string_lossy()
        .into_owned();
    let s2 = add_workspace_member(&s, &rel_new, &p.old, &p.new, &rel_old)?;
    if s2 != s {
        t.write(&ws_manifest, &s2)?;
    }

    // 5. Users: rewrite explicit `old::m` / `old::Name` paths; such crates
    //    need the new crate as a dependency. Crates that only reach the
    //    module as `crate::m` through a glob re-export chain need nothing:
    //    the alias added to the glob re-exporter covers them.
    for u in &p.users {
        let uk = &ws.crates[u];
        let glob = p.glob_reexporters.contains(u);
        let mut files = Vec::new();
        crate::split::rust_files(&uk.dir, &mut files);
        let nested: Vec<PathBuf> = ws
            .crates
            .values()
            .filter(|o| o.dir != uk.dir && o.dir.starts_with(&uk.dir))
            .map(|o| o.dir.clone())
            .collect();
        let mut rewrote = false;
        for f in files
            .iter()
            .filter(|f| !nested.iter().any(|d| f.starts_with(d)))
        {
            let s = std::fs::read_to_string(f)?;
            let mut s2 = rewrite_user(&s, &p.old_ident, &p.module, &p.new_ident);
            for (name, path) in &p.reexports {
                s2 = rewrite_reexported(&s2, &p.old_ident, name, &p.new_ident, path, glob);
            }
            if s2 != s {
                t.write(f, &s2)?;
                rewrote = true;
            }
        }
        if rewrote || glob {
            let m = uk.dir.join("Cargo.toml");
            let s = std::fs::read_to_string(&m)?;
            let s2 = add_user_dependency(&s, &p.old, &p.new, &uk.dir, &p.new_dir)?;
            if s2 != s {
                t.write(&m, &s2)?;
            }
        }
        if glob {
            let rf = uk.root_file.as_deref().unwrap();
            let orig = std::fs::read_to_string(rf)?;
            let s2 = add_glob_alias(&orig, &p.old_ident, &p.new_ident, &p.module, &p.reexports)?;
            if s2 != orig {
                t.write(rf, &s2)?;
            }
        }
    }
    Ok(())
}

/// Rewrite a moved file. `crate::x` meant the old crate; `super::` chains
/// that climb out of the moved subtree meant the old crate's
/// `<parent_path>`. `self::` and inner `super::` keep working because the
/// subtree keeps its shape.
pub fn rewrite_moved(
    src: &str,
    old: &str,
    parent_path: &str,
    is_top: bool,
    depth: usize,
) -> String {
    static CRATE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let crate_re = CRATE.get_or_init(|| Regex::new(r"\bcrate::").unwrap());
    let old_parent = if parent_path.is_empty() {
        format!("{old}::")
    } else {
        format!("{old}::{parent_path}::")
    };
    let s = crate_re.replace_all(src, format!("{old}::")).into_owned();
    // `super::` chains climb out of the moved subtree when they are longer
    // than the current module's depth inside it: the file's own depth below
    // the subtree root plus any inline `mod x { .. }` it is nested in.
    let s = rewrite_escaping_super(&s, depth, &old_parent);
    // `pub(super)` at the top was visible to the parent, which stays in the
    // old crate and is now a dependent's view: make it public. Inline
    // modules keep their own `pub(super)`.
    if is_top {
        rewrite_top_level_pub_super(&s)
    } else {
        s
    }
}

/// Replace `super::` chains that leave the moved subtree. Tracks inline
/// `mod name {` blocks by brace depth (strings, chars and comments are
/// skipped so braces inside them do not count).
fn rewrite_escaping_super(src: &str, file_depth: usize, old_parent: &str) -> String {
    static MOD_OPEN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let mod_open = MOD_OPEN
        .get_or_init(|| Regex::new(r"^(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z_]\w*\s*\{").unwrap());
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    // Brace depth at which each open inline module started.
    let mut inline: Vec<usize> = Vec::new();
    let mut braces = 0usize;
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        // Skip comments, strings and char literals.
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            i += 2;
            while i + 1 < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i += 2;
            continue;
        }
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if b[i] == b'\'' && i + 2 < b.len() && (b[i + 2] == b'\'' || b[i + 1] == b'\\') {
            // A char literal like 'a' or '\n' (not a lifetime).
            i += if b[i + 1] == b'\\' { 4 } else { 3 };
            continue;
        }
        let at_word = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if at_word && (b[i] == b'm' || b[i] == b'p') && mod_open.is_match(&src[i..]) {
            let m = mod_open.find(&src[i..]).unwrap();
            i += m.end();
            braces += 1;
            inline.push(braces);
            continue;
        }
        match b[i] {
            b'{' => braces += 1,
            b'}' => {
                if inline.last() == Some(&braces) {
                    inline.pop();
                }
                braces = braces.saturating_sub(1);
            }
            b's' if at_word && b[i..].starts_with(b"super::") => {
                let mut n = 0;
                let mut j = i;
                while b[j..].starts_with(b"super::") {
                    n += 1;
                    j += 7;
                }
                let depth_here = file_depth + inline.len();
                if n > depth_here {
                    // Climbs depth_here levels to the subtree root, then
                    // n - depth_here - 1 more above the old parent.
                    let extra = n - depth_here - 1;
                    out.push_str(&src[last..i]);
                    let mut target = old_parent.trim_end_matches("::").to_owned();
                    for _ in 0..extra {
                        match target.rfind("::") {
                            Some(p) => target.truncate(p),
                            None => break,
                        }
                    }
                    out.push_str(&target);
                    out.push_str("::");
                    last = j;
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str(&src[last..]);
    out
}

/// `pub(super)` on items outside any inline module becomes `pub`.
fn rewrite_top_level_pub_super(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut braces = 0i32;
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        if braces == 0 && t.starts_with("pub(super) ") {
            out.push_str(&line.replacen("pub(super) ", "pub ", 1));
        } else {
            out.push_str(line);
        }
        for c in line.chars() {
            match c {
                '{' => braces += 1,
                '}' => braces -= 1,
                _ => {}
            }
        }
    }
    out
}

/// Remove `mod <leaf>;` (with its attributes, doc comments and `pub`).
pub fn remove_mod_decl(src: &str, leaf: &str) -> Result<String> {
    let re = Regex::new(&format!(
        r"(?m)^(?:[ \t]*(?:#\[[^\]]*\]|///[^\n]*)[ \t]*\n)*[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+{}\s*;[ \t]*\n",
        regex::escape(leaf)
    ))?;
    let n = re.find_iter(src).count();
    if n != 1 {
        bail!("expected exactly one `mod {leaf};` declaration, found {n}");
    }
    Ok(re.replace(src, "").into_owned())
}

/// Drop `pub use <leaf>::...;` re-exports from the old crate root.
pub fn remove_reexports(src: &str, leaf: &str) -> String {
    let re = Regex::new(&format!(
        r"(?m)^[ \t]*pub(?:\([^)]*\))?\s+use\s+(?:self::|crate::)?{}::[^;]*;[ \t]*\n",
        regex::escape(leaf)
    ))
    .unwrap();
    re.replace_all(src, "").into_owned()
}

/// Names the old crate root re-exports from the module, as (exported name,
/// path inside the module): `pub use m::A;` gives (A, A), `pub use m::x::B
/// as C;` gives (C, x::B), `pub use m::{A, B as D};` gives both. A glob
/// (`pub use m::*;`) gives ("*", "*").
pub fn root_reexports(src: &str, leaf: &str) -> Vec<(String, String)> {
    let re = Regex::new(&format!(
        r"(?m)^[ \t]*pub(?:\([^)]*\))?\s+use\s+(?:self::|crate::)?{}::([^;]*);",
        regex::escape(leaf)
    ))
    .unwrap();
    let mut out = Vec::new();
    let item = |s: &str, prefix: &str, out: &mut Vec<(String, String)>| {
        let s = s.trim();
        if s.is_empty() {
            return;
        }
        let (path, alias) = match s.split_once(" as ") {
            Some((p, a)) => (p.trim(), Some(a.trim())),
            None => (s, None),
        };
        let full = format!("{prefix}{path}");
        let name = alias
            .map(str::to_owned)
            .unwrap_or_else(|| path.rsplit("::").next().unwrap_or(path).to_owned());
        out.push((name, full));
    };
    for c in re.captures_iter(src) {
        let rest = c[1].trim();
        if let Some(open) = rest.find('{') {
            let prefix = &rest[..open];
            let inner = rest[open + 1..].trim_end_matches('}');
            for part in inner.split(',') {
                item(part, prefix, &mut out);
            }
        } else {
            item(rest, "", &mut out);
        }
    }
    out
}

/// Rewrite references in a user crate: `old::m` -> `new`, `old::m::x` ->
/// `new::x`, and `old::{.., m, ..}` / `old::{.., m::x, ..}` split out.
pub fn rewrite_user(src: &str, old: &str, module: &str, new: &str) -> String {
    // `use old::m;` binds `m`; `use new as m;` keeps that binding.
    let use_whole = Regex::new(&format!(
        r"(?m)^([ \t]*(?:pub(?:\([^)]*\))?\s+)?use\s+){}::{}\s*;",
        regex::escape(old),
        regex::escape(module)
    ))
    .unwrap();
    let s = use_whole
        .replace_all(src, format!("${{1}}{new} as {module};"))
        .into_owned();
    let full = Regex::new(&format!(
        r"\b{}::{}\b",
        regex::escape(old),
        regex::escape(module)
    ))
    .unwrap();
    let mut s = full.replace_all(&s, new).into_owned();
    // `use old::{a, m, b};` -> `use old::{a, b}; use new as m;`
    let group = Regex::new(&format!(
        r"(?m)^([ \t]*(?:pub(?:\([^)]*\))?\s+)?use\s+){}::\{{([^{{}}]*)\}};",
        regex::escape(old)
    ))
    .unwrap();
    let leaf_re = Regex::new(&format!(r"^{}(?:::(.+))?$", regex::escape(module))).unwrap();
    s = group
        .replace_all(&s, |c: &regex::Captures| {
            let prefix = &c[1];
            let mut keep = Vec::new();
            let mut moved = Vec::new();
            for item in c[2].split(',').map(str::trim).filter(|x| !x.is_empty()) {
                match leaf_re.captures(item) {
                    Some(m) => moved.push(match m.get(1) {
                        Some(rest) => format!("{new}::{}", rest.as_str()),
                        // `use old::{m}` bound the name `m`; keep it bound.
                        None => format!("{new} as {module}"),
                    }),
                    None => keep.push(item.to_owned()),
                }
            }
            if moved.is_empty() {
                return c[0].to_owned();
            }
            let mut out = String::new();
            if !keep.is_empty() {
                out.push_str(&format!("{prefix}{old}::{{{}}};", keep.join(", ")));
                out.push('\n');
            }
            let indent: String = prefix.chars().take_while(|c| c.is_whitespace()).collect();
            let vis = prefix.trim().strip_suffix("use").unwrap_or("").trim_end();
            let vis = if vis.is_empty() {
                String::new()
            } else {
                format!("{vis} ")
            };
            out.push_str(
                &moved
                    .iter()
                    .map(|m| format!("{indent}{vis}use {m};"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            out
        })
        .into_owned();
    s
}

/// Rewrite `old::Name` (a name the old root re-exported from the module) to
/// `new::path`, including inside `old::{..}` groups. In a glob re-exporter,
/// `crate::Name` keeps resolving through an added alias, so only explicit
/// `old::` paths change.
pub fn rewrite_reexported(
    src: &str,
    old: &str,
    name: &str,
    new: &str,
    path: &str,
    _glob: bool,
) -> String {
    let target = format!("{new}::{path}");
    let full = Regex::new(&format!(
        r"\b{}::{}\b",
        regex::escape(old),
        regex::escape(name)
    ))
    .unwrap();
    let mut s = full.replace_all(src, target.as_str()).into_owned();
    let group = Regex::new(&format!(
        r"(?m)^([ \t]*(?:pub(?:\([^)]*\))?\s+)?use\s+){}::\{{([^{{}}]*)\}};",
        regex::escape(old)
    ))
    .unwrap();
    let item_re = Regex::new(&format!(r"^{}(\s+as\s+\w+)?$", regex::escape(name))).unwrap();
    s = group
        .replace_all(&s, |c: &regex::Captures| {
            let prefix = &c[1];
            let mut keep = Vec::new();
            let mut moved = Vec::new();
            for item in c[2].split(',').map(str::trim).filter(|x| !x.is_empty()) {
                match item_re.captures(item) {
                    Some(m) => moved.push(format!(
                        "{target}{}",
                        m.get(1).map(|a| a.as_str()).unwrap_or("")
                    )),
                    None => keep.push(item.to_owned()),
                }
            }
            if moved.is_empty() {
                return c[0].to_owned();
            }
            let mut out = String::new();
            if !keep.is_empty() {
                out.push_str(&format!("{prefix}{old}::{{{}}};\n", keep.join(", ")));
            }
            out.push_str(
                &moved
                    .iter()
                    .map(|m| format!("{prefix}{m};"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            out
        })
        .into_owned();
    s
}

/// In a crate that does `pub use old::*;`, keep `crate::<module>` resolving.
pub fn add_glob_alias(
    src: &str,
    old: &str,
    new: &str,
    module: &str,
    reexports: &[(String, String)],
) -> Result<String> {
    if module.contains("::") {
        bail!(
            "{module} is nested; a glob re-export of {old} cannot alias it. Move a top-level \
             module, or update `crate::{module}` users by hand"
        );
    }
    let re = Regex::new(&format!(
        r"(?m)^([ \t]*)pub\s+use\s+{}::\*\s*;[^\n]*\n",
        regex::escape(old)
    ))?;
    let Some(m) = re.find(src) else {
        return Ok(src.to_owned());
    };
    // Skip names the file already binds (e.g. an explicit `pub use old::m;`
    // that was just rewritten to `pub use new as m;`).
    let bound = |name: &str| {
        Regex::new(&format!(
            r"(?m)^[ \t]*pub(?:\([^)]*\))?\s+use\s+[^;]*\bas\s+{}\s*;",
            regex::escape(name)
        ))
        .map(|r| r.is_match(src))
        .unwrap_or(false)
    };
    let mut line = String::new();
    if !bound(module) {
        line.push_str(&format!("pub use {new} as {module};\n"));
    }
    for (name, path) in reexports {
        if !bound(name) {
            line.push_str(&format!("pub use {new}::{path} as {name};\n"));
        }
    }
    let mut out = String::with_capacity(src.len() + line.len());
    out.push_str(&src[..m.end()]);
    out.push_str(&line);
    out.push_str(&src[m.end()..]);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Manifests.

fn rel_path(from_dir: &Path, to: &Path) -> String {
    // Both absolute and normalized: walk up from `from_dir` to the common
    // prefix, then down to `to`.
    let f: Vec<_> = from_dir.components().collect();
    let t: Vec<_> = to.components().collect();
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut p = PathBuf::new();
    for _ in common..f.len() {
        p.push("..");
    }
    for c in &t[common..] {
        p.push(c);
    }
    p.to_string_lossy().into_owned()
}

/// Identifiers of external crates a source uses (`foo::` at path start,
/// `use foo`, `extern crate foo`), restricted to `candidates`.
fn used_crates(src: &str, candidates: &BTreeSet<String>) -> BTreeSet<String> {
    candidates
        .iter()
        .filter(|c| {
            let re = Regex::new(&format!(
                r"(?:^|[^\w:]){}::|\buse\s+{}\b",
                regex::escape(c),
                regex::escape(c)
            ))
            .unwrap();
            re.is_match(src)
        })
        .cloned()
        .collect()
}

fn new_manifest(ws: &Workspace, p: &Plan) -> Result<String> {
    let k = &ws.crates[&p.old];
    let old_manifest: toml_edit::DocumentMut = std::fs::read_to_string(k.dir.join("Cargo.toml"))?
        .parse()
        .context("parsing the old crate's Cargo.toml")?;
    let mut doc = toml_edit::DocumentMut::new();
    let mut pkg = toml_edit::Table::new();
    pkg.insert("name", toml_edit::value(p.new.clone()));
    if let Some(old_pkg) = old_manifest.get("package").and_then(|x| x.as_table()) {
        for key in [
            "version",
            "edition",
            "license",
            "publish",
            "rust-version",
            "authors",
        ] {
            if let Some(v) = old_pkg.get(key) {
                pkg.insert(key, v.clone());
            }
        }
    }
    doc.insert("package", toml_edit::Item::Table(pkg));
    if let Some(l) = old_manifest.get("lints") {
        doc.insert("lints", l.clone());
    }

    // Moved sources, split into non-test and test text.
    let mut main_src = String::new();
    let mut test_src = String::new();
    for (_, to, _) in &p.files {
        let src = std::fs::read_to_string(p.new_dir.join(to)).unwrap_or_default();
        let body = crate::split::strip_test_tail_pub(&src);
        main_src.push_str(body);
        test_src.push_str(&src[body.len()..]);
    }

    let mut deps = toml_edit::Table::new();
    // The old crate, by the same spec a user uses (keeps features and
    // default-features), with the path recomputed from the new crate.
    deps.insert(&p.old, old_dep_spec(ws, p)?);
    let mut dev = toml_edit::Table::new();
    let tables = |name: &str| -> Option<toml_edit::Table> {
        old_manifest.get(name).and_then(|x| x.as_table()).cloned()
    };
    let old_deps = tables("dependencies").unwrap_or_default();
    let old_dev = tables("dev-dependencies").unwrap_or_default();
    let ident_of = |key: &str, item: &toml_edit::Item| -> String {
        item.get("package")
            .and_then(|v| v.as_str())
            .map(|_| key.to_owned())
            .unwrap_or_else(|| key.to_owned())
            .replace('-', "_")
    };
    let all: BTreeSet<String> = old_deps
        .iter()
        .chain(old_dev.iter())
        .map(|(k, v)| ident_of(k, v))
        .collect();
    let used_main = used_crates(&main_src, &all);
    let used_test = used_crates(&test_src, &all);
    for (key, item) in old_deps.iter() {
        let id = ident_of(key, item);
        if item.get("optional").and_then(|v| v.as_bool()) == Some(true) && used_main.contains(&id) {
            bail!("the moved code uses optional dependency {key}; move it by hand");
        }
        if used_main.contains(&id) {
            deps.insert(key, fix_paths(item.clone(), &k.dir, &p.new_dir));
        } else if used_test.contains(&id) {
            dev.insert(key, fix_paths(item.clone(), &k.dir, &p.new_dir));
        }
    }
    for (key, item) in old_dev.iter() {
        let id = ident_of(key, item);
        if (used_main.contains(&id) || used_test.contains(&id)) && !deps.contains_key(key) {
            dev.insert(key, fix_paths(item.clone(), &k.dir, &p.new_dir));
        }
    }
    doc.insert("dependencies", toml_edit::Item::Table(deps));
    if !dev.is_empty() {
        doc.insert("dev-dependencies", toml_edit::Item::Table(dev));
    }
    let mut out = format!(
        "# Split out of {} by `justrust split --apply {}::{}`, so edits here no longer\n\
         # rebuild {} and the crates that depend on it.\n",
        p.old, p.old, p.module, p.old
    );
    out.push_str(&doc.to_string());
    Ok(out)
}

/// Rebase relative `path = ".."` values from the old crate dir to the new.
fn fix_paths(mut item: toml_edit::Item, old_dir: &Path, new_dir: &Path) -> toml_edit::Item {
    let fix = |v: &mut toml_edit::Value| {
        if let Some(s) = v.as_str()
            && !Path::new(s).is_absolute()
        {
            let abs = normalize(&old_dir.join(s));
            *v = toml_edit::Value::from(rel_path(new_dir, &abs));
        }
    };
    if let Some(t) = item.as_inline_table_mut()
        && let Some(v) = t.get_mut("path")
    {
        fix(v);
    } else if let Some(t) = item.as_table_mut()
        && let Some(toml_edit::Item::Value(v)) = t.get_mut("path")
    {
        fix(v);
    }
    item
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            o => out.push(o),
        }
    }
    out
}

/// How the new crate depends on the old one: copy a user's spec (so
/// `default-features = false` and features carry over), else a plain path.
fn old_dep_spec(ws: &Workspace, p: &Plan) -> Result<toml_edit::Item> {
    let k = &ws.crates[&p.old];
    for u in ws.dependents(&p.old) {
        let uk = &ws.crates[&u];
        let doc: toml_edit::DocumentMut =
            match std::fs::read_to_string(uk.dir.join("Cargo.toml"))?.parse() {
                Ok(d) => d,
                Err(_) => continue,
            };
        if let Some(item) = doc.get("dependencies").and_then(|d| d.get(&p.old)) {
            if item.get("workspace").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(item.clone());
            }
            return Ok(fix_paths(item.clone(), &uk.dir, &p.new_dir));
        }
    }
    let mut t = toml_edit::InlineTable::new();
    t.insert("path", rel_path(&p.new_dir, &k.dir).into());
    Ok(toml_edit::Item::Value(toml_edit::Value::InlineTable(t)))
}

/// Add the new crate to `[workspace] members` next to the old one, and to
/// `[workspace.dependencies]` if the old one is declared there.
pub fn add_workspace_member(
    src: &str,
    rel_new: &str,
    old: &str,
    new: &str,
    rel_old: &str,
) -> Result<String> {
    let mut doc: toml_edit::DocumentMut =
        src.parse().context("parsing the workspace Cargo.toml")?;
    let Some(ws) = doc.get_mut("workspace").and_then(|w| w.as_table_mut()) else {
        bail!("no [workspace] table in the root Cargo.toml");
    };
    if let Some(members) = ws.get_mut("members").and_then(|m| m.as_array_mut()) {
        let covered = members.iter().any(|m| {
            m.as_str().is_some_and(|g| {
                g == rel_new
                    || (g.ends_with("/*")
                        && Path::new(rel_new).parent() == Some(Path::new(g.trim_end_matches("/*"))))
            })
        });
        if !covered {
            let pos = members
                .iter()
                .position(|m| m.as_str() == Some(rel_old))
                .map(|i| i + 1)
                .unwrap_or(members.len());
            members.insert(pos, rel_new);
            // Keep the one-per-line layout the old entry had.
            if let Some(prev) = members.get(pos.saturating_sub(1)) {
                let decor = prev.decor().clone();
                if let Some(v) = members.get_mut(pos) {
                    *v.decor_mut() = decor;
                }
            }
        }
    }
    if let Some(wd) = ws
        .get_mut("dependencies")
        .and_then(|d| d.as_table_like_mut())
        && wd.contains_key(old)
        && !wd.contains_key(new)
    {
        let mut t = toml_edit::InlineTable::new();
        t.insert("path", rel_new.into());
        wd.insert(
            new,
            toml_edit::Item::Value(toml_edit::Value::InlineTable(t)),
        );
    }
    Ok(doc.to_string())
}

/// Make a user crate depend on the new one, in the same style as its
/// dependency on the old one (`.workspace = true` or a path).
pub fn add_user_dependency(
    src: &str,
    old: &str,
    new: &str,
    user_dir: &Path,
    new_dir: &Path,
) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = src.parse().context("parsing a user Cargo.toml")?;
    let mut done = false;
    for table in ["dependencies", "dev-dependencies"] {
        let Some(deps) = doc.get_mut(table).and_then(|d| d.as_table_like_mut()) else {
            continue;
        };
        let Some(old_item) = deps.get(old) else {
            continue;
        };
        if deps.contains_key(new) {
            done = true;
            continue;
        }
        let workspace = old_item.get("workspace").and_then(|v| v.as_bool()) == Some(true);
        let mut t = toml_edit::InlineTable::new();
        if workspace {
            t.insert("workspace", true.into());
        } else {
            t.insert("path", rel_path(user_dir, new_dir).into());
        }
        let item = if workspace && old_item.as_value().is_some_and(|v| v.is_inline_table()) {
            toml_edit::Item::Value(toml_edit::Value::InlineTable(t))
        } else if workspace {
            // `foo.workspace = true` dotted style.
            let mut tt = toml_edit::Table::new();
            tt.set_dotted(true);
            tt.insert("workspace", toml_edit::value(true));
            toml_edit::Item::Table(tt)
        } else {
            toml_edit::Item::Value(toml_edit::Value::InlineTable(t))
        };
        deps.insert(new, item);
        done = true;
        break;
    }
    if !done {
        bail!("{old} is not a direct dependency here; add {new} by hand");
    }
    Ok(doc.to_string())
}

// ---------------------------------------------------------------------------
// Verification.

fn cargo(root: &Path, args: &[&str]) -> Result<(bool, String)> {
    // Through `justrust cargo` so the builds are recorded and use this
    // session's build slot like any other agent build. Falls back to cargo.
    let me = std::env::current_exe().ok();
    let mut cmd = match &me {
        Some(j) if std::env::var_os("JUSTRUST_SPLIT_PLAIN_CARGO").is_none() => {
            let mut c = Command::new(j);
            c.arg("cargo");
            c
        }
        _ => Command::new(paths::real_cargo()?),
    };
    let o = cmd
        .args(args)
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .output()
        .context("running cargo")?;
    let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&o.stderr));
    Ok((o.status.success(), text))
}

fn first_errors(out: &str, n: usize) -> String {
    // `--message-format=short` puts the path first: `src/x.rs:1:2: error[E..]: ..`.
    let errs: Vec<&str> = out
        .lines()
        .filter(|l| l.contains(": error") || l.starts_with("error["))
        .take(n)
        .collect();
    if errs.is_empty() {
        out.lines()
            .filter(|l| l.starts_with("error"))
            .take(n)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        errs.join("\n")
    }
}

fn verify(ws: &Workspace, p: &Plan, t: &mut Touched) -> Result<()> {
    let root = &ws.root;
    let manifest = p.new_dir.join("Cargo.toml");
    // Rule-based fixes for the new crate, bounded.
    for round in 0..4 {
        println!(
            "verify: cargo check -p {} --all-targets (round {})",
            p.new,
            round + 1
        );
        let (ok, out) = cargo(
            root,
            &[
                "check",
                "-p",
                &p.new,
                "--all-targets",
                "--message-format=short",
            ],
        )?;
        if ok {
            break;
        }
        let fixed = fix_new_crate(ws, p, &manifest, &out)?;
        if !fixed {
            bail!(
                "the new crate does not compile and no automatic fix applies:\n{}",
                first_errors(&out, 8)
            );
        }
    }
    // Format before the workspace check, so the check covers the result.
    format_touched(ws, p, t);
    println!("verify: cargo check --workspace --all-targets");
    let (ok, out) = cargo(
        root,
        &[
            "check",
            "--workspace",
            "--all-targets",
            "--message-format=short",
        ],
    )?;
    if !ok {
        bail!(
            "the workspace does not compile after the split:\n{}",
            first_errors(&out, 8)
        );
    }
    println!("verify: cargo test -p {}", p.new);
    let (ok, out) = cargo(root, &["test", "-p", &p.new])?;
    if !ok {
        bail!("tests of the new crate fail:\n{}", first_errors(&out, 8));
    }
    let ran = count_tests(&out);
    if ran < p.test_count {
        bail!(
            "the moved code had {} tests but only {ran} ran in the new crate; something \
             (a cfg or a feature) dropped tests",
            p.test_count
        );
    }
    println!(
        "verify: {ran} tests ran in {} (moved: {})",
        p.new, p.test_count
    );
    Ok(())
}

/// rustfmt the Rust files the split wrote, but only files that were already
/// rustfmt-clean at HEAD (checked against their HEAD content), so the split
/// commit never carries unrelated formatting changes. Best effort: a
/// missing rustfmt or a parse failure leaves files as written.
fn format_touched(ws: &Workspace, p: &Plan, t: &Touched) {
    let k = &ws.crates[&p.old];
    let edition = std::fs::read_to_string(k.dir.join("Cargo.toml"))
        .ok()
        .and_then(|s| s.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|d| {
            d.get("package")
                .and_then(|p| p.get("edition"))
                .and_then(|e| e.as_str().map(str::to_owned))
        })
        .or_else(|| {
            std::fs::read_to_string(ws.root.join("Cargo.toml"))
                .ok()
                .and_then(|s| s.parse::<toml_edit::DocumentMut>().ok())
                .and_then(|d| {
                    d.get("workspace")
                        .and_then(|w| w.get("package"))
                        .and_then(|p| p.get("edition"))
                        .and_then(|e| e.as_str().map(str::to_owned))
                })
        })
        .unwrap_or_else(|| "2021".into());
    let clean_at_head = |rel: &str| -> bool {
        // `git show` output is trimmed at the end; compare ignoring that.
        let Ok(orig) = git(&ws.root, &["show", &format!("HEAD:{rel}")]) else {
            return false;
        };
        let mut c = Command::new("rustfmt");
        c.args(["--edition", &edition, "--emit", "stdout"])
            .current_dir(&ws.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(mut child) = c.spawn() else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(orig.as_bytes());
            let _ = stdin.write_all(b"\n");
        }
        match child.wait_with_output() {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim_end() == orig.trim_end()
            }
            _ => false,
        }
    };
    let mut targets: Vec<PathBuf> = Vec::new();
    for f in &t.modified {
        if f.extension().is_some_and(|x| x == "rs") && f.exists() {
            let rel = f
                .strip_prefix(&ws.root)
                .unwrap_or(f)
                .to_string_lossy()
                .into_owned();
            if clean_at_head(&rel) {
                targets.push(f.clone());
            }
        }
    }
    // Moved files: formatted if their original was clean.
    for (from, to, _) in &p.files {
        let rel = from
            .strip_prefix(&ws.root)
            .unwrap_or(from)
            .to_string_lossy()
            .into_owned();
        if clean_at_head(&rel) {
            targets.push(p.new_dir.join(to));
        }
    }
    if targets.is_empty() {
        return;
    }
    // Through stdin, one file at a time: a path argument would make rustfmt
    // follow `mod` declarations and reformat files the split never touched.
    for f in targets {
        let Ok(src) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(mut child) = Command::new("rustfmt")
            .args(["--edition", &edition, "--emit", "stdout"])
            .current_dir(&ws.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(src.as_bytes());
        }
        if let Ok(o) = child.wait_with_output()
            && o.status.success()
            && !o.stdout.is_empty()
        {
            let _ = std::fs::write(&f, &o.stdout);
        }
    }
}

/// Passed + failed + ignored across `test result:` lines (doc tests too).
fn count_tests(out: &str) -> usize {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored").unwrap()
    });
    re.captures_iter(out)
        .map(|c| {
            (1..=3)
                .map(|i| c[i].parse::<usize>().unwrap_or(0))
                .sum::<usize>()
        })
        .sum()
}

/// Apply one round of known fixes from `cargo check` output. Returns
/// whether anything changed.
fn fix_new_crate(ws: &Workspace, p: &Plan, manifest: &Path, out: &str) -> Result<bool> {
    let k = &ws.crates[&p.old];
    let mut doc: toml_edit::DocumentMut = std::fs::read_to_string(manifest)?.parse()?;
    let old_doc: toml_edit::DocumentMut =
        std::fs::read_to_string(k.dir.join("Cargo.toml"))?.parse()?;
    let mut changed = false;
    // E0433/E0432: unresolved crate `x` that the old crate depends on.
    static UNRESOLVED: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = UNRESOLVED.get_or_init(|| {
        Regex::new(r"(?:use of unresolved module or unlinked crate|unresolved import|cannot find module or crate|failed to resolve: use of undeclared crate or module) `(\w+)`").unwrap()
    });
    let in_tests = out.contains("(lib test)") || out.contains("--test");
    for c in re.captures_iter(out) {
        let id = &c[1];
        for (table, target) in [
            (
                "dependencies",
                if in_tests {
                    "dev-dependencies"
                } else {
                    "dependencies"
                },
            ),
            ("dev-dependencies", "dev-dependencies"),
        ] {
            let found = old_doc
                .get(table)
                .and_then(|t| t.as_table_like())
                .and_then(|t| t.iter().find(|(key, _)| key.replace('-', "_") == id))
                .map(|(key, item)| (key.to_owned(), item.clone()));
            if let Some((key, item)) = found {
                let tgt = doc
                    .entry(target)
                    .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                    .as_table_like_mut()
                    .context("dependency table")?;
                if !tgt.contains_key(&key) {
                    tgt.insert(&key, fix_paths(item, &k.dir, &p.new_dir));
                    println!("  fix: add {key} to [{target}]");
                    changed = true;
                }
                break;
            }
        }
    }
    // Items of the old crate behind its `test-support` feature, used by the
    // moved tests: enable the feature for tests only.
    let old_has_test_support = old_doc
        .get("features")
        .and_then(|f| f.get("test-support"))
        .is_some();
    let missing_in_old = Regex::new(&format!(
        r"cannot find \w+ `\w+` in (?:module|crate) `{}(?:::\w+)*`",
        regex::escape(&p.old_ident)
    ))?;
    if old_has_test_support && missing_in_old.is_match(out) {
        let dev = doc
            .entry("dev-dependencies")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_like_mut()
            .context("dev-dependencies")?;
        if !dev.contains_key(&p.old) {
            let mut spec = old_dep_spec(ws, p)?;
            if let Some(t) = spec.as_inline_table_mut() {
                let mut feats = toml_edit::Array::new();
                feats.push("test-support");
                t.insert("features", toml_edit::Value::Array(feats));
                t.fmt();
            }
            dev.insert(&p.old, spec);
            println!("  fix: enable {}'s test-support feature for tests", p.old);
            changed = true;
        }
    }
    if changed {
        std::fs::write(manifest, doc.to_string())?;
    }
    Ok(changed)
}

// ---------------------------------------------------------------------------
// Commit.

fn commit(root: &Path, p: &Plan, pre: &str, t: &Touched) -> Result<String> {
    let mut paths: Vec<String> = t
        .modified
        .iter()
        .chain(t.created_dirs.iter())
        .filter_map(|x| x.strip_prefix(root).ok())
        .map(|x| x.to_string_lossy().into_owned())
        .collect();
    // Cargo.lock changes with the new crate.
    if root.join("Cargo.lock").exists() {
        paths.push("Cargo.lock".into());
    }
    let mut args = vec!["add", "-A", "--"];
    args.extend(paths.iter().map(String::as_str));
    git(root, &args)?;
    let msg = format!(
        "split: move {}::{} into new crate {}\n\n\
         Automated by `justrust split --apply`. Pre-split commit: {pre}.\n\
         Edits to this code no longer rebuild {} and the crates that only depend on it.",
        p.old, p.module, p.new, p.old
    );
    git(root, &["commit", "-q", "-m", &msg])?;
    git(root, &["rev-parse", "--short=12", "HEAD"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_paths_in_moved_files() {
        let src = "use crate::logging;\nuse super::helpers::x;\n\
                   fn f() { crate::auth::a(); super::super::y(); self::z(); }\n\
                   pub(super) fn g() {}\n\
                   #[cfg(test)]\nmod tests {\n    use super::*;\n    pub(super) fn h() {}\n    \
                   fn t() { super::super::helpers::w(); let s = \"super::nope\"; }\n}\n";
        // Top file of `net::http` (parent `net`), depth 0.
        let out = rewrite_moved(src, "base", "net", true, 0);
        assert!(out.contains("use base::logging;"), "{out}");
        assert!(out.contains("use base::net::helpers::x;"), "{out}");
        assert!(out.contains("base::auth::a()"), "{out}");
        // Two levels up from the top: the old crate root.
        assert!(out.contains("base::y()"), "{out}");
        assert!(out.contains("self::z()"), "{out}");
        assert!(out.contains("pub fn g()"), "{out}");
        // Inside the inline test module `super::*` is the moved module.
        assert!(out.contains("    use super::*;"), "{out}");
        assert!(out.contains("    pub(super) fn h()"), "{out}");
        assert!(out.contains("base::net::helpers::w()"), "{out}");
        assert!(out.contains("\"super::nope\""), "{out}");
        // A child file one level down: `super::` stays, `super::super::` escapes.
        let child = rewrite_moved(
            "use super::super::q;\nuse super::sib;\n",
            "base",
            "",
            false,
            1,
        );
        assert!(child.contains("use base::q;"), "{child}");
        assert!(child.contains("use super::sib;"), "{child}");
    }

    #[test]
    fn removes_exactly_one_mod_decl_with_attrs() {
        let src = "pub mod a;\n/// Docs.\n#[cfg(unix)]\npub mod emails;\nmod b;\n";
        let out = remove_mod_decl(src, "emails").unwrap();
        assert_eq!(out, "pub mod a;\nmod b;\n");
        assert!(remove_mod_decl("mod a;\n", "emails").is_err());
    }

    #[test]
    fn rewrites_user_paths_and_groups() {
        let src = "use base::{logging, emails, auth::x};\nuse base::emails::Email;\nfn f() { base::emails::send(); base::emailsx(); }\n    pub use base::{emails::E as F};\npub use base::emails;\n";
        let out = rewrite_user(src, "base", "emails", "base_emails");
        // The binding `emails` is kept, whichever form brought it in.
        assert!(
            out.contains("use base::{logging, auth::x};\nuse base_emails as emails;"),
            "{out}"
        );
        assert!(out.contains("use base_emails::Email;"), "{out}");
        assert!(out.contains("base_emails::send()"), "{out}");
        assert!(out.contains("base::emailsx()"), "{out}");
        assert!(out.contains("    pub use base_emails::E as F;"), "{out}");
        assert!(out.contains("\npub use base_emails as emails;\n"), "{out}");
        // A multi-line group, as rustfmt writes them.
        let multi = "pub(crate) use base::{\n    a, b, emails,\n    c,\n};\n";
        let out = rewrite_user(multi, "base", "emails", "base_emails");
        assert!(out.contains("pub(crate) use base::{a, b, c};"), "{out}");
        assert!(
            out.contains("pub(crate) use base_emails as emails;"),
            "{out}"
        );
        // The alias for the glob re-exporter is not doubled.
        let alias = add_glob_alias(
            "pub use base::*;\npub use base_emails as emails;\n",
            "base",
            "base_emails",
            "emails",
            &[],
        )
        .unwrap();
        assert_eq!(alias.matches("as emails;").count(), 1, "{alias}");
    }

    #[test]
    fn glob_alias_goes_after_the_glob() {
        let src = "//! doc\npub use base::*;\npub use base::other;\n";
        let out = add_glob_alias(src, "base", "base_emails", "emails", &[]).unwrap();
        assert_eq!(
            out,
            "//! doc\npub use base::*;\npub use base_emails as emails;\npub use base::other;\n"
        );
        assert!(add_glob_alias(src, "base", "x", "a::b", &[]).is_err());
        let out = add_glob_alias(
            src,
            "base",
            "base_emails",
            "emails",
            &[("Email".into(), "Email".into())],
        )
        .unwrap();
        assert!(
            out.contains("pub use base_emails::Email as Email;"),
            "{out}"
        );
    }

    #[test]
    fn root_reexports_and_their_users() {
        let root = "pub mod emails;\npub use emails::Email;\npub use self::emails::{send, parse::P as Q};\n";
        assert_eq!(
            root_reexports(root, "emails"),
            [
                ("Email".to_owned(), "Email".to_owned()),
                ("send".to_owned(), "send".to_owned()),
                ("Q".to_owned(), "parse::P".to_owned()),
            ]
        );
        let user = "use base::{logging, Email};\nfn f() -> base::Email { base::Q::new() }\n";
        let mut out = user.to_owned();
        for (n, p) in root_reexports(root, "emails") {
            out = rewrite_reexported(&out, "base", &n, "base_emails", &p, false);
        }
        assert!(
            out.contains("use base::{logging};\nuse base_emails::Email;"),
            "{out}"
        );
        assert!(out.contains("-> base_emails::Email"), "{out}");
        assert!(out.contains("base_emails::parse::P::new()"), "{out}");
    }

    #[test]
    fn workspace_member_and_dependency() {
        let src = "[workspace]\nmembers = [\n    \"crates/a\",\n    \"crates/base\",\n]\n\n[workspace.dependencies]\nbase = { path = \"crates/base\" }\n";
        let out = add_workspace_member(
            src,
            "crates/base-emails",
            "base",
            "base-emails",
            "crates/base",
        )
        .unwrap();
        assert!(
            out.contains("\"crates/base\",\n    \"crates/base-emails\""),
            "{out}"
        );
        assert!(
            out.contains("base-emails = { path = \"crates/base-emails\" }"),
            "{out}"
        );
        let glob = "[workspace]\nmembers = [\"crates/*\"]\n";
        assert_eq!(
            add_workspace_member(
                glob,
                "crates/base-emails",
                "base",
                "base-emails",
                "crates/base"
            )
            .unwrap(),
            glob
        );
    }

    #[test]
    fn user_dependency_matches_style() {
        let ws_style = "[dependencies]\nbase.workspace = true\nserde = \"1\"\n";
        let out = add_user_dependency(
            ws_style,
            "base",
            "base-emails",
            Path::new("/w/app"),
            Path::new("/w/crates/base-emails"),
        )
        .unwrap();
        assert!(out.contains("base-emails.workspace = true"), "{out}");
        let path_style =
            "[dependencies]\nbase = { path = \"../crates/base\", default-features = false }\n";
        let out = add_user_dependency(
            path_style,
            "base",
            "base-emails",
            Path::new("/w/app"),
            Path::new("/w/crates/base-emails"),
        )
        .unwrap();
        assert!(
            out.contains("base-emails = { path = \"../crates/base-emails\" }"),
            "{out}"
        );
        assert!(
            add_user_dependency(
                "[dependencies]\n",
                "base",
                "x",
                Path::new("/a"),
                Path::new("/b")
            )
            .is_err()
        );
    }

    #[test]
    fn counts_tests_across_binaries() {
        let out = "test result: ok. 4 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n\
                   test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        assert_eq!(count_tests(out), 7);
    }

    #[test]
    fn relative_paths() {
        assert_eq!(
            rel_path(Path::new("/w/crates/new"), Path::new("/w/crates/base")),
            "../base"
        );
        assert_eq!(
            rel_path(Path::new("/w/app"), Path::new("/x/jcode/crates/b")),
            "../../x/jcode/crates/b"
        );
    }
}
