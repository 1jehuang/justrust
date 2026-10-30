//! Who receives what a script printed, as cargo 1.99 hands it out: `build_map` in
//! `src/compiler/custom_build.rs` (which scripts each unit is linked with), `add_native_deps`,
//! `add_plugin_deps` and `add_custom_flags` in `src/compiler/mod.rs` (what rustc is given), and
//! `fingerprint/mod.rs` (when a script runs again), in rust-lang/cargo at 5f94df478.

use std::{
    collections::BTreeSet,
    hash::Hash,
    path::{Path, PathBuf},
    time::SystemTime,
};

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    output::{LibraryPath, LinkArgTarget, Output},
    unit::{Dependency, Package, Target},
};

impl LinkArgTarget {
    /// Whether the argument is for `target`, built as a test when `test`.
    pub fn applies_to(&self, target: &Target, test: bool) -> bool {
        match self {
            LinkArgTarget::All => true,
            LinkArgTarget::Cdylib => !test && target.is_cdylib(),
            LinkArgTarget::Bin => target.is_bin(),
            LinkArgTarget::SingleBin(name) => target.is_bin() && target.name == *name,
            LinkArgTarget::Test => target.is_test(),
            LinkArgTarget::Bench => target.is_bench(),
            LinkArgTarget::Example => target.is_exe_example(),
        }
    }
}

/// A script's output among those a unit is linked with: whether it is the unit's own
/// package's.
#[derive(Clone, Copy, Debug)]
pub struct Linked<'a> {
    pub own: bool,
    pub output: &'a Output,
}

/// What rustc is given for one target of a package, from the scripts it is linked with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compile {
    /// `-L`, those inside a script's `OUT_DIR` first.
    pub search: Vec<PathBuf>,
    /// `-l`.
    pub libs: Vec<String>,
    /// `-C link-arg=`.
    pub link_args: Vec<String>,
    /// `--cfg`.
    pub cfgs: Vec<String>,
    /// `--check-cfg`.
    pub check_cfgs: Vec<String>,
    /// Variables rustc runs with, and so `env!` reads.
    pub env: Vec<(String, String)>,
}

impl Compile {
    /// What `target` of `package` is given. `linked` is what [`to_link`] lists for the unit,
    /// in its order; `own` are the outputs of the package's own script runs for the unit.
    pub fn of(
        package: &Package,
        target: &Target,
        test: bool,
        linked: &[Linked<'_>],
        own: &[&Output],
    ) -> Compile {
        // A program of a package with a library links the library, which has the `-l`s.
        let pass_l = target.is_lib() || !package.targets.iter().any(Target::is_lib);
        let mut compile = Compile::default();
        let mut paths: Vec<&LibraryPath> =
            linked.iter().flat_map(|it| &it.output.library_paths).collect();
        paths.sort_by_key(|it| matches!(it, LibraryPath::External(_)));
        compile.search = paths.into_iter().map(|it| it.path().to_owned()).collect();
        for it in linked {
            if it.own && pass_l {
                compile.libs.extend(it.output.library_links.iter().cloned());
            }
            for (to, arg) in &it.output.linker_args {
                // cdylib arguments reach a cdylib from its dependencies too: cargo's issue 9562.
                if to.applies_to(target, test) && (it.own || *to == LinkArgTarget::Cdylib) {
                    compile.link_args.push(arg.clone());
                }
            }
        }
        for output in own {
            compile.cfgs.extend(output.cfgs.iter().cloned());
            compile.check_cfgs.extend(output.check_cfgs.iter().cloned());
            compile.env.extend(output.env.iter().cloned());
        }
        compile
    }

    /// rustc's arguments, in cargo's order.
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for path in &self.search {
            args.extend(["-L".to_owned(), path.display().to_string()]);
        }
        for lib in &self.libs {
            args.extend(["-l".to_owned(), lib.clone()]);
        }
        for arg in &self.link_args {
            args.extend(["-C".to_owned(), format!("link-arg={arg}")]);
        }
        for cfg in &self.cfgs {
            args.extend(["--cfg".to_owned(), cfg.clone()]);
        }
        for cfg in &self.check_cfgs {
            args.extend(["--check-cfg".to_owned(), cfg.clone()]);
        }
        args
    }
}

/// The scripts a unit is linked with, and those of its host-only dependencies (build
/// dependencies, proc macros), whose library directories are put on the path dynamic
/// libraries are found by. Each is a run of a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildScripts<R> {
    /// Topologically sorted, each once.
    pub to_link: Vec<R>,
    pub plugins: BTreeSet<R>,
}

/// One unit of the graph, as [`to_link`] reads it.
pub struct GraphUnit<U, R> {
    /// The units it depends on, sorted by their packages' ids as cargo sorts them.
    pub deps: Vec<U>,
    /// The runs of its package's script it is built with, unless it is the script itself or
    /// its package has none.
    pub own_runs: Vec<R>,
    /// A build script, proc macro or build dependency: built for the host.
    pub for_host: bool,
    /// A library that is linked into what depends on it.
    pub linkable: bool,
}

/// What [`BuildScripts`] `unit` has in `graph`, as cargo's `build_map` computes it.
pub fn to_link<U, R>(
    unit: U,
    graph: &dyn Fn(&U) -> GraphUnit<U, R>,
    memo: &mut FxHashMap<U, BuildScripts<R>>,
) -> BuildScripts<R>
where
    U: Clone + Eq + Hash,
    R: Clone + Eq + Hash + Ord,
{
    if let Some(it) = memo.get(&unit) {
        return it.clone();
    }
    let node = graph(&unit);
    let mut seen = FxHashSet::default();
    let mut scripts = BuildScripts { to_link: Vec::new(), plugins: BTreeSet::new() };
    let mut add = |scripts: &mut BuildScripts<R>, run: &R| {
        if seen.insert(run.clone()) {
            scripts.to_link.push(run.clone());
        }
    };
    for run in &node.own_runs {
        add(&mut scripts, run);
    }
    for dep in node.deps {
        let of_dep = to_link(dep.clone(), graph, memo);
        let dep_node = graph(&dep);
        if dep_node.for_host {
            scripts.plugins.extend(of_dep.to_link.iter().cloned());
        } else if dep_node.linkable {
            for run in &of_dep.to_link {
                add(&mut scripts, run);
            }
        }
    }
    memo.insert(unit, scripts.clone());
    scripts
}

impl Dependency {
    /// A dependency whose script printed `output`, as the scripts of the packages that depend
    /// on it directly see it.
    pub fn of(name: &str, links: Option<&str>, output: &Output) -> Dependency {
        Dependency {
            name: name.to_owned(),
            links: links.map(str::to_owned),
            metadata: output.metadata.clone(),
        }
    }
}

/// When a script runs again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rerun {
    /// It named nothing it reads: for any change to the files of its package (the package's
    /// sources' newest time for a path dependency, its version for a registry's).
    Package,
    /// For a change to these: each file (relative to the package's directory) or directory
    /// (anything below it), and each variable.
    Watched { files: Vec<PathBuf>, env: Vec<String> },
}

impl Output {
    pub fn rerun(&self, package_dir: &Path) -> Rerun {
        if self.rerun_if_changed.is_empty() && self.rerun_if_env_changed.is_empty() {
            return Rerun::Package;
        }
        let files = self
            .rerun_if_changed
            .iter()
            .map(|it| it.strip_prefix(package_dir).unwrap_or(it).to_owned())
            .collect();
        Rerun::Watched { files, env: self.rerun_if_env_changed.clone() }
    }
}

/// Why a script whose run is kept runs again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stale {
    Missing(PathBuf),
    Newer(PathBuf),
    Env { name: String, then: Option<String>, now: Option<String> },
}

impl Rerun {
    /// Whether a run that began at `ran` must run again. A variable's value is `[env]`'s when
    /// that has it, else cargo's environment's: `then` is what it was at the run, `now` what
    /// it is. A file newer than `ran` is a change; one as old is not. `None` for
    /// [`Rerun::Package`], whose files the package's source says.
    pub fn stale(
        &self,
        package_dir: &Path,
        ran: SystemTime,
        then: &dyn Fn(&str) -> Option<String>,
        now: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Option<Stale>> {
        let Rerun::Watched { files, env } = self else { return None };
        for file in files {
            let path = package_dir.join(file);
            match newest(&path) {
                None => return Some(Some(Stale::Missing(path))),
                Some(it) if it > ran => return Some(Some(Stale::Newer(path))),
                Some(_) => {}
            }
        }
        for name in env {
            let (then, now) = (then(name), now(name));
            if then != now {
                return Some(Some(Stale::Env { name: name.clone(), then, now }));
            }
        }
        Some(None)
    }
}

/// The newest time of the file at `path`, or of the directory there and everything below
/// it, as cargo's `mtime_recursive` takes it: links followed, a link's own time and its
/// target's, what cannot be read left out.
fn newest(path: &Path) -> Option<SystemTime> {
    let meta = std::fs::metadata(path).ok()?;
    let mut newest = meta.modified().ok()?;
    if !meta.is_dir() {
        return Some(newest);
    }
    let mut visited: FxHashSet<PathBuf> = std::fs::canonicalize(path).into_iter().collect();
    let mut pending = vec![path.to_owned()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(link) = std::fs::symlink_metadata(&path) else { continue };
            if let Ok(it) = link.modified() {
                newest = newest.max(it);
            }
            let target = match link.file_type().is_symlink() {
                true => std::fs::metadata(&path).ok(),
                false => Some(link),
            };
            let Some(target) = target else { continue };
            if let Ok(it) = target.modified() {
                newest = newest.max(it);
            }
            if target.is_dir() && std::fs::canonicalize(&path).is_ok_and(|it| visited.insert(it)) {
                pending.push(path);
            }
        }
    }
    Some(newest)
}

#[cfg(test)]
mod tests;
