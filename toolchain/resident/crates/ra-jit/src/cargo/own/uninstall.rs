//! `cargo uninstall`: removes the programs `cargo install` put in its root's `bin`, and their
//! entries in the two files cargo tracks them in, `.crates.toml` and `.crates2.json`, as cargo
//! 1.99.0's `cargo_uninstall.rs` and `InstallTracker` do.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use blaze_manifest::config::{ConfigRelativePath, Definition};
use blaze_manifest::manifest::{EitherManifest, read_manifest};
use blaze_manifest::targets::TargetKind;
use blaze_manifest::{Config, Context};
use blaze_resolve::{PackageId, PackageIdSpec, SourceId};
use serde::{Deserialize, Serialize};

use super::args::{Args, Read, valued};
use super::manifest::{config, failed, read_or_refuse_taking};

/// `cargo uninstall [SPEC]... [--bin NAME]... [--root DIR]`.
pub(super) fn command(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [valued(&["--root"]), valued(&["-p", "--package"]), valued(&["--bin"])];
    let usage = "cargo blaze uninstall [OPTIONS] [SPEC]...";
    let read = match read_or_refuse_taking(&args, &flags, usage, usize::MAX) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    match uninstall(&read) {
        Ok(()) => Ok(0),
        Err(err) => Ok(failed(&err)),
    }
}

fn uninstall(read: &Read) -> anyhow::Result<()> {
    let ctx = Context::new(config("uninstall", read)?);
    let config = &ctx.config;
    let quiet = read.has("--quiet");
    let specs = match &read.positional[..] {
        [] => read.values("--package"),
        given => given.to_vec(),
    };
    let bins = read.values("--bin");
    anyhow::ensure!(
        specs.len() <= 1 || bins.is_empty(),
        "A binary can only be associated with a single installed package, specifying multiple \
         specs with --bin is redundant."
    );
    let root = root(read.value("--root").as_deref(), config)?;
    match &specs[..] {
        [] => {
            let tracker = Tracker::load(&root, quiet)?;
            let id = cwd_package(&ctx)?;
            remove(&root, tracker, id, &bins, quiet)
        }
        [spec] => one(&root, spec, &bins, quiet),
        specs => {
            let (mut succeeded, mut failed) = (Vec::new(), Vec::new());
            for spec in specs {
                match one(&root, spec, &bins, quiet) {
                    Ok(()) => succeeded.push(spec.as_str()),
                    Err(err) => {
                        super::super::metadata::refused(&err, config.cwd());
                        failed.push(spec.as_str());
                    }
                }
            }
            let mut summary = Vec::new();
            if !succeeded.is_empty() {
                summary.push(format!("Successfully uninstalled {}!", succeeded.join(", ")));
            }
            if !failed.is_empty() {
                summary.push(format!(
                    "Failed to uninstall {} (see error(s) above).",
                    failed.join(", ")
                ));
            }
            super::super::say::status(quiet, "Summary", &summary.join(" "));
            anyhow::ensure!(failed.is_empty(), "some packages failed to uninstall");
            Ok(())
        }
    }
}

fn one(root: &Path, spec: &str, bins: &[String], quiet: bool) -> anyhow::Result<()> {
    let tracker = Tracker::load(root, quiet)?;
    let all: Vec<PackageId> = tracker.v1.v1.keys().copied().collect();
    let id = PackageIdSpec::query_str(spec, &all)?;
    remove(root, tracker, id, bins, quiet)
}

/// The package of the manifest in the working directory, which `cargo uninstall` with no
/// spec uninstalls.
fn cwd_package(ctx: &Context) -> anyhow::Result<PackageId> {
    let cwd = ctx.config.cwd().to_owned();
    let manifest = cwd.join("Cargo.toml");
    let source = blaze_manifest::source::SourceId::for_path(&cwd)?;
    let package = match read_manifest(&manifest, source, ctx)? {
        EitherManifest::Real(package) => package,
        EitherManifest::Virtual(_) => anyhow::bail!(
            "found a virtual manifest at `{}` instead of a package manifest",
            manifest.display()
        ),
    };
    let kinds = |example: bool| {
        package.targets.iter().any(|it| match &it.kind {
            TargetKind::Bin => !example,
            TargetKind::ExampleBin | TargetKind::ExampleLib(_) => example,
            _ => false,
        })
    };
    anyhow::ensure!(kinds(false) || kinds(true), "no packages found with binaries or examples");
    let id = &package.package_id;
    Ok(PackageId::new(id.name.as_str().into(), id.version.clone(), SourceId::for_path(&cwd)?))
}

fn remove(
    root: &Path,
    mut tracker: Tracker,
    id: PackageId,
    bins: &[String],
    quiet: bool,
) -> anyhow::Result<()> {
    let Some(installed) = tracker.v1.v1.get(&id).cloned() else {
        anyhow::bail!("package `{id}` is not installed");
    };
    let dst = root.join("bin");
    for bin in &installed {
        let bin = dst.join(bin);
        anyhow::ensure!(
            bin.exists(),
            "corrupt metadata, `{}` does not exist when it should",
            bin.display()
        );
    }
    let suffix = std::env::consts::EXE_SUFFIX;
    let bins: BTreeSet<String> = bins
        .iter()
        .map(|it| if it.ends_with(suffix) { it.clone() } else { format!("{it}{suffix}") })
        .collect();
    for bin in &bins {
        anyhow::ensure!(installed.contains(bin), "binary `{bin}` not installed as part of `{id}`");
    }
    for bin in if bins.is_empty() { installed } else { bins } {
        let path = dst.join(&bin);
        super::super::say::status(quiet, "Removing", &path.display().to_string());
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove file `{}`", path.display()))?;
        tracker.remove_bin(id, &bin);
        tracker.save()?;
    }
    Ok(())
}

/// Where `cargo install` put its programs: `--root`, `CARGO_INSTALL_ROOT`, `install.root`, or
/// cargo's home.
fn root(flag: Option<&str>, config: &Config) -> anyhow::Result<PathBuf> {
    let configured = config.get::<Option<ConfigRelativePath>>("install.root")?.map(|it| {
        let resolved = it.resolve_program(config.cwd());
        if resolved.is_relative()
            && let Definition::Path(_) = &it.value().definition
        {
            let suggested = format!("{}/", resolved.display());
            super::super::say::warning(&format!(
                "the `install.root` value `{}` defined in {} without a trailing slash is \
                 deprecated\n  |\n  = note: a future version of Cargo will treat it as relative \
                 to the configuration directory\n  = help: add a trailing slash (`{suggested}`) \
                 to adopt the correct behavior and silence this warning\n  = note: see more at \
                 https://doc.rust-lang.org/cargo/reference/config.html#config-relative-paths",
                resolved.display(),
                it.value().definition
            ));
        }
        resolved
    });
    let set = config.env("CARGO_INSTALL_ROOT").map(PathBuf::from);
    Ok(flag.map(PathBuf::from).or(set).or(configured).unwrap_or_else(|| config.home().to_owned()))
}

/// The two files cargo tracks what it installed in, locked while they are read and written.
struct Tracker {
    v1: ListingV1,
    v2: ListingV2,
    files: [(PathBuf, File); 2],
}

/// `.crates.toml`: each package installed and the programs it installed.
#[derive(Default, Deserialize, Serialize)]
struct ListingV1 {
    #[serde(with = "by_id")]
    v1: BTreeMap<PackageId, BTreeSet<String>>,
}

/// `.crates2.json`: each package installed and how, with what newer cargos add kept.
#[derive(Default, Deserialize, Serialize)]
struct ListingV2 {
    #[serde(with = "by_id")]
    installs: BTreeMap<PackageId, InstallInfo>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
struct InstallInfo {
    version_req: Option<String>,
    bins: BTreeSet<String>,
    features: BTreeSet<String>,
    all_features: bool,
    no_default_features: bool,
    profile: String,
    target: Option<String>,
    rustc: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

/// A map keyed by package, written with each key as `name version (source)`, in the order
/// cargo sorts packages.
mod by_id {
    use std::collections::BTreeMap;

    use blaze_resolve::PackageId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer, V: Serialize>(
        map: &BTreeMap<PackageId, V>,
        to: S,
    ) -> Result<S::Ok, S::Error> {
        to.collect_map(map.iter().map(|(id, it)| (id.serialized(), it)))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>, V: Deserialize<'de>>(
        from: D,
    ) -> Result<BTreeMap<PackageId, V>, D::Error> {
        let read = BTreeMap::<String, V>::deserialize(from)?;
        let ids = read.into_iter().map(|(id, it)| {
            PackageId::from_serialized(&id).map(|id| (id, it)).map_err(serde::de::Error::custom)
        });
        ids.collect()
    }
}

impl Tracker {
    fn load(root: &Path, quiet: bool) -> anyhow::Result<Tracker> {
        let v1 = locked(root, ".crates.toml", quiet)?;
        let v2 = locked(root, ".crates2.json", quiet)?;
        let read = |(path, mut file): (&Path, &File)| {
            let mut text = String::new();
            file.read_to_string(&mut text).with_context(|| {
                format!("failed to parse crate metadata at `{}`", path.display())
            })?;
            anyhow::Ok(text)
        };
        let text = read((&v1.0, &v1.1))?;
        let listed: ListingV1 = match text.is_empty() {
            true => ListingV1::default(),
            false => {
                toml::from_str(&text).context("invalid TOML found for metadata").with_context(
                    || format!("failed to parse crate metadata at `{}`", v1.0.display()),
                )?
            }
        };
        let text = read((&v2.0, &v2.1))?;
        let mut installs: ListingV2 = match text.is_empty() {
            true => ListingV2::default(),
            false => serde_json::from_str(&text)
                .context("invalid JSON found for metadata")
                .with_context(|| {
                    format!("failed to parse crate metadata at `{}`", v2.0.display())
                })?,
        };
        installs.sync(&listed);
        Ok(Tracker { v1: listed, v2: installs, files: [v1, v2] })
    }

    fn remove_bin(&mut self, id: PackageId, bin: &str) {
        if let Some(bins) = self.v1.v1.get_mut(&id) {
            bins.remove(bin);
            if bins.is_empty() {
                self.v1.v1.remove(&id);
            }
        }
        if let Some(info) = self.v2.installs.get_mut(&id) {
            info.bins.remove(bin);
            if info.bins.is_empty() {
                self.v2.installs.remove(&id);
            }
        }
    }

    fn save(&mut self) -> anyhow::Result<()> {
        let v1 = toml::to_string_pretty(&self.v1)?;
        let v2 = serde_json::to_string(&self.v2)?;
        for ((path, file), text) in self.files.iter_mut().zip([v1, v2]) {
            (|| {
                file.rewind()?;
                file.set_len(0)?;
                file.write_all(text.as_bytes())
            })()
            .with_context(|| format!("failed to write crate metadata at `{}`", path.display()))?;
        }
        Ok(())
    }
}

impl ListingV2 {
    /// `.crates.toml` is what is installed: an older cargo writes only it.
    fn sync(&mut self, v1: &ListingV1) {
        for (id, bins) in &v1.v1 {
            let info = self.installs.entry(*id).or_insert_with(|| InstallInfo {
                version_req: None,
                bins: BTreeSet::new(),
                features: BTreeSet::new(),
                all_features: false,
                no_default_features: false,
                profile: "release".to_owned(),
                target: None,
                rustc: None,
                other: BTreeMap::new(),
            });
            info.bins = bins.clone();
        }
        self.installs.retain(|id, _| v1.v1.contains_key(id));
    }
}

/// `name` in `root`, made when it is not there, locked for this process alone.
fn locked(root: &Path, name: &str, quiet: bool) -> anyhow::Result<(PathBuf, File)> {
    let path = root.join(name);
    std::fs::create_dir_all(root)
        .with_context(|| format!("failed to create directory `{}`", root.display()))?;
    let file = File::options().read(true).write(true).create(true).truncate(false).open(&path);
    let file = file.with_context(|| format!("failed to open: {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            super::super::say::status(quiet, "Blocking", "waiting for file lock on crate metadata");
            file.lock().with_context(|| format!("failed to lock file: {}", path.display()))?;
        }
        Err(std::fs::TryLockError::Error(error)) => {
            return Err(error).with_context(|| format!("failed to lock file: {}", path.display()));
        }
    }
    Ok((path, file))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "[v1]\n\"a 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"a\", \"a2\"]\n\"b 0.2.0 (path+file:///w/b)\" = [\"b\"]\n";

    #[test]
    fn what_is_left_is_written_as_cargo_writes_it() {
        let mut v1: ListingV1 = toml::from_str(V1).unwrap();
        let id = *v1.v1.keys().next().unwrap();
        v1.v1.get_mut(&id).unwrap().remove("a2");
        assert_eq!(
            toml::to_string_pretty(&v1).unwrap(),
            "[v1]\n\"a 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"a\"]\n\"b 0.2.0 (path+file:///w/b)\" = [\"b\"]\n"
        );
        let mut v2 = ListingV2::default();
        v2.sync(&v1);
        let json = serde_json::to_string(&v2).unwrap();
        assert!(json.starts_with("{\"installs\":{\"a 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)\":{\"version_req\":null,\"bins\":[\"a\"],\"features\":[],\"all_features\":false,\"no_default_features\":false,\"profile\":\"release\",\"target\":null,\"rustc\":null}"), "{json}");
    }

    #[test]
    fn packages_are_in_cargos_order_not_the_texts() {
        let text = "[v1]\n\"a 1.10.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"x\"]\n\"a 1.9.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"y\"]\n";
        let v1: ListingV1 = toml::from_str(text).unwrap();
        let written = toml::to_string_pretty(&v1).unwrap();
        assert!(written.find("1.9.0").unwrap() < written.find("1.10.0").unwrap(), "{written}");
    }
}
