//! Cargo manifest, workspace, target and feature model (G8).
//!
//! Reads `Cargo.toml` with the native TOML reader and produces a resolved
//! package graph: workspace members, path dependencies, targets discovered
//! the way Cargo does (`src/lib.rs`, `src/main.rs`, `src/bin/*`, `tests/*`,
//! `examples/*`, explicit `[lib]`/`[[bin]]`/`[[test]]` tables), profiles and
//! enabled features. Registry and git dependencies have no native source
//! and are reported as unsupported rather than silently dropped.

use super::inputs::{DiscoveryInputs, Recorder};
use super::toml::{self, Spanned, Table, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    /// Whether default test selection requests library doctests.
    pub doctest: bool,
    pub harness: bool,
}

#[derive(Clone, Debug)]
pub struct Dependency {
    /// Name used in source (`package` renames apply).
    pub name: String,
    pub package: String,
    pub source: DependencySource,
    pub dev: bool,
    /// Whether this edge applies to the pinned native target. Inactive edges
    /// remain declared for feature-name validation but never demand a crate.
    pub target_active: bool,
    pub optional: bool,
    pub features: Vec<String>,
    pub default_features: bool,
}

#[derive(Clone, Debug)]
pub enum DependencySource {
    Path(PathBuf),
    Registry(LockedRegistryPackage),
    UnresolvedRegistry,
    Git,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockedRegistryPackage {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: String,
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
    pub locked: Option<usize>,
    /// Authenticated registry source (crates.io cache): compiled with lints
    /// capped at allow, like Cargo does for non-path dependencies.
    pub registry: bool,
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
    pub inherits: Option<String>,
    pub debug_assertions: Option<bool>,
    pub overflow_checks: Option<bool>,
    pub opt_level: Option<String>,
}

/// Resolve Cargo's built-in profile bases and explicit custom inheritance.
/// These are manifest facts only, not mutable compiler configuration.
pub fn resolve_profile(
    profiles: &BTreeMap<String, Profile>,
    name: &str,
) -> Result<Profile, ManifestError> {
    let mut seen = BTreeSet::new();
    let mut chain = Vec::new();
    let mut current = name;
    let release;
    loop {
        if !seen.insert(current) {
            return err(format!(
                "profile.{name}: cyclic profile inheritance at `{current}`"
            ));
        }
        if let Some(profile) = profiles.get(current) {
            chain.push(profile);
        }
        match current {
            "dev" | "release" => {
                release = current == "release";
                break;
            }
            "test" => current = "dev",
            "bench" => current = "release",
            _ => {
                current = profiles
                    .get(current)
                    .and_then(|p| p.inherits.as_deref())
                    .ok_or_else(|| ManifestError(format!("profile `{current}` is not defined")))?;
                if matches!(current, "test" | "bench") {
                    return err(format!(
                        "profile.{name}: inheriting from `{current}` is unsupported"
                    ));
                }
            }
        }
    }
    let mut resolved = Profile {
        debug_assertions: Some(!release),
        overflow_checks: Some(!release),
        ..Profile::default()
    };
    for profile in chain.into_iter().rev() {
        resolved.debug_assertions = profile.debug_assertions.or(resolved.debug_assertions);
        resolved.overflow_checks = profile.overflow_checks.or(resolved.overflow_checks);
        resolved.opt_level = profile.opt_level.clone().or(resolved.opt_level);
    }
    Ok(resolved)
}

#[derive(Clone, Debug)]
pub struct Workspace {
    /// Immutable external observations captured while this workspace was loaded.
    pub inputs: Arc<DiscoveryInputs>,
    pub root: PathBuf,
    /// Every loaded package, workspace members first.
    pub packages: Vec<Package>,
    pub members: Vec<usize>,
    pub default_members: Vec<usize>,
    pub profiles: BTreeMap<String, Profile>,
    pub lockfile: Lockfile,
}

#[derive(Clone, Debug, Default)]
pub struct Lockfile {
    pub packages: Vec<LockedPackage>,
}

#[derive(Clone, Debug)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub checksum: Option<String>,
    pub dependencies: Vec<LockedDependencyRef>,
}

#[derive(Clone, Debug)]
pub struct LockedDependencyRef {
    pub name: String,
    pub version: Option<String>,
    pub source: Option<String>,
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

fn read_manifest(path: &Path, inputs: &mut Recorder) -> Result<Table, ManifestError> {
    let text = inputs
        .read_to_string(path)
        .map_err(|e| ManifestError(format!("failed to read `{}`: {e}", path.display())))?;
    toml::parse(&text).map_err(|e| {
        ManifestError(format!(
            "failed to parse manifest at `{}`\n\n{e}",
            path.display()
        ))
    })
}

fn read_toml(path: &Path, kind: &str, inputs: &mut Recorder) -> Result<Table, ManifestError> {
    let text = inputs
        .read_to_string(path)
        .map_err(|e| ManifestError(format!("failed to read `{}`: {e}", path.display())))?;
    toml::parse(&text).map_err(|e| {
        ManifestError(format!(
            "failed to parse {kind} at `{}`\n\n{e}",
            path.display()
        ))
    })
}

fn read_lockfile(path: &Path, inputs: &mut Recorder) -> Result<Lockfile, ManifestError> {
    if !inputs.is_file(path) {
        return Ok(Lockfile::default());
    }
    let table = read_toml(path, "lockfile", inputs)?;
    if !matches!(get(&table, "version"), Some(Value::Integer(3 | 4))) {
        return err("unsupported Cargo.lock format version (expected 3 or 4)");
    }
    let Some(Value::Array(packages)) = get(&table, "package") else {
        return Ok(Lockfile::default());
    };
    let mut out = Vec::new();
    for entry in packages {
        let Value::Table(pkg) = &entry.value else {
            return err(format!("{}: [[package]] must be a table", path.display()));
        };
        let name = get_str(pkg, "name", "package")?.ok_or_else(|| {
            ManifestError(format!("{}: lock package.name is required", path.display()))
        })?;
        let version = get_str(pkg, "version", "package")?.ok_or_else(|| {
            ManifestError(format!(
                "{}: lock package.version is required",
                path.display()
            ))
        })?;
        let source = get_str(pkg, "source", "package")?;
        let checksum = get_str(pkg, "checksum", "package")?;
        let dependencies = match get(pkg, "dependencies") {
            Some(Value::Array(list)) => list
                .iter()
                .map(|dep| match &dep.value {
                    Value::String(s) => parse_lock_dependency_ref(s),
                    v => err(format!(
                        "{}: lock package.dependencies entry expected a string, found {}",
                        path.display(),
                        v.type_name()
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
            Some(v) => {
                return err(format!(
                    "{}: lock package.dependencies expected an array, found {}",
                    path.display(),
                    v.type_name()
                ));
            }
        };
        out.push(LockedPackage {
            name,
            version,
            source,
            checksum,
            dependencies,
        });
    }
    Ok(Lockfile { packages: out })
}

fn parse_lock_dependency_ref(spec: &str) -> Result<LockedDependencyRef, ManifestError> {
    let (left, source) = if let Some(open) = spec.rfind(" (") {
        if spec.ends_with(')') {
            (
                &spec[..open],
                Some(spec[open + 2..spec.len() - 1].to_string()),
            )
        } else {
            (spec, None)
        }
    } else {
        (spec, None)
    };
    let mut parts = left.split_whitespace();
    let Some(name) = parts.next() else {
        return err("lock dependency reference must not be empty");
    };
    let version = parts.next().map(str::to_string);
    if parts.next().is_some() || name.contains(['(', ')']) {
        return err("malformed Cargo.lock dependency reference");
    }
    Ok(LockedDependencyRef {
        name: name.to_string(),
        version,
        source,
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
    find_manifest_observed(start, &mut Recorder::new())
}

fn find_manifest_observed(start: &Path, inputs: &mut Recorder) -> Result<PathBuf, ManifestError> {
    let start = inputs
        .canonicalize(start)
        .map_err(|e| ManifestError(format!("{}: {e}", start.display())))?;
    let mut dir = if inputs.is_file(&start) {
        start.parent().map(Path::to_path_buf).unwrap_or(start)
    } else {
        start
    };
    loop {
        let candidate = dir.join("Cargo.toml");
        if inputs.is_file(&candidate) {
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
    load_request(manifest, false)
}

/// Discover the nearest manifest and retain the negative ancestor probes too.
pub fn load_from_directory(start: &Path) -> Result<Workspace, ManifestError> {
    load_request(start, true)
}

fn load_request(path: &Path, discover: bool) -> Result<Workspace, ManifestError> {
    let mut inputs = Recorder::new();
    for name in ["CARGO_TARGET_DIR", "CARGO_HOME", "HOME"] {
        inputs.env(name);
    }
    let manifest = if discover {
        find_manifest_observed(path, &mut inputs)?
    } else {
        path.to_owned()
    };
    let mut workspace = load_observed(&manifest, &mut inputs)?;
    workspace.inputs = inputs.finish();
    Ok(workspace)
}

fn load_observed(manifest: &Path, inputs: &mut Recorder) -> Result<Workspace, ManifestError> {
    let manifest = inputs
        .canonicalize(manifest)
        .map_err(|e| ManifestError(format!("{}: {e}", manifest.display())))?;
    // A package manifest may belong to a workspace rooted higher up.
    let mut root_manifest = manifest.clone();
    let own = read_manifest(&manifest, inputs)?;
    if get(&own, "workspace").is_none() {
        let mut dir = manifest.parent().unwrap().to_path_buf();
        while dir.pop() {
            let candidate = dir.join("Cargo.toml");
            if inputs.is_file(&candidate)
                && let Ok(table) = read_manifest(&candidate, inputs)
                && let Some(Value::Table(ws)) = get(&table, "workspace")
                && workspace_includes(ws, &dir, manifest.parent().unwrap(), inputs)?
            {
                root_manifest = candidate;
                break;
            }
        }
    }
    let root_dir = root_manifest.parent().unwrap().to_path_buf();
    let root_table = read_manifest(&root_manifest, inputs)?;
    if get(&root_table, "patch").is_some() || get(&root_table, "replace").is_some() {
        return err("manifest patch/replace source overrides are unsupported");
    }
    let lockfile = read_lockfile(&root_dir.join("Cargo.lock"), inputs)?;
    let mut loader = Loader {
        inputs,
        packages: Vec::new(),
        by_dir: BTreeMap::new(),
        workspace_deps: Table::new(),
        workspace_package: Table::new(),
        lockfile: lockfile.clone(),
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
                for dir in expand_member(&root_dir, &pattern, loader.inputs)? {
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
                for dir in expand_member(&root_dir, &pattern, loader.inputs)? {
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
            // Path declarations must still name a real package, as Cargo
            // validates manifests even for a non-selected target. Registry
            // source acquisition is unnecessary for an inactive edge.
            if !dep.target_active && !matches!(dep.source, DependencySource::Path(_)) {
                continue;
            }
            match &dep.source {
                DependencySource::Path(path) => {
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
                DependencySource::Registry(registry) => {
                    loader.inputs.uncontrolled();
                    // An optional source is not required until its feature is
                    // activated. Keep the locked identity in the manifest;
                    // visible_dependencies still authenticates an active edge
                    // and rejects missing or corrupted source before compiling.
                    if dep.optional && registry_source_root(registry).is_err() {
                        continue;
                    }
                    let index = loader.registry_package(registry)?;
                    if loader.packages[index].name != dep.package {
                        return err(format!(
                            "locked registry package `{}` resolved to `{}`",
                            dep.package, loader.packages[index].name
                        ));
                    }
                }
                DependencySource::UnresolvedRegistry | DependencySource::Git => {}
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
    if get(&root_table, "profile").is_some_and(|value| !matches!(value, Value::Table(_))) {
        return err("profile: expected a table");
    }
    if let Some(Value::Table(table)) = get(&root_table, "profile") {
        for (name, value) in table {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                || matches!(name.as_str(), "debug" | "build-override" | "package")
            {
                return err(format!("invalid or reserved profile name `{name}`"));
            }
            let Value::Table(p) = &value.value else {
                return err(format!("profile.{name}: expected a table"));
            };
            let what = format!("profile.{name}");
            let builtin = matches!(name.as_str(), "dev" | "release" | "test" | "bench");
            let inherits = get_str(p, "inherits", &what)?;
            if builtin && inherits.is_some() {
                return err(format!(
                    "{what}.inherits: built-in profiles cannot specify inheritance"
                ));
            }
            if !builtin && inherits.is_none() {
                return err(format!(
                    "{what}: custom profiles require an inherits setting"
                ));
            }
            for key in p.keys() {
                match key.as_str() {
                    "inherits" | "debug-assertions" | "overflow-checks" | "opt-level" => {}
                    // Semantics-neutral settings: they affect only code quality,
                    // debug information or artifact layout, never observable
                    // program behavior, so the native compiler validates and
                    // ignores them.
                    "debug" => match get(p, key) {
                        Some(Value::Boolean(_)) | Some(Value::Integer(0..=2)) => {}
                        Some(Value::String(s))
                            if matches!(
                                s.as_str(),
                                "none"
                                    | "line-directives-only"
                                    | "line-tables-only"
                                    | "limited"
                                    | "full"
                            ) => {}
                        _ => return err(format!("{what}.debug: invalid value")),
                    },
                    "lto" => match get(p, key) {
                        Some(Value::Boolean(_)) => {}
                        Some(Value::String(s)) if matches!(s.as_str(), "off" | "thin" | "fat") => {}
                        _ => return err(format!("{what}.lto: invalid value")),
                    },
                    "codegen-units" => match get(p, key) {
                        Some(Value::Integer(n)) if *n >= 1 => {}
                        _ => return err(format!("{what}.codegen-units: invalid value")),
                    },
                    "incremental" | "rpath" => {
                        get_bool(p, key, &what)?;
                    }
                    "strip" => match get(p, key) {
                        Some(Value::Boolean(_)) => {}
                        Some(Value::String(s))
                            if matches!(s.as_str(), "none" | "debuginfo" | "symbols") => {}
                        _ => return err(format!("{what}.strip: invalid value")),
                    },
                    "split-debuginfo" => match get(p, key) {
                        Some(Value::String(s))
                            if matches!(s.as_str(), "off" | "packed" | "unpacked") => {}
                        _ => return err(format!("{what}.split-debuginfo: invalid value")),
                    },
                    _ => return err(format!("{what}.{key}: profile setting is unsupported")),
                }
            }
            let opt_level = match get(p, "opt-level") {
                None => None,
                Some(Value::Integer(n @ 0..=3)) => Some(n.to_string()),
                Some(Value::String(s)) if matches!(s.as_str(), "s" | "z") => Some(s.clone()),
                Some(Value::Integer(_) | Value::String(_)) => {
                    return err(format!("{what}.opt-level: invalid value"));
                }
                Some(v) => return err(format!("{what}.opt-level: unexpected {}", v.type_name())),
            };
            let overflow_checks = get_bool(p, "overflow-checks", &what)?;
            profiles.insert(
                name.clone(),
                Profile {
                    inherits,
                    debug_assertions: get_bool(p, "debug-assertions", &what)?,
                    overflow_checks,
                    opt_level,
                },
            );
        }
    }
    for name in profiles.keys() {
        resolve_profile(&profiles, name)?;
    }
    Ok(Workspace {
        inputs: DiscoveryInputs::untracked(),
        root: root_dir,
        packages: loader.packages,
        members,
        default_members,
        profiles,
        lockfile,
    })
}

fn workspace_includes(
    ws: &Table,
    root: &Path,
    package: &Path,
    inputs: &mut Recorder,
) -> Result<bool, ManifestError> {
    if let Some(Value::Array(a)) = get(ws, "exclude") {
        for e in strings(a, "workspace.exclude")? {
            if package.starts_with(root.join(e)) {
                return Ok(false);
            }
        }
    }
    if let Some(Value::Array(a)) = get(ws, "members") {
        for pattern in strings(a, "workspace.members")? {
            if expand_member(root, &pattern, inputs)?
                .iter()
                .any(|d| d == package)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Expand a member path; a trailing `*` path component globs directories.
fn expand_member(
    root: &Path,
    pattern: &str,
    inputs: &mut Recorder,
) -> Result<Vec<PathBuf>, ManifestError> {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        let dir = root.join(prefix);
        let mut out = Vec::new();
        let entries = inputs
            .read_dir(&dir)
            .map_err(|e| ManifestError(format!("{}: {e}", dir.display())))?;
        for path in entries {
            if inputs.is_file(&path.join("Cargo.toml")) {
                out.push(inputs.canonicalize(&path).unwrap_or(path));
            }
        }
        out.sort();
        return Ok(out);
    }
    if pattern.contains(['*', '?', '[']) {
        return err(format!("unsupported workspace member glob `{pattern}`"));
    }
    let path = root.join(pattern);
    Ok(vec![inputs.canonicalize(&path).map_err(|e| {
        ManifestError(format!(
            "failed to load workspace member `{}`: {e}",
            path.display()
        ))
    })?])
}

struct Loader<'a> {
    inputs: &'a mut Recorder,
    packages: Vec<Package>,
    by_dir: BTreeMap<PathBuf, usize>,
    workspace_deps: Table,
    workspace_package: Table,
    lockfile: Lockfile,
}

impl Loader<'_> {
    fn package(&mut self, dir: &Path) -> Result<usize, ManifestError> {
        self.package_with_lock(dir, None)
    }

    fn package_with_lock(
        &mut self,
        dir: &Path,
        locked: Option<usize>,
    ) -> Result<usize, ManifestError> {
        let dir = self
            .inputs
            .canonicalize(dir)
            .map_err(|e| ManifestError(format!("{}: {e}", dir.display())))?;
        if let Some(index) = self.by_dir.get(&dir) {
            return Ok(*index);
        }
        let path = dir.join("Cargo.toml");
        let table = read_manifest(&path, self.inputs)?;
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
        match edition.as_str() {
            "2015" | "2018" | "2021" | "2024" => {}
            _ => {
                return err(format!(
                    "`{}`: invalid Rust edition `{edition}`; expected 2015, 2018, 2021, or 2024",
                    path.display()
                ));
            }
        }
        let locked = match locked {
            Some(index) => Some(index),
            None => self.find_package_lock(&name, &version, None),
        };
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
                    dependencies.push(self.dependency(
                        &dir,
                        locked,
                        key,
                        &spec.value,
                        dev,
                        section,
                    )?);
                }
            }
        }
        if let Some(targets) = get(&table, "target") {
            let Value::Table(targets) = targets else {
                return err("target: expected a table");
            };
            for (selector, target) in targets {
                let active = super::target_cfg::matches(selector)
                    .map_err(|error| ManifestError(format!("target.{selector}: {error}")))?;
                let Value::Table(sections) = &target.value else {
                    return err(format!("target.{selector}: expected a table"));
                };
                for (section, dependencies_value) in sections {
                    let dev = match section.as_str() {
                        "dependencies" | "build-dependencies" => false,
                        "dev-dependencies" => true,
                        _ => return err(format!("unsupported target.{selector}.{section}")),
                    };
                    let Value::Table(deps) = &dependencies_value.value else {
                        return err(format!("target.{selector}.{section}: expected a table"));
                    };
                    if active && section == "build-dependencies" && !deps.is_empty() {
                        return err(format!(
                            "`{}`: build-dependencies (build scripts) are unsupported",
                            path.display()
                        ));
                    }
                    for (key, spec) in deps {
                        let mut dependency =
                            self.dependency(&dir, locked, key, &spec.value, dev, section)?;
                        dependency.target_active = active;
                        dependencies.push(dependency);
                    }
                }
            }
        }
        let build_script = match get(package, "build") {
            Some(Value::Boolean(false)) => false,
            Some(Value::Boolean(true)) => true,
            Some(Value::String(_)) => true,
            Some(v) => return err(format!("package.build: unexpected {}", v.type_name())),
            None => self.inputs.is_file(&dir.join("build.rs")),
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
        let targets = discover_targets(&dir, &name, &table, self.inputs)?;
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
            locked,
            registry: false,
        });
        Ok(index)
    }

    fn registry_package(
        &mut self,
        package: &LockedRegistryPackage,
    ) -> Result<usize, ManifestError> {
        self.inputs.uncontrolled();
        let root = registry_source_root(package)?;
        let index = self
            .find_lock_package(&package.name, &package.version, Some(&package.source))
            .ok_or_else(|| {
                ManifestError(format!(
                    "Cargo.lock does not contain locked registry package `{} {}` from {}",
                    package.name, package.version, package.source
                ))
            })?;
        let loaded = self.package_with_lock(&root, Some(index))?;
        let cached = &self.packages[loaded];
        if cached
            .dependencies
            .iter()
            .any(|dep| matches!(dep.source, DependencySource::Path(_)))
        {
            return err("registry package path dependencies are unsupported");
        }
        for target in &cached.targets {
            let target_path = self
                .inputs
                .canonicalize(&target.path)
                .map_err(|e| ManifestError(e.to_string()))?;
            if !target_path.starts_with(&cached.root) {
                return err("registry target escapes authenticated package source");
            }
        }
        if self.packages[loaded].name != package.name
            || self.packages[loaded].version != package.version
        {
            return err("cached registry manifest does not match locked package identity");
        }
        self.packages[loaded].registry = true;
        Ok(loaded)
    }

    fn find_package_lock(&self, name: &str, version: &str, source: Option<&str>) -> Option<usize> {
        let mut matches = self
            .lockfile
            .packages
            .iter()
            .enumerate()
            .filter(|(_, pkg)| {
                pkg.name == name && pkg.version == version && pkg.source.as_deref() == source
            });
        let found = matches.next().map(|(index, _)| index);
        if matches.next().is_some() {
            None
        } else {
            found
        }
    }

    fn find_lock_package(&self, name: &str, version: &str, source: Option<&str>) -> Option<usize> {
        self.find_package_lock(name, version, source)
    }

    fn dependency(
        &mut self,
        dir: &Path,
        locked: Option<usize>,
        key: &str,
        spec: &Value,
        dev: bool,
        section: &str,
    ) -> Result<Dependency, ManifestError> {
        let what = format!("{section}.{key}");
        let mut table = match spec {
            Value::String(version) => Table::from([(
                "version".into(),
                toml::Spanned {
                    value: Value::String(version.clone()),
                    offset: 0,
                },
            )]),
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
                Value::String(version) => Table::from([(
                    "version".into(),
                    toml::Spanned {
                        value: Value::String(version.clone()),
                        offset: 0,
                    },
                )]),
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
            while !self.inputs.is_file(&root.join("Cargo.toml"))
                || read_manifest(&root.join("Cargo.toml"), self.inputs)
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
        let package = get_str(&table, "package", &what)?.unwrap_or_else(|| key.to_string());
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
        let source = if let Some(path) = path {
            DependencySource::Path(base.join(path))
        } else if get(&table, "git").is_some() {
            DependencySource::Git
        } else if let Some(locked) = locked {
            match self.locked_registry_dependency(locked, &package)? {
                Some(pkg) => DependencySource::Registry(pkg),
                None => DependencySource::UnresolvedRegistry,
            }
        } else {
            DependencySource::UnresolvedRegistry
        };
        if let DependencySource::Registry(registry) = &source {
            if get(&table, "registry").is_some() || get(&table, "registry-index").is_some() {
                return err(format!("{what}: alternate registries are unsupported"));
            }
            if let Some(requirement) = get_str(&table, "version", &what)? {
                validate_locked_version(&requirement, &registry.version)?;
            }
        }
        Ok(Dependency {
            name: key.replace('-', "_"),
            package,
            source,
            dev,
            target_active: true,
            optional: get_bool(&table, "optional", &what)?.unwrap_or(false),
            features,
            default_features: get_bool(&table, "default-features", &what)?
                .or(get_bool(&table, "default_features", &what)?)
                .unwrap_or(true),
        })
    }

    fn locked_registry_dependency(
        &self,
        locked: usize,
        package: &str,
    ) -> Result<Option<LockedRegistryPackage>, ManifestError> {
        let locked_package = &self.lockfile.packages[locked];
        let mut dependencies = locked_package
            .dependencies
            .iter()
            .filter(|dep| dep.name == package);
        let dep = dependencies.next();
        if dependencies.next().is_some() {
            return err(format!("Cargo.lock dependency `{package}` is ambiguous"));
        }
        let Some(dep) = dep else {
            return Ok(None);
        };
        let candidates: Vec<_> = self
            .lockfile
            .packages
            .iter()
            .filter(|candidate| {
                candidate.name == dep.name
                    && dep
                        .version
                        .as_ref()
                        .is_none_or(|version| &candidate.version == version)
                    && dep
                        .source
                        .as_ref()
                        .is_none_or(|source| candidate.source.as_ref() == Some(source))
            })
            .collect();
        if candidates.len() > 1 {
            return err(format!(
                "Cargo.lock dependency `{}` from package `{}` is ambiguous; include an exact package entry",
                dep.name, locked_package.name
            ));
        }
        let Some(target) = candidates.into_iter().next() else {
            return err(format!(
                "Cargo.lock dependency `{}` referenced by `{}` was not found",
                dep.name, locked_package.name
            ));
        };
        let Some(source) = &target.source else {
            return Ok(None);
        };
        if !source.starts_with("registry+") {
            return Ok(None);
        }
        let Some(checksum) = &target.checksum else {
            return err(format!(
                "Cargo.lock package `{} {}` is missing a checksum",
                target.name, target.version
            ));
        };
        Ok(Some(LockedRegistryPackage {
            name: target.name.clone(),
            version: target.version.clone(),
            source: source.clone(),
            checksum: checksum.clone(),
        }))
    }
}

fn discover_targets(
    dir: &Path,
    package: &str,
    table: &Table,
    inputs: &mut Recorder,
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
    if lib_table.is_some() || (auto("autolib") && inputs.is_file(&lib_default)) {
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
            doctest: get_bool(&t, "doctest", "lib")?.unwrap_or(true),
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
                        if inputs.is_file(&flat) {
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
                    doctest: false,
                    harness: get_bool(t, "harness", key)?.unwrap_or(true),
                });
            }
        }
        let mut found = Vec::new();
        if auto(auto_key) {
            if kind == TargetKind::Bin && inputs.is_file(&dir.join("src/main.rs")) {
                found.push((package.to_string(), dir.join("src/main.rs")));
            }
            if let Ok(entries) = inputs.read_dir(&dir.join(default_dir)) {
                for path in entries {
                    if path.extension().is_some_and(|e| e == "rs") && inputs.is_file(&path) {
                        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                        found.push((name, path));
                    } else if inputs.is_file(&path.join("main.rs")) {
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
                    doctest: false,
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
        if !dep.target_active {
            return None;
        }
        let path = match &dep.source {
            DependencySource::Path(path) => path.canonicalize().ok()?,
            DependencySource::Registry(registry) => registry_source_root(registry).ok()?,
            DependencySource::UnresolvedRegistry | DependencySource::Git => return None,
        };
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
                let matching: Vec<_> = package
                    .dependencies
                    .iter()
                    .filter(|d| d.name == dep.replace('-', "_"))
                    .collect();
                if matching.is_empty() {
                    return err(format!(
                        "package `{}`: unknown dependency `{dep}` in feature",
                        package.name
                    ));
                }
                for d in matching {
                    if !weak && d.optional {
                        active_deps[index].insert(d.name.clone());
                    }
                    if (!d.optional || active_deps[index].contains(&d.name))
                        && let Some(target) = dep_index(ws, d)
                    {
                        work.push((target, sub.to_string()));
                        queue.push((target, false));
                    }
                }
                continue;
            }
            if let Some(dep) = feature.strip_prefix("dep:") {
                active_deps[index].insert(dep.replace('-', "_"));
                for d in package
                    .dependencies
                    .iter()
                    .filter(|d| d.name == dep.replace('-', "_"))
                {
                    if let Some(target) = dep_index(ws, d) {
                        queue.push((target, false));
                    }
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
                    let matching: Vec<_> = package
                        .dependencies
                        .iter()
                        .filter(|d| d.optional && d.name == feature.replace('-', "_"))
                        .collect();
                    if matching.is_empty() {
                        return err(format!(
                            "package `{}` does not have the feature `{feature}`",
                            package.name
                        ));
                    }
                    for d in matching {
                        active_deps[index].insert(d.name.clone());
                        if let Some(target) = dep_index(ws, d) {
                            queue.push((target, false));
                        }
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
            if !d.target_active
                || (d.dev && !dev)
                || (d.optional && !active_deps[index].contains(&d.name))
            {
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
        if !d.target_active {
            continue;
        }
        if d.dev && !dev {
            continue;
        }
        if d.optional && !active_deps.contains(&d.name) {
            continue;
        }
        let Some(path) = dependency_root(d).and_then(|p| p.canonicalize().ok()) else {
            unresolved.push(match &d.source {
                DependencySource::Registry(registry) => format!(
                    "package `{}`: locked registry dependency `{}` {} from {} is not available in the local Cargo cache or failed checksum validation",
                    ws.packages[package].name, registry.name, registry.version, registry.source
                ),
                DependencySource::UnresolvedRegistry => format!(
                    "package `{}`: dependency `{}` is a registry dependency but Cargo.lock does not pin it to a cached package the native driver can use offline",
                    ws.packages[package].name, d.package
                ),
                DependencySource::Git => format!(
                    "package `{}`: dependency `{}` is a git dependency, which the native driver cannot fetch",
                    ws.packages[package].name, d.package
                ),
                DependencySource::Path(path) => format!(
                    "package `{}`: dependency path `{}` is missing",
                    ws.packages[package].name, path.display()
                ),
            });
            continue;
        };
        if let Some(index) = ws.packages.iter().position(|p| p.root == path) {
            out.push((d.name.clone(), index));
        }
    }
    out
}

fn dependency_root(dep: &Dependency) -> Option<PathBuf> {
    match &dep.source {
        DependencySource::Path(path) => Some(path.clone()),
        DependencySource::Registry(registry) => registry_source_root(registry).ok(),
        DependencySource::UnresolvedRegistry | DependencySource::Git => None,
    }
}

fn registry_source_root(package: &LockedRegistryPackage) -> Result<PathBuf, ManifestError> {
    let cargo_home = cargo_home();
    let registry = cargo_home.join("registry");
    if package.source != "registry+https://github.com/rust-lang/crates.io-index" {
        return err(format!(
            "unsupported locked registry source `{}`",
            package.source
        ));
    }
    if !package
        .name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || package.name.is_empty()
        || !package
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'+')
        || package.version.is_empty()
        || package.checksum.len() != 64
        || !package
            .checksum
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return err("invalid locked registry package identity or checksum");
    }
    let stem = format!("{}-{}", package.name, package.version);
    let src_root = registry.join("src");
    let cache_root = registry.join("cache");
    let mut candidates = Vec::new();
    let entries = std::fs::read_dir(&src_root)
        .map_err(|e| ManifestError(format!("{}: {e}", src_root.display())))?;
    for entry in entries.flatten() {
        let registry_dir = entry.path();
        let Some(name) = registry_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !matches!(
            name,
            "index.crates.io-1949cf8c6b5b557f" | "github.com-1ecc6299db9ec823"
        ) {
            continue;
        }
        let source_dir = registry_dir.join(&stem);
        let archive = cache_root.join(name).join(format!("{stem}.crate"));
        if source_dir.join("Cargo.toml").is_file() && archive.is_file() {
            candidates.push((source_dir, archive));
        }
    }
    if candidates.is_empty() {
        return err(format!(
            "locked registry package `{}` {} from {} is not cached under `{}`",
            package.name,
            package.version,
            package.source,
            registry.display()
        ));
    }
    let mut matched = Vec::new();
    for (source_dir, archive) in candidates {
        let bytes = std::fs::read(&archive)
            .map_err(|e| ManifestError(format!("{}: {e}", archive.display())))?;
        if sha256(&bytes) == package.checksum {
            verify_registry_sources(&source_dir, &archive, &stem)?;
            matched.push(source_dir);
        }
    }
    match matched.len() {
        1 => matched
            .pop()
            .unwrap()
            .canonicalize()
            .map_err(|e| ManifestError(e.to_string())),
        0 => err(format!(
            "locked registry package `{}` {} from {} failed checksum validation against every cached archive",
            package.name, package.version, package.source
        )),
        _ => err(format!(
            "locked registry package `{}` {} from {} matched multiple local registries",
            package.name, package.version, package.source
        )),
    }
}

fn validate_locked_version(requirement: &str, locked: &str) -> Result<(), ManifestError> {
    // Explicitly bounded Cargo requirement slice. Never guess at unsupported ranges.
    let requirement = requirement.trim();
    let (exact, text) = if let Some(text) = requirement.strip_prefix('=') {
        (true, text.trim())
    } else {
        (false, requirement.strip_prefix('^').unwrap_or(requirement))
    };
    let parse = |text: &str| -> Option<Vec<u64>> {
        text.split('.')
            .map(|part| {
                if part.is_empty()
                    || !part.bytes().all(|b| b.is_ascii_digit())
                    || (part.len() > 1 && part.starts_with('0'))
                {
                    None
                } else {
                    part.parse().ok()
                }
            })
            .collect()
    };
    let wanted = parse(text).filter(|v| !v.is_empty() && v.len() <= 3);
    let actual = parse(locked).filter(|v| v.len() == 3);
    let (Some(wanted), Some(actual)) = (wanted, actual) else {
        return err(format!(
            "unsupported locked registry version requirement `{requirement}` or version `{locked}`"
        ));
    };
    let mut lower = [0; 3];
    lower[..wanted.len()].copy_from_slice(&wanted);
    let actual = [actual[0], actual[1], actual[2]];
    let matches = if exact {
        actual[..wanted.len()] == wanted
    } else {
        let pivot = wanted
            .iter()
            .position(|n| *n != 0)
            .unwrap_or(wanted.len() - 1);
        actual >= lower && actual[..=pivot] == lower[..=pivot]
    };
    if !matches {
        return err(format!(
            "Cargo.lock version `{locked}` does not satisfy manifest requirement `{requirement}`"
        ));
    }
    Ok(())
}

/// Compare every extracted file against the checksum-authenticated archive.
/// Cargo's extraction marker and checksum JSON are not source authority.
/// This read-only bootstrap uses system tar, never Cargo, rustc, or a fetcher.
fn verify_registry_sources(root: &Path, archive: &Path, stem: &str) -> Result<(), ManifestError> {
    use std::collections::BTreeSet;
    use std::process::Command;
    let listing = Command::new("tar")
        .env_remove("TAR_OPTIONS")
        .env_remove("GZIP")
        .arg("-tzf")
        .arg(archive)
        .output()
        .map_err(|e| ManifestError(format!("registry archive inspection failed: {e}")))?;
    if !listing.status.success() {
        return err("registry archive inspection failed");
    }
    let listing = std::str::from_utf8(&listing.stdout)
        .map_err(|_| ManifestError("registry archive has non-UTF-8 paths".into()))?;
    let mut expected = BTreeSet::new();
    for entry in listing.lines() {
        let path = Path::new(entry);
        if !path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return err("registry archive has unsafe paths");
        }
        let relative = path
            .strip_prefix(stem)
            .map_err(|_| ManifestError("registry archive package identity mismatch".into()))?;
        if relative.as_os_str().is_empty() || entry.ends_with('/') {
            continue;
        }
        if !expected.insert(relative.to_path_buf()) {
            return err("registry archive has duplicate file paths");
        }
        let local = root.join(relative);
        if !std::fs::symlink_metadata(&local).is_ok_and(|m| m.file_type().is_file()) {
            return err(format!(
                "registry source checksum validation failed at `{}`",
                local.display()
            ));
        }
        let original = Command::new("tar")
            .env_remove("TAR_OPTIONS")
            .env_remove("GZIP")
            .arg("-xOzf")
            .arg(archive)
            .arg("--")
            .arg(entry)
            .output()
            .map_err(|e| ManifestError(format!("registry archive inspection failed: {e}")))?;
        let bytes = std::fs::read(&local)
            .map_err(|e| ManifestError(format!("{}: {e}", local.display())))?;
        if !original.status.success() || original.stdout != bytes {
            return err(format!(
                "registry source checksum validation failed at `{}`",
                local.display()
            ));
        }
    }
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_dir()) {
            return err("registry source contains a symlink or non-directory");
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| ManifestError(e.to_string()))? {
            let entry = entry.map_err(|e| ManifestError(e.to_string()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| ManifestError(e.to_string()))?;
            if kind.is_dir() {
                dirs.push(path);
            } else if !kind.is_file()
                || (!expected.contains(path.strip_prefix(root).unwrap())
                    && path != root.join(".cargo-ok")
                    && path != root.join(".cargo-checksum.json"))
            {
                return err(format!(
                    "registry source has unauthenticated file `{}`",
                    path.display()
                ));
            }
        }
    }
    if !expected.contains(Path::new("Cargo.toml")) {
        return err("registry archive lacks Cargo.toml");
    }
    Ok(())
}

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
}

fn sha256(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64) * 8;
    let blocks = (input.len() + 9).div_ceil(64);
    for block in 0..blocks {
        let mut bytes = [0u8; 64];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            let pos = block * 64 + offset;
            *byte = if pos < input.len() {
                input[pos]
            } else if pos == input.len() {
                0x80
            } else if pos >= blocks * 64 - 8 {
                bit_len.to_be_bytes()[pos - (blocks * 64 - 8)]
            } else {
                0
            };
        }
        let mut w = [0u32; 64];
        for (i, word) in w[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(add);
        }
    }
    use std::fmt::Write;
    let mut output = String::with_capacity(64);
    for word in state {
        write!(&mut output, "{word:08x}").unwrap();
    }
    output
}
