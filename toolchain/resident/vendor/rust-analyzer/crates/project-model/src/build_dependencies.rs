//! Logic to invoke `cargo` for building build-dependencies (build scripts and proc-macros) as well as
//! executing the build scripts to fetch required dependency information (`OUT_DIR` env var, extra
//! cfg flags, etc).
//!
//! In essence this just invokes `cargo` with the appropriate output format which we consume,
//! but if enabled we will also use `RUSTC_WRAPPER` to only compile the build scripts and
//! proc-macros and skip everything else.

use std::{cell::RefCell, io, mem, process::Command};

use base_db::Env;
use cargo_metadata::{Message, PackageId, camino::Utf8Path};
use cfg::CfgAtom;
use itertools::Itertools;
use la_arena::ArenaMap;
use paths::{AbsPath, AbsPathBuf, Utf8PathBuf};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize as _;
use stdx::never;
use toolchain::Tool;
use triomphe::Arc;

use crate::{
    CargoConfig, CargoFeatures, CargoWorkspace, InvocationStrategy, ManifestPath, Package, Sysroot,
    TargetKind,
    cargo_config_file::{LockfileCopy, LockfileUsage, make_lockfile_copy},
    utf8_stdout,
};

/// Output of the build script and proc-macro building steps for a workspace.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkspaceBuildScripts {
    outputs: ArenaMap<Package, BuildScriptOutput>,
    error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProcMacroDylibPath {
    Path(AbsPathBuf),
    DylibNotFound,
    NotProcMacro,
    #[default]
    NotBuilt,
}

/// Output of the build script and proc-macro building step for a concrete package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BuildScriptOutput {
    /// List of config flags defined by this package's build script.
    pub(crate) cfgs: Vec<CfgAtom>,
    /// List of cargo-related environment variables with their value.
    ///
    /// If the package has a build script which defines environment variables,
    /// they can also be found here.
    pub(crate) envs: Env,
    /// Directory where a build script might place its output.
    pub(crate) out_dir: Option<AbsPathBuf>,
    /// Path to the proc-macro library file if this package exposes proc-macros.
    pub(crate) proc_macro_dylib_path: ProcMacroDylibPath,
}

impl BuildScriptOutput {
    fn is_empty(&self) -> bool {
        self.cfgs.is_empty()
            && self.envs.is_empty()
            && self.out_dir.is_none()
            && matches!(
                self.proc_macro_dylib_path,
                ProcMacroDylibPath::NotBuilt | ProcMacroDylibPath::NotProcMacro
            )
    }
}

impl WorkspaceBuildScripts {
    /// Runs the build scripts for the given workspace
    pub(crate) fn run_for_workspace(
        config: &CargoConfig,
        workspace: &CargoWorkspace,
        progress: &dyn Fn(String),
        sysroot: &Sysroot,
        toolchain: Option<&semver::Version>,
    ) -> io::Result<WorkspaceBuildScripts> {
        let current_dir = workspace.workspace_root();

        let allowed_features = workspace.workspace_features();
        let (_guard, cmd) = Self::build_command(
            config,
            &allowed_features,
            workspace.manifest_path(),
            workspace.target_directory().as_ref(),
            current_dir,
            sysroot,
            toolchain,
        )?;
        Self::run_per_ws(cmd, workspace, progress)
    }

    /// Runs the build scripts by invoking the configured command *once*.
    /// This populates the outputs for all passed in workspaces.
    pub(crate) fn run_once(
        config: &CargoConfig,
        workspaces: &[&CargoWorkspace],
        progress: &dyn Fn(String),
        working_directory: &AbsPathBuf,
    ) -> io::Result<Vec<WorkspaceBuildScripts>> {
        assert_eq!(config.invocation_strategy, InvocationStrategy::Once);

        let (_guard, cmd) = Self::build_command(
            config,
            &Default::default(),
            // These are not gonna be used anyways, so just construct a dummy here
            &ManifestPath::try_from(working_directory.clone()).unwrap(),
            working_directory.as_ref(),
            working_directory,
            &Sysroot::empty(),
            None,
        )?;
        // NB: Cargo.toml could have been modified between `cargo metadata` and
        // `cargo check`. We shouldn't assume that package ids we see here are
        // exactly those from `config`.
        let mut by_id = FxHashMap::default();
        // some workspaces might depend on the same crates, so we need to duplicate the outputs
        // to those collisions
        let mut collisions = Vec::new();
        let mut res: Vec<_> = workspaces
            .iter()
            .enumerate()
            .map(|(idx, workspace)| {
                let mut res = WorkspaceBuildScripts::default();
                for package in workspace.packages() {
                    res.outputs.insert(package, BuildScriptOutput::default());
                    if by_id.contains_key(&workspace[package].id) {
                        collisions.push((&workspace[package].id, idx, package));
                    } else {
                        by_id.insert(workspace[package].id.clone(), (package, idx));
                    }
                }
                res
            })
            .collect();

        let errors = Self::run_command(
            cmd,
            |package, cb| {
                if let Some(&(package, workspace)) = by_id.get(package) {
                    cb(&workspaces[workspace][package].name, &mut res[workspace].outputs[package]);
                } else {
                    tracing::error!("Received compiler message for unknown package: {}", package);
                }
            },
            progress,
        )?;
        res.iter_mut().for_each(|it| it.error.clone_from(&errors));
        collisions.into_iter().for_each(|(id, workspace, package)| {
            if let Some(&(p, w)) = by_id.get(id) {
                res[workspace].outputs[package] = res[w].outputs[p].clone();
            }
        });

        if tracing::enabled!(tracing::Level::INFO) {
            for (idx, workspace) in workspaces.iter().enumerate() {
                for package in workspace.packages() {
                    let package_build_data: &mut BuildScriptOutput = &mut res[idx].outputs[package];
                    if !package_build_data.is_empty() {
                        tracing::info!("{}: {package_build_data:?}", workspace[package].manifest,);
                    }
                }
            }
        }

        Ok(res)
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Where the build script of `package` wrote, when it ran.
    pub fn out_dir(&self, package: Package) -> Option<&AbsPath> {
        self.outputs.get(package)?.out_dir.as_deref()
    }

    /// What a load reads of `package`, for an embedder to show: its script's cfgs and
    /// variables, where it wrote, and its proc macro's library when it has one built.
    pub fn read_of(&self, package: Package) -> Option<crate::OwnBuildOutput> {
        let output = self.outputs.get(package)?;
        Some(crate::OwnBuildOutput {
            cfgs: output.cfgs.iter().map(ToString::to_string).collect(),
            envs: output.envs.iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
            out_dir: output.out_dir.clone(),
            proc_macro_dylib: match &output.proc_macro_dylib_path {
                ProcMacroDylibPath::Path(path) => Some(path.clone()),
                _ => None,
            },
        })
    }

    pub(crate) fn get_output(&self, idx: Package) -> Option<&BuildScriptOutput> {
        self.outputs.get(idx)
    }

    /// Assembles build script outputs for the rustc crates via `--print target-libdir`.
    pub(crate) fn rustc_crates(
        rustc: &CargoWorkspace,
        current_dir: &AbsPath,
        extra_env: &FxHashMap<String, Option<String>>,
        sysroot: &Sysroot,
    ) -> Self {
        let mut bs = WorkspaceBuildScripts::default();
        for p in rustc.packages() {
            bs.outputs.insert(p, BuildScriptOutput::default());
        }
        let res = (|| {
            let target_libdir = (|| {
                let mut cargo_config = sysroot.tool(Tool::Cargo, current_dir, extra_env);
                cargo_config
                    .args(["rustc", "-Z", "unstable-options", "--print", "target-libdir"])
                    .env("RUSTC_BOOTSTRAP", "1");
                if let Ok(it) = utf8_stdout(&mut cargo_config) {
                    return Ok(it);
                }
                let mut cmd = sysroot.tool(Tool::Rustc, current_dir, extra_env);
                cmd.args(["--print", "target-libdir"]);
                utf8_stdout(&mut cmd)
            })()?;

            let target_libdir = AbsPathBuf::try_from(Utf8PathBuf::from(target_libdir))
                .map_err(|_| anyhow::format_err!("target-libdir was not an absolute path"))?;
            tracing::info!("Loading rustc proc-macro paths from {target_libdir}");

            let proc_macro_dylibs: Vec<(String, AbsPathBuf)> = std::fs::read_dir(target_libdir)?
                .filter_map(|entry| {
                    let dir_entry = entry.ok()?;
                    // Use `fs::metadata` rather than `DirEntry::file_type` so that symlinks
                    // are followed; sysroots assembled out of symlinks (e.g. by nix) link
                    // the proc-macro dylibs into the target libdir.
                    if std::fs::metadata(dir_entry.path()).ok()?.is_file() {
                        let path = dir_entry.path();
                        let extension = path.extension()?;
                        if extension == std::env::consts::DLL_EXTENSION {
                            let name = path
                                .file_stem()?
                                .to_str()?
                                .split_once('-')?
                                .0
                                .trim_start_matches("lib")
                                .to_owned();
                            let path = match Utf8PathBuf::from_path_buf(path) {
                                Ok(path) => path,
                                Err(path) => {
                                    tracing::warn!(
                                        "Proc-macro dylib path contains non-UTF8 characters: {:?}",
                                        path.display()
                                    );
                                    return None;
                                }
                            };
                            return match AbsPathBuf::try_from(path) {
                                Ok(path) => Some((name, path)),
                                Err(path) => {
                                    tracing::error!(
                                        "proc-macro dylib path is not absolute: {:?}",
                                        path
                                    );
                                    None
                                }
                            };
                        }
                    }
                    None
                })
                .collect();
            for p in rustc.packages() {
                let package = &rustc[p];
                bs.outputs[p].proc_macro_dylib_path =
                    if package.targets.iter().any(|&it| {
                        matches!(rustc[it].kind, TargetKind::Lib { is_proc_macro: true })
                    }) {
                        match proc_macro_dylibs.iter().find(|(name, _)| *name == package.name) {
                            Some((_, path)) => ProcMacroDylibPath::Path(path.clone()),
                            _ => ProcMacroDylibPath::DylibNotFound,
                        }
                    } else {
                        ProcMacroDylibPath::NotProcMacro
                    }
            }

            if tracing::enabled!(tracing::Level::INFO) {
                for package in rustc.packages() {
                    let package_build_data = &bs.outputs[package];
                    if !package_build_data.is_empty() {
                        tracing::info!("{}: {package_build_data:?}", rustc[package].manifest,);
                    }
                }
            }
            Ok(())
        })();
        if let Err::<_, anyhow::Error>(e) = res {
            bs.error = Some(e.to_string());
        }
        bs
    }

    fn run_per_ws(
        cmd: Command,
        workspace: &CargoWorkspace,
        progress: &dyn Fn(String),
    ) -> io::Result<WorkspaceBuildScripts> {
        let mut res = WorkspaceBuildScripts::default();
        let outputs = &mut res.outputs;
        // NB: Cargo.toml could have been modified between `cargo metadata` and
        // `cargo check`. We shouldn't assume that package ids we see here are
        // exactly those from `config`.
        let mut by_id: FxHashMap<Arc<PackageId>, Package> = FxHashMap::default();
        for package in workspace.packages() {
            outputs.insert(package, BuildScriptOutput::default());
            by_id.insert(workspace[package].id.clone(), package);
        }

        res.error = Self::run_command(
            cmd,
            |package, cb| {
                if let Some(&package) = by_id.get(package) {
                    cb(&workspace[package].name, &mut outputs[package]);
                } else {
                    never!(
                        "Received compiler message for unknown package: {}\n {}",
                        package,
                        by_id.keys().join(", ")
                    );
                }
            },
            progress,
        )?;

        if tracing::enabled!(tracing::Level::INFO) {
            for package in workspace.packages() {
                let package_build_data = &outputs[package];
                if !package_build_data.is_empty() {
                    tracing::info!("{}: {package_build_data:?}", workspace[package].manifest,);
                }
            }
        }

        Ok(res)
    }

    /// What an embedder's own build said of the scripts and the proc macros of `workspace`
    /// (`CargoConfig::own_build_scripts`), as `cargo check --compile-time-deps` would.
    pub fn of_own(
        workspace: &CargoWorkspace,
        outputs: Vec<(Package, crate::OwnBuildOutput)>,
    ) -> anyhow::Result<WorkspaceBuildScripts> {
        let mut res = WorkspaceBuildScripts::default();
        for package in workspace.packages() {
            let is_proc_macro = workspace[package]
                .targets
                .iter()
                .any(|&it| matches!(workspace[it].kind, TargetKind::Lib { is_proc_macro: true }));
            let data = BuildScriptOutput {
                proc_macro_dylib_path: match is_proc_macro {
                    true => ProcMacroDylibPath::NotBuilt,
                    false => ProcMacroDylibPath::NotProcMacro,
                },
                ..BuildScriptOutput::default()
            };
            res.outputs.insert(package, data);
        }
        for (package, own) in outputs {
            let data = &mut res.outputs[package];
            if let Some(out_dir) = own.out_dir {
                data.cfgs = own
                    .cfgs
                    .iter()
                    .map(|it| crate::parse_cfg(it).map_err(|err| anyhow::format_err!("{err}")))
                    .collect::<anyhow::Result<_>>()?;
                data.envs.extend(own.envs);
                data.envs.insert("OUT_DIR", out_dir.as_str());
                data.out_dir = Some(out_dir);
            }
            if let Some(dylib) = own.proc_macro_dylib {
                data.proc_macro_dylib_path = ProcMacroDylibPath::Path(dylib);
            }
        }
        Ok(res)
    }

    /// What cargo said of the scripts and the proc macros of `workspace` in `lines` of its
    /// `--message-format=json`, for whoever ran cargo itself: nothing is run for it here.
    pub fn of_messages<'a>(
        workspace: &CargoWorkspace,
        lines: impl IntoIterator<Item = &'a str>,
    ) -> WorkspaceBuildScripts {
        let mut res = WorkspaceBuildScripts::default();
        let outputs = &mut res.outputs;
        let mut by_id: FxHashMap<Arc<PackageId>, Package> = FxHashMap::default();
        for package in workspace.packages() {
            outputs.insert(package, BuildScriptOutput::default());
            by_id.insert(workspace[package].id.clone(), package);
        }
        let errors = RefCell::new(String::new());
        let push_err = |err: &str| {
            let mut e = errors.borrow_mut();
            e.push_str(err);
            e.push('\n');
        };
        let mut with_output_for =
            |package: &PackageId, cb: &mut dyn FnMut(&str, &mut BuildScriptOutput)| {
                if let Some(&package) = by_id.get(package) {
                    cb(&workspace[package].name, &mut outputs[package]);
                }
            };
        for line in lines {
            Self::said(line, &mut with_output_for, &|_| {}, &push_err);
        }
        res.error = Some(errors.into_inner()).filter(|it| !it.is_empty());
        res
    }

    /// Takes in one line of what cargo printed.
    fn said(
        line: &str,
        with_output_for: &mut dyn FnMut(&PackageId, &mut dyn FnMut(&str, &mut BuildScriptOutput)),
        progress: &dyn Fn(String),
        push_err: &dyn Fn(&str),
    ) {
        // Copy-pasted from existing cargo_metadata. It seems like we
        // should be using serde_stacker here?
        let mut deserializer = serde_json::Deserializer::from_str(line);
        deserializer.disable_recursion_limit();
        let message = Message::deserialize(&mut deserializer)
            .unwrap_or_else(|_| Message::TextLine(line.to_owned()));

        match message {
            Message::BuildScriptExecuted(mut message) => {
                with_output_for(&message.package_id, &mut |name, data| {
                    // A package both the target and the host build (a dependency of a proc
                    // macro) has its script run for each, which can say different things;
                    // the code read is the target's. With `--target` cargo runs the host's in
                    // `<dir>/<profile>` and the target's in `<dir>/<triple>/<profile>`.
                    let depth = message.out_dir.components().count();
                    if data.out_dir.as_ref().is_some_and(|was| {
                        !message.out_dir.as_str().is_empty()
                            && depth < Utf8Path::new(was.as_str()).components().count()
                    }) {
                        return;
                    }
                    progress(format!("build script {name} run"));
                    let cfgs = {
                        let mut acc = Vec::new();
                        for cfg in &message.cfgs {
                            match crate::parse_cfg(cfg) {
                                Ok(it) => acc.push(it),
                                Err(err) => {
                                    push_err(&format!("invalid cfg from cargo-metadata: {err}"));
                                    return;
                                }
                            };
                        }
                        acc
                    };
                    data.envs.extend(message.env.drain(..));
                    // cargo_metadata crate returns default (empty) path for
                    // older cargos, which is not absolute, so work around that.
                    let out_dir = mem::take(&mut message.out_dir);
                    if !out_dir.as_str().is_empty() {
                        let out_dir = AbsPathBuf::assert(out_dir);
                        // inject_cargo_env(package, package_build_data);
                        data.envs.insert("OUT_DIR", out_dir.as_str());
                        data.out_dir = Some(out_dir);
                        data.cfgs = cfgs;
                    }
                });
            }
            Message::CompilerArtifact(message) => {
                with_output_for(&message.package_id, &mut |name, data| {
                    if data.proc_macro_dylib_path == ProcMacroDylibPath::NotBuilt {
                        data.proc_macro_dylib_path = ProcMacroDylibPath::NotProcMacro;
                    }
                    if !matches!(data.proc_macro_dylib_path, ProcMacroDylibPath::Path(_))
                        && message.target.kind.contains(&cargo_metadata::TargetKind::ProcMacro)
                    {
                        progress(format!("proc-macro {name} built"));
                        data.proc_macro_dylib_path =
                            match message.filenames.iter().find(|file| is_dylib(file)) {
                                Some(filename) => {
                                    let filename = AbsPath::assert(filename);
                                    ProcMacroDylibPath::Path(filename.to_owned())
                                }
                                None => ProcMacroDylibPath::DylibNotFound,
                            };
                    }
                });
            }
            Message::CompilerMessage(message) => {
                progress(format!("received compiler message for: {}", message.target.name));

                if let Some(diag) = message.message.rendered.as_deref() {
                    push_err(diag);
                }
            }
            Message::BuildFinished(_) => {}
            Message::TextLine(_) => {}
            _ => {}
        }
    }

    fn run_command(
        cmd: Command,
        // ideally this would be something like:
        // with_output_for: impl FnMut(&str, dyn FnOnce(&mut BuildScriptOutput)),
        // but owned trait objects aren't a thing
        mut with_output_for: impl FnMut(&PackageId, &mut dyn FnMut(&str, &mut BuildScriptOutput)),
        progress: &dyn Fn(String),
    ) -> io::Result<Option<String>> {
        let errors = RefCell::new(String::new());
        let push_err = |err: &str| {
            let mut e = errors.borrow_mut();
            e.push_str(err);
            e.push('\n');
        };

        tracing::info!("Running build scripts: {:?}", cmd);
        let output = stdx::process::spawn_with_streaming_output(
            cmd,
            &mut |line| Self::said(line, &mut with_output_for, progress, &push_err),
            &mut |line| {
                push_err(line);
            },
        )?;

        let errors = if !output.status.success() {
            let errors = errors.into_inner();
            Some(if errors.is_empty() { "cargo check failed".to_owned() } else { errors })
        } else {
            None
        };
        Ok(errors)
    }

    fn build_command(
        config: &CargoConfig,
        allowed_features: &FxHashSet<String>,
        manifest_path: &ManifestPath,
        target_dir: &Utf8Path,
        current_dir: &AbsPath,
        sysroot: &Sysroot,
        toolchain: Option<&semver::Version>,
    ) -> io::Result<(Option<LockfileCopy>, Command)> {
        match config.run_build_script_command.as_deref() {
            Some([program, args @ ..]) => {
                let mut cmd = toolchain::command(program, current_dir, &config.extra_env);
                cmd.args(args);
                Ok((None, cmd))
            }
            _ => {
                let mut requires_unstable_options = false;
                let mut cmd = sysroot.tool(Tool::Cargo, current_dir, &config.extra_env);

                cmd.args(["check", "--quiet", "--message-format=json"]);
                if !config.only_manifest_package {
                    cmd.arg("--workspace");
                }
                cmd.args(&config.extra_args);
                if let Some(config_path) = &config.config_path {
                    cmd.arg("--config").arg(config_path);
                }

                cmd.arg("--manifest-path");
                cmd.arg(manifest_path);

                if let Some(target_dir) = config.target_dir_config.target_dir(Some(target_dir)) {
                    cmd.arg("--target-dir");
                    cmd.arg(target_dir.as_ref());
                }

                toolchain::cargo_use_targets(toolchain, &mut cmd, config.target.as_slice());
                let mut lockfile_copy = None;
                if let Some(toolchain) = toolchain {
                    let lockfile_path =
                        <_ as AsRef<Utf8Path>>::as_ref(manifest_path).with_extension("lock");
                    lockfile_copy = make_lockfile_copy(toolchain, &lockfile_path);
                    if let Some(lockfile_copy) = &lockfile_copy {
                        requires_unstable_options = true;
                        match lockfile_copy.usage {
                            LockfileUsage::WithFlag => {
                                cmd.arg("--lockfile-path");
                                cmd.arg(lockfile_copy.path.as_str());
                            }
                            LockfileUsage::WithEnvVarUnstable => {
                                cmd.arg("-Zlockfile-path");
                                cmd.env(
                                    "CARGO_RESOLVER_LOCKFILE_PATH",
                                    lockfile_copy.path.as_os_str(),
                                );
                            }
                            LockfileUsage::WithEnvVar => {
                                cmd.env(
                                    "CARGO_RESOLVER_LOCKFILE_PATH",
                                    lockfile_copy.path.as_os_str(),
                                );
                            }
                        }
                    }
                }
                match &config.features {
                    CargoFeatures::All => {
                        cmd.arg("--all-features");
                    }
                    CargoFeatures::Selected { features, no_default_features } => {
                        if *no_default_features {
                            cmd.arg("--no-default-features");
                        }
                        if !features.is_empty() {
                            cmd.arg("--features");
                            cmd.arg(
                                features
                                    .iter()
                                    .filter(|&feat| allowed_features.contains(feat))
                                    .join(","),
                            );
                        }
                    }
                }

                if manifest_path.is_rust_manifest() {
                    requires_unstable_options = true;
                    cmd.arg("-Zscript");
                }

                cmd.arg("--keep-going");

                // If [`--compile-time-deps` flag](https://github.com/rust-lang/cargo/issues/14434) is
                // available in current toolchain's cargo, use it to build compile time deps only.
                const COMP_TIME_DEPS_MIN_TOOLCHAIN_VERSION: semver::Version = semver::Version {
                    major: 1,
                    minor: 89,
                    patch: 0,
                    pre: semver::Prerelease::EMPTY,
                    build: semver::BuildMetadata::EMPTY,
                };

                let cargo_comp_time_deps_available =
                    toolchain.is_some_and(|v| *v >= COMP_TIME_DEPS_MIN_TOOLCHAIN_VERSION);

                if cargo_comp_time_deps_available {
                    requires_unstable_options = true;
                    cmd.arg("--compile-time-deps");
                    // The binaries are not built, so this succeeds even on targets without
                    // libtest. It also runs the build scripts and builds the proc macros of
                    // dev-dependencies, which a build that has no tests in it does not.
                    if config.all_targets {
                        cmd.arg("--all-targets");
                    }
                } else {
                    // --all-targets includes tests, benches and examples in addition to the
                    // default lib and bins. This is an independent concept from the --target
                    // flag below.
                    if config.all_targets {
                        cmd.arg("--all-targets");
                    }

                    if config.wrap_rustc_in_build_scripts {
                        // Setup RUSTC_WRAPPER to point to `rust-analyzer` binary itself. We use
                        // that to compile only proc macros and build scripts during the initial
                        // `cargo check`.
                        // We don't need this if we are using `--compile-time-deps` flag.
                        let myself = std::env::current_exe()?;
                        cmd.env("RUSTC_WRAPPER", myself);
                        cmd.env("RA_RUSTC_WRAPPER", "1");
                    }
                }
                if requires_unstable_options {
                    cmd.env("__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS", "nightly");
                    cmd.arg("-Zunstable-options");
                }
                // ra-jit: what this cargo asks rustc is answered as the embedder answers it.
                crate::ready_question(&mut cmd);
                Ok((lockfile_copy, cmd))
            }
        }
    }
}

// FIXME: Find a better way to know if it is a dylib.
fn is_dylib(path: &Utf8Path) -> bool {
    match path.extension().map(|e| e.to_ascii_lowercase()) {
        None => false,
        Some(ext) => matches!(ext.as_str(), "dll" | "dylib" | "so"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executed(out_dir: &str, cfgs: &[&str]) -> String {
        serde_json::json!({
            "reason": "build-script-executed",
            "package_id": "path+file:///w/dep#0.1.0",
            "linked_libs": [],
            "linked_paths": [],
            "cfgs": cfgs,
            "env": [],
            "out_dir": out_dir,
        })
        .to_string()
    }

    /// The cfgs a package's script is read to have said after `lines`.
    fn cfgs_after(lines: &[String]) -> Vec<String> {
        let mut data = BuildScriptOutput::default();
        let mut with_output_for =
            |_: &PackageId, cb: &mut dyn FnMut(&str, &mut BuildScriptOutput)| cb("dep", &mut data);
        for line in lines {
            WorkspaceBuildScripts::said(line, &mut with_output_for, &|_| {}, &|e| panic!("{e}"));
        }
        data.cfgs.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_targets_run_of_a_script_the_host_also_runs_is_kept() {
        let host = executed("/w/target/debug/build/dep/1111/out", &["host_only"]);
        let target =
            executed("/w/target/wasm32-unknown-unknown/debug/build/dep/2222/out", &["for_target"]);
        assert_eq!(cfgs_after(&[host.clone(), target.clone()]), ["for_target"]);
        assert_eq!(cfgs_after(&[target, host]), ["for_target"]);
    }

    #[test]
    fn a_script_run_again_says_what_it_said_last() {
        let first = executed("/w/target/debug/build/dep/1111/out", &["first"]);
        let again = executed("/w/target/debug/build/dep/1111/out", &["again"]);
        assert_eq!(cfgs_after(&[first, again]), ["again"]);
    }
}
