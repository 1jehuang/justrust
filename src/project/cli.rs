//! `jr-rust check|build|test|run [cargo options]`: Cargo's command-line
//! surface over the native project driver.

use super::driver::{Command, Driver, Selection};
use super::manifest;
use std::path::PathBuf;

pub const USAGE: &str = "jr-rust (check|build|test|run) [--manifest-path PATH] [-p NAME]... [--workspace]\n  [--lib] [--bin NAME] [--bins] [--test NAME] [--tests] [--example NAME] [--all-targets]\n  [--release] [--profile NAME] [-F/--features LIST] [--all-features] [--no-default-features]\n  [--no-run] [-j N] [-- ARGS...]\n";

/// Parse `args` (after the subcommand) and run; returns the exit code.
pub fn main(command: &str, args: &[String]) -> i32 {
    match run(command, args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            101
        }
    }
}

fn run(command: &str, args: &[String]) -> Result<i32, Box<dyn std::error::Error>> {
    let command = match command {
        "check" => Command::Check,
        "build" => Command::Build,
        "test" => Command::Test,
        "run" => Command::Run,
        other => return Err(format!("unknown command `{other}`\n{USAGE}").into()),
    };
    let mut sel = Selection {
        workers: std::thread::available_parallelism().map_or(1, |n| n.get()),
        ..Selection::default()
    };
    let mut manifest_path: Option<PathBuf> = None;
    let mut i = 0;
    let value = |i: &mut usize, flag: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("{flag} requires a value"))
    };
    while i < args.len() {
        let arg = args[i].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg, None),
        };
        let take = |i: &mut usize| -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => value(i, flag),
            }
        };
        match flag {
            "--" => {
                sel.trailing = args[i + 1..].to_vec();
                break;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(0);
            }
            "--manifest-path" => manifest_path = Some(take(&mut i)?.into()),
            "-p" | "--package" => sel.packages.push(take(&mut i)?),
            "--workspace" | "--all" => sel.workspace = true,
            "--lib" => sel.lib = true,
            "--bin" => sel.bins.push(take(&mut i)?),
            "--bins" => sel.all_bins = true,
            "--test" => sel.tests.push(take(&mut i)?),
            "--tests" => {
                sel.all_tests = true;
                sel.lib = true;
                sel.all_bins = true;
            }
            "--example" => sel.examples.push(take(&mut i)?),
            "--all-targets" => sel.all_targets = true,
            "-r" | "--release" => sel.release = true,
            "--profile" => sel.profile = Some(take(&mut i)?),
            "-F" | "--features" => sel.features.extend(
                take(&mut i)?
                    .split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .map(String::from),
            ),
            "--all-features" => sel.all_features = true,
            "--no-default-features" => sel.no_default_features = true,
            "--no-run" => sel.no_run = true,
            "-j" | "--jobs" => sel.workers = take(&mut i)?.parse()?,
            "-q" | "--quiet" | "--locked" | "--offline" | "--frozen" => {}
            other if other.starts_with('-') => {
                return Err(format!("unexpected argument '{other}' found\n{USAGE}").into());
            }
            other if command == Command::Test => {
                // Test name filter, forwarded to the harness.
                sel.trailing.insert(0, other.to_string());
            }
            other => return Err(format!("unexpected argument '{other}' found").into()),
        }
        i += 1;
    }
    if sel.release && sel.profile.is_some() {
        return Err("--release and --profile cannot be used together".into());
    }
    let manifest = match manifest_path {
        Some(path) => path,
        None => manifest::find_manifest(&std::env::current_dir()?)?,
    };
    let driver = Driver::new(&manifest, sel)?;
    Ok(driver.run(command)?)
}
