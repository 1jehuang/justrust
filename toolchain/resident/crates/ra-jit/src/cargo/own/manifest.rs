//! The subcommands that read the workspace and print it: `metadata`, `locate-project` and
//! `verify-project`, from blaze-manifest's reading and blaze-metadata's resolution, as cargo
//! 1.99.0 prints them.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
use blaze_manifest::metadata::{export_no_deps, root_manifest};
use blaze_manifest::{Config, Context, Workspace};

use super::args::{Args, Read, switch, valued};

/// The status cargo ends with when it refuses its command line.
pub(super) const USAGE: i32 = 1;

/// The configuration of a `cargo <command>` run in this process, with its `--config`s.
pub(super) fn config(command: &str, read: &Read) -> anyhow::Result<Config> {
    let mut config = Config::from_process()?;
    config.start(command, &read.values("--config"))?;
    Ok(config)
}

/// The flags read, or the status of the refusal, said as cargo says it.
pub(super) fn read_or_refuse(
    args: &Args<'_>,
    flags: &[super::args::Flag],
    usage: &str,
) -> Result<Read, i32> {
    read_or_refuse_taking(args, flags, usage, 0)
}

/// [`read_or_refuse`] for a command that takes up to `positionals` words that are no flags.
pub(super) fn read_or_refuse_taking(
    args: &Args<'_>,
    flags: &[super::args::Flag],
    usage: &str,
    positionals: usize,
) -> Result<Read, i32> {
    match args.read(flags) {
        Ok(read) if read.has("--help") => {
            println!("{usage}");
            Err(0)
        }
        Ok(read) => match read.positional.get(positionals) {
            Some(word) => {
                super::super::say::error(&format!("unexpected argument '{word}' found"));
                Err(USAGE)
            }
            None => Ok(read),
        },
        Err(why) => {
            super::super::say::error(&why);
            eprintln!("\nUsage: {usage}\n\nFor more information, try '--help'.");
            Err(USAGE)
        }
    }
}

/// Ends a subcommand that failed with `err`, as cargo ends: its message, status 101.
pub(super) fn failed(err: &anyhow::Error) -> i32 {
    super::super::metadata::refused(err, &std::env::current_dir().unwrap_or_default());
    super::super::FAILED
}

/// `cargo metadata`: the workspace and, unless `--no-deps`, every package of its resolve,
/// read, fetched and resolved by blaze.
pub(super) fn metadata(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [
        valued(&["--format-version"]),
        switch(&["--no-deps"]),
        valued(&["--filter-platform"]),
        valued(&["-F", "--features"]),
        switch(&["--all-features"]),
        switch(&["--no-default-features"]),
        valued(&["--manifest-path"]),
    ];
    let usage = "cargo blaze metadata [--format-version 1] [--no-deps] [--filter-platform <TRIPLE>] \
                 [--features <FEATURES>] [--all-features] [--no-default-features] \
                 [--manifest-path <PATH>]";
    let read = match read_or_refuse(&args, &flags, usage) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    match read.value("--format-version").as_deref() {
        None => super::super::say::warning(
            "please specify `--format-version` flag explicitly to avoid compatibility problems",
        ),
        Some("1") => {}
        Some(other) => {
            super::super::say::error(&format!(
                "invalid value '{other}' for '--format-version <VERSION>'\n  [possible values: 1]"
            ));
            return Ok(USAGE);
        }
    }
    let manifest_path = read.value("--manifest-path").map(PathBuf::from);
    let printed = if read.has("--no-deps") {
        no_deps(&read, manifest_path.as_deref())
    } else {
        resolved(&read, manifest_path.as_deref())
    };
    match printed {
        Ok(json) => {
            println!("{json}");
            Ok(0)
        }
        Err(err) => Ok(failed(&err)),
    }
}

fn no_deps(read: &Read, manifest_path: Option<&Path>) -> anyhow::Result<String> {
    let ctx = Context::new(config("metadata", read)?);
    let root = root_manifest(manifest_path, &ctx)?;
    let ws = Workspace::new(&root, &ctx)?;
    Ok(serde_json::to_string(&export_no_deps(&ws))?)
}

fn resolved(read: &Read, manifest_path: Option<&Path>) -> anyhow::Result<String> {
    let cwd = std::env::current_dir().context("couldn't get the current directory")?;
    let platforms = read.values("--filter-platform");
    let cfg;
    let platform = match &platforms[..] {
        [] => None,
        [triple] => {
            cfg = target_cfg(triple)?;
            Some(blaze_metadata::TargetPlatform { triple, cfg: &cfg })
        }
        [_, _, ..] => anyhow::bail!(
            "blaze does not filter `cargo metadata` by more than one platform yet; give one \
             `--filter-platform`"
        ),
    };
    let config = read.values("--config");
    let features = read.features();
    let asked = blaze_metadata::Asked {
        cwd: &cwd,
        env: std::env::vars().collect(),
        manifest_path,
        config: &config,
        features: &features,
        all_features: read.has("--all-features"),
        no_default_features: read.has("--no-default-features"),
        build: None,
        platform,
        offline: read.has("--offline") || read.has("--frozen"),
        store: None,
        rustc_version: Some(crate::cargo::toolchain::ours_release()),
    };
    blaze_metadata::metadata(&asked)
}

/// What the toolchain's rustc would print for `args`, as blaze answers it itself, or none
/// when blaze has no answer of its own.
fn own_rustc(args: &[&str]) -> anyhow::Result<Option<String>> {
    use super::super::toolchain::Chosen;
    let (Chosen::Given(sysroot) | Chosen::Rustup(sysroot) | Chosen::Own(sysroot)) =
        super::super::toolchain::settle(false).map_err(anyhow::Error::msg)?;
    let rustc = format!("rustc{}", std::env::consts::EXE_SUFFIX);
    let mut asked = Command::new(sysroot.join("bin").join(rustc));
    asked.args(args);
    let printed = crate::answers::own_answer(&asked, &[]).map(String::from_utf8).transpose();
    printed.context("rustc's answer")
}

/// The host's triple, as blaze answers `rustc -vV` itself.
pub(in crate::cargo) fn host_triple() -> anyhow::Result<String> {
    let told = own_rustc(&["-vV"])?.context("blaze does not know the host's triple")?;
    let host = told.lines().find_map(|it| it.strip_prefix("host: "));
    Ok(host.context("rustc names no host")?.to_owned())
}

/// The `cfg` of `triple`, as blaze answers `rustc --print cfg --target <triple>` itself.
pub(in crate::cargo) fn target_cfg(triple: &str) -> anyhow::Result<Vec<cargo_platform::Cfg>> {
    let Some(printed) = own_rustc(&["--print", "cfg", "--target", triple])? else {
        anyhow::bail!("blaze does not know the target `{triple}`");
    };
    let cfg = printed.lines().filter(|it| !it.is_empty()).map(|it| {
        use std::str::FromStr as _;
        cargo_platform::Cfg::from_str(it).with_context(|| format!("the cfg {it}"))
    });
    cfg.collect()
}

/// `cargo locate-project`: the manifest of the package of the working directory (or
/// `--manifest-path`), or with `--workspace` its workspace's.
pub(super) fn locate_project(args: Args<'_>) -> anyhow::Result<i32> {
    let flags =
        [switch(&["--workspace"]), valued(&["--message-format"]), valued(&["--manifest-path"])];
    let usage = "cargo blaze locate-project [--workspace] [--message-format <FMT>] \
                 [--manifest-path <PATH>]";
    let read = match read_or_refuse(&args, &flags, usage) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let plain = match read.value("--message-format").as_deref() {
        None | Some("json") => false,
        Some("plain") => true,
        Some(other) => {
            let why = format!("invalid message format specifier: `{other}`");
            return Ok(failed(&anyhow::anyhow!(why)));
        }
    };
    let located = (|| {
        let ctx = Context::new(config("locate-project", &read)?);
        let manifest_path = read.value("--manifest-path").map(PathBuf::from);
        let root = root_manifest(manifest_path.as_deref(), &ctx)?;
        let root = match read.has("--workspace") {
            true => Workspace::new(&root, &ctx)?.root_manifest().to_owned(),
            false => root,
        };
        root.to_str()
            .map(str::to_owned)
            .context("your package path contains characters not representable in Unicode")
    })();
    match located {
        Ok(root) if plain => println!("{root}"),
        Ok(root) => println!("{}", serde_json::json!({ "root": root })),
        Err(err) => return Ok(failed(&err)),
    }
    Ok(0)
}

/// `cargo verify-project`: whether the workspace of the manifest reads, as JSON.
pub(super) fn verify_project(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [valued(&["--manifest-path"])];
    let read = match read_or_refuse(
        &args,
        &flags,
        "cargo blaze verify-project [--manifest-path <PATH>]",
    ) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let verified = (|| {
        let ctx = Context::new(config("verify-project", &read)?);
        let manifest_path = read.value("--manifest-path").map(PathBuf::from);
        let root = root_manifest(manifest_path.as_deref(), &ctx)?;
        Workspace::new(&root, &ctx).map(drop)
    })();
    match verified {
        Ok(()) => {
            println!("{}", serde_json::json!({ "success": "true" }));
            Ok(0)
        }
        Err(err) => {
            println!("{}", serde_json::json!({ "invalid": err.to_string() }));
            Ok(1)
        }
    }
}

/// `cargo read-manifest`: the package of the manifest, as `cargo metadata --no-deps` prints it.
pub(super) fn read_manifest(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [valued(&["-m", "--manifest-path"])];
    let usage = "cargo blaze read-manifest [--manifest-path <PATH>]";
    let read = match read_or_refuse(&args, &flags, usage) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let printed = (|| {
        let ctx = Context::new(config("read-manifest", &read)?);
        let manifest_path = read.value("--manifest-path").map(PathBuf::from);
        let root = root_manifest(manifest_path.as_deref(), &ctx)?;
        let ws = Workspace::new(&root, &ctx)?;
        let Some((path, manifest)) = ws.members().find(|(path, _)| *path == root) else {
            anyhow::bail!(
                "manifest path `{}` is a virtual manifest, but this command requires running against an actual package in this workspace",
                root.display()
            );
        };
        Ok(serde_json::to_string(&blaze_manifest::metadata::export_package(path, manifest))?)
    })();
    match printed {
        Ok(json) => {
            println!("{json}");
            Ok(0)
        }
        Err(err) => Ok(failed(&err)),
    }
}

/// `cargo pkgid`: the full spec of the package `SPEC` (or `-p`) names in the lock file, or of
/// the package of the working directory.
pub(super) fn pkgid(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [valued(&["-p", "--package"]), valued(&["--manifest-path"])];
    let usage = "cargo blaze pkgid [--package <SPEC>] [--manifest-path <PATH>] [SPEC]";
    let read = match read_or_refuse_taking(&args, &flags, usage, 1) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let found = (|| {
        let cwd = std::env::current_dir().context("couldn't get the current directory")?;
        let manifest_path = read.value("--manifest-path").map(PathBuf::from);
        let config = read.values("--config");
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
            offline: true,
            store: None,
            rustc_version: Some(crate::cargo::toolchain::ours_release()),
        };
        let locked = blaze_metadata::locked(&asked)?;
        let locked = locked.context("a Cargo.lock must exist for this command")?;
        let spec = read.positional.first().cloned().or_else(|| read.value("--package"));
        let id = match spec {
            Some(spec) => blaze_resolve::PackageIdSpec::query_str(&spec, &locked.ids)?,
            None => match locked.current {
                Some(id) => id,
                None => {
                    let ctx = Context::new(self::config("pkgid", &read)?);
                    let root = root_manifest(manifest_path.as_deref(), &ctx)?;
                    anyhow::bail!(
                        "manifest path `{}` is a virtual manifest, but this command requires running against an actual package in this workspace",
                        root.display()
                    );
                }
            },
        };
        anyhow::Ok(id.to_spec())
    })();
    match found {
        Ok(spec) => {
            println!("{spec}");
            Ok(0)
        }
        Err(err) => Ok(failed(&err)),
    }
}

/// `cargo fetch`: every package the lock file names, fetched where cargo keeps them, for every
/// platform (`--target` fetches no fewer).
pub(super) fn fetch(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [valued(&["--target"]), valued(&["--manifest-path"])];
    let usage = "cargo blaze fetch [--target <TRIPLE>] [--manifest-path <PATH>]";
    let read = match read_or_refuse(&args, &flags, usage) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let fetched = (|| {
        let cwd = std::env::current_dir().context("couldn't get the current directory")?;
        let manifest_path = read.value("--manifest-path").map(PathBuf::from);
        let config = read.values("--config");
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
        blaze_metadata::fetch_locked(&asked)?.context(
            "there is no Cargo.lock, and blaze does not choose the versions to write one yet (#321)",
        )
    })();
    match fetched {
        Ok(_) => Ok(0),
        Err(err) => Ok(failed(&err)),
    }
}
