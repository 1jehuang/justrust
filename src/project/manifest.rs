//! Cargo manifest, workspace, target and feature model (G8).
//!
//! Reads `Cargo.toml` with the native TOML reader and produces a resolved
//! package graph: workspace members, path dependencies, targets discovered
//! the way Cargo does (`src/lib.rs`, `src/main.rs`, `src/bin/*`, `tests/*`,
//! `examples/*`, explicit `[lib]`/`[[bin]]`/`[[test]]` tables), profiles and
//! enabled features. Registry and git dependencies have no native source
//! and are reported as unsupported rather than silently dropped.

use super::toml::{self, Spanned, Table, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TargetKind {
    Lib,
    Bin,
    Test,
    Example,
    Bench,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub kind: TargetKind,
    pub name: String,
    pub path: PathBuf,
    /// `test = false` excludes the target from `test` runs.
    pub test: bool,
    pub harness: bool,
}

#[derive(Clone, Debug)]
pub struct Dependency {
    /// Name used in source (`package` renames apply).
    pub name: String,
    pub package: String,
    pub path: Option<PathBuf>,
    pub dev: bool,
    pub optional: bool,
    pub features: Vec<String>,
    pub default_features: bool,
}

#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub root: PathBuf,
    pub targets: Vec<Target>,
    pub dependencies: Vec<Dependency>,
    pub features: BTreeMap<String, Vec<String>>,
}

impl Package {
    pub fn lib(&self) -> Option<&Target> {
        self.targets.iter().find(|t| t.kind == TargetKind::Lib)
    }
    /// The crate name used in source for this package's library.
    pub fn crate_name(&self) -> String {
        self.lib()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| self.name.replace('-', "_"))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub debug_assertions: Option<bool>,
    pub overflow_checks: Option<bool>,
    pub opt_level: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub root: PathBuf,
    /// Every loaded package, workspace members first.
    pub packages: Vec<Package>,
    pub members: Vec<usize>,
    pub default_members: Vec<usize>,
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug)]
pub struct ManifestError(pub String);
impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ManifestError {}

fn err<T>(message: impl Into<String>) -> Result<T, ManifestError> {
    Err(ManifestError(message.into()))
}

fn read_manifest(path: &Path) -> Result<Table, ManifestError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ManifestError(format!("failed to read `{}`: {e}", path.display())))?;
    toml::parse(&text).map_err(|e| {
        ManifestError(format!(
            "failed to parse manifest at `{}`\n\n{e}",
            path.display()
        ))
    })
}

fn get<'a>(table: &'a Table, key: &str) -> Option<&'a Value> {
    table.get(key).map(|s| &s.value)
}
fn get_str(table: &Table, key: &str, what: &str) -> Result<Option<String>, ManifestError> {
    match get(table, key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(v) => err(format!(
            "{what}.{key}: expected a string, found {}",
            v.type_name()
        )),
    }
}
fn get_bool(table: &Table, key: &str, what: &str) -> Result<Option<bool>, ManifestError> {
    match get(table, key) {
        None => Ok(None),
        Some(Value::Boolean(b)) => Ok(Some(*b)),
        Some(v) => err(format!(
            "{what}.{key}: expected a boolean, found {}",
            v.type_name()
        )),
    }
}
fn strings(value: &[Spanned], what: &str) -> Result<Vec<String>, ManifestError> {
    value
        .iter()
        .map(|s| match &s.value {
            Value::String(s) => Ok(s.clone()),
            v => err(format!(
                "{what}: expected a string, found {}",
                v.type_name()
            )),
        })
        .collect()
}

/// Find the manifest governing `start`: the nearest `Cargo.toml` upward.
pub fn find_manifest(start: &Path) -> Result<PathBuf, ManifestError> {
    let start = start
        .canonicalize()
        .map_err(|e| ManifestError(format!("{}: {e}", start.display())))?;
    let mut dir = if start.is_file() {
        start.parent().map(Path::to_path_buf).unwrap_or(start)
    } else {
        start
    };
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            return err(format!(
                "could not find `Cargo.toml` in `{}` or any parent directory",
                dir.display()
            ));
        }
    }
}

/// Load the workspace containing `manifest`.
pub fn load(manifest: &Path) -> Result<Workspace, ManifestError> {
    let manifest = manifest
        .canonicalize()
        .map_err(|e| ManifestError(format!("{}: {e}", manifest.display())))?;
    // A package manifest may belong to a workspace rooted higher up.
    let mut root_manifest = manifest.clone();
    let own = read_manifest(&manifest)?;
    if get(&own, "workspace").is_none() {
        let mut dir = manifest.parent().unwrap().to_path_buf();
        while dir.pop() {
            let candidate = dir.join("Cargo.toml");
            if candidate.is_file()
                && let Ok(table) = read_manifest(&candidate)
                && let Some(Value::Table(ws)) = get(&table, "workspace")
                && workspace_includes(ws, &dir, manifest.parent().unwrap())?
            {
                root_manifest = candidate;
                break;
            }
        }
    }
    let root_dir = root_manifest.parent().unwrap().to_path_buf();
    let root_table = read_manifest(&root_manifest)?;
    let mut loader = Loader {
        packages: Vec::new(),
        by_dir: BTreeMap::new(),
        workspace_deps: Table::new(),
        workspace_package: Table::new(),
    };
    let mut members = Vec::new();
    let mut default_members = Vec::new();
    if let Some(Value::Table(ws)) = get(&root_table, "workspace") {
        if let Some(Value::Table(deps)) = get(ws, "dependencies") {
            loader.workspace_deps = deps.clone();
        }
        if let Some(Value::Table(package)) = get(ws, "package") {
            loader.workspace_package = package.clone();
        }
        let excluded: Vec<PathBuf> = match get(ws, "exclude") {
            Some(Value::Array(a)) => strings(a, "workspace.exclude")?
                .into_iter()
                .map(|p| root_dir.join(p))
                .collect(),
            _ => Vec::new(),
        };
        let mut dirs = Vec::new();
        if get(&root_table, "package").is_some() {
            dirs.push(root_dir.clone());
        }
        if let Some(Value::Array(a)) = get(ws, "members") {
            for pattern in strings(a, "workspace.members")? {
                for dir in expand_member(&root_dir, &pattern)? {
                    if !excluded.iter().any(|e| dir.starts_with(e)) && !dirs.contains(&dir) {
                        dirs.push(dir);
                    }
                }
            }
        }
        for dir in &dirs {
            members.push(loader.package(dir)?);
        }
        if let Some(Value::Array(a)) = get(ws, "default-members") {
            for pattern in strings(a, "workspace.default-members")? {
                for dir in expand_member(&root_dir, &pattern)? {
                    default_members.push(loader.package(&dir)?);
                }
            }
        }
    } else {
        members.push(loader.package(&root_dir)?);
    }
    // Path dependencies are loaded transitively.
    let mut i = 0;
    while i < loader.packages.len() {
        let deps: Vec<_> = loader.packages[i].dependencies.clone();
        for dep in deps {
            if let Some(path) = &dep.path {
                let index = loader.package(path)?;
                if loader.packages[index].name != dep.package {
                    return err(format!(
                        "no matching package named `{}` found at `{}` (found `{}`)",
                        dep.package,
                        path.display(),
                        loader.packages[index].name
                    ));
                }
            }
        }
        i += 1;
    }
    if default_members.is_empty() {
        // A non-virtual root package is the default; otherwise all members.
        let root_index = loader.by_dir.get(&root_dir).copied();
        default_members = match root_index {
            Some(index) if manifest == root_manifest => vec![index],
            _ if manifest != root_manifest => {
                vec![loader.package(manifest.parent().unwrap())?]
            }
            _ => members.clone(),
        };
    } else if manifest != root_manifest {
        default_members = vec![loader.package(manifest.parent().unwrap())?];
    }
    let mut profiles = BTreeMap::new();
    if let Some(Value::Table(table)) = get(&root_table, "profile") {
        for (name, value) in table {
            let Value::Table(p) = &value.value else {
                return err(format!("profile.{name}: expected a table"));
            };
            let what = format!("profile.{name}");
            let opt_level = match get(p, "opt-level") {
                None => None,
                Some(Value::Integer(n)) => Some(n.to_string()),
                Some(Value::String(s)) => Some(s.clone()),
                Some(v) => return err(format!("{what}.opt-level: unexpected {}", v.type_name())),
            };
            profiles.insert(
                name.clone(),
                Profile {
                    debug_assertions: get_bool(p, "debug-assertions", &what)?,
                    overflow_checks: get_bool(p, "overflow-checks", &what)?,
                    opt_level,
                },
            );
        }
    }
    Ok(Workspace {
        root: root_dir,
        packages: loader.packages,
        members,
        default_members,
        profiles,
    })
}

fn workspace_includes(ws: &Table, root: &Path, package: &Path) -> Result<bool, ManifestError> {
    if let Some(Value::Array(a)) = get(ws, "exclude") {
        for e in strings(a, "workspace.exclude")? {
            if package.starts_with(root.join(e)) {
                return Ok(false);
            }
        }
    }
    if let Some(Value::Array(a)) = get(ws, "members") {
        for pattern in strings(a, "workspace.members")? {
            if expand_member(root, &pattern)?.iter().any(|d| d == package) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Expand a member path; a trailing `*` path component globs directories.
fn expand_member(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, ManifestError> {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        let dir = root.join(prefix);
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| ManifestError(format!("{}: {e}", dir.display())))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("Cargo.toml").is_file() {
                out.push(path.canonicalize().unwrap_or(path));
            }
        }
        out.sort();
        return Ok(out);
    }
    if pattern.contains(['*', '?', '[']) {
        return err(format!("unsupported workspace member glob `{pattern}`"));
    }
    let path = root.join(pattern);
    Ok(vec![path.canonicalize().map_err(|e| {
        ManifestError(format!(
            "failed to load workspace member `{}`: {e}",
            path.display()
        ))
    })?])
}

struct Loader {
    packages: Vec<Package>,
    by_dir: BTreeMap<PathBuf, usize>,
    workspace_deps: Table,
    workspace_package: Table,
}

impl Loader {
    fn package(&mut self, dir: &Path) -> Result<usize, ManifestError> {
        let dir = dir
            .canonicalize()
            .map_err(|e| ManifestError(format!("{}: {e}", dir.display())))?;
        if let Some(index) = self.by_dir.get(&dir) {
            return Ok(*index);
        }
        let path = dir.join("Cargo.toml");
        let table = read_manifest(&path)?;
        let Some(Value::Table(package)) = get(&table, "package") else {
            return err(format!(
                "manifest at `{}` has no [package] section (virtual manifest used as a package)",
                path.display()
            ));
        };
        let name = get_str(package, "name", "package")?.ok_or_else(|| {
            ManifestError(format!("`{}`: package.name is required", path.display()))
        })?;
        let inherited = |key: &str| -> Result<Option<String>, ManifestError> {
            match get(package, key) {
                Some(Value::Table(t)) if get_bool(t, "workspace", key)? == Some(true) => {
                    get_str(&self.workspace_package, key, "workspace.package")
                }
                _ => get_str(package, key, "package"),
            }
        };
        let version = inherited("version")?.unwrap_or_else(|| "0.0.0".into());
        let edition = inherited("edition")?.unwrap_or_else(|| "2015".into());
        let mut dependencies = Vec::new();
        for (section, dev) in [
            ("dependencies", false),
            ("dev-dependencies", true),
            ("build-dependencies", false),
        ] {
            if let Some(Value::Table(deps)) = get(&table, section) {
                if section == "build-dependencies" && !deps.is_empty() {
                    return err(format!(
                        "`{}`: build-dependencies (build scripts) are unsupported",
                        path.display()
                    ));
                }
                for (key, spec) in deps {
                    dependencies.push(self.dependency(&dir, key, &spec.value, dev, section)?);
                }
            }
        }
        if let Some(Value::Table(targets)) = get(&table, "target") {
            if !targets.is_empty() {
                return err(format!(
                    "`{}`: target-specific dependency tables are unsupported",
                    path.display()
                ));
            }
        }
        let build_script = match get(package, "build") {
            Some(Value::Boolean(false)) => false,
            Some(Value::Boolean(true)) => true,
            Some(Value::String(_)) => true,
            Some(v) => return err(format!("package.build: unexpected {}", v.type_name())),
            None => dir.join("build.rs").is_file(),
        };
        if build_script {
            return err(format!(
                "`{}`: build scripts are unsupported",
                path.display()
            ));
        }
        let mut features = BTreeMap::new();
        if let Some(Value::Table(table)) = get(&table, "features") {
            for (feature, list) in table {
                let Value::Array(list) = &list.value else {
                    return err(format!("features.{feature}: expected an array"));
                };
                features.insert(feature.clone(), strings(list, "features")?);
            }
        }
        let targets = discover_targets(&dir, &name, &table)?;
        let index = self.packages.len();
        self.by_dir.insert(dir.clone(), index);
        self.packages.push(Package {
            name,
            version,
            edition,
            root: dir,
            targets,
            dependencies,
            features,
        });
        Ok(index)
    }

    fn dependency(
        &self,
        dir: &Path,
        key: &str,
        spec: &Value,
        dev: bool,
        section: &str,
    ) -> Result<Dependency, ManifestError> {
        let what = format!("{section}.{key}");
        let mut table = match spec {
            Value::String(_) => Table::new(),
            Value::Table(t) => t.clone(),
            v => {
                return err(format!(
                    "{what}: expected a string or table, found {}",
                    v.type_name()
                ));
            }
        };
        let mut base = dir.to_path_buf();
        if get_bool(&table, "workspace", &what)? == Some(true) {
            let Some(inherited) = self.workspace_deps.get(key) else {
                return err(format!("{what}: not found in workspace.dependencies"));
            };
            let mut merged = match &inherited.value {
                Value::Table(t) => t.clone(),
                Value::String(_) => Table::new(),
                _ => return err(format!("workspace.dependencies.{key}: expected a table")),
            };
            for (k, v) in table {
                if k != "workspace" {
                    merged.insert(k, v);
                }
            }
            table = merged;
            // Workspace dependency paths are relative to the workspace root,
            // which `load` resolves; the root is an ancestor of `dir`.
            let mut root = dir.to_path_buf();
            while !root.join("Cargo.toml").is_file()
                || read_manifest(&root.join("Cargo.toml"))
                    .ok()
                    .and_then(|t| t.get("workspace").cloned())
                    .is_none()
            {
                if !root.pop() {
                    break;
                }
            }
            base = root;
        }
        // Registry and git sources have no native fetcher. They are kept
        // unresolved and rejected only when a build actually activates them.
        let path = if get(&table, "git").is_some() {
            None
        } else {
            get_str(&table, "path", &what)?
        };
        let features = match get(&table, "features") {
            Some(Value::Array(a)) => strings(a, &what)?,
            None => Vec::new(),
            Some(_) => return err(format!("{what}.features: expected an array")),
        };
        Ok(Dependency {
            name: key.replace('-', "_"),
            package: get_str(&table, "package", &what)?.unwrap_or_else(|| key.to_string()),
            path: path.map(|path| base.join(path)),
            dev,
            optional: get_bool(&table, "optional", &what)?.unwrap_or(false),
            features,
            default_features: get_bool(&table, "default-features", &what)?
                .or(get_bool(&table, "default_features", &what)?)
                .unwrap_or(true),
        })
    }
}

fn discover_targets(
    dir: &Path,
    package: &str,
    table: &Table,
) -> Result<Vec<Target>, ManifestError> {
    let crate_name = package.replace('-', "_");
    let mut targets = Vec::new();
    let auto = |key: &str| -> bool {
        match table.get("package").map(|p| &p.value) {
            Some(Value::Table(p)) => !matches!(get(p, key), Some(Value::Boolean(false))),
            _ => true,
        }
    };
    // [lib]
    let lib_table = match get(table, "lib") {
        Some(Value::Table(t)) => Some(t.clone()),
        None => None,
        Some(_) => return err("lib: expected a table"),
    };
    let lib_default = dir.join("src/lib.rs");
    if lib_table.is_some() || (auto("autolib") && lib_default.is_file()) {
        let t = lib_table.unwrap_or_default();
        if let Some(Value::Boolean(true)) = get(&t, "proc-macro") {
            return err("proc-macro crates are unsupported");
        }
        targets.push(Target {
            kind: TargetKind::Lib,
            name: get_str(&t, "name", "lib")?
                .unwrap_or_else(|| crate_name.clone())
                .replace('-', "_"),
            path: get_str(&t, "path", "lib")?
                .map(|p| dir.join(p))
                .unwrap_or(lib_default),
            test: get_bool(&t, "test", "lib")?.unwrap_or(true),
            harness: get_bool(&t, "harness", "lib")?.unwrap_or(true),
        });
    }
    for (kind, key, auto_key, default_dir) in [
        (TargetKind::Bin, "bin", "autobins", "src/bin"),
        (TargetKind::Test, "test", "autotests", "tests"),
        (TargetKind::Example, "example", "autoexamples", "examples"),
        (TargetKind::Bench, "bench", "autobenches", "benches"),
    ] {
        let mut explicit = Vec::new();
        if let Some(value) = get(table, key) {
            let Value::Array(list) = value else {
                return err(format!("{key}: expected an array of tables"));
            };
            for entry in list {
                let Value::Table(t) = &entry.value else {
                    return err(format!("[[{key}]]: expected a table"));
                };
                let name = get_str(t, "name", key)?
                    .ok_or_else(|| ManifestError(format!("[[{key}]] requires a name")))?;
                let path = match get_str(t, "path", key)? {
                    Some(p) => dir.join(p),
                    None if kind == TargetKind::Bin && name == package => dir.join("src/main.rs"),
                    None => {
                        let flat = dir.join(default_dir).join(format!("{name}.rs"));
                        if flat.is_file() {
                            flat
                        } else {
                            dir.join(default_dir).join(&name).join("main.rs")
                        }
                    }
                };
                explicit.push(Target {
                    kind,
                    name,
                    path,
                    test: get_bool(t, "test", key)?.unwrap_or(kind != TargetKind::Example),
                    harness: get_bool(t, "harness", key)?.unwrap_or(true),
                });
            }
        }
        let mut found = Vec::new();
        if auto(auto_key) {
            if kind == TargetKind::Bin && dir.join("src/main.rs").is_file() {
                found.push((package.to_string(), dir.join("src/main.rs")));
            }
            if let Ok(entries) = std::fs::read_dir(dir.join(default_dir)) {
                let mut list: Vec<_> = entries.flatten().map(|e| e.path()).collect();
                list.sort();
                for path in list {
                    if path.extension().is_some_and(|e| e == "rs") && path.is_file() {
                        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                        found.push((name, path));
                    } else if path.join("main.rs").is_file() {
                        let name = path.file_name().unwrap().to_string_lossy().into_owned();
                        found.push((name, path.join("main.rs")));
                    }
                }
            }
        }
        let mut names: BTreeSet<String> = explicit.iter().map(|t| t.name.clone()).collect();
        let paths: BTreeSet<PathBuf> = explicit.iter().map(|t| t.path.clone()).collect();
        targets.extend(explicit);
        for (name, path) in found {
            if !paths.contains(&path) && names.insert(name.clone()) {
                targets.push(Target {
                    kind,
                    name,
                    path,
                    test: kind != TargetKind::Example,
                    harness: true,
                });
            }
        }
    }
    if targets.is_empty() {
        return err(format!(
            "no targets specified in the manifest\neither src/lib.rs, src/main.rs, a [lib] section, or [[bin]] section must be present (package `{package}`)"
        ));
    }
    Ok(targets)
}

/// Enabled features of every package after unifying requests from the
/// selected roots and dependency edges (Cargo's resolver 2 without
/// target/host splitting).
pub fn resolve_features(
    ws: &Workspace,
    roots: &[(usize, Vec<String>, bool)],
    include_dev: bool,
) -> Result<Features, ManifestError> {
    let mut enabled: Vec<BTreeSet<String>> = vec![BTreeSet::new(); ws.packages.len()];
    let mut active_deps: Vec<BTreeSet<String>> = vec![BTreeSet::new(); ws.packages.len()];
    let mut work: Vec<(usize, String)> = Vec::new();
    let mut visited = vec![false; ws.packages.len()];
    let mut queue: Vec<(usize, bool)> = Vec::new();
    for (index, features, defaults) in roots {
        if *defaults {
            work.push((*index, "default".into()));
        }
        for f in features {
            work.push((*index, f.clone()));
        }
        queue.push((*index, include_dev));
    }
    let dep_index = |ws: &Workspace, dep: &Dependency| -> Option<usize> {
        let path = dep.path.as_ref()?.canonicalize().ok()?;
        ws.packages.iter().position(|p| p.root == path)
    };
    loop {
        while let Some((index, feature)) = work.pop() {
            let package = &ws.packages[index];
            if let Some((dep, sub)) = feature.split_once('/') {
                let (dep, weak) = match dep.strip_suffix('?') {
                    Some(d) => (d, true),
                    None => (dep, false),
                };
                let Some(d) = package
                    .dependencies
                    .iter()
                    .find(|d| d.name == dep.replace('-', "_"))
                else {
                    return err(format!(
                        "package `{}`: unknown dependency `{dep}` in feature",
                        package.name
                    ));
                };
                if !weak && d.optional {
                    active_deps[index].insert(d.name.clone());
                }
                if (!d.optional || active_deps[index].contains(&d.name))
                    && let Some(target) = dep_index(ws, d)
                {
                    work.push((target, sub.to_string()));
                    queue.push((target, false));
                }
                continue;
            }
            if let Some(dep) = feature.strip_prefix("dep:") {
                active_deps[index].insert(dep.replace('-', "_"));
                if let Some(d) = package
                    .dependencies
                    .iter()
                    .find(|d| d.name == dep.replace('-', "_"))
                    && let Some(target) = dep_index(ws, d)
                {
                    queue.push((target, false));
                }
                continue;
            }
            if !enabled[index].insert(feature.clone()) {
                continue;
            }
            match package.features.get(&feature) {
                Some(list) => {
                    for f in list {
                        work.push((index, f.clone()));
                    }
                }
                None if feature == "default" => {}
                None => {
                    // An optional dependency used as an implicit feature.
                    if let Some(d) = package
                        .dependencies
                        .iter()
                        .find(|d| d.optional && d.name == feature.replace('-', "_"))
                    {
                        active_deps[index].insert(d.name.clone());
                        if let Some(target) = dep_index(ws, d) {
                            queue.push((target, false));
                        }
                    } else {
                        return err(format!(
                            "package `{}` does not have the feature `{feature}`",
                            package.name
                        ));
                    }
                }
            }
        }
        let Some((index, dev)) = queue.pop() else {
            break;
        };
        if visited[index] && !dev {
            continue;
        }
        visited[index] = true;
        for d in &ws.packages[index].dependencies {
            if (d.dev && !dev) || (d.optional && !active_deps[index].contains(&d.name)) {
                continue;
            }
            if let Some(target) = dep_index(ws, d) {
                if d.default_features {
                    work.push((target, "default".into()));
                }
                for f in &d.features {
                    work.push((target, f.clone()));
                }
                queue.push((target, false));
            }
        }
    }
    // Implicit features of optional deps that got activated.
    for (index, deps) in active_deps.iter().enumerate() {
        for d in deps {
            if !ws.packages[index].features.contains_key(d) {
                enabled[index].insert(d.clone());
            }
        }
    }
    // `default` is only a feature when the package declares it.
    for (index, package) in ws.packages.iter().enumerate() {
        if !package.features.contains_key("default") {
            enabled[index].remove("default");
        }
    }
    Ok(Features {
        enabled,
        active_deps,
    })
}

/// Result of feature resolution: enabled features and activated optional
/// dependencies per package.
#[derive(Clone, Debug)]
pub struct Features {
    pub enabled: Vec<BTreeSet<String>>,
    pub active_deps: Vec<BTreeSet<String>>,
}

/// Dependencies of `package` visible to a target of the given kind, as
/// `(extern name, package index)`. Optional deps need their feature.
pub fn visible_dependencies(
    ws: &Workspace,
    package: usize,
    dev: bool,
    active_deps: &BTreeSet<String>,
    unresolved: &mut Vec<String>,
) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for d in &ws.packages[package].dependencies {
        if d.dev && !dev {
            continue;
        }
        if d.optional && !active_deps.contains(&d.name) {
            continue;
        }
        let Some(path) = d.path.as_ref().and_then(|p| p.canonicalize().ok()) else {
            unresolved.push(format!(
                "package `{}`: dependency `{}` is a registry or git dependency, which the native driver cannot fetch; use a path dependency",
                ws.packages[package].name, d.package
            ));
            continue;
        };
        if let Some(index) = ws.packages.iter().position(|p| p.root == path) {
            out.push((d.name.clone(), index));
        }
    }
    out
}
