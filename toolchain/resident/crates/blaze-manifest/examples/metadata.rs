//! `cargo metadata` as blaze-manifest computes it, for `harness/resolve.py` to compare with
//! cargo's: `cargo run -p blaze-manifest --example metadata -- --no-deps --manifest-path X`.
//! Takes cargo's arguments and prints cargo's JSON, warnings and errors.

#![allow(clippy::print_stderr, clippy::print_stdout, reason = "a command line program")]

use std::path::PathBuf;
use std::process::ExitCode;

use blaze_manifest::diagnostic::report;
use blaze_manifest::metadata::{export_no_deps, root_manifest};
use blaze_manifest::{Config, Context, Workspace};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            eprintln!("{}", report(&err, &cwd));
            ExitCode::from(101)
        }
    }
}

fn run() -> anyhow::Result<()> {
    let mut manifest_path = None;
    let mut no_deps = false;
    let mut format_version = None;
    let mut config_args = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next().ok_or_else(|| anyhow::anyhow!("a value is required for '{name} <VALUE>'"))
        };
        match arg.as_str() {
            "--no-deps" => no_deps = true,
            "--offline" | "--frozen" | "--locked" => {}
            "--manifest-path" => manifest_path = Some(PathBuf::from(value("--manifest-path")?)),
            "--format-version" => format_version = Some(value("--format-version")?),
            "--config" => config_args.push(value("--config")?),
            _ => {
                if let Some(v) = arg.strip_prefix("--manifest-path=") {
                    manifest_path = Some(PathBuf::from(v));
                } else if let Some(v) = arg.strip_prefix("--format-version=") {
                    format_version = Some(v.to_owned());
                } else if let Some(v) = arg.strip_prefix("--config=") {
                    config_args.push(v.to_owned());
                } else {
                    anyhow::bail!("unexpected argument '{arg}' found");
                }
            }
        }
    }
    let mut config = Config::from_process()?;
    let started = config.start("metadata", &config_args);
    for warning in config.take_warnings() {
        eprintln!("warning: {warning}");
    }
    started?;
    if !no_deps {
        anyhow::bail!("blaze-manifest resolves no dependencies yet: pass `--no-deps`");
    }
    let ctx = Context::new(config);
    let root = root_manifest(manifest_path.as_deref(), &ctx)?;
    let ws = Workspace::new(&root, &ctx)?;
    for warning in ws.workspace_warnings() {
        eprintln!("warning: {warning}");
    }
    for (path, warning) in ws.warnings() {
        eprintln!("warning: {}: {warning}", path.display());
    }
    let version = match format_version.as_deref() {
        None => {
            eprintln!(
                "warning: please specify `--format-version` flag explicitly to avoid compatibility problems"
            );
            1
        }
        Some(v) => v.parse()?,
    };
    if version != 1 {
        anyhow::bail!("metadata version {version} not supported, only 1 is currently supported");
    }
    println!("{}", serde_json::to_string(&export_no_deps(&ws))?);
    Ok(())
}
