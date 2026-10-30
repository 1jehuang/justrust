//! `cargo remove`: dependencies taken out of a member's manifest, as cargo 1.99.0's
//! `commands/remove.rs`, `ops/cargo_remove.rs` and `util/toml_mut/manifest.rs` do: the
//! features that named them, then the `[workspace.dependencies]`, `[profile.*.package]`,
//! `[replace]` and `[patch]` entries nothing uses any more, and the lock file, resolved again
//! by blaze with every version it keeps as `Cargo.lock` locks it.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use blaze_manifest::metadata::root_manifest;
use blaze_manifest::{Context, Workspace};
use blaze_resolve::{PackageId, PackageIdSpec, SourceId, closest};
use toml_edit::{DocumentMut, Item};
use url::Url;

use super::args::{Args, Read, switch, valued};
use super::manifest::{USAGE, config, failed};

const HELP: &str = "\
Remove dependencies from a Cargo.toml manifest file

Usage: cargo remove [OPTIONS] <DEP_ID>...

Arguments:
  <DEP_ID>...  Dependencies to be removed

Options:
  -n, --dry-run                  Don't actually write the manifest
  -v, --verbose...               Use verbose output (-vv very verbose/build.rs output)
  -q, --quiet                    Do not print cargo log messages
      --color <WHEN>             Coloring [possible values: auto, always, never]
      --config <KEY=VALUE|PATH>  Override a configuration value
  -Z <FLAG>                      Unstable (nightly-only) flags to Cargo, see 'cargo -Z help' for
                                 details
  -h, --help                     Print help

Section:
      --dev              Remove from dev-dependencies
      --build            Remove from build-dependencies
      --target <TARGET>  Remove from target-dependencies

Package Selection:
  -p, --package [<SPEC>]  Package to remove from

Manifest Options:
  -m, --manifest-path <PATH>  Path to Cargo.toml
      --locked                Assert that `Cargo.lock` will remain unchanged
      --offline               Run without accessing the network
      --frozen                Equivalent to specifying both --locked and --offline

Run `cargo help remove` for more detailed information.";

const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// Each flag of `cargo remove` as its usage line writes it, by every name it has.
const USED: &[(&[&str], &str)] = &[
    (&["-n", "--dry-run"], "--dry-run"),
    (&["--dev"], "--dev"),
    (&["--build"], "--build"),
    (&["--target"], "--target <TARGET>"),
    (&["-p", "--package"], "--package [<SPEC>]"),
    (&["-m", "--manifest-path"], "--manifest-path <PATH>"),
    (&["-v", "-vv", "--verbose"], "--verbose..."),
    (&["-q", "--quiet"], "--quiet"),
    (&["--color"], "--color <WHEN>"),
    (&["--config"], "--config <KEY=VALUE|PATH>"),
    (&["-Z"], "-Z <FLAG>"),
    (&["--locked"], "--locked"),
    (&["--offline"], "--offline"),
    (&["--frozen"], "--frozen"),
];

/// The usage line of a refused command line: the flags `typed` has, as clap lists them,
/// in the order typed and each once, but for `left_out`.
fn usage_line(typed: &[String], left_out: Option<&str>) -> String {
    let mut line = vec!["cargo remove"];
    let mut words = typed.iter().peekable();
    while let Some(word) = words.next() {
        if word == "--" {
            break;
        }
        let name = word.split_once('=').map_or(word.as_str(), |(name, _)| name);
        let Some((_, shown)) = USED.iter().find(|(names, _)| names.contains(&name)) else {
            continue;
        };
        if shown.contains(" <") && !word.contains('=') {
            words.next();
        }
        if shown.contains("[<") && !word.contains('=') {
            words.next_if(|it| !it.starts_with('-'));
        }
        if Some(*shown) != left_out && !line.contains(shown) {
            line.push(shown);
        }
    }
    line.push("<DEP_ID>...");
    line.join(" ")
}

/// `cargo remove [OPTIONS] <DEP_ID>...`.
pub(super) fn command(typed: &[String]) -> anyhow::Result<i32> {
    let flags = typed.iter().position(|it| it == "--").unwrap_or(typed.len());
    let bare = |at: usize| {
        let next = typed.get(at + 1);
        at < flags
            && matches!(typed[at].as_str(), "-p" | "--package")
            && next.is_none_or(|it| it.starts_with('-'))
    };
    let list = (0..typed.len()).any(bare);
    let kept: Vec<String> =
        typed.iter().enumerate().filter(|&(at, _)| !bare(at)).map(|(_, it)| it.clone()).collect();
    let args = Args::new(&kept);
    let flags = [
        switch(&["-n", "--dry-run"]),
        switch(&["--dev"]),
        switch(&["--build"]),
        valued(&["--target"]),
        valued(&["-p", "--package"]),
        valued(&["-m", "--manifest-path"]),
    ];
    let usage = |why: &str, line: &str| {
        super::super::say::error(why);
        eprintln!("\nUsage: {line}\n\nFor more information, try '--help'.");
        Ok(USAGE)
    };
    let read = match args.read(&flags) {
        Ok(read) if read.has("--help") => {
            println!("{HELP}");
            return Ok(0);
        }
        Ok(read) => read,
        Err(why) => return usage(&why, "cargo remove [OPTIONS] <DEP_ID>..."),
    };
    if read.positional.is_empty() {
        let why = "the following required arguments were not provided:\n  <DEP_ID>...";
        return usage(why, &usage_line(typed, None));
    }
    if read.has("--dev") && read.has("--build") {
        let dev = typed.iter().position(|it| it == "--dev");
        let build = typed.iter().position(|it| it == "--build");
        let (first, second) = match dev < build {
            true => ("--dev", "--build"),
            false => ("--build", "--dev"),
        };
        let why = format!("the argument '{first}' cannot be used with '{second}'");
        return usage(&why, &usage_line(typed, Some(second)));
    }
    match remove(&read, list) {
        Ok(()) => Ok(0),
        Err(err) => Ok(failed(&err)),
    }
}

/// `list`: a `-p` with no value, which has the members listed instead.
fn remove(read: &Read, list: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("couldn't get the current directory")?;
    let quiet = read.has("--quiet");
    let dry_run = read.has("--dry-run");
    let ctx = Context::new(config("remove", read)?);
    let manifest_path = read.value("--manifest-path").map(|it| cwd.join(it));
    let ws = Workspace::new(&root_manifest(manifest_path.as_deref(), &ctx)?, &ctx)?;
    if list {
        let mut text =
            "\"--package <SPEC>\" requires a SPEC format value, which can be any package \
                        ID specifier in the dependency graph.\nRun `cargo help pkgid` for more \
                        information about SPEC format.\n\nPossible packages/workspace members:\n"
                .to_owned();
        for (_, m) in ws.members() {
            text.push_str(&format!("    {}\n", m.package_id.name));
        }
        bail!(text);
    }
    let path = selected(&ws, read.value("--package").as_deref())?;

    let kind = match (read.has("--dev"), read.has("--build")) {
        (true, _) => KINDS[1],
        (_, true) => KINDS[2],
        _ => KINDS[0],
    };
    let target = read.value("--target");
    let table: Vec<String> = match &target {
        Some(target) => vec!["target".to_owned(), target.clone(), kind.to_owned()],
        None => vec![kind.to_owned()],
    };
    let section = match &target {
        Some(target) => format!("{kind} for target `{target}`"),
        None => kind.to_owned(),
    };
    let lock = ws.root().join("Cargo.lock");
    if !dry_run && !lock.exists() {
        bail!(
            "{} has no lock file, and writing one chooses versions, which blaze's resolver does \
             not yet (#321)",
            ws.root().display()
        );
    }

    let mut manifest = Local::read(&path)?;
    for dep in &read.positional {
        super::super::say::status(quiet, "Removing", &format!("{dep} from {section}"));
        manifest.remove_from_table(&table, dep)?;
        manifest.gc_dep(dep);
    }
    if dry_run {
        super::super::say::warning("aborting remove due to dry run");
        return Ok(());
    }
    manifest.write()?;

    let members: Vec<PathBuf> = ws.members().map(|(path, _)| path.to_owned()).collect();
    gc_workspace(&ctx, ws.root_manifest(), &members)?;
    let locked = match (read.has("--frozen"), read.has("--locked")) {
        (true, _) => Some("--frozen"),
        (_, true) => Some("--locked"),
        _ => None,
    };
    let config: Vec<String> = read.values("--config");
    let asked = blaze_metadata::Asked {
        cwd: &cwd,
        env: std::env::vars().collect(),
        manifest_path: manifest_path.as_deref(),
        config: &config,
        features: &[],
        all_features: false,
        no_default_features: false,
        build: None,
        platform: None,
        offline: read.has("--offline") || read.has("--frozen"),
        store: None,
        rustc_version: Some(crate::cargo::toolchain::ours_release()),
    };
    let unused = blaze_metadata::relock(&asked, "remove", locked)?;
    if gc_unused_patches(&ctx, ws.root_manifest(), &unused)? {
        blaze_metadata::relock(&asked, "remove", locked)?;
    }
    Ok(())
}

/// The manifest of the one package `-p` or the default members select.
fn selected(ws: &Workspace<'_>, spec: Option<&str>) -> anyhow::Result<PathBuf> {
    let mut picked = Vec::new();
    match spec {
        Some(spec) => {
            let parsed = PackageIdSpec::parse(spec)?;
            for (path, m) in ws.members() {
                let dir = path.parent().context("a manifest's directory")?;
                let version = m.package_id.version.to_string();
                let id =
                    PackageId::try_new(&m.package_id.name, &version, SourceId::for_path(dir)?)?;
                if parsed.matches(id) {
                    picked.push((path.to_owned(), m.package_id.name.to_string()));
                }
            }
            if picked.is_empty() {
                bail!("package(s) `{spec}` not found in workspace `{}`", ws.root().display());
            }
        }
        None => {
            let defaults = ws.default_members();
            picked
                .extend(defaults.map(|(path, m)| (path.to_owned(), m.package_id.name.to_string())));
        }
    }
    match picked.len() {
        1 => Ok(picked.remove(0).0),
        0 => bail!("no package selected to modify\nhelp: specify a package with `-p <PKGID>`"),
        _ => {
            let names: Vec<String> = picked.into_iter().map(|it| it.1).collect();
            bail!(
                "no package selected to modify\nhelp: specify a package with `-p <PKGID>`\n      \
                 available packages: {}",
                names.join(", ")
            )
        }
    }
}

/// A manifest as `toml_edit` keeps it, comments and layout and all: cargo's `LocalManifest`.
struct Local {
    path: PathBuf,
    data: DocumentMut,
}

impl Local {
    fn read(path: &Path) -> anyhow::Result<Local> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read `{}`", path.display()))?;
        let data = raw
            .parse::<DocumentMut>()
            .context("manifest not valid TOML")
            .context("unable to parse Cargo.toml")?;
        Ok(Local { path: path.to_owned(), data })
    }

    fn write(&self) -> anyhow::Result<()> {
        let dir = self.path.parent().context("a manifest's directory")?;
        let temp = dir.join(format!(".{}.blaze-remove", std::process::id()));
        std::fs::write(&temp, self.data.to_string())
            .and_then(|()| std::fs::rename(&temp, &self.path))
            .with_context(|| format!("failed to write `{}`", self.path.display()))
    }

    fn get_table_mut(&mut self, path: &[String]) -> Option<&mut Item> {
        fn descend<'a>(input: &'a mut Item, path: &[String]) -> Option<&'a mut Item> {
            let Some(segment) = path.first() else { return Some(input) };
            let mut default = toml_edit::Table::new();
            default.set_implicit(true);
            let value = input[segment.as_str()].or_insert(Item::Table(default));
            match value.is_table_like() {
                true => descend(value, &path[1..]),
                false => None,
            }
        }
        descend(self.data.as_item_mut(), path)
    }

    /// Every table of dependencies, with the path of keys to it.
    fn sections(&self) -> Vec<(Vec<String>, Item)> {
        let mut sections = Vec::new();
        for kind in KINDS {
            if let Some(table) = self.data.get(kind).filter(|it| it.is_table_like()) {
                sections.push((vec![kind.to_owned()], table.clone()));
            }
            let targets = self.data.as_table().get("target").and_then(Item::as_table_like);
            for (name, target) in targets.into_iter().flat_map(|it| it.iter()) {
                if let Some(table) = target.get(kind).filter(|it| it.is_table_like()) {
                    let path = vec!["target".to_owned(), name.to_owned(), kind.to_owned()];
                    sections.push((path, table.clone()));
                }
            }
        }
        sections
    }

    fn remove_from_table(&mut self, path: &[String], name: &str) -> anyhow::Result<()> {
        let parent = self.get_table_mut(path).context("a dependency table that is not a table")?;
        if let Some(dep) = parent.get_mut(name).filter(|it| !it.is_none()) {
            *dep = Item::None;
            if parent.as_table_like().is_some_and(|it| it.is_empty()) {
                *parent = Item::None;
            }
            return Ok(());
        }
        let names: Vec<String> = parent
            .as_table_like()
            .into_iter()
            .flat_map(|it| it.iter().map(|(key, _)| key.to_owned()))
            .collect();
        let alt_name = closest(name, names.iter(), |it| it.as_str()).cloned();
        let alt_path = self
            .sections()
            .into_iter()
            .find_map(|(path, table)| table.get(name).is_some().then_some(path));
        let mut error =
            format!("the dependency `{name}` could not be found in `{}`", path.join("."));
        if let Some(alt_path) = alt_path {
            let mut flags: Vec<&str> = Vec::new();
            match (path.last().unwrap().as_str(), alt_path.last().unwrap().as_str()) {
                (a, b) if a == b => {}
                (_, "build-dependencies") => flags.push("--build"),
                (_, "dev-dependencies") => flags.push("--dev"),
                _ => {}
            }
            if path[0] != "target" && alt_path[0] == "target" {
                flags.push("--target");
                flags.push(&alt_path[1]);
            }
            let alt = alt_path.join(".");
            match flags.is_empty() {
                false => error.push_str(&format!(
                    "\n\nhelp: pass `{}` to remove `{name}` from `{alt}`",
                    flags.join(" ")
                )),
                true => error.push_str(&format!(
                    "\n\nhelp: a dependency with the same name exists in `{alt}`"
                )),
            }
        } else if let Some(alt_name) = alt_name {
            error.push_str(&format!(
                "\n\nhelp: a dependency with a similar name exists: `{alt_name}`"
            ));
        }
        Err(anyhow!(error))
    }

    /// Takes `dep_key` out of the features that named it, now that no table has it, or, if
    /// another table still does, out of those that would turn it on.
    fn gc_dep(&mut self, dep_key: &str) {
        let explicit = self.is_explicit_dep_activation(dep_key);
        let status = self.dep_status(dep_key);
        if let Some(Item::Table(features)) = self.data.as_table_mut().get_mut("features") {
            for (_, values) in features.iter_mut() {
                if let Item::Value(toml_edit::Value::Array(values)) = values {
                    fix_feature_activations(values, dep_key, status, explicit);
                }
            }
        }
    }

    fn is_explicit_dep_activation(&self, dep_key: &str) -> bool {
        let Some(Item::Table(features)) = self.data.as_table().get("features") else {
            return false;
        };
        features
            .iter()
            .filter_map(|(_, it)| it.as_value()?.as_array())
            .flat_map(|it| it.iter().filter_map(|v| v.as_str()))
            .any(|it| matches!(FeatureValue::new(it), FeatureValue::Dep { dep_name } if dep_name == dep_key))
    }

    fn dep_status(&self, dep_key: &str) -> Status {
        let mut status = Status::None;
        for (_, table) in self.sections() {
            if let Item::Table(table) = table
                && let Some(dep) = table.get(dep_key)
            {
                let optional = dep.get("optional").and_then(|it| it.as_value()?.as_bool());
                if optional.unwrap_or(false) {
                    return Status::Optional;
                }
                status = Status::Required;
            }
        }
        status
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    None,
    Optional,
    Required,
}

/// A value of a feature, as cargo's `FeatureValue::new` reads it.
enum FeatureValue<'a> {
    Feature(&'a str),
    Dep { dep_name: &'a str },
    DepFeature { dep_name: &'a str, dep_feature: &'a str, weak: bool },
}

impl<'a> FeatureValue<'a> {
    fn new(text: &'a str) -> FeatureValue<'a> {
        match text.split_once('/') {
            Some((dep, dep_feature)) => {
                let weak = dep.ends_with('?');
                let dep_name = dep.strip_suffix('?').unwrap_or(dep);
                FeatureValue::DepFeature { dep_name, dep_feature, weak }
            }
            None => match text.strip_prefix("dep:") {
                Some(dep_name) => FeatureValue::Dep { dep_name },
                None => FeatureValue::Feature(text),
            },
        }
    }
}

fn fix_feature_activations(
    values: &mut toml_edit::Array,
    dep_key: &str,
    status: Status,
    explicit: bool,
) {
    let remove: Vec<usize> = values
        .iter()
        .enumerate()
        .filter_map(|(at, value)| Some((at, value.as_str()?)))
        .filter(|(_, value)| match (status, FeatureValue::new(value)) {
            (Status::None, FeatureValue::Feature(name)) => !explicit && name == dep_key,
            (
                Status::None,
                FeatureValue::Dep { dep_name } | FeatureValue::DepFeature { dep_name, .. },
            ) => dep_name == dep_key,
            (Status::Optional, _) => false,
            (Status::Required, FeatureValue::Feature(name)) => !explicit && name == dep_key,
            (Status::Required, FeatureValue::Dep { dep_name }) => dep_name == dep_key,
            (Status::Required, FeatureValue::DepFeature { .. }) => false,
        })
        .map(|(at, _)| at)
        .collect();
    for at in remove.into_iter().rev() {
        remove_array_index(values, at);
    }
    if status == Status::Required {
        for value in values.iter_mut() {
            let Some(text) = value.as_str() else { continue };
            if let FeatureValue::DepFeature { dep_name, dep_feature, weak: true } =
                FeatureValue::new(text)
                && dep_name == dep_key
            {
                let mut new = toml_edit::Value::from(format!("{dep_name}/{dep_feature}"));
                *new.decor_mut() = value.decor().clone();
                *value = new;
            }
        }
    }
}

/// Removes `array[index]`, keeping the lines of comments around it.
fn remove_array_index(array: &mut toml_edit::Array, index: usize) {
    let value = array.remove(index);
    let prefix = value.decor().prefix().and_then(|it| it.as_str()?.rsplit_once('\n'));
    let suffix = value.decor().suffix().and_then(|it| it.as_str()?.split_once('\n'));
    let mut merged = String::new();
    if let Some((lines, _)) = prefix {
        merged.push_str(lines);
        merged.push('\n');
    }
    if let Some((_, lines)) = suffix {
        merged.push_str(lines);
        merged.push('\n');
    }
    match array.get_mut(index) {
        Some(next) => {
            let decor = next.decor_mut();
            merged.push_str(decor.prefix().and_then(|it| it.as_str()).unwrap_or_default());
            decor.set_prefix(merged);
        }
        None => {
            merged.push_str(array.trailing().as_str().unwrap_or_default());
            array.set_trailing(merged);
        }
    }
}

/// What the clean-up reads of a dependency: cargo's `toml_mut::Dependency`, of which it uses
/// the key, the package's name, the version asked for and the source.
#[derive(Clone)]
struct Dep {
    key: String,
    name: String,
    version: Option<String>,
    /// None: `workspace = true`.
    source: Option<Url>,
}

impl Dep {
    fn from_toml(ctx: &Context, crate_root: &Path, key: &str, item: &Item) -> anyhow::Result<Dep> {
        let mut dep =
            Dep { key: key.to_owned(), name: key.to_owned(), version: None, source: None };
        if let Some(version) = item.as_str() {
            dep.version = Some(version.to_owned());
            dep.source = Some(SourceId::crates_io().url().clone());
            return Ok(dep);
        }
        let table = item
            .as_table_like()
            .with_context(|| format!("unrecognized dependency entry for `{key}`"))?;
        let text = |field: &str| table.get(field).and_then(|it| it.as_str()).map(str::to_owned);
        if let Some(name) = text("package") {
            dep.name = name;
        }
        dep.version = text("version");
        if table.get("workspace").and_then(|it| it.as_bool()) == Some(true) {
            return Ok(dep);
        }
        dep.source = Some(if let Some(path) = text("path") {
            let path = normalize(&crate_root.join(path));
            Url::from_file_path(&path).map_err(|()| anyhow!("invalid path `{}`", path.display()))?
        } else if let Some(git) = text("git") {
            Url::parse(&git).with_context(|| format!("invalid url `{git}`"))?
        } else if let Some(index) = text("registry-index") {
            Url::parse(&index).with_context(|| format!("invalid url `{index}`"))?
        } else if let Some(registry) = text("registry") {
            registry_url(ctx, &registry)?
        } else {
            SourceId::crates_io().url().clone()
        });
        Ok(dep)
    }
}

fn registry_url(ctx: &Context, name: &str) -> anyhow::Result<Url> {
    if name == "crates-io" {
        return Ok(SourceId::crates_io().url().clone());
    }
    let key = format!("registries.{name}.index");
    let index = ctx.config.get_string(&key)?.map(|it| it.val);
    let index = index
        .with_context(|| format!("registry index was not found in any configuration: `{name}`"))?;
    Url::parse(&index).with_context(|| format!("invalid url `{index}`"))
}

/// `path` with its `.` and `..` resolved by the words alone, as cargo's `normalize_path`.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Removes from the workspace's root manifest the `[workspace.dependencies]`,
/// `[profile.*.package]` and `[replace]` entries no member's dependency uses any more.
fn gc_workspace(ctx: &Context, root_manifest: &Path, members: &[PathBuf]) -> anyhow::Result<()> {
    let mut root = Local::read(root_manifest)?;
    let root_dir = root_manifest.parent().context("the workspace's directory")?;
    let mut deps = Vec::new();
    for member in members {
        let manifest = Local::read(member)?;
        let dir = member.parent().context("a member's directory")?;
        for (_, table) in manifest.sections() {
            for (key, item) in table.as_table_like().into_iter().flat_map(|it| it.iter()) {
                deps.push(Dep::from_toml(ctx, dir, key, item)?);
            }
        }
    }

    if let Some(Item::Table(table)) =
        root.data.get_mut("workspace").and_then(|it| it.get_mut("dependencies"))
    {
        table.set_implicit(true);
        for (key, item) in table.iter_mut() {
            let ws_dep = Dep::from_toml(ctx, root_dir, key.get(), item)?;
            let mut used = false;
            for dep in deps.iter_mut().filter(|it| it.key == key.get() && it.source.is_none()) {
                *dep = ws_dep.clone();
                used = true;
            }
            if !used {
                *item = Item::None;
            }
        }
    }

    if let Some(Item::Table(profiles)) = root.data.get_mut("profile") {
        profiles.set_implicit(true);
        for (_, profile) in profiles.iter_mut() {
            let Item::Table(profile) = profile else { continue };
            profile.set_implicit(true);
            let Some(Item::Table(packages)) = profile.get_mut("package") else { continue };
            packages.set_implicit(true);
            for (key, item) in packages.iter_mut() {
                if key.get().contains(['*', '?', '[', ']']) {
                    continue;
                }
                if !spec_has_match(&PackageIdSpec::parse(key.get())?, &deps)? {
                    *item = Item::None;
                }
            }
        }
    }

    if let Some(Item::Table(table)) = root.data.get_mut("replace") {
        table.set_implicit(true);
        for (key, item) in table.iter_mut() {
            if !spec_has_match(&PackageIdSpec::parse(key.get())?, &deps)? {
                *item = Item::None;
            }
        }
    }
    root.write()
}

/// Whether `spec` matches a dependency that is not the workspace's.
fn spec_has_match(spec: &PackageIdSpec, deps: &[Dep]) -> anyhow::Result<bool> {
    for dep in deps.iter().filter(|it| it.name == spec.name()) {
        let version = match (spec.version(), &dep.version) {
            (Some(version), Some(req)) => semver::VersionReq::parse(req)?.matches(&version),
            (Some(_), None) => false,
            (None, _) => true,
        };
        if !version {
            continue;
        }
        if let Some(source) = &dep.source
            && spec.url().is_none_or(|url| url == source)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Removes the `[patch]` entries the resolve did not use. Whether it removed any.
fn gc_unused_patches(
    ctx: &Context,
    root_manifest: &Path,
    unused: &[PackageId],
) -> anyhow::Result<bool> {
    let mut root = Local::read(root_manifest)?;
    let mut modified = false;
    if let Some(Item::Table(patches)) = root.data.get_mut("patch") {
        patches.set_implicit(true);
        for (_, patch) in patches.iter_mut() {
            let Item::Table(patch) = patch else { continue };
            patch.set_implicit(true);
            for (key, item) in patch.iter_mut() {
                // cargo reads a patch's `path` against the manifest's own path, not its
                // directory, so a path patch is never found unused.
                let dep = Dep::from_toml(ctx, root_manifest, key.get(), item)?;
                let Some(source) = dep.source else { continue };
                let url = format!("{source}#{}", dep.name);
                if PackageIdSpec::query_str(&url, unused).is_ok() {
                    *item = Item::None;
                    modified = true;
                }
            }
        }
    }
    if modified {
        root.write()?;
    }
    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::{FeatureValue, usage_line};

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_usage_line_lists_what_was_typed() {
        let line = usage_line(&words("-q --offline --target=foo -n -v -v"), None);
        assert_eq!(
            line,
            "cargo remove --quiet --offline --target <TARGET> --dry-run --verbose... <DEP_ID>..."
        );
        let line = usage_line(&words("--dev --target x --build"), Some("--build"));
        assert_eq!(line, "cargo remove --dev --target <TARGET> <DEP_ID>...");
    }

    #[test]
    fn feature_values() {
        assert!(matches!(FeatureValue::new("dep:c"), FeatureValue::Dep { dep_name: "c" }));
        assert!(matches!(
            FeatureValue::new("c?/x"),
            FeatureValue::DepFeature { dep_name: "c", dep_feature: "x", weak: true }
        ));
        assert!(matches!(FeatureValue::new("c"), FeatureValue::Feature("c")));
    }
}
