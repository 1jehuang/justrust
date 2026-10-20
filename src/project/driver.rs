//! Cargo-compatible project driver: plans compilation units for the
//! selected packages and targets of a workspace, compiles each with the
//! native compiler (dependency crates mounted as extern crates), and runs
//! binaries and test harnesses with Cargo-like output and exit status.

use super::manifest::{self, ManifestError, Package, TargetKind, Workspace};
use crate::compiler;
use crate::compiler::source::{CfgSet, ExternCrate, SourceOptions};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Check,
    Build,
    Test,
    Run,
}

#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub packages: Vec<String>,
    pub workspace: bool,
    pub lib: bool,
    pub bins: Vec<String>,
    pub all_bins: bool,
    pub tests: Vec<String>,
    pub all_tests: bool,
    pub examples: Vec<String>,
    pub all_targets: bool,
    pub release: bool,
    pub profile: Option<String>,
    pub features: Vec<String>,
    pub all_features: bool,
    pub no_default_features: bool,
    pub no_run: bool,
    pub workers: usize,
    /// Arguments after `--` for `run`/`test`.
    pub trailing: Vec<String>,
}

/// One compilation unit: a target of a package in a mode.
#[derive(Clone, Debug)]
pub struct Unit {
    pub package: usize,
    pub kind: TargetKind,
    pub name: String,
    pub path: PathBuf,
    /// Compiled with `cfg(test)` and the native harness.
    pub test: bool,
}

pub fn err(message: impl Into<String>) -> ManifestError {
    ManifestError(message.into())
}

fn selected_packages(ws: &Workspace, sel: &Selection) -> Result<Vec<usize>, ManifestError> {
    if sel.workspace {
        return Ok(ws.members.clone());
    }
    if sel.packages.is_empty() {
        return Ok(ws.default_members.clone());
    }
    sel.packages
        .iter()
        .map(|name| {
            ws.packages
                .iter()
                .position(|p| &p.name == name)
                .ok_or_else(|| {
                    err(format!(
                        "package ID specification `{name}` did not match any packages"
                    ))
                })
        })
        .collect()
}

/// Plan the units of `command` for the selection, in Cargo's order.
pub fn plan(ws: &Workspace, command: Command, sel: &Selection) -> Result<Vec<Unit>, ManifestError> {
    let packages = selected_packages(ws, sel)?;
    let explicit = explicit_targets(sel);
    let mut units = Vec::new();
    for &index in &packages {
        let package = &ws.packages[index];
        for target in &package.targets {
            let wanted = if explicit {
                sel.all_targets
                    || match target.kind {
                        TargetKind::Lib => sel.lib,
                        TargetKind::Bin => sel.all_bins || sel.bins.contains(&target.name),
                        TargetKind::Test => sel.all_tests || sel.tests.contains(&target.name),
                        TargetKind::Example => sel.examples.contains(&target.name),
                        TargetKind::Bench => false,
                    }
            } else {
                match command {
                    Command::Check | Command::Build => {
                        matches!(target.kind, TargetKind::Lib | TargetKind::Bin)
                    }
                    Command::Test => target.test && target.kind != TargetKind::Example,
                    Command::Run => target.kind == TargetKind::Bin,
                }
            };
            if !wanted {
                continue;
            }
            let test = match command {
                Command::Test => {
                    matches!(
                        target.kind,
                        TargetKind::Lib | TargetKind::Bin | TargetKind::Test
                    ) && target.harness
                }
                _ => target.kind == TargetKind::Test,
            };
            units.push(Unit {
                package: index,
                kind: target.kind,
                name: target.name.clone(),
                path: target.path.clone(),
                test,
            });
        }
    }
    if command == Command::Run {
        let bins: Vec<_> = units
            .iter()
            .filter(|u| u.kind != TargetKind::Lib)
            .cloned()
            .collect();
        if bins.is_empty() {
            return Err(err("a bin target must be available for `cargo run`"));
        }
        if bins.len() > 1 {
            let names: Vec<_> = bins.iter().map(|u| u.name.as_str()).collect();
            return Err(err(format!(
                "`cargo run` could not determine which binary to run. Use the `--bin` option to specify a binary, or the `default-run` manifest key.\navailable binaries: {}",
                names.join(", ")
            )));
        }
        return Ok(bins);
    }
    // Cargo orders units by package name, then lib, bins, tests, examples.
    units.sort_by_key(|u| {
        (
            ws.packages[u.package].name.clone(),
            match u.kind {
                TargetKind::Lib => 0,
                TargetKind::Bin => 1,
                TargetKind::Test => 2,
                _ => 3,
            },
        )
    });
    Ok(units)
}

fn cfg_for(package: &Package, features: &BTreeSet<String>) -> Result<CfgSet, ManifestError> {
    if package.edition != "2024" {
        return Err(err(format!(
            "`{}`: Rust edition {} is unsupported; the native compiler currently supports only edition 2024",
            package.root.join("Cargo.toml").display(),
            package.edition
        )));
    }
    let mut cfg = CfgSet::native_x86_64_linux();
    for f in features {
        cfg.insert_user(&format!("feature={f:?}")).map_err(err)?;
    }
    Ok(cfg)
}

/// Every crate a unit needs mounted: its own package's lib (for bins and
/// integration tests) and the transitive closure of library dependencies.
fn externs_for(
    ws: &Workspace,
    unit: &Unit,
    resolved: &manifest::Features,
) -> Result<Vec<ExternCrate>, ManifestError> {
    let mut out: Vec<ExternCrate> = Vec::new();
    let mut unresolved = Vec::new();
    let mut seen = BTreeSet::new();
    let dev = unit.test || matches!(unit.kind, TargetKind::Test | TargetKind::Example);
    let mut stack: Vec<(String, usize)> = manifest::visible_dependencies(
        ws,
        unit.package,
        dev,
        &resolved.active_deps[unit.package],
        &mut unresolved,
    );
    if unit.kind != TargetKind::Lib
        && let Some(lib) = ws.packages[unit.package].lib()
    {
        stack.push((lib.name.clone(), unit.package));
    }
    stack.reverse();
    while let Some((name, index)) = stack.pop() {
        let package = &ws.packages[index];
        let Some(lib) = package.lib() else {
            return Err(err(format!(
                "dependency `{}` has no library target",
                package.name
            )));
        };
        if !seen.insert(index) {
            continue;
        }
        if let Some(existing) = out.iter().find(|e| e.name == name) {
            return Err(err(format!(
                "two dependency crates named `{name}` ({} and {}) are unsupported",
                existing.root.display(),
                lib.path.display()
            )));
        }
        out.push(ExternCrate {
            name,
            root: lib.path.clone(),
            cfg: cfg_for(package, &resolved.enabled[index])?,
        });
        for dep in manifest::visible_dependencies(
            ws,
            index,
            false,
            &resolved.active_deps[index],
            &mut unresolved,
        ) {
            stack.push(dep);
        }
    }
    if let Some(first) = unresolved.into_iter().next() {
        return Err(err(first));
    }
    Ok(out)
}

pub struct Driver {
    pub ws: Workspace,
    pub sel: Selection,
    pub target_dir: PathBuf,
}

fn kind_label(unit: &Unit) -> &'static str {
    match unit.kind {
        TargetKind::Lib => "lib",
        TargetKind::Bin => "bin",
        TargetKind::Test => "test",
        TargetKind::Example => "example",
        TargetKind::Bench => "bench",
    }
}

impl Driver {
    pub fn new(manifest_path: &Path, sel: Selection) -> Result<Self, ManifestError> {
        let ws = manifest::load(manifest_path)?;
        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| ws.root.join("target"))
            .join("jr");
        Ok(Self {
            ws,
            sel,
            target_dir,
        })
    }

    fn profile_name(&self, command: Command) -> String {
        self.sel.profile.clone().unwrap_or_else(|| {
            if self.sel.release {
                "release".into()
            } else if command == Command::Test {
                "test".into()
            } else {
                "dev".into()
            }
        })
    }

    /// Run `command`; returns the process exit code.
    pub fn run(&self, command: Command) -> Result<i32, ManifestError> {
        let profile_name = self.profile_name(command);
        let profile = manifest::resolve_profile(&self.ws.profiles, &profile_name)?;
        let units = plan(&self.ws, command, &self.sel)?;
        let packages = selected_packages(&self.ws, &self.sel)?;
        // Doctest discovery and execution are not implemented. A source scan
        // cannot prove there are zero tests, and test=false only disables the
        // unit harness. Reject the requested work before publishing success.
        if command == Command::Test && !self.sel.no_run && !explicit_targets(&self.sel) {
            for &index in &packages {
                let package = &self.ws.packages[index];
                if let Some(lib) = package.lib().filter(|lib| lib.doctest) {
                    return Err(err(format!(
                        "Unsupported: doc-tests for library `{}` cannot be executed by the native driver; use --lib or --tests to select only supported test targets",
                        lib.name
                    )));
                }
            }
        }
        let roots: Vec<_> = packages
            .iter()
            .map(|&index| {
                let features = if self.sel.all_features {
                    self.ws.packages[index].features.keys().cloned().collect()
                } else {
                    self.sel.features.clone()
                };
                (index, features, !self.sel.no_default_features)
            })
            .collect();
        let resolved = manifest::resolve_features(&self.ws, &roots, command == Command::Test)?;
        let start = Instant::now();
        let profile_dir = match profile_name.as_str() {
            "dev" | "test" => "debug".to_string(),
            "bench" => "release".to_string(),
            _ => profile_name.clone(),
        };
        let out_dir = self.target_dir.join(&profile_dir);
        std::fs::create_dir_all(&out_dir)
            .map_err(|e| err(format!("{}: {e}", out_dir.display())))?;
        let check = command == Command::Check;
        let mut artifacts = Vec::new();
        let mut announced = BTreeSet::new();
        for unit in &units {
            let package = &self.ws.packages[unit.package];
            if announced.insert(unit.package) {
                eprintln!(
                    "{:>12} {} v{} ({})",
                    if check { "Checking" } else { "Compiling" },
                    package.name,
                    package.version,
                    package.root.display()
                );
            }
            let mut options = SourceOptions {
                cfg: cfg_for(package, &resolved.enabled[unit.package])?,
                externs: externs_for(&self.ws, unit, &resolved)?,
                library: unit.kind == TargetKind::Lib,
                overflow_checks: profile.overflow_checks.unwrap_or(true),
                ..SourceOptions::default()
            };
            options
                .cfg
                .set_debug_assertions(profile.debug_assertions.unwrap_or(true));
            if unit.test {
                options.cfg.names.insert("test".into());
                options.test_filter = test_filter(&self.sel.trailing)?;
            }
            let compiled = compiler::compile_file_observed_with_options(
                unit.path.clone(),
                self.sel.workers.max(1),
                check,
                options,
            );
            let compiled = match compiled {
                Ok(compiled) => compiled,
                Err(failure) => {
                    eprintln!("error: {}: {failure}", unit.path.display());
                    eprintln!(
                        "error: could not compile `{}` ({} \"{}\"{}) due to 1 previous error",
                        package.name,
                        kind_label(unit),
                        unit.name,
                        if unit.test { " test" } else { "" }
                    );
                    return Ok(101);
                }
            };
            if let Some(bytes) = compiled.executable {
                let file = out_dir.join(format!(
                    "{}{}",
                    unit.name,
                    match (unit.test, unit.kind) {
                        (true, TargetKind::Lib) => "-lib-test",
                        (true, TargetKind::Bin) => "-bin-test",
                        (_, TargetKind::Test) => "-test",
                        (_, TargetKind::Example) => "-example",
                        (_, TargetKind::Lib) => "-lib",
                        _ => "",
                    }
                ));
                write_executable(&file, &bytes)?;
                artifacts.push((unit.clone(), file));
            }
        }
        let profile_label = format!(
            "`{profile_name}` profile [native unoptimized, overflow checks {}]",
            if profile.overflow_checks.unwrap_or(true) {
                "on"
            } else {
                "off"
            }
        );
        eprintln!(
            "{:>12} {profile_label} target(s) in {:.2}s",
            "Finished",
            start.elapsed().as_secs_f64()
        );
        match command {
            Command::Check | Command::Build => Ok(0),
            Command::Run => {
                let (_, file) = artifacts
                    .into_iter()
                    .find(|(u, _)| u.kind == TargetKind::Bin || u.kind == TargetKind::Example)
                    .ok_or_else(|| err("no executable was produced"))?;
                eprintln!("{:>12} `{}`", "Running", file.display());
                let status = std::process::Command::new(&file)
                    .args(&self.sel.trailing)
                    .status()
                    .map_err(|e| err(format!("{}: {e}", file.display())))?;
                Ok(exit_code(status))
            }
            Command::Test => {
                if self.sel.no_run {
                    for (unit, file) in &artifacts {
                        eprintln!(
                            "  Executable {} ({})",
                            describe(unit, &self.ws.packages[unit.package]),
                            file.display()
                        );
                    }
                    return Ok(0);
                }
                for (unit, file) in &artifacts {
                    if unit.kind == TargetKind::Lib && !unit.test {
                        continue;
                    }
                    eprintln!(
                        "{:>12} {} ({})",
                        "Running",
                        describe(unit, &self.ws.packages[unit.package]),
                        file.display()
                    );
                    let _ = std::io::stdout().flush();
                    let status = std::process::Command::new(file)
                        .status()
                        .map_err(|e| err(format!("{}: {e}", file.display())))?;
                    if !status.success() {
                        let flag = match unit.kind {
                            TargetKind::Lib => "--lib".to_string(),
                            TargetKind::Bin => format!("--bin {}", unit.name),
                            TargetKind::Test => format!("--test {}", unit.name),
                            _ => format!("--example {}", unit.name),
                        };
                        let package = &self.ws.packages[unit.package];
                        eprintln!(
                            "error: test failed, to rerun pass `{}{flag}`",
                            if self.ws.members.len() > 1 {
                                format!("-p {} ", package.name)
                            } else {
                                String::new()
                            }
                        );
                        return Ok(101);
                    }
                }
                Ok(0)
            }
        }
    }
}

/// Libtest arguments, applied when the harness is synthesized.
fn test_filter(args: &[String]) -> Result<compiler::source::TestFilter, ManifestError> {
    let mut filter = compiler::source::TestFilter::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--exact" => filter.exact = true,
            "--ignored" => filter.ignored = true,
            "--include-ignored" => filter.include_ignored = true,
            "--skip" => {
                i += 1;
                filter.skip.push(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| err("--skip requires a value"))?,
                );
            }
            "--nocapture" | "--show-output" | "-q" | "--quiet" => {
                return Err(err(format!(
                    "test harness option `{}` is unsupported by the native harness",
                    args[i]
                )));
            }
            "--test-threads" => i += 1,
            arg if arg.starts_with("--test-threads=") => {}
            arg if arg.starts_with('-') => {
                return Err(err(format!(
                    "test harness option `{arg}` is unsupported by the native harness"
                )));
            }
            name => filter.filters.push(name.to_string()),
        }
        i += 1;
    }
    Ok(filter)
}

fn explicit_targets(sel: &Selection) -> bool {
    sel.lib
        || sel.all_bins
        || !sel.bins.is_empty()
        || sel.all_tests
        || !sel.tests.is_empty()
        || !sel.examples.is_empty()
        || sel.all_targets
}

fn describe(unit: &Unit, package: &Package) -> String {
    let relative = unit
        .path
        .strip_prefix(&package.root)
        .unwrap_or(&unit.path)
        .display()
        .to_string();
    match unit.kind {
        TargetKind::Lib | TargetKind::Bin => format!("unittests {relative}"),
        _ => relative,
    }
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), ManifestError> {
    use std::os::unix::fs::PermissionsExt;
    // Replace atomically so a running copy is never truncated.
    let tmp = path.with_extension("jr-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| err(format!("{}: {e}", tmp.display())))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| err(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| err(format!("{}: {e}", path.display())))
}
