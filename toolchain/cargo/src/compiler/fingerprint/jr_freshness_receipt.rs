//! Opt-in native unit-input prerequisites, never whole-command replay authority.
//!
//! Only Cargo's actual fresh decision can produce a supported unit record. The
//! command-planning, discovery/negative, config, and sysroot closures are not
//! captured here, so every receipt explicitly denies command replay.

use super::dep_info::ChecksumAlgo;
use super::{Checksum, Fingerprint, FingerprintMethod, LocalFingerprint};
use crate::compiler::{BuildContext, BuildRunner, CompileKind, CompileMode, Unit};
use filetime::FileTime;
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type Handle = Arc<Mutex<Capture>>;

pub struct Capture {
    path: PathBuf,
    nonce: Option<String>,
    reasons: Vec<String>,
    units: Vec<Value>,
}

impl Capture {
    pub fn new(bcx: &BuildContext<'_, '_>) -> Option<Handle> {
        let gctx = bcx.gctx;
        let path = gctx.get_env_os("JUSTRUST_CARGO_FRESHNESS_RECEIPT")?;
        if path.is_empty() {
            return None;
        }
        let nonce = gctx
            .get_env("JUSTRUST_CARGO_FRESHNESS_NONCE")
            .ok()
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let mut reasons = Vec::new();
        if nonce.is_none() {
            reasons.push("missing-invocation-nonce".to_owned());
        }
        if bcx.unit_graph.len() != 1 || bcx.roots.len() != 1 {
            reasons.push("unsupported-unit-closure".to_owned());
        }
        let capture = Self {
            path: gctx.cwd().join(path),
            nonce,
            reasons,
            units: Vec::new(),
        };
        capture.publish(None);
        Some(Arc::new(Mutex::new(capture)))
    }

    pub fn finish(handle: &Option<Handle>, success: bool) {
        if let Some(handle) = handle {
            if let Ok(capture) = handle.lock() {
                capture.publish(Some(success));
            }
        }
    }

    fn publish(&self, success: Option<bool>) {
        let supported = success == Some(true)
            && self.reasons.is_empty()
            && !self.units.is_empty()
            && self.units.iter().all(|unit| unit["supported"] == true);
        let receipt = json!({
            "schema": 1,
            "producer": "jr-cargo",
            "scope": "unit-freshness-input-prerequisite",
            "invocation_nonce": self.nonce,
            "native_result": match success { None => "in-progress", Some(true) => "succeeded", Some(false) => "failed" },
            "prerequisite_supported": supported,
            "unsupported_reasons": self.reasons,
            "command_replay_eligible": false,
            "trusted": false,
            "content_snapshot_authorizes_freshness": false,
            "trust": "observational-prerequisite-not-replay-authority",
            "replay_denials": [
                "compiler-read-content-to-output-binding-not-captured",
                "planning-and-resolution-closure-not-captured",
                "config-files-and-config-environment-closure-not-captured",
                "directory-discovery-and-negative-input-closure-not-captured",
                "compiler-sysroot-and-native-library-closure-not-captured",
                "remote-external-path-mapping-not-established"
            ],
            "units": self.units,
        });
        // Private tempfile, atomic replacement, no following an existing sink
        // symlink. Recording failure must never alter Cargo's build result.
        let write = || -> anyhow::Result<()> {
            let parent = self
                .path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("receipt-sink-has-no-parent"))?;
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            temporary.write_all(&serde_json::to_vec_pretty(&receipt)?)?;
            temporary.write_all(b"\n")?;
            temporary.persist(&self.path)?;
            Ok(())
        };
        if let Err(error) = write() {
            tracing::debug!("jr-freshness-receipt: publication failed: {error}");
        }
    }
}

/// Observe the native comparison result, not a second guessed source scan.
pub fn observe(
    build_runner: &BuildRunner<'_, '_>,
    unit: &Unit,
    fingerprint: &Fingerprint,
    fingerprint_path: &Path,
    fresh: bool,
) {
    let Some(handle) = &build_runner.freshness_receipt else {
        return;
    };
    let mut reasons = Vec::<String>::new();
    let mut reject = |reason: &str| reasons.push(reason.to_owned());
    if !fresh {
        reject("native-unit-not-fresh");
    }
    if !matches!(unit.mode, CompileMode::Check { test: false }) || !unit.target.is_lib() {
        reject("only-check-library-units-supported");
    }
    if unit.target.proc_macro() {
        reject("proc-macro-unsupported");
    }
    if unit
        .pkg
        .targets()
        .iter()
        .any(|target| target.is_custom_build())
    {
        reject("build-script-unsupported");
    }
    if !build_runner.unit_deps(unit).is_empty() {
        reject("dependency-closure-unsupported");
    }
    if !unit.pkg.package_id().source_id().is_path() {
        reject("non-path-source-unsupported");
    }
    if !matches!(unit.kind, CompileKind::Host) {
        reject("cross-or-custom-target-unsupported");
    }
    if !unit.rustflags.is_empty()
        || build_runner
            .bcx
            .extra_args_for(unit)
            .is_some_and(|args| !args.is_empty())
    {
        reject("extra-compiler-flags-unsupported");
    }
    let rustc = build_runner.bcx.rustc();
    if rustc.path.to_str().is_none() {
        reject("non-utf8-compiler-path-unsupported");
    }
    if rustc.wrapper.is_some() || rustc.workspace_wrapper.is_some() {
        reject("compiler-wrapper-unsupported");
    }

    let mut files = Vec::new();
    let mut environment = Vec::new();
    let mut outputs = Vec::new();
    if reasons.is_empty() {
        let collect = || -> anyhow::Result<(Vec<Value>, Vec<Value>, Vec<Value>)> {
            let mut files = Vec::new();
            let mut environment = Vec::new();
            let expected_fingerprint = crate::util::to_hex(fingerprint.hash_u64());
            if fs::read_to_string(fingerprint_path)? != expected_fingerprint {
                anyhow::bail!("native-fingerprint-changed-before-observation");
            }
            let build_root = super::build_root(build_runner);
            let locals = fingerprint
                .local
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned-local-fingerprint"))?;
            if locals.len() != 1 {
                anyhow::bail!("unsupported-local-fingerprint-closure");
            }
            let LocalFingerprint::CheckDepInfo {
                dep_info,
                fingerprint: FingerprintMethod::Mtime,
            } = &locals[0]
            else {
                anyhow::bail!("unsupported-local-fingerprint-kind");
            };
            let dep_info = build_root.join(dep_info);
            let reference_mtime = cargo_util::paths::mtime(&dep_info)?;
            let info = super::dep_info::parse_dep_info(unit.pkg.root(), &build_root, &dep_info)?
                .ok_or_else(|| anyhow::anyhow!("missing-or-invalid-native-dep-info"))?;
            if info.files.is_empty() {
                anyhow::bail!("empty-native-input-closure");
            }
            let mut source_paths: Vec<_> = info.files.into_keys().collect();
            source_paths.sort();
            for path in source_paths {
                if !path.starts_with(unit.pkg.root()) {
                    anyhow::bail!("external-source-input-unsupported");
                }
                files.push(file_observation(&path, "rustc-dep-info-input")?);
                if cargo_util::paths::mtime(&path)? > reference_mtime {
                    anyhow::bail!("source-changed-after-native-freshness-comparison");
                }
            }
            files.push(file_observation(&dep_info, "native-dep-info")?);
            files.push(file_observation(fingerprint_path, "native-fingerprint")?);
            files.push(file_observation(
                unit.pkg.manifest_path(),
                "manifest-context-not-planning-closure",
            )?);
            for (name, previous) in info.env {
                environment.push(json!({
                    "name": name,
                    "expected_present": previous.is_some(),
                    "expected_value_sha256": previous.as_ref().map(|value| Checksum::compute(ChecksumAlgo::Sha256, value.as_bytes()).map(|hash| hash.to_string())).transpose()?,
                    "authority": "native-fresh-dep-info-environment-comparison"
                }));
            }
            environment.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
            let mut output_paths = fingerprint.outputs.clone();
            output_paths.sort();
            if output_paths.is_empty() {
                anyhow::bail!("missing-native-output-closure");
            }
            let outputs = output_paths
                .iter()
                .map(|path| file_observation(path, "native-validated-output"))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let super::FsStatus::UpToDate { mtimes } = &fingerprint.fs_status else {
                anyhow::bail!("native-output-state-not-fresh");
            };
            for (path, expected) in mtimes {
                if cargo_util::paths::mtime(path)? != *expected {
                    anyhow::bail!("output-changed-after-native-freshness-comparison");
                }
            }
            if fs::read_to_string(fingerprint_path)? != expected_fingerprint {
                anyhow::bail!("native-fingerprint-changed-during-observation");
            }
            Ok((files, environment, outputs))
        };
        match collect() {
            Ok(observed) => {
                (files, environment, outputs) = observed;
            }
            Err(error) => reasons.push(error.to_string()),
        }
    }
    let record = json!({
        "package": unit.pkg.package_id().to_string(),
        "target": unit.target.name(),
        "mode": unit.mode,
        "native_fresh": fresh,
        "compiler_read_content_bound_to_outputs": false,
        "supported": reasons.is_empty(),
        "unsupported_reasons": reasons,
        "native_fingerprint": crate::util::to_hex(fingerprint.hash_u64()),
        "compiler_version_fingerprint": fingerprint.rustc,
        "config_fingerprint_context_only": fingerprint.config,
        "compiler": { "path": rustc.path.to_str(), "verbose_version": rustc.verbose_version, "filesystem_closure_complete": false },
        "files": files,
        "environment": environment,
        "outputs": outputs,
    });
    if let Ok(mut capture) = handle.lock() {
        capture.units.push(record);
    }
}

fn file_observation(path: &Path, role: &str) -> anyhow::Result<Value> {
    let path_text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-utf8-file-path-unsupported"))?;
    if fs::canonicalize(path)? != path {
        anyhow::bail!("symlink-or-noncanonical-input-unsupported");
    }
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() {
        anyhow::bail!("directory-or-special-input-unsupported");
    }
    let hash = Checksum::compute(ChecksumAlgo::Sha256, File::open(path)?)?;
    let after = fs::symlink_metadata(path)?;
    let mtime = FileTime::from_last_modification_time(&before);
    if before.len() != after.len() || mtime != FileTime::from_last_modification_time(&after) {
        anyhow::bail!("input-changed-during-observation");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (
            before.dev(),
            before.ino(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (after.dev(), after.ino(), after.ctime(), after.ctime_nsec())
        {
            anyhow::bail!("input-identity-changed-during-observation");
        }
    }
    Ok(
        json!({ "path": path_text, "role": role, "length": before.len(), "mtime_seconds": mtime.unix_seconds(), "mtime_nanoseconds": mtime.nanoseconds(), "sha256": hash.to_string() }),
    )
}
