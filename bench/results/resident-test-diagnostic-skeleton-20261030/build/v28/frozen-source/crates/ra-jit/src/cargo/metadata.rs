//! The workspace as `cargo metadata --format-version 1` prints it, read by blaze-manifest as
//! cargo 1.99.0 reads it and resolved by blaze-resolve, with no cargo run (docs/resolve.md).

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use blaze_manifest::diagnostic::ManifestDiagnostic;
use blaze_manifest::metadata::{export_no_deps, root_manifest};
use blaze_manifest::{Config, Context, Workspace};
use cargo_metadata::Metadata;

/// The workspace found from the current directory, or from `manifest_path`, with cargo's
/// `--config` arguments `config`. A workspace cargo refuses ends the command as cargo ends:
/// its message, status 101.
pub(crate) fn no_deps(manifest_path: Option<&Path>, config: &[String]) -> Metadata {
    let cwd = std::env::current_dir().unwrap_or_default();
    let config = Config::from_process().and_then(|mut process| {
        process.start("metadata", config)?;
        Ok(process)
    });
    match config.and_then(|config| read(config, manifest_path)) {
        Ok(metadata) => metadata,
        Err(err) => {
            refused(&err, &cwd);
            std::process::exit(101)
        }
    }
}

fn read(config: Config, manifest_path: Option<&Path>) -> anyhow::Result<Metadata> {
    let ctx = Context::new(config);
    let root = root_manifest(manifest_path, &ctx)?;
    let ws = Workspace::new(&root, &ctx)?;
    let json = serde_json::to_value(export_no_deps(&ws))?;
    serde_json::from_value(json).context("blaze's own reading of the workspace is not cargo's form")
}

/// How the daemon's loads read a workspace for the build `selection` names: blaze's own
/// reading and resolution, the graph that of the build, its features unified over the other
/// members only as `resolver.feature-unification` says. `RA_JIT_OWN_RESOLVE=0` leaves both to
/// cargo.
pub(crate) fn own(selection: &crate::plan::Selection) -> Option<project_model::OwnMetadata> {
    if std::env::var_os("RA_JIT_OWN_RESOLVE").is_some_and(|it| it == "0") {
        return None;
    }
    let selection = selection.clone();
    let root = |manifest: &project_model::ManifestPath| {
        let manifest: &Path = manifest.as_ref();
        let cwd = manifest.parent().unwrap_or(Path::new("/"));
        let root = blaze_metadata::workspace_root(cwd, std::env::vars().collect(), manifest)?;
        let root = paths::AbsPathBuf::assert_utf8(root);
        Ok(project_model::ManifestPath::try_from(root).expect("a manifest is a file"))
    };
    let metadata = move |manifest: &project_model::ManifestPath,
                         triple: Option<&str>,
                         cfg: &[cfg::CfgAtom],
                         env: &rustc_hash::FxHashMap<String, Option<String>>| {
        // The build's members are those cargo would pick where it was asked, as `-p`,
        // `--workspace` and the default members choose them from there.
        let loaded: &Path = manifest.as_ref();
        let (cwd, manifest) = match &selection.asked_in {
            Some(asked_in) if asked_in.is_dir() => (asked_in.clone(), None),
            Some(asked_in) => {
                (asked_in.parent().unwrap_or(Path::new("/")).to_owned(), Some(asked_in.as_path()))
            }
            None => (loaded.parent().unwrap_or(Path::new("/")).to_owned(), Some(loaded)),
        };
        let mut vars: std::collections::BTreeMap<String, String> = std::env::vars().collect();
        for (key, value) in env {
            match value {
                Some(value) => vars.insert(key.clone(), value.clone()),
                None => vars.remove(key),
            };
        }
        let cfg: Vec<cargo_platform::Cfg> = cfg.iter().filter_map(platform_cfg).collect();
        let triple = triple.context("no target triple")?;
        let asked = blaze_metadata::Asked {
            cwd: &cwd,
            env: vars.into_iter().collect(),
            manifest_path: manifest,
            config: &selection.config,
            features: &selection.features,
            all_features: selection.all_features,
            no_default_features: selection.no_default_features,
            build: Some(blaze_metadata::Build {
                packages: &selection.packages,
                workspace: selection.workspace,
                excludes: &selection.excludes,
                dev_units: selection.dev_deps,
            }),
            platform: Some(blaze_metadata::TargetPlatform { triple, cfg: &cfg }),
            offline: false,
            store: None,
            rustc_version: Some(crate::cargo::toolchain::ours_release()),
        };
        let json = blaze_metadata::metadata(&asked)?;
        serde_json::from_str(&json).context("blaze's own metadata is not cargo's form")
    };
    Some(project_model::OwnMetadata {
        root: std::sync::Arc::new(root),
        metadata: std::sync::Arc::new(metadata),
        library: std::sync::Arc::new(library),
    })
}

/// What a library's metadata is asked with: its manifest, the features, the target, whether
/// `--no-deps`, and the variables set or unset for it.
type LibraryAsked = (PathBuf, String, Option<String>, bool, Vec<(String, Option<String>)>);

/// `cargo metadata` of the standard library's workspace at `manifest` (a toolchain's
/// `library/Cargo.toml`), as rust-analyzer's `Sysroot::load_workspace` asked cargo for it, with
/// cargo told it is a nightly and the toolchain's lock file never written: read by
/// blaze-manifest, resolved by blaze-metadata, its crates.io packages those the toolchain
/// vendors (`library/vendor`, which its `.cargo/config.toml` replaces crates.io with). The
/// toolchain's source does not change, so each way of asking is answered once per process.
fn library(
    manifest: &project_model::ManifestPath,
    features: &project_model::CargoFeatures,
    triple: Option<&str>,
    no_deps: bool,
    env: &rustc_hash::FxHashMap<String, Option<String>>,
) -> anyhow::Result<Metadata> {
    static READ: std::sync::OnceLock<
        std::sync::Mutex<rustc_hash::FxHashMap<LibraryAsked, Metadata>>,
    > = std::sync::OnceLock::new();
    let manifest: &Path = manifest.as_ref();
    let mut set: Vec<(String, Option<String>)> =
        env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    set.sort();
    let key =
        (manifest.to_owned(), format!("{features:?}"), triple.map(str::to_owned), no_deps, set);
    let read = READ.get_or_init(Default::default);
    if let Some(metadata) = read.lock().unwrap_or_else(|it| it.into_inner()).get(&key) {
        return Ok(metadata.clone());
    }
    let metadata = library_read(manifest, features, triple, no_deps, env)?;
    read.lock().unwrap_or_else(|it| it.into_inner()).insert(key, metadata.clone());
    Ok(metadata)
}

fn library_read(
    manifest: &Path,
    features: &project_model::CargoFeatures,
    triple: Option<&str>,
    no_deps: bool,
    env: &rustc_hash::FxHashMap<String, Option<String>>,
) -> anyhow::Result<Metadata> {
    // cargo is run in the library's directory, whose `.cargo/config.toml` is read.
    let cwd = manifest.parent().context("a manifest with no directory")?;
    let mut vars: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    for (key, value) in env {
        match value {
            Some(value) => vars.insert(key.clone(), value.clone()),
            None => vars.remove(key),
        };
    }
    let (features, all_features, no_default_features) = match features {
        project_model::CargoFeatures::All => (Vec::new(), true, false),
        project_model::CargoFeatures::Selected { features, no_default_features } => {
            (features.clone(), false, *no_default_features)
        }
    };
    // `--filter-platform`: the target's `cfg`, as the toolchain's rustc answers it.
    let cfg = match triple {
        None => Vec::new(),
        Some(triple) => {
            let sysroot = manifest.ancestors().nth(6).context("a library not in a toolchain")?;
            let rustc = sysroot.join("bin").join(format!("rustc{}", std::env::consts::EXE_SUFFIX));
            let args = ["--print", "cfg", "--target", triple];
            let printed = super::state::rustc_says(rustc.as_os_str(), &args)
                .filter(|it| !it.is_empty())
                .with_context(|| {
                    format!("`{} {}` says nothing", rustc.display(), args.join(" "))
                })?;
            let cfg = printed.lines().filter(|it| !it.is_empty()).map(|it| {
                use std::str::FromStr;
                cargo_platform::Cfg::from_str(it).with_context(|| format!("the cfg {it}"))
            });
            cfg.collect::<anyhow::Result<Vec<_>>>()?
        }
    };
    let asked = blaze_metadata::Asked {
        cwd,
        env: vars.into_iter().collect(),
        manifest_path: Some(manifest),
        config: &[],
        features: &features,
        all_features,
        no_default_features,
        build: None,
        platform: triple.map(|triple| blaze_metadata::TargetPlatform { triple, cfg: &cfg }),
        offline: false,
        store: None,
        rustc_version: Some(crate::cargo::toolchain::ours_release()),
    };
    let json = match no_deps {
        true => blaze_metadata::metadata_no_deps(&asked)?,
        false => blaze_metadata::metadata_locked(&asked)?,
    };
    serde_json::from_str(&json).context("blaze's own metadata of the library is not cargo's form")
}

/// The programs a build of `selection` links of `targets`, asked for in `cwd` (of the package
/// of `manifest`, when there is one), planned with blaze's own resolution for the triple
/// `target` or the host's: what cargo's unit graph has.
pub(crate) fn programs(
    selection: &crate::plan::Selection,
    cwd: &Path,
    manifest: Option<&Path>,
    targets: blaze_metadata::Targets<'_>,
) -> anyhow::Result<Vec<blaze_metadata::Program>> {
    {
        let rustc = super::state::rustc_path();
        let asked = |args: &[&str]| {
            let said = super::state::rustc_says(&rustc, args).filter(|it| !it.is_empty());
            said.with_context(|| format!("`rustc {}` says nothing", args.join(" ")))
        };
        let triple = match &selection.target {
            Some(triple) => triple.clone(),
            None => {
                let told = asked(&["-vV"])?;
                let host = told.lines().find_map(|it| it.strip_prefix("host: "));
                host.context("rustc names no host")?.to_owned()
            }
        };
        let printed = asked(&["--print", "cfg", "--target", &triple])?;
        let cfg = printed.lines().filter(|it| !it.is_empty()).map(|it| {
            use std::str::FromStr;
            cargo_platform::Cfg::from_str(it).with_context(|| format!("the cfg {it}"))
        });
        let cfg = cfg.collect::<anyhow::Result<Vec<_>>>()?;
        let asked = blaze_metadata::Asked {
            cwd,
            env: std::env::vars().collect(),
            manifest_path: manifest,
            config: &selection.config,
            features: &selection.features,
            all_features: selection.all_features,
            no_default_features: selection.no_default_features,
            build: Some(blaze_metadata::Build {
                packages: &selection.packages,
                workspace: selection.workspace,
                excludes: &selection.excludes,
                dev_units: selection.dev_deps,
            }),
            platform: Some(blaze_metadata::TargetPlatform { triple: &triple, cfg: &cfg }),
            offline: false,
            store: None,
            rustc_version: Some(crate::cargo::toolchain::ours_release()),
        };
        Ok(blaze_metadata::programs(&asked, targets)?.1)
    }
}

/// The unit graph of the build `selection` names, asked for in `cwd` (of the package of
/// `manifest`, when there is one): what `cargo build --unit-graph -Zunstable-options` prints
/// for it, made by blaze-units from blaze's own resolution with no cargo run. `triple` and
/// `cfg` are the platform built for; `env` the variables of whoever asks, where they are not
/// this process's. A build blaze-units cannot make the graph of is an error naming why.
pub(crate) fn units(
    selection: &crate::plan::Selection,
    cwd: &Path,
    manifest: Option<&Path>,
    asked: &crate::plan::Asked<'_>,
) -> anyhow::Result<blaze_units::UnitGraph> {
    if let Some(flag) = selection.unstable.first() {
        anyhow::bail!("blaze's unit graph is not made for cargo's unstable flag `-Z {flag}`");
    }
    let mut vars: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    for (key, value) in asked.env {
        match value {
            Some(value) => vars.insert(key.clone(), value.clone()),
            None => vars.remove(key),
        };
    }
    let cfg: Vec<cargo_platform::Cfg> = asked.cfg.iter().filter_map(platform_cfg).collect();
    let built_for = blaze_units::Platformed { triple: asked.triple.to_owned(), cfg };
    // Build scripts and proc macros are built for the host, which is the platform built for
    // unless `--target` names another.
    let target = selection.target.as_ref().map(|_| built_for.clone());
    let host = match &target {
        None => built_for,
        Some(_) => {
            let rustc = super::state::rustc_path();
            let said = |args: &[&str]| {
                let said = super::state::rustc_says(&rustc, args).filter(|it| !it.is_empty());
                said.with_context(|| format!("`rustc {}` says nothing", args.join(" ")))
            };
            let told = said(&["-vV"])?;
            let host = told.lines().find_map(|it| it.strip_prefix("host: "));
            let host = host.context("rustc names no host")?.to_owned();
            let printed = said(&["--print", "cfg", "--target", &host])?;
            let cfg = printed.lines().filter(|it| !it.is_empty()).map(|it| {
                use std::str::FromStr;
                cargo_platform::Cfg::from_str(it).with_context(|| format!("the cfg {it}"))
            });
            blaze_units::Platformed { triple: host, cfg: cfg.collect::<anyhow::Result<_>>()? }
        }
    };
    // ra-jit asks for what `cargo build --profile <profile>` builds, with `--all-targets`
    // when the tests are built too.
    let filter = match selection.dev_deps {
        true => blaze_units::Filter::all_targets(),
        false => blaze_units::Filter::Default { required_features_filterable: true },
    };
    let intent = blaze_units::Intent::Build;
    let platformed = target.as_ref().unwrap_or(&host);
    let asked = blaze_metadata::Asked {
        cwd,
        env: vars.into_iter().collect(),
        manifest_path: manifest,
        config: &selection.config,
        features: &selection.features,
        all_features: selection.all_features,
        no_default_features: selection.no_default_features,
        build: Some(blaze_metadata::Build {
            packages: &selection.packages,
            workspace: selection.workspace,
            excludes: &selection.excludes,
            dev_units: filter.need_dev_deps(intent),
        }),
        platform: Some(blaze_metadata::TargetPlatform {
            triple: &platformed.triple,
            cfg: &platformed.cfg,
        }),
        offline: false,
        store: None,
        rustc_version: Some(crate::cargo::toolchain::ours_release()),
    };
    let units = blaze_metadata::Units {
        intent,
        filter: &filter,
        release: false,
        profile: Some(&selection.profile),
        host: &host,
        target: target.as_ref(),
    };
    blaze_metadata::units(&asked, &units).context("making the build's unit graph")
}

/// The targets the flags `targets` pick (`--lib`, `--tests`, `--bin NAME`, ...), as cargo's
/// `from_raw_arguments` reads them.
fn filter(targets: &[String]) -> blaze_units::Filter {
    let (mut lib, mut all_targets) = (false, false);
    let (mut all_bins, mut all_tests, mut all_examples, mut all_benches) =
        (false, false, false, false);
    let (mut bins, mut tests, mut examples, mut benches) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut flags = targets.iter();
    while let Some(flag) = flags.next() {
        let mut named = |to: &mut Vec<String>| to.extend(flags.next().cloned());
        match flag.as_str() {
            "--lib" => lib = true,
            "--all-targets" => all_targets = true,
            "--bins" => all_bins = true,
            "--tests" => all_tests = true,
            "--examples" => all_examples = true,
            "--benches" => all_benches = true,
            "--bin" => named(&mut bins),
            "--test" => named(&mut tests),
            "--example" => named(&mut examples),
            "--bench" => named(&mut benches),
            _ => {}
        }
    }
    blaze_units::Filter::from_flags(
        lib,
        bins,
        all_bins,
        tests,
        all_tests,
        examples,
        all_examples,
        benches,
        all_benches,
        all_targets,
    )
}

/// The unit graph of `cargo check` of `selection` with the target flags `targets`, asked for
/// in `cwd` (of the package of `manifest`, when there is one), for the triple `selection`
/// names or the host's: the targets the check picks, each checked as a test or not, and what
/// they depend on.
pub(crate) fn checked(
    selection: &crate::plan::Selection,
    targets: &[String],
    cwd: &Path,
    manifest: Option<&Path>,
) -> anyhow::Result<blaze_units::UnitGraph> {
    if let Some(flag) = selection.unstable.first() {
        anyhow::bail!("blaze's unit graph is not made for cargo's unstable flag `-Z {flag}`");
    }
    let rustc = super::state::rustc_path();
    let said = |args: &[&str]| {
        let said = super::state::rustc_says(&rustc, args).filter(|it| !it.is_empty());
        said.with_context(|| format!("`rustc {}` says nothing", args.join(" ")))
    };
    let platformed = |triple: &str| -> anyhow::Result<blaze_units::Platformed> {
        let printed = said(&["--print", "cfg", "--target", triple])?;
        let cfg = printed.lines().filter(|it| !it.is_empty()).map(|it| {
            use std::str::FromStr;
            cargo_platform::Cfg::from_str(it).with_context(|| format!("the cfg {it}"))
        });
        let cfg = cfg.collect::<anyhow::Result<_>>()?;
        Ok(blaze_units::Platformed { triple: triple.to_owned(), cfg })
    };
    let told = said(&["-vV"])?;
    let host = told.lines().find_map(|it| it.strip_prefix("host: "));
    let host = platformed(host.context("rustc names no host")?)?;
    let target = selection.target.as_deref().map(platformed).transpose()?;
    let filter = filter(targets);
    let intent = blaze_units::Intent::Check { test: false };
    let built_for = target.as_ref().unwrap_or(&host);
    let asked = blaze_metadata::Asked {
        cwd,
        env: std::env::vars().collect(),
        manifest_path: manifest,
        config: &selection.config,
        features: &selection.features,
        all_features: selection.all_features,
        no_default_features: selection.no_default_features,
        build: Some(blaze_metadata::Build {
            packages: &selection.packages,
            workspace: selection.workspace,
            excludes: &selection.excludes,
            dev_units: filter.need_dev_deps(intent),
        }),
        platform: Some(blaze_metadata::TargetPlatform {
            triple: &built_for.triple,
            cfg: &built_for.cfg,
        }),
        offline: false,
        store: None,
        rustc_version: told.lines().find_map(|it| it.strip_prefix("release: ")),
    };
    let units = blaze_metadata::Units {
        intent,
        filter: &filter,
        release: false,
        profile: Some(&selection.profile),
        host: &host,
        target: target.as_ref(),
    };
    blaze_metadata::units(&asked, &units).context("making the check's unit graph")
}

/// A `cfg` as rust-analyzer has it, as cargo's platforms read it.
fn platform_cfg(atom: &cfg::CfgAtom) -> Option<cargo_platform::Cfg> {
    use std::str::FromStr;
    match atom {
        cfg::CfgAtom::Flag(name) => cargo_platform::Cfg::from_str(name.as_str()).ok(),
        cfg::CfgAtom::KeyValue { key, value } => {
            cargo_platform::Cfg::from_str(&format!("{key}={:?}", value.as_str())).ok()
        }
    }
}

/// `err` as cargo prints it.
pub(super) fn refused(err: &anyhow::Error, cwd: &Path) {
    if let Some(diagnostic) = err.downcast_ref::<ManifestDiagnostic>() {
        eprintln!("{}", diagnostic.render(cwd));
        return;
    }
    let mut chain = err.chain();
    super::say::error(&chain.next().map(ToString::to_string).unwrap_or_default());
    for cause in chain {
        eprintln!("\nCaused by:");
        for line in cause.to_string().lines() {
            if line.is_empty() { eprintln!() } else { eprintln!("  {line}") }
        }
    }
}

// `expected.json` is what cargo printed on Linux, Linux paths put in a JSON text as they are,
// which a Windows path's `\` is not (blaze-manifest's `tests/fixtures.rs`).
#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};

    use cargo_metadata::Metadata;

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &to.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }

    /// Each workspace of blaze-manifest's fixtures that cargo reads is, as `cargo_metadata`
    /// gives it to the rest of blaze, what cargo 1.99.0 printed for it.
    #[test]
    fn fixtures_are_what_cargo_metadata_gives() {
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../blaze-manifest/tests/fixtures");
        let scratch = std::env::temp_dir().join(format!("ra-jit-metadata-{}", std::process::id()));
        let home = scratch.join("cargo-home");
        std::fs::create_dir_all(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let mut read = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let fixture = entry.unwrap().path();
            let expected = std::fs::read_to_string(fixture.join("expected.json")).unwrap();
            if expected.contains("\"error\"") {
                continue;
            }
            let root = scratch.join(fixture.file_name().unwrap());
            copy_dir(&fixture, &root);
            let root = root.canonicalize().unwrap();
            let cwd = match std::fs::read_to_string(fixture.join("fixture.json")) {
                Ok(text) => {
                    let spec: serde_json::Value = serde_json::from_str(&text).unwrap();
                    root.join(spec["cwd"].as_str().unwrap_or(""))
                }
                Err(_) => root.clone(),
            };
            let expected = expected
                .replace("$ROOT", &root.display().to_string())
                .replace("$CARGO_HOME", &home.display().to_string());
            let expected: Metadata = serde_json::from_str(&expected).unwrap();
            let env = [("CARGO_HOME".to_owned(), home.display().to_string())];
            let config = blaze_manifest::Config::load(&cwd, env, &[]).unwrap();
            let got = super::read(config, None).unwrap();
            assert_eq!(got, expected, "{}", fixture.display());
            read += 1;
        }
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(read >= 3, "{read} fixtures read");
    }

    /// The features of `shared` in the build of `packages` of #596's workspace, where only
    /// `other` asks for `shared/extra`, with `config` as its `.cargo/config.toml`; or why
    /// blaze refuses the build.
    fn shared_features(config: &str, packages: &[&str]) -> Result<Vec<String>, String> {
        let root = std::env::temp_dir().join(format!(
            "ra-jit-unification-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let package = |name: &str, deps: &str| {
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{deps}")
        };
        let files = [
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"app\", \"other\", \"shared\"]\nresolver = \"2\"\n"
                    .to_owned(),
            ),
            (".cargo/config.toml", config.to_owned()),
            ("shared/Cargo.toml", package("shared", "\n[features]\nextra = []\n")),
            (
                "shared/src/lib.rs",
                "pub fn extra() -> bool { cfg!(feature = \"extra\") }\n".to_owned(),
            ),
            (
                "app/Cargo.toml",
                package("app", "\n[dependencies]\nshared = { path = \"../shared\" }\n"),
            ),
            (
                "app/src/main.rs",
                "fn main() { println!(\"extra = {}\", shared::extra()); }\n".to_owned(),
            ),
            (
                "other/Cargo.toml",
                package(
                    "other",
                    "\n[dependencies]\nshared = { path = \"../shared\", features = [\"extra\"] }\n",
                ),
            ),
            ("other/src/lib.rs", "pub fn f() -> bool { shared::extra() }\n".to_owned()),
        ];
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let root = root.canonicalize().unwrap();
        let home = root.join("cargo-home");
        std::fs::create_dir_all(&home).unwrap();
        let packages: Vec<String> = packages.iter().map(|it| it.to_string()).collect();
        let asked = blaze_metadata::Asked {
            cwd: &root,
            env: vec![("CARGO_HOME".to_owned(), home.display().to_string())],
            manifest_path: None,
            config: &[],
            features: &[],
            all_features: false,
            no_default_features: false,
            build: Some(blaze_metadata::Build {
                packages: &packages,
                workspace: false,
                excludes: &[],
                dev_units: false,
            }),
            platform: Some(blaze_metadata::TargetPlatform {
                triple: "x86_64-unknown-linux-gnu",
                cfg: &[],
            }),
            offline: true,
            store: None,
            rustc_version: Some(crate::cargo::toolchain::ours_release()),
        };
        let made = blaze_metadata::metadata(&asked);
        let _ = std::fs::remove_dir_all(&root);
        let metadata: serde_json::Value =
            serde_json::from_str(&made.map_err(|err| format!("{err:#}"))?).unwrap();
        let nodes = metadata["resolve"]["nodes"].as_array().unwrap();
        let shared =
            nodes.iter().find(|it| it["id"].as_str().unwrap().contains("shared#")).unwrap();
        let features = shared["features"].as_array().unwrap().iter();
        Ok(features.map(|it| it.as_str().unwrap().to_owned()).collect())
    }

    /// `resolver.feature-unification`, as nightly cargo reads it (#596): `workspace` resolves
    /// the features of every member, `selected` (and the setting without
    /// `-Zfeature-unification`) of those built, `package` of each package built alone.
    #[test]
    fn feature_unification_is_cargo_s() {
        let mode = |mode: &str| {
            format!(
                "[unstable]\nfeature-unification = true\n\n[resolver]\nfeature-unification = \"{mode}\"\n"
            )
        };
        let extra = Ok(vec!["extra".to_owned()]);
        assert_eq!(shared_features(&mode("workspace"), &["app"]), extra);
        assert_eq!(shared_features(&mode("selected"), &["app"]), Ok(vec![]));
        assert_eq!(shared_features(&mode("package"), &["app"]), Ok(vec![]));
        assert_eq!(shared_features(&mode("package"), &["other"]), extra);
        assert_eq!(shared_features(&mode("selected"), &["app", "other"]), extra);
        assert_eq!(shared_features("", &["app"]), Ok(vec![]));
        let unflagged = "[resolver]\nfeature-unification = \"workspace\"\n";
        assert_eq!(shared_features(unflagged, &["app"]), Ok(vec![]));
        let refused = shared_features(&mode("package"), &["app", "other"]).unwrap_err();
        assert!(refused.contains("resolver.feature-unification = \"package\""), "{refused}");
    }
}
