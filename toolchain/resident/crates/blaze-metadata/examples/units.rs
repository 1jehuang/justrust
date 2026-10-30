//! `units build|check|test|bench --host TRIPLE --host-cfg-file FILE [--target TRIPLE
//! --target-cfg-file FILE] [--rustc-version RELEASE] [FLAGS]`: the unit graph blaze makes for
//! the cargo command of those flags, as `cargo <command> --unit-graph -Zunstable-options` prints
//! it, from the current directory. `harness/resolve.py units` compares the two. Each cfg file is
//! what `rustc --print cfg --target TRIPLE` prints, `--rustc-version` the `release` `rustc -vV`
//! prints, which the harness asks.

#![allow(clippy::print_stderr, clippy::print_stdout, reason = "a command line program")]

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context as _, bail};
use blaze_units::{Filter, Intent, Platformed};

fn platformed(triple: &str, cfg_file: Option<&Path>) -> anyhow::Result<Platformed> {
    let cfg_file = cfg_file.with_context(|| format!("no cfg file for {triple}"))?;
    let printed = std::fs::read_to_string(cfg_file)
        .with_context(|| format!("reading {}", cfg_file.display()))?;
    let cfg = printed
        .lines()
        .filter(|it| !it.is_empty())
        .map(|it| blaze_resolve::Cfg::from_str(it).with_context(|| format!("the cfg {it}")))
        .collect::<anyhow::Result<_>>()?;
    Ok(Platformed { triple: triple.to_owned(), cfg })
}

#[derive(Default)]
struct Flags {
    release: bool,
    profile: Option<String>,
    host: Option<String>,
    host_cfg_file: Option<PathBuf>,
    target: Option<String>,
    target_cfg_file: Option<PathBuf>,
    rustc_version: Option<String>,
    packages: Vec<String>,
    workspace: bool,
    excludes: Vec<String>,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    manifest_path: Option<PathBuf>,
    config: Vec<String>,
    lib: bool,
    bins: Vec<String>,
    all_bins: bool,
    tests: Vec<String>,
    all_tests: bool,
    examples: Vec<String>,
    all_examples: bool,
    benches: Vec<String>,
    all_benches: bool,
    all_targets: bool,
}

fn parse(args: &[String]) -> anyhow::Result<(String, Flags)> {
    let mut args = args.iter().cloned();
    let command = args.next().context("no command given")?;
    let mut flags = Flags::default();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_owned(), Some(value.to_owned()))
            }
            _ => (arg.clone(), None),
        };
        let mut value = || {
            inline.clone().or_else(|| args.next()).with_context(|| format!("a value for {flag}"))
        };
        match flag.as_str() {
            "--release" | "-r" => flags.release = true,
            "--profile" => flags.profile = Some(value()?),
            "--host" => flags.host = Some(value()?),
            "--host-cfg-file" => flags.host_cfg_file = Some(PathBuf::from(value()?)),
            "--target" => flags.target = Some(value()?),
            "--target-cfg-file" => flags.target_cfg_file = Some(PathBuf::from(value()?)),
            "--rustc-version" => flags.rustc_version = Some(value()?),
            "-p" | "--package" => flags.packages.push(value()?),
            "--workspace" | "--all" => flags.workspace = true,
            "--exclude" => flags.excludes.push(value()?),
            "-F" | "--features" => flags.features.push(value()?),
            "--all-features" => flags.all_features = true,
            "--no-default-features" => flags.no_default_features = true,
            "--manifest-path" => flags.manifest_path = Some(PathBuf::from(value()?)),
            "--config" => flags.config.push(value()?),
            "--lib" => flags.lib = true,
            "--bin" => flags.bins.push(value()?),
            "--bins" => flags.all_bins = true,
            "--test" => flags.tests.push(value()?),
            "--tests" => flags.all_tests = true,
            "--example" => flags.examples.push(value()?),
            "--examples" => flags.all_examples = true,
            "--bench" => flags.benches.push(value()?),
            "--benches" => flags.all_benches = true,
            "--all-targets" => flags.all_targets = true,
            "--no-run" | "--unit-graph" | "-Zunstable-options" => {}
            other => bail!("units does not read the flag {other}"),
        }
    }
    Ok((command, flags))
}

fn main() {
    // As cargo ends when it refuses a build: its message, status 101.
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(101);
    }
}

fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, flags) = parse(&args)?;
    let intent = match (command.as_str(), flags.profile.as_deref()) {
        ("build", _) => Intent::Build,
        ("check", Some("test")) => Intent::Check { test: true },
        ("check", _) => Intent::Check { test: false },
        ("test", _) => Intent::Test,
        ("bench", _) => Intent::Bench,
        (other, _) => bail!("units makes the graph of build, check, test or bench, not {other}"),
    };
    let filter = Filter::from_flags(
        flags.lib,
        flags.bins,
        flags.all_bins,
        flags.tests,
        flags.all_tests,
        flags.examples,
        flags.all_examples,
        flags.benches,
        flags.all_benches,
        flags.all_targets,
    );
    let host = flags.host.as_deref().context("no --host given")?;
    let host = platformed(host, flags.host_cfg_file.as_deref())?;
    let target = flags
        .target
        .as_deref()
        .map(|it| platformed(it, flags.target_cfg_file.as_deref()))
        .transpose()?;
    let cwd = std::env::current_dir()?;
    let asked = blaze_metadata::Asked {
        cwd: &cwd,
        env: std::env::vars().collect(),
        manifest_path: flags.manifest_path.as_deref().map(Path::new),
        config: &flags.config,
        features: &flags.features,
        all_features: flags.all_features,
        no_default_features: flags.no_default_features,
        build: Some(blaze_metadata::Build {
            packages: &flags.packages,
            workspace: flags.workspace,
            excludes: &flags.excludes,
            dev_units: filter.need_dev_deps(intent),
        }),
        platform: Some(blaze_metadata::TargetPlatform {
            triple: &target.as_ref().unwrap_or(&host).triple,
            cfg: &target.as_ref().unwrap_or(&host).cfg,
        }),
        offline: false,
        store: None,
        rustc_version: flags.rustc_version.as_deref(),
    };
    let units = blaze_metadata::Units {
        intent,
        filter: &filter,
        release: flags.release,
        profile: flags.profile.as_deref(),
        host: &host,
        target: target.as_ref(),
    };
    let graph = blaze_metadata::units(&asked, &units)?;
    println!("{}", serde_json::to_string(&graph)?);
    Ok(())
}
