//! Client side of remote builds: `justrust remote check|test|...` and the
//! automatic routing in `justrust check|test|...`.
//!
//! 1. Find every source root the build reads: the workspace plus each path
//!    dependency outside it (for Jcode Desktop, the sibling `~/jcode`), each
//!    widened to its git repository root. Cached per lockfile.
//! 2. Hand the run to the sync daemon (`remote_daemon`). It keeps those
//!    roots mirrored at the *same absolute paths* on the machine, pushing
//!    edits as they are saved, so by the time a build asks there is usually
//!    nothing left to send.
//! 3. The machine runs `justrust <sub> <args>` and its compact output
//!    streams back. The run is recorded locally too (`justrust runs`,
//!    `justrust show`), marked as remote, so routing can learn from it.

use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;
use std::time::Instant;

use crate::remote_backend::{self, Target};
use crate::remote_proto::{Msg, read_frame, write_frame};

// Forward only public build controls, never credentials or machine-local paths.
// Opt-ins and opt-outs must mean the same thing on hosted as they do locally.
const FORWARDED_ENV: &[&str] = &[
    "JUSTRUST_RELEASE_BUILD",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_INCREMENTAL",
    "CARGO_BUILD_JOBS",
    "RUST_TEST_THREADS",
    "JCODE_FRONTEND_THREADS",
    "JCODE_PARALLEL_FRONTEND",
    "JUSTRUST_FRONTEND_THREADS",
    "JUSTRUST_TEST_PARALLEL",
    "JUSTRUST_TEST_JOBS",
    "JUSTRUST_TEST_SPLIT",
    "CARGO_PROFILE_RELEASE_OPT_LEVEL",
    "CARGO_PROFILE_RELEASE_DEBUG",
    "CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO",
    "CARGO_PROFILE_RELEASE_STRIP",
    "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS",
    "CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS",
    "CARGO_PROFILE_RELEASE_LTO",
    "CARGO_PROFILE_RELEASE_PANIC",
    "CARGO_PROFILE_RELEASE_INCREMENTAL",
    "CARGO_PROFILE_RELEASE_CODEGEN_UNITS",
    "CARGO_PROFILE_RELEASE_RPATH",
    "RUSTUP_TOOLCHAIN",
    "JCODE_DESKTOP_VERSION",
    "JCODE_DESKTOP_BUILD_METADATA",
    "SOURCE_DATE_EPOCH",
    "JCODE_RELEASE_BUILD",
    "JCODE_BUILD_METADATA_FILE",
    "JCODE_BUILD_SEMVER",
    "JCODE_BUILD_GIT_HASH",
    "JCODE_BUILD_GIT_DATE",
    "JCODE_BUILD_GIT_TAG",
    "JCODE_BUILD_GIT_DIRTY",
    "JCODE_BUILD_CHANGELOG_RAW",
    "JUSTRUST_MAX_WARNINGS",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "JCODE_SESSION_ID",
    "CARGO_TERM_COLOR",
    "JUSTRUST_DEPCACHE_EXPLAIN",
    "JUSTRUST_PASSES",
    "JUSTRUST_RECORD_PHASE_TRACE",
    // Portable slot policy must reach the machine that owns Cargo's locks.
    // Never forward target-directory paths from the caller's filesystem.
    "JUSTRUST_SLOTS",
    "JUSTRUST_SLOT_REFRESH",
    "JUSTRUST_SLOTS_GC",
    "JUSTRUST_SLOTS_MAX_AGE_DAYS",
    "JUSTRUST_SLOTS_MAX_GB",
    "JUSTRUST_JOIN",
    "JUSTRUST_JOIN_WAIT",
    "JUSTRUST_SLOT_WARM_REUSE",
    // The agent that asked owns the slot on the machine too, so a prebuild
    // (which drops JCODE_SESSION_ID) warms the target dir its request uses.
    "JUSTRUST_SLOT_SESSION",
    "JUSTRUST_REMOTE_EVICT_INTERVAL_SECS",
    "JUSTRUST_DEPCACHE",
    // A name, never a path: the machine resolves it under its own home.
    "JUSTRUST_DEPCACHE_NAMESPACE",
    "JUSTRUST_LOCALCACHE",
    "JUSTRUST_LOCALCACHE_TARGET_BOUND",
    "JUSTRUST_PEER_CACHE",
    "JUSTRUST_TOOLCHAIN",
    "JUSTRUST_TOOLCHAIN_REQUIRED",
    "JUSTRUST_TOOLCHAIN_DIST",
    "JUSTRUST_TOOLCHAIN_AUTO_INSTALL",
    "JUSTRUST_PREBUILT",
    "JUSTRUST_SHADOW",
    "JUSTRUST_CACHE_PROC_MACROS",
    "JUSTRUST_CUTOFF",
    "JUSTRUST_CARGO_CUTOFF",
    "JUSTRUST_CARGO_SUBTREE_CACHE",
    "JUSTRUST_CARGO_FORK",
    "JUSTRUST_CARGO_DAEMON",
    "JUSTRUST_CARGO_DAEMON_TRACE",
    "JUSTRUST_UNIFY_FEATURES",
    "JUSTRUST_EARLY",
    "JUSTRUST_EARLY_EXPERIMENTAL",
    "JUSTRUST_CANONICAL_WS",
];

/// Keep the allowlist explicit. In particular, a wrapper executable on the
/// caller's machine is not a portable hosted build control. An empty Cargo
/// config override is portable: it disables a configured wrapper without
/// overriding justrust's own RUSTC_WRAPPER instrumentation.
pub(crate) fn forwarded_env(
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let mut env = Vec::new();
    for &name in FORWARDED_ENV {
        if let Some(value) = lookup(name) {
            env.push((name.to_owned(), value));
        }
    }
    if let Some(id) = lookup("JUSTRUST_DARWIN_SDK")
        && id.len() == 64
        && id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        env.push(("JUSTRUST_DARWIN_SDK".into(), id));
    }
    for name in [
        "JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE",
        "JUSTRUST_INSTALL_LLVM_TOOLS",
    ] {
        if lookup(name).as_deref() == Some("1") {
            env.push((name.into(), "1".into()));
        }
    }
    if lookup("CARGO_BUILD_RUSTC_WRAPPER").as_deref() == Some("") {
        env.push(("CARGO_BUILD_RUSTC_WRAPPER".into(), String::new()));
    }
    env
}

#[cfg(test)]
mod platform_env_tests {
    use super::forwarded_env;

    #[test]
    fn llvm_provisioning_opt_in_is_not_sdk_license_consent() {
        for value in ["yes", "true", "0", "1; unsafe", ""] {
            assert!(
                forwarded_env(|name| (name == "JUSTRUST_INSTALL_LLVM_TOOLS").then(|| value.into()))
                    .is_empty()
            );
        }
        assert_eq!(
            forwarded_env(|name| (name == "JUSTRUST_INSTALL_LLVM_TOOLS").then(|| "1".into())),
            vec![("JUSTRUST_INSTALL_LLVM_TOOLS".into(), "1".into())]
        );
    }

    #[test]
    fn sdk_selection_never_forwards_paths_or_signing_keys() {
        for value in ["/private/sdk", "../sdk", "secret", ""] {
            assert!(
                forwarded_env(|name| (name == "JUSTRUST_DARWIN_SDK").then(|| value.into()))
                    .is_empty()
            );
        }
        let id = "a".repeat(64);
        assert_eq!(
            forwarded_env(|name| (name == "JUSTRUST_DARWIN_SDK").then(|| id.clone())),
            vec![("JUSTRUST_DARWIN_SDK".into(), id)]
        );
        assert!(
            forwarded_env(|name| match name {
                "APPLE_SIGNING_KEY" | "WINDOWS_CERTIFICATE_PASSWORD" | "SDKROOT" =>
                    Some("secret".into()),
                _ => None,
            })
            .is_empty()
        );
        assert!(
            forwarded_env(
                |name| (name == "JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE").then(|| "yes".into())
            )
            .is_empty()
        );
        assert_eq!(
            forwarded_env(
                |name| (name == "JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE").then(|| "1".into())
            ),
            vec![("JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE".into(), "1".into())]
        );
    }
}

/// Every directory the machine must mirror for a build started in `cwd`.
pub fn source_roots(cwd: &Path) -> Result<Vec<PathBuf>> {
    let cache_key = roots_cache_key(cwd);
    let cache = crate::remote::dir()?.join("roots.json");
    if let Some(key) = &cache_key
        && let Ok(b) = std::fs::read(&cache)
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b)
        && let Some(list) = v.get(key).and_then(|l| l.as_array())
    {
        let roots: Vec<PathBuf> = list
            .iter()
            .filter_map(|p| p.as_str().map(PathBuf::from))
            .collect();
        if !roots.is_empty() && roots.iter().all(|r| r.is_dir()) && mirrors(&roots, cwd) {
            return Ok(roots);
        }
    }
    let metadata = |offline: bool| {
        let mut c = Command::new(crate::paths::real_cargo().unwrap_or_else(|_| "cargo".into()));
        c.args(["metadata", "--format-version", "1"]);
        if offline {
            c.arg("--offline");
        }
        c.current_dir(cwd)
            .stderr(Stdio::piped())
            .output()
            .context("cargo metadata")
    };
    let mut out = match metadata(true) {
        Ok(out) => out,
        // A source-only machine (an agent sandbox with no Rust toolchain)
        // still builds remotely: follow `path = "..."` in the manifests.
        Err(e) if cargo_missing(&e) => return Ok(manifest_roots(cwd)),
        Err(e) => return Err(e),
    };
    if !out.status.success() {
        out = metadata(false)?;
    }
    if !out.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let m: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let mut dirs = BTreeSet::new();
    if let Some(ws) = m["workspace_root"].as_str() {
        let ws = Path::new(ws);
        dirs.insert(repo_root(ws));
        dirs.extend(excluded_root(ws));
    }
    let mut packages = Vec::new();
    for p in m["packages"].as_array().into_iter().flatten() {
        if p["source"].is_null()
            && let Some(manifest) = p["manifest_path"].as_str()
            && let Some(d) = Path::new(manifest).parent()
        {
            dirs.extend(roots_of(d));
            packages.push(d.to_path_buf());
        }
    }
    // Files the sources reach by path (`#[path]`, `include!`) from outside
    // every root, and those an earlier failed build named as missing.
    let named = packages
        .iter()
        .flat_map(|d| crate::remote_sync::escaping_source_dirs(d))
        .chain(learned_dirs(&cache, cwd));
    let package_roots = outermost(dirs.clone());
    for dir in named {
        dirs.extend(outside_root(&package_roots, &dir));
    }
    let roots = outermost(dirs);
    if let Some(key) = cache_key {
        let mut v: serde_json::Value = std::fs::read(&cache)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        // Entries no build can ask for again: other key schemes, and this
        // workspace under lockfiles that have changed since.
        if let (Some(entries), Some((workspace, _))) = (v.as_object_mut(), key.split_once('|')) {
            entries.retain(|k, _| {
                k.starts_with("v4:") && k.split_once('|').is_none_or(|(w, _)| w != workspace)
            });
        }
        v[key] = serde_json::json!(roots);
        let _ = std::fs::write(&cache, v.to_string());
    }
    Ok(roots)
}

/// A root for `dir`, which holds a source file named by path, when no root
/// in `roots` mirrors it. None for an ancestor of a root: one stray
/// `../../x.rs` must not mirror a home directory. That build still fails on
/// the machine, naming the file.
fn outside_root(roots: &[PathBuf], dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() || mirrors(roots, dir) || roots.iter().any(|r| r.starts_with(dir)) {
        return None;
    }
    Some(excluded_root(dir).unwrap_or_else(|| dir.to_path_buf()))
}

fn learned_key(cwd: &Path) -> String {
    format!("v3:learned|{}", workspace_dir(cwd).display())
}

fn learned_dirs(cache: &Path, cwd: &Path) -> Vec<PathBuf> {
    std::fs::read(cache)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get(learned_key(cwd))?.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(PathBuf::from))
        .collect()
}

/// Files a failed build on the machine could not find or read that exist
/// here outside every mirrored root: the paths in rustc's "couldn't find
/// file" and "couldn't read" errors, which are relative to the workspace.
fn unmirrored_missing(output: &str, cwd: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for marker in ["couldn't find file `", "couldn't read `"] {
        for rest in output.split(marker).skip(1) {
            let Some(named) = rest.split('`').next() else {
                continue;
            };
            let found = [workspace_dir(cwd), cwd.to_path_buf()]
                .iter()
                .map(|base| crate::remote_sync::lexical_normalize(&base.join(named)))
                .find(|p| p.is_file());
            if let Some(file) = found
                && file.parent().is_some_and(|d| !mirrors(roots, d))
                && !out.contains(&file)
            {
                out.push(file);
            }
        }
    }
    out
}

/// After a failed remote build: remember the directories of files it named
/// as missing that exist here, so the next build mirrors them. Returns the
/// files that will now be mirrored.
fn learn_missing(output: &str, cwd: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = unmirrored_missing(output, cwd, roots);
    files.retain(|f| f.parent().is_some_and(|d| outside_root(roots, d).is_some()));
    if files.is_empty() {
        return files;
    }
    let Ok(cache) = crate::remote::dir().map(|d| d.join("roots.json")) else {
        return Vec::new();
    };
    let mut v: serde_json::Value = std::fs::read(&cache)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let mut dirs: BTreeSet<PathBuf> = learned_dirs(&cache, cwd).into_iter().collect();
    dirs.extend(
        files
            .iter()
            .filter_map(|f| f.parent().map(Path::to_path_buf)),
    );
    v[learned_key(cwd)] = serde_json::json!(dirs);
    // The cached roots predate this: compute them again on the next build.
    if let (Some(entries), Some(key)) = (v.as_object_mut(), roots_cache_key(cwd)) {
        entries.remove(&key);
    }
    if std::fs::write(&cache, v.to_string()).is_err() {
        return Vec::new();
    }
    files
}

fn cargo_missing(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
    })
}

/// `source_roots` without cargo: the manifest walk the build daemon uses,
/// widened to repository or workspace roots like the cargo metadata path.
/// Not cached, so installing cargo later takes effect at once. Like the
/// daemon's walk it does not read member manifests, so a path dependency
/// that only a member names, outside the repository, is not mirrored.
fn manifest_roots(cwd: &Path) -> Vec<PathBuf> {
    let dirs = crate::buildd::source_roots(cwd)
        .into_iter()
        .flat_map(|d| roots_of(&d))
        .collect();
    outermost(dirs)
}

/// The roots to mirror so that the machine has the local package in `d`.
fn roots_of(d: &Path) -> Vec<PathBuf> {
    let root = repo_root(d);
    // Outside git (a tarball, a benchmark snapshot) the crate dir alone
    // misses its workspace's root manifest and lockfile, which build scripts
    // and cargo read: mirror the whole workspace.
    if !root.join(".git").exists() {
        return vec![workspace_dir(d)];
    }
    let mut roots = vec![root];
    roots.extend(excluded_root(d));
    roots
}

/// A root of its own for `d` when its repository's `.justrustignore` keeps it
/// out of the repository's mirror.
///
/// The ignore file names trees a build of the repository does not read
/// (here: `/toolchain/`, 95k files). A build started inside one does read
/// it, and without a root of its own the machine compiled whatever an older
/// mirror had left there: `justrust check` in `toolchain/cargo` reported
/// success on three-day-old sources, with "0 files" to sync.
fn excluded_root(d: &Path) -> Option<PathBuf> {
    let repo = repo_root(d);
    let excluded = |p: &Path| {
        p.strip_prefix(&repo)
            .is_ok_and(|rel| crate::remote_sync::excluded_from_mirror(&repo, rel))
    };
    if !excluded(d) {
        return None;
    }
    // The package's workspace when that is excluded too, so its root manifest
    // and lockfile travel. Never wider: the rest of the tree stays out.
    let ws = workspace_dir(d);
    Some(if excluded(&ws) { ws } else { d.to_path_buf() })
}

/// Whether one of `roots` holds `d` in its mirror: a root above it whose
/// `.justrustignore` does not leave it out.
fn mirrors(roots: &[PathBuf], d: &Path) -> bool {
    roots.iter().any(|root| holds(root, d))
}

fn holds(root: &Path, d: &Path) -> bool {
    d.strip_prefix(root).is_ok_and(|rel| {
        rel.as_os_str().is_empty() || !crate::remote_sync::excluded_from_mirror(root, rel)
    })
}

/// The workspace of `cwd` and every lockfile at or above it with its mtime:
/// path dependencies only change with a lockfile.
///
/// `v3`: the key was the outermost lockfile alone, which a workspace nested
/// in a repository with a lockfile of its own (`toolchain/cargo` here) shares
/// with the repository. Whichever built first after that lockfile changed
/// cached its roots for both, so after a build in the repository a build in
/// `toolchain/cargo` mirrored the repository only, which excludes
/// `/toolchain/`, and the machine compiled the sources an older mirror had
/// left: `justrust test` reported 3 tests passing with "0 files" to sync
/// while a fourth, new one was never sent.
fn roots_cache_key(cwd: &Path) -> Option<String> {
    let mut locks = Vec::new();
    let mut d = Some(cwd);
    while let Some(dir) = d {
        let lock = dir.join("Cargo.lock");
        if let Some(hash) = crate::depcache::hash_file_content(&lock) {
            locks.push(format!(
                "{}@{}",
                lock.display(),
                &hash[..16.min(hash.len())]
            ));
        }
        d = dir.parent();
    }
    if locks.is_empty() {
        return None;
    }
    Some(format!(
        "v4:{}|{}",
        workspace_dir(cwd).display(),
        locks.join("|")
    ))
}

/// The nearest ancestor of `d` (itself included) whose Cargo.toml declares
/// a `[workspace]`: the package's workspace root. `d` when there is none.
fn workspace_dir(d: &Path) -> PathBuf {
    let mut cur = Some(d);
    while let Some(c) = cur {
        if std::fs::read_to_string(c.join("Cargo.toml"))
            .is_ok_and(|s| s.lines().any(|l| l.trim() == "[workspace]"))
        {
            return c.to_path_buf();
        }
        cur = c.parent();
    }
    d.to_path_buf()
}

fn repo_root(d: &Path) -> PathBuf {
    let mut cur = Some(d);
    while let Some(c) = cur {
        if c.join(".git").exists() {
            return c.to_path_buf();
        }
        cur = c.parent();
    }
    d.to_path_buf()
}

/// Drop roots nested inside another root, unless that root's mirror excludes
/// them (`excluded_root`).
fn outermost(dirs: BTreeSet<PathBuf>) -> Vec<PathBuf> {
    let covers = holds;
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !out.iter().any(|o| covers(o, &d)) {
            out.retain(|o| !covers(&d, o));
            out.push(d);
        }
    }
    out
}

pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-=:,+@%".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Release asset this binary would be, when it is a release build. The
/// machine downloads it from GitHub (0.3 s from us-east-1) instead of us
/// uploading 11 MB over a home uplink (40-130 s measured, FINDINGS 65).
fn release_asset_url() -> Option<String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return None;
    }
    Some(format!(
        "https://github.com/1jehuang/justrust/releases/download/v{}/justrust-x86_64-unknown-linux-musl.tar.gz",
        env!("CARGO_PKG_VERSION")
    ))
}

/// Install this exact binary on the machine from its GitHub release, if the
/// release asset is byte-identical to the running executable (sha256). A
/// dev build never matches and falls back to the upload. Never errors.
fn fetch_release_on_machine(t: &Target) -> bool {
    if std::env::var("JUSTRUST_REMOTE_FETCH_RELEASE").as_deref() == Ok("0") {
        return false;
    }
    let Some(url) = release_asset_url() else {
        return false;
    };
    let Ok(exe) = crate::procfs::running_executable_path() else {
        return false;
    };
    let Ok(want) = crate::toolchain::sha256(&exe) else {
        return false;
    };
    let script = format!(
        "set -e; d=$(mktemp -d); trap 'rm -rf \"$d\"' EXIT; \
         curl -fsSL --max-time 30 {url} | tar xz -C \"$d\"; \
         f=$(find \"$d\" -type f -name justrust | head -1); \
         [ \"$(sha256sum \"$f\" | cut -d' ' -f1)\" = {want} ]; \
         mkdir -p ~/.cargo/bin; chmod +x \"$f\"; mv \"$f\" ~/.cargo/bin/.justrust.upload; \
         mv ~/.cargo/bin/.justrust.upload ~/.cargo/bin/justrust",
        url = shell_quote(&url),
        want = shell_quote(&want),
    );
    t.command()
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A Mac release cannot upload its Mach-O executable to a Linux build host.
/// Use the host's asset from the same release, not an unpublished crates.io
/// package. Source builds still take the existing source-install fallback.
fn cross_release_install_script(remote_arch: &str, version: Option<&str>) -> Option<String> {
    release_install_script(remote_arch, version, false)
}

/// With `keep_fleet_copy` the verified executable is also kept under its
/// version in [`FLEET_COPIES`], taken before the shared path is replaced so
/// it is these bytes whoever replaces that path next.
fn release_install_script(
    remote_arch: &str,
    version: Option<&str>,
    keep_fleet_copy: bool,
) -> Option<String> {
    let version = version?;
    let keep = if keep_fleet_copy {
        keep_fleet_copy_step(version)
    } else {
        String::new()
    };
    let target = match remote_arch {
        "Linux x86_64" => "x86_64-unknown-linux-musl",
        "Linux aarch64" => "aarch64-unknown-linux-musl",
        _ => return None,
    };
    let asset = format!("justrust-{target}.tar.gz");
    let url = format!("https://github.com/1jehuang/justrust/releases/download/v{version}/{asset}");
    Some(format!(
        "set -e; d=$(mktemp -d); stage=; \
         trap 'rm -rf \"$d\"; [ -z \"$stage\" ] || rm -f \"$stage\"' EXIT; \
         curl -fsSL --max-time 30 {url} -o \"$d/{asset}\"; \
         curl -fsSL --max-time 30 {checksum_url} -o \"$d/checksum\"; \
         expected=$(awk 'NR == 1 {{print $1}}' \"$d/checksum\"); \
         [ \"${{#expected}}\" = 64 ]; \
         printf '%s  %s\\n' \"$expected\" \"$d/{asset}\" | sha256sum -c -; \
         tar xzf \"$d/{asset}\" -C \"$d\"; \
         f=\"$d/justrust-{target}/justrust\"; \
         [ \"$(\"$f\" --version)\" = {expected_version} ]; \
         mkdir -p ~/.cargo/bin; chmod +x \"$f\"; {keep}\
         stage=$(mktemp ~/.cargo/bin/.justrust.upload.XXXXXX); \
         mv \"$f\" \"$stage\"; mv \"$stage\" ~/.cargo/bin/justrust",
        url = shell_quote(&url),
        checksum_url = shell_quote(&format!("{url}.sha256")),
        expected_version = shell_quote(&format!("justrust {version}")),
    ))
}

fn fetch_cross_release_on_machine(t: &Target, remote_arch: &str) -> bool {
    if std::env::var("JUSTRUST_REMOTE_FETCH_RELEASE").as_deref() == Ok("0") {
        return false;
    }
    let own = crate::update::release_build().then_some(env!("CARGO_PKG_VERSION"));
    let Some(script) = cross_release_install_script(remote_arch, own) else {
        return false;
    };
    t.command()
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Same linker on both sides: with wild here and not there, the machine
/// linked the 670 MB Desktop test binary with mold, 1.26 s against 0.25 s
/// (FINDINGS 33). Best effort: a failure only costs speed.
fn ensure_wild(t: &Target) {
    if crate::linker::is_installed() && std::env::var("JUSTRUST_LINKER").as_deref() != Ok("off") {
        let _ = t
            .command()
            .arg("~/.cargo/bin/justrust toolchain wild >/dev/null 2>&1")
            .stdin(Stdio::null())
            .status();
    }
}

// Fleet semver alone does not establish support for the private durable-session
// protocol. Those sessions must use the existing exact-client install path.
fn fleet_policy_allows(hosted: bool, dev_mode: bool, durable_session: bool) -> bool {
    hosted && !dev_mode && !durable_session
}

// Durable hosted sessions are Unix-only (see `remote_session` in lib.rs).
#[cfg_attr(windows, allow(dead_code))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub(crate) struct PinnedSessionExecutable {
    pub path: String,
    pub sha256: String,
}

#[cfg_attr(windows, allow(dead_code))]
impl PinnedSessionExecutable {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                && Path::new(&self.path).is_absolute()
                && !Path::new(&self.path).components().any(|c| matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                ))
                && !self.path.contains(['\0', '\n', '\r'])
                && self.path.ends_with(&format!(
                    "/.justrust/remote/session-executables/{}/justrust",
                    self.sha256
                )),
            "invalid immutable hosted session executable identity"
        );
        Ok(())
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn session_executable_prepare_script(digest: &str) -> String {
    format!(
        r#"set -eu; umask 077
sha() {{ sha256sum "$1" | cut -d' ' -f1; }}
owned_dir() {{
  [ ! -L "$1" ]; if [ ! -e "$1" ]; then mkdir "$1"; fi
  [ -d "$1" ] && [ -O "$1" ]
}}
private_dir() {{ owned_dir "$1"; [ "$(stat -c %a "$1")" = 700 ]; }}
owned_dir "$HOME/.justrust"; owned_dir "$HOME/.justrust/remote"
private_dir "$HOME/.justrust/remote/session-executables"
d="$HOME/.justrust/remote/session-executables/{digest}"; private_dir "$d"
f="$d/justrust"
valid() {{ [ ! -L "$f" ] && [ -f "$f" ] && [ -O "$f" ] && [ -x "$f" ] && [ "$(stat -c %a "$f")" = 500 ] && [ "$(sha "$f")" = '{digest}' ]; }}
if [ -e "$f" ] || [ -L "$f" ]; then valid; printf 'READY\n%s\n' "$f"; exit; fi
tmp=$(mktemp "$d/.upload.XXXXXXXXXX"); trap 'rm -f -- "$tmp"' EXIT HUP INT TERM
if cp "$HOME/.cargo/bin/justrust" "$tmp" 2>/dev/null && [ "$(sha "$tmp")" = '{digest}' ]; then
  chmod 500 "$tmp"; ln "$tmp" "$f" 2>/dev/null || valid
  valid; printf 'READY\n%s\n' "$f"
else
  printf 'UPLOAD\n%s\n%s\n' "$f" "$tmp"; trap - EXIT HUP INT TERM
fi"#
    )
}

#[cfg_attr(windows, allow(dead_code))]
fn session_executable_publish_script(pin: &PinnedSessionExecutable, temp: &str) -> Result<String> {
    pin.validate()?;
    let parent = Path::new(&pin.path).parent().unwrap();
    anyhow::ensure!(
        Path::new(temp).parent() == Some(parent)
            && Path::new(temp)
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.starts_with(".upload.")
                    && v.len() == 18
                    && v.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.')),
        "invalid private executable upload receipt"
    );
    Ok(format!(
        r#"set -eu; umask 077
f={path}; tmp={temp}; d={parent}
trap 'rm -f -- "$tmp"' EXIT HUP INT TERM
[ ! -L "$d" ] && [ -d "$d" ] && [ -O "$d" ] && [ "$(stat -c %a "$d")" = 700 ]
[ ! -L "$tmp" ] && [ -f "$tmp" ] && [ -O "$tmp" ]
sha() {{ sha256sum "$1" | cut -d' ' -f1; }}
[ "$(sha "$tmp")" = '{digest}' ]; chmod 500 "$tmp"
valid() {{ [ ! -L "$f" ] && [ -f "$f" ] && [ -O "$f" ] && [ -x "$f" ] && [ "$(stat -c %a "$f")" = 500 ] && [ "$(sha "$f")" = '{digest}' ]; }}
ln "$tmp" "$f" 2>/dev/null || valid
valid"#,
        path = shell_quote(&pin.path),
        temp = shell_quote(temp),
        parent = shell_quote(&parent.to_string_lossy()),
        digest = pin.sha256
    ))
}

/// Unlike the mutable fleet launcher, this exact-byte executable survives other
/// clients replacing ~/.cargo/bin/justrust between create and reattachment.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn ensure_session_executable(target: &Target) -> Result<PinnedSessionExecutable> {
    let arch = target
        .command()
        .arg("uname -sm")
        .stdin(Stdio::null())
        .output()?;
    let remote = String::from_utf8_lossy(&arch.stdout).trim().to_owned();
    let local = crate::remote::local_platform();
    anyhow::ensure!(
        arch.status.success() && remote == local && remote.starts_with("Linux "),
        "durable hosted sessions require an exact matching Linux executable (local {local}, remote {remote}); JUSTRUST_HOSTED_SESSION=0 selects legacy transport"
    );
    let exe = crate::procfs::running_executable_path()?;
    let digest = crate::toolchain::sha256(&exe)?;
    let result = crate::remote_sync::executable_output(
        target
            .command()
            .arg(session_executable_prepare_script(&digest))
            .stdin(Stdio::null()),
        "private hosted session executable preparation",
    )?;
    let output = std::str::from_utf8(&result.stdout).context("invalid executable receipt")?;
    let fields: Vec<_> = output.lines().collect();
    anyhow::ensure!(
        matches!(fields.as_slice(), ["READY", _] | ["UPLOAD", _, _]),
        "invalid executable receipt"
    );
    let pin = PinnedSessionExecutable {
        path: fields[1].into(),
        sha256: digest,
    };
    pin.validate()?;
    if fields[0] == "UPLOAD" {
        let publish = session_executable_publish_script(&pin, fields[2])?;
        if let Err(error) = crate::remote_sync::upload_file(target, &exe, fields[2]) {
            // Only our random private temporary receipt may be removed.
            let _ = target
                .command()
                .arg(format!("rm -f -- {}", shell_quote(fields[2])))
                .stdin(Stdio::null())
                .status();
            return Err(error.context("uploading exact hosted session executable failed"));
        }
        crate::remote_sync::executable_step(
            target.command().arg(publish).stdin(Stdio::null()),
            "publishing verified hosted session executable",
        )?;
    }
    Ok(pin)
}

/// The fleet version a hosted machine should run, or `None` to install this
/// client's own build (other backends, dev mode, durable session, no version).
fn fleet_target_version() -> Option<String> {
    if !fleet_policy_allows(
        matches!(
            crate::remote_backend::load(),
            Some(crate::remote_backend::Backend::Hosted)
        ),
        crate::remote_hosted::dev_mode(),
        crate::remote_session::enabled(),
    ) {
        return None;
    }
    let v = crate::remote_hosted::fleet_version()?;
    fleet_usable(&v, env!("CARGO_PKG_VERSION")).then_some(v)
}

/// The machine's agent serves every protocol up to its own, so the fleet
/// release must be at least this client's version. A client ahead of the
/// fleet (pre-release, dev build) installs itself instead.
fn fleet_usable(fleet: &str, own: &str) -> bool {
    !crate::update::newer(own, fleet)
}

/// Install release `version` on the machine for its own architecture,
/// checksum- and version-verified before replacing the binary.
fn install_fleet_release(t: &Target, version: &str) -> bool {
    let Ok(arch) = t.command().arg("uname -sm").stdin(Stdio::null()).output() else {
        return false;
    };
    let arch = String::from_utf8_lossy(&arch.stdout).trim().to_string();
    let Some(script) = release_install_script(&arch, Some(version), true) else {
        return false;
    };
    t.command()
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Local copy of the justrust binary last installed on the machine, the
/// base for [`upload_patch`].
fn installed_copy() -> Option<PathBuf> {
    Some(crate::remote::dir().ok()?.join("installed-justrust"))
}

/// Remember the binary now on the machine, so the next dev build uploads a
/// patch against it. Best effort.
fn remember_installed(exe: &Path) {
    if let Some(p) = installed_copy() {
        let t = p.with_extension("tmp");
        if std::fs::copy(exe, &t).is_ok() {
            let _ = std::fs::rename(&t, &p);
        }
    }
}

/// Install a dev build by sending a zstd patch against the binary the
/// machine already runs (`--patch-from`), checked by sha256 on both ends.
/// Consecutive dev builds of justrust share most of their 11 MB: the patch
/// is ~1.8 MB against ~3.7 MB for the compressed full upload (rsync -z;
/// rsync's block delta finds little in a relinked binary), which at a home
/// uplink is the bulk of the 1115 s of installs measured on hosted
/// (FINDINGS 69). `false` (caller uploads in full) when either side has no
/// zstd, there is no base, or the machine's binary is not the base.
fn upload_patch(t: &Target) -> bool {
    if std::env::var("JUSTRUST_REMOTE_PATCH").as_deref() == Ok("0") {
        return false;
    }
    let (Some(base), Ok(exe)) = (installed_copy(), crate::procfs::running_executable_path()) else {
        return false;
    };
    let (Ok(base_sha), Ok(want)) = (
        crate::toolchain::sha256(&base),
        crate::toolchain::sha256(&exe),
    ) else {
        return false;
    };
    let Ok(out) = Command::new("zstd")
        .args(["-q", "-9", "-T0", "--long=27", "-c"])
        .arg(format!("--patch-from={}", base.display()))
        .arg(&exe)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    if !out.status.success() || out.stdout.is_empty() {
        return false;
    }
    let script = format!(
        "set -e; export PATH=\"$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.homebrew/bin:/usr/local/bin:$PATH\"; \
         command -v zstd >/dev/null; b=~/.cargo/bin/justrust; u=~/.cargo/bin/.justrust.upload; \
         sha() {{ (sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\") | awk '{{print $1}}'; }}; \
         [ \"$(sha \"$b\")\" = {base_sha} ]; \
         zstd -q -d -f --long=27 --patch-from=\"$b\" -o \"$u\"; \
         [ \"$(sha \"$u\")\" = {want} ] || {{ rm -f \"$u\"; exit 1; }}; \
         chmod +x \"$u\"; \
         (command -v codesign >/dev/null 2>&1 && codesign -s - -f \"$u\" 2>/dev/null || true); \
         mv \"$u\" \"$b\"",
        base_sha = shell_quote(&base_sha),
        want = shell_quote(&want),
    );
    let Ok(mut child) = t
        .command()
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut s| s.write_all(&out.stdout).is_ok());
    child.wait().is_ok_and(|s| s.success()) && wrote
}

/// What a build waiting in silence for its setup should do next.
#[derive(Debug, PartialEq, Eq)]
enum Silence {
    Quiet,
    Say,
    GiveUp,
}

/// Notices at 10 s, 30 s, 60 s and every minute after. Only `connecting` is
/// bounded: a sync lock or a first mirror can be long and still be progress,
/// but a connect that long is a link that is not coming up.
fn setup_silence(phase: &str, waited: Duration, notices: u32, bound: Option<Duration>) -> Silence {
    if phase == "connecting" && bound.is_some_and(|b| waited >= b) {
        return Silence::GiveUp;
    }
    let due = match notices {
        0 => 10,
        1 => 30,
        n => 60 * (u64::from(n) - 1),
    };
    if waited >= Duration::from_secs(due) {
        Silence::Say
    } else {
        Silence::Quiet
    }
}

/// How long a build may sit in `connecting` before it is given back
/// (`JUSTRUST_REMOTE_CONNECT_TIMEOUT` seconds, 0 for no bound).
pub(crate) fn connect_bound() -> Option<Duration> {
    let secs = std::env::var("JUSTRUST_REMOTE_CONNECT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);
    (secs > 0).then(|| Duration::from_secs(secs))
}

struct SetupState {
    phase: String,
    since: Instant,
    notices: u32,
    done: bool,
    gave_up: Option<u64>,
}

/// Says on stderr that a build is still waiting for its setup, and what for.
///
/// Setup phases went only to the live status file. On 2026-11-27 builds sat
/// up to 25 minutes at `justrust: remote` with nothing printed while the
/// daemon reinstalled and reconnected over a failing link (one connect took
/// 427 s, eleven builds queued behind it for up to 482 s).
struct SetupWatch(Arc<(Mutex<SetupState>, Condvar)>);

impl SetupWatch {
    fn start(sock: Option<crate::sys::net::UnixStream>) -> SetupWatch {
        let shared = Arc::new((
            Mutex::new(SetupState {
                phase: "waiting for the sync daemon".into(),
                since: Instant::now(),
                notices: 0,
                done: false,
                gave_up: None,
            }),
            Condvar::new(),
        ));
        let watch = shared.clone();
        let bound = connect_bound();
        let _ = std::thread::Builder::new()
            .name("remote-setup-watch".into())
            .spawn(move || {
                let (lock, cv) = &*watch;
                let mut st = lock.lock().unwrap();
                while !st.done {
                    st = cv.wait_timeout(st, Duration::from_secs(1)).unwrap().0;
                    if st.done {
                        break;
                    }
                    let waited = st.since.elapsed();
                    match setup_silence(&st.phase, waited, st.notices, bound) {
                        Silence::Quiet => {}
                        Silence::Say => {
                            st.notices += 1;
                            eprintln!(
                                "justrust remote: still {} after {}s (`justrust remote status`; JUSTRUST_REMOTE_CONNECT_TIMEOUT bounds connecting)",
                                st.phase,
                                waited.as_secs()
                            );
                        }
                        Silence::GiveUp => {
                            st.gave_up = Some(waited.as_secs());
                            st.done = true;
                            // Wakes the blocked read: the build is given back.
                            if let Some(sock) = &sock {
                                let _ = sock.shutdown(std::net::Shutdown::Both);
                            }
                        }
                    }
                }
            });
        SetupWatch(shared)
    }

    fn phase(&self, phase: &str) {
        let mut st = self.0.0.lock().unwrap();
        if st.phase != phase {
            st.phase = phase.into();
            st.since = Instant::now();
            st.notices = 0;
        }
    }

    /// Setup is over: the run started or a result frame arrived.
    fn done(&self) {
        self.0.0.lock().unwrap().done = true;
        self.0.1.notify_all();
    }

    fn gave_up(&self) -> Option<u64> {
        self.0.0.lock().unwrap().gave_up
    }
}

impl Drop for SetupWatch {
    fn drop(&mut self) {
        self.done();
    }
}

/// The sha256 of this client's executable when its build id is one.
fn own_sha() -> Option<String> {
    let id = crate::remote_daemon::build_id();
    let sha = id.strip_prefix("sha256-")?;
    (sha.len() == 64 && sha.bytes().all(|c| c.is_ascii_hexdigit())).then(|| sha.to_owned())
}

/// Where the machine keeps exact-byte copies of client executables, one
/// directory per sha256 (created by [`ensure_session_executable`]).
const OWN_COPIES: &str = "$HOME/.justrust/remote/session-executables";

/// Where the machine keeps the fleet releases it was given, one directory
/// per version. A fleet client runs its version from here: the shared path
/// is replaced by every client that installs its own build.
const FLEET_COPIES: &str = "$HOME/.justrust/remote/fleet-executables";

/// Shell word for the executable in `dir` when the machine has it, else the
/// shared path. Running it marks the copy as in use (see `prune_copies`).
fn copy_or_shared(dir: &str) -> String {
    format!(
        "\"$(d=\"{dir}\"; b=\"$d/justrust\"; if [ -x \"$b\" ]; then touch \"$d\" 2>/dev/null; else b=\"$HOME/.cargo/bin/justrust\"; fi; echo \"$b\")\""
    )
}

/// Shell that removes the copies under `$d` other than `keep` which nothing
/// has used for two weeks and no process is still running (a durable
/// session outlives its client).
fn prune_copies(keep: &str) -> String {
    format!(
        "[ ! -d \"$d\" ] || find \"$d\" -mindepth 1 -maxdepth 1 -type d -mtime +14 ! -name {keep} 2>/dev/null | while IFS= read -r o; do \
           ls -l /proc/[0-9]*/exe 2>/dev/null | grep -qF \"$o/\" || rm -rf \"$o\"; done"
    )
}

/// Shell that keeps the verified release executable `$f` under `version`.
fn keep_fleet_copy_step(version: &str) -> String {
    format!(
        "c=\"{FLEET_COPIES}/{version}\"; mkdir -p \"$c\"; cp \"$f\" \"$c/.justrust.new\"; \
         chmod 500 \"$c/.justrust.new\"; mv -f \"$c/.justrust.new\" \"$c/justrust\"; "
    )
}

/// The fleet version an install stamp records, when it records one.
fn fleet_of_stamp(stamp: &str) -> Option<&str> {
    let (_, v) = stamp.rsplit_once(" fleet ")?;
    (!v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)))
    .then_some(v)
}

fn machine_justrust_from(stamp: Option<&str>, sha: Option<&str>) -> String {
    match stamp.and_then(fleet_of_stamp) {
        Some(v) => copy_or_shared(&format!("{FLEET_COPIES}/{v}")),
        None => machine_justrust_for(sha),
    }
}

/// Exit 0 when the machine holds fleet release `version` under its name.
/// Marks it used and prunes the versions nothing uses any more.
fn fleet_copy_script(version: &str) -> String {
    format!(
        "d=\"{FLEET_COPIES}\"; f=\"$d/{version}/justrust\"; \
         [ ! -L \"$f\" ] && [ -f \"$f\" ] && [ -O \"$f\" ] && [ -x \"$f\" ] && [ \"$(\"$f\" --version 2>/dev/null)\" = {expected} ]; ok=$?; \
         [ $ok != 0 ] || touch \"$d/{version}\"; \
         {prune}; \
         exit $ok",
        expected = shell_quote(&format!("justrust {version}")),
        prune = prune_copies(version),
    )
}

/// Does the machine already hold this fleet release? Then nothing is
/// installed and the shared path is left as it is.
fn fleet_copy_on_machine(t: &Target, version: &str) -> bool {
    t.command()
        .arg(fleet_copy_script(version))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn machine_justrust_for(sha: Option<&str>) -> String {
    match sha {
        Some(sha) => copy_or_shared(&format!("{OWN_COPIES}/{sha}")),
        None => "~/.cargo/bin/justrust".into(),
    }
}

/// Shell word for this client's justrust on the machine: the fleet release
/// when that is what the install stamp records, else this client's own
/// exact-byte copy. Either falls back to the shared `~/.cargo/bin/justrust`
/// when the machine has no such copy.
///
/// There is one shared path and any number of clients. Two versions in use
/// at once (an installed release and a candidate run by explicit path) each
/// replaced it for the other, an install per alternation, and whichever ran
/// second served the first one's artifact requests. A copy named by its
/// digest is never anyone else's.
pub fn machine_justrust() -> String {
    let stamp = crate::remote::dir()
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join("remote-justrust")).ok());
    machine_justrust_from(stamp.as_deref(), own_sha().as_deref())
}

/// Exit 0 when the machine holds a valid own copy for `sha`. Marks it used,
/// and removes copies no client has used for two weeks and no process is
/// still running (a durable session outlives its client).
fn own_copy_script(sha: &str) -> String {
    format!(
        "d=\"{OWN_COPIES}\"; f=\"$d/{sha}/justrust\"; \
         sha() {{ (sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\" 2>/dev/null) | awk '{{print $1}}'; }}; \
         [ ! -L \"$f\" ] && [ -f \"$f\" ] && [ -O \"$f\" ] && [ -x \"$f\" ] && [ \"$(sha \"$f\")\" = {sha} ]; ok=$?; \
         [ $ok != 0 ] || touch \"$d/{sha}\"; \
         {prune}; \
         exit $ok",
        prune = prune_copies(sha),
    )
}

/// Does the machine already hold this exact client? Then nothing is
/// installed: the shared path stays whatever its last installer left.
fn own_copy_on_machine(t: &Target) -> bool {
    own_sha().is_some_and(|sha| {
        t.command()
            .arg(own_copy_script(&sha))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Does `reply` (the machine's platform, the sha256 of its justrust and its
/// `--version`, one per line) describe the binary `stamp` says was installed?
fn stamp_describes(stamp: &str, dest: &str, own: (&str, &str, &str), reply: &str) -> bool {
    let (own_platform, own_sha, own_version) = own;
    let mut lines = reply.lines().map(str::trim);
    let (Some(platform), Some(sha), Some(version)) = (lines.next(), lines.next(), lines.next())
    else {
        return false;
    };
    let Some(installed) = stamp.strip_prefix(dest).and_then(|s| s.strip_prefix(' ')) else {
        return false;
    };
    match installed.strip_prefix("fleet ") {
        Some(fleet) => version == format!("justrust {fleet}"),
        // This client's own bytes, or its release built for another platform.
        None => sha == own_sha || (platform != own_platform && version == own_version),
    }
}

/// After a handshake that got no answer: is the machine's justrust still the
/// one the stamp records? One short command instead of a reinstall. `None`
/// when the machine did not answer either, which says nothing about its
/// binary: a dropped link must not cost an install over that same link.
pub fn remote_justrust_intact(t: &Target) -> Option<bool> {
    let stamp = std::fs::read_to_string(crate::remote::dir().ok()?.join("remote-justrust")).ok()?;
    let out = t
        .command()
        .arg(format!(
            "uname -sm; b={}; \
             (sha256sum \"$b\" 2>/dev/null || shasum -a 256 \"$b\" 2>/dev/null || echo -) | awk '{{print $1}}'; \
             \"$b\" --version 2>/dev/null || echo -; echo answered",
            // A fleet stamp is about the fleet release, wherever it runs from.
            machine_justrust_from(Some(&stamp), own_sha().as_deref())
        ))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let reply = String::from_utf8_lossy(&out.stdout);
    // The last line shows the whole reply arrived, not a cut-off part of it.
    if !out.status.success() || reply.lines().last().map(str::trim) != Some("answered") {
        return None;
    }
    let exe = crate::procfs::running_executable_path().ok()?;
    let sha = crate::toolchain::sha256(&exe).ok()?;
    Some(stamp_describes(
        &stamp,
        &t.dest,
        (
            &crate::remote::local_platform(),
            &sha,
            concat!("justrust ", env!("CARGO_PKG_VERSION")),
        ),
        &reply,
    ))
}

/// Make sure the machine runs this exact justrust binary. Same OS and
/// architecture (the common case): fetch it from its release on the
/// machine, else copy it. Otherwise build it there from crates.io at this
/// version.
pub fn ensure_remote_justrust(t: &Target) -> Result<()> {
    let stamp = crate::remote::dir()?.join("remote-justrust");
    // Hosted machines run the fleet version the service advertises, so a
    // shared machine and its caches see one justrust regardless of which
    // client connects. The agent serves every older protocol version.
    let fleet = fleet_target_version();
    if let Some(v) = &fleet {
        let want = format!("{} fleet {v}", t.dest);
        if std::fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str()) {
            return Ok(());
        }
        let t0 = Instant::now();
        if fleet_copy_on_machine(t, v) {
            std::fs::write(&stamp, want)?;
            crate::remote::log_event(&format!(
                "fleet justrust {v} already on {}, nothing installed ({:.1}s)",
                t.dest,
                t0.elapsed().as_secs_f64()
            ));
            return Ok(());
        }
        if install_fleet_release(t, v) {
            ensure_wild(t);
            std::fs::write(&stamp, want)?;
            crate::remote::log_event(&format!(
                "fleet justrust {v} installed on {} in {:.1}s",
                t.dest,
                t0.elapsed().as_secs_f64()
            ));
            return Ok(());
        }
        // Fail open: push this client's own binary as before.
        crate::remote::log_event(&format!(
            "fleet justrust {v} install failed on {}, installing this client's build",
            t.dest
        ));
    }
    let id = crate::remote_daemon::build_id();
    let want = format!("{} {id}", t.dest);
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str()) {
        return Ok(());
    }
    let t0 = Instant::now();
    if own_copy_on_machine(t) {
        std::fs::write(&stamp, want)?;
        crate::remote::log_event(&format!(
            "justrust {} already on {} as its own copy, nothing installed ({:.1}s)",
            env!("CARGO_PKG_VERSION"),
            t.dest,
            t0.elapsed().as_secs_f64()
        ));
        return Ok(());
    }
    let arch = t
        .command()
        .arg("uname -sm")
        .stdin(Stdio::null())
        .output()
        .context("ssh uname")?;
    let remote_arch = String::from_utf8_lossy(&arch.stdout).trim().to_string();
    let local_arch = crate::remote::local_platform();
    let t0 = Instant::now();
    if remote_arch == local_arch && fetch_release_on_machine(t) {
        crate::remote::log_event(&format!(
            "justrust {} fetched from its release on {} in {:.1}s",
            env!("CARGO_PKG_VERSION"),
            t.dest,
            t0.elapsed().as_secs_f64()
        ));
    } else if remote_arch == local_arch && upload_patch(t) {
        crate::remote::log_event(&format!(
            "justrust patched on {} in {:.1}s",
            t.dest,
            t0.elapsed().as_secs_f64()
        ));
    } else if remote_arch == local_arch {
        let exe = crate::procfs::running_executable_path()?;
        let tmp = ".cargo/bin/.justrust.upload";
        crate::remote_sync::executable_step(
            t.command()
                .arg("mkdir -p ~/.cargo/bin")
                .stdin(Stdio::null()),
            "preparing remote executable directory",
        )?;
        crate::remote_sync::upload_file(t, &exe, tmp)
            .context("copying justrust to the remote machine failed")?;
        crate::remote_sync::executable_step(t.command()
                .arg(format!(
                    "chmod +x ~/{tmp} && (command -v codesign >/dev/null 2>&1 && codesign -s - -f ~/{tmp} 2>/dev/null || true) && mv ~/{tmp} ~/.cargo/bin/justrust"
                ))
                .stdin(Stdio::null()), "installing uploaded remote executable")?;
    } else if remote_arch != local_arch && fetch_cross_release_on_machine(t, &remote_arch) {
        crate::remote::log_event(&format!(
            "justrust {} fetched its {remote_arch} release on {} in {:.1}s",
            env!("CARGO_PKG_VERSION"),
            t.dest,
            t0.elapsed().as_secs_f64()
        ));
    } else {
        eprintln!("justrust remote: building justrust on the machine ({remote_arch})...");
        build_justrust_on_machine(t)?;
    }
    ensure_wild(t);
    // Our own AWS machine only (never a user's ssh host): /bin/sh as bash,
    // see remote::SH_IS_BASH. Best effort.
    if matches!(
        crate::remote_backend::load(),
        Some(crate::remote_backend::Backend::Aws)
    ) {
        let _ = t
            .command()
            .arg(format!(
                "sudo -n sh -c '{}'",
                crate::remote::SH_IS_BASH.replace('\'', "'\\''")
            ))
            .stdin(Stdio::null())
            .status();
    }
    if let Ok(exe) = crate::procfs::running_executable_path() {
        remember_installed(&exe);
    }
    std::fs::write(&stamp, want)?;
    crate::remote::log_event(&format!(
        "installed justrust on {} in {:.1}s",
        t.dest,
        t0.elapsed().as_secs_f64()
    ));
    Ok(())
}

/// Build and install `justrust` natively on a cross-OS/cross-arch remote host.
/// Prefers syncing the local source checkout to `~/.justrust/bootstrap-src` so
/// development builds and protocol changes match the client binary; falls back
/// to `cargo install` when no local checkout is available.
fn build_justrust_on_machine(t: &Target) -> Result<()> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if manifest_dir.join("Cargo.toml").is_file() && manifest_dir.join("src/main.rs").is_file() {
        let _ = t
            .command()
            .arg("mkdir -p ~/.justrust/bootstrap-src ~/.cargo/bin")
            .stdin(Stdio::null())
            .status();
        let src = format!("{}/", manifest_dir.display());
        let dst = crate::remote_sync::rsync_destination(t, ".justrust/bootstrap-src/");
        let mut sync = Command::new("rsync");
        crate::remote_sync::configure_rsync(&mut sync, t);
        let synced = sync
            .args([
                "-az",
                "--delete",
                "--exclude=/target/",
                "--exclude=/.git/",
                "--exclude=/.justrust/",
                "--exclude=/toolchain/",
                "--exclude=/bench/",
                "--exclude=/data/",
            ])
            .arg(&src)
            .arg(&dst)
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if synced {
            let st = t
                .command()
                .arg(
                    "set -e; . ~/.cargo/env 2>/dev/null || true; \
                     export PATH=\"$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.homebrew/bin:/usr/local/bin:$PATH\"; \
                     cd ~/.justrust/bootstrap-src; \
                     cargo build --release --bin justrust --locked -q; \
                     mkdir -p ~/.cargo/bin; \
                     u=~/.cargo/bin/.justrust.upload; \
                     cp target/release/justrust \"$u\"; \
                     chmod +x \"$u\"; \
                     (command -v codesign >/dev/null 2>&1 && codesign -s - -f \"$u\" 2>/dev/null || true); \
                     mv \"$u\" ~/.cargo/bin/justrust",
                )
                .stdin(Stdio::null())
                .status()?;
            if st.success() {
                return Ok(());
            }
        }
    }
    let st = t
        .command()
        .arg(format!(
            ". ~/.cargo/env 2>/dev/null || true; \
             export PATH=\"$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.homebrew/bin:/usr/local/bin:$PATH\"; \
             cargo install justrust --version {} --locked -q",
            env!("CARGO_PKG_VERSION")
        ))
        .status()?;
    if !st.success() {
        bail!("installing justrust on the remote machine failed");
    }
    Ok(())
}

/// Why a remote attempt did not produce a build result. The router falls
/// back to a local build on any of these.
#[derive(Debug)]
pub struct Infra(pub String);

impl Infra {
    /// Billing refusals (402, signed out) are complete one-line
    /// instructions, shown as they are.
    pub fn is_billing(&self) -> bool {
        self.0.starts_with(BILLING)
    }

    /// The attempt failed before reaching the service, for a reason in
    /// this project or invocation (no workspace, cwd outside the synced
    /// roots). Says nothing about remote health, so it must not pause
    /// hosted routing for other builds.
    pub fn is_client_side(&self) -> bool {
        self.0.starts_with(CLIENT_SIDE)
    }
}

/// Marks an [`Infra`] whose text is a billing instruction.
pub const BILLING: &str = "justrust: ";

/// Marks an [`Infra`] caused by this invocation, not the remote.
pub const CLIENT_SIDE: &str = "local: ";

/// Hosted only: charge this run before anything runs remotely. Err holds
/// the line to show; the build then runs locally.
/// `Ok(true)` when a hosted charge happened (the run then reports its
/// lifecycle to the operator dashboard).
fn charge_hosted(
    run_id: &str,
    cmd: &str,
    project: Option<&str>,
) -> Result<std::result::Result<bool, String>> {
    if remote_backend::load() != Some(remote_backend::Backend::Hosted) {
        return Ok(Ok(false));
    }
    let c = match crate::remote_hosted::creds() {
        Ok(c) => c,
        Err(e) => return Ok(Err(format!("{BILLING}{e}"))),
    };
    let admission = await_hosted_admission(
        || crate::remote_hosted::admit_run_labeled(&c, run_id, Some(cmd), project),
        crate::remote_hosted::current_update,
        wait_for_hosted_update,
    )?;
    Ok(match admission {
        Ok(_) => Ok(true),
        Err(e)
            if e.contains("justrust upgrade")
                || e.contains("justrust update")
                || e.contains("jcode.sh/account")
                || e.contains("justrust login") =>
        {
            Err(format!("{BILLING}{e}"))
        }
        Err(e) => Err(e),
    })
}

/// New servers check maintenance in the mandatory authenticated admission.
/// A legacy server still gets its status check before execution. Maintenance
/// failures are outer errors so they can never trigger a local fallback.
fn await_hosted_admission(
    mut admit: impl FnMut() -> std::result::Result<crate::remote_hosted::RunAdmission, String>,
    mut legacy_status: impl FnMut() -> Result<Option<crate::remote_hosted::HostedUpdate>>,
    mut wait: impl FnMut(crate::remote_hosted::HostedUpdate) -> Result<()>,
) -> Result<std::result::Result<Option<crate::remote_hosted::Builds>, String>> {
    use crate::remote_hosted::RunAdmission;
    let mut updates = BTreeSet::new();
    loop {
        match admit() {
            Ok(RunAdmission::Charged {
                builds,
                maintenance_checked,
            }) => {
                if !maintenance_checked
                    && let Ok(Some(update)) = legacy_status()
                    && matches!(update.state.as_str(), "announced" | "restarting")
                {
                    wait(update)?;
                }
                return Ok(Ok(builds));
            }
            Ok(RunAdmission::Updating(update)) => {
                if updates.len() >= 2 || !updates.insert(update.id.clone()) {
                    bail!("hosted maintenance admission did not clear; no local fallback");
                }
                eprintln!(
                    "justrust remote: planned hosted update {}: {}; waiting before admission (no local fallback)",
                    update.id,
                    update.label()
                );
                wait(update)?;
            }
            Ok(RunAdmission::UpdateRequired {
                min_version,
                message,
            }) => {
                if let Some(err) =
                    crate::update::try_reexec_for_required_update(min_version.as_deref())
                {
                    return Ok(Err(format!("{message} ({err})")));
                }
                return Ok(Err(message));
            }
            Err(message) => return Ok(Err(message)),
        }
    }
}

pub struct Outcome {
    pub code: i32,
    #[allow(dead_code)] // used by the router (in progress)
    pub run_id: String,
    pub cancelled: bool,
}

#[cfg(unix)]
mod cancel_signal {
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicI32, Ordering};

    static SIGNAL: AtomicI32 = AtomicI32::new(0);
    static SOCK_FD: AtomicI32 = AtomicI32::new(-1);

    extern "C" fn handler(sig: libc::c_int) {
        // A second termination signal while cleaning up exits immediately so a
        // wedged cleanup step can never block cancellation.
        if SIGNAL
            .compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // SAFETY: _exit(2) is async-signal-safe.
            unsafe {
                libc::_exit(128 + sig);
            }
        }
        let fd = SOCK_FD.load(Ordering::Acquire);
        if fd >= 0 {
            // SAFETY: shutdown(2) is async-signal-safe per POSIX signal-safety(7).
            unsafe {
                libc::shutdown(fd, libc::SHUT_RDWR);
            }
        }
    }

    pub(super) struct Guard {
        prev: [(libc::c_int, libc::sigaction); 3],
    }

    impl Guard {
        pub(super) fn install() -> Self {
            SIGNAL.store(0, Ordering::SeqCst);
            SOCK_FD.store(-1, Ordering::Release);
            // SAFETY: sigaction is a plain C struct initialized before sigaction(2).
            let mut prev: [(libc::c_int, libc::sigaction); 3] =
                [(0, unsafe { std::mem::zeroed() }); 3];
            // SAFETY: handler only touches atomics and async-signal-safe syscalls.
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = handler as *const () as usize;
                // Omit SA_RESTART so blocking I/O wakes immediately on cancellation.
                sa.sa_flags = 0;
                libc::sigemptyset(&mut sa.sa_mask);
                for (slot, sig) in prev
                    .iter_mut()
                    .zip([libc::SIGTERM, libc::SIGINT, libc::SIGHUP])
                {
                    slot.0 = sig;
                    libc::sigaction(sig, &sa, &mut slot.1);
                }
            }
            Self { prev }
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            SOCK_FD.store(-1, Ordering::Release);
            // SAFETY: restoring previous sigaction structs saved during install().
            unsafe {
                for (sig, old) in &self.prev {
                    libc::sigaction(*sig, old, std::ptr::null_mut());
                }
            }
        }
    }

    pub(super) struct ActiveSocket;

    impl ActiveSocket {
        pub(super) fn register(fd: &impl AsRawFd) -> Self {
            let raw = fd.as_raw_fd();
            SOCK_FD.store(raw, Ordering::Release);
            if SIGNAL.load(Ordering::SeqCst) != 0 {
                // SAFETY: raw is still open because the caller holds `fd`.
                unsafe {
                    libc::shutdown(raw, libc::SHUT_RDWR);
                }
            }
            Self
        }
    }

    impl Drop for ActiveSocket {
        fn drop(&mut self) {
            // Clear before the owning stream drops so a reused fd is never shut down.
            SOCK_FD.store(-1, Ordering::Release);
        }
    }

    pub(super) fn received() -> Option<i32> {
        match SIGNAL.load(Ordering::SeqCst) {
            0 => None,
            sig => Some(sig),
        }
    }

    fn label(sig: i32) -> &'static str {
        match sig {
            libc::SIGTERM => "SIGTERM",
            libc::SIGINT => "SIGINT",
            libc::SIGHUP => "SIGHUP",
            _ => "signal",
        }
    }

    pub(super) fn reason(sig: i32, prebuild: bool) -> String {
        if prebuild {
            format!("prebuild cancelled ({})", label(sig))
        } else {
            format!("cancelled ({})", label(sig))
        }
    }
}

#[cfg(not(unix))]
mod cancel_signal {
    pub(super) struct Guard;
    impl Guard {
        pub(super) fn install() -> Self {
            Self
        }
    }

    pub(super) struct ActiveSocket;
    impl ActiveSocket {
        pub(super) fn register<T>(_fd: &T) -> Self {
            Self
        }
    }

    pub(super) fn received() -> Option<i32> {
        None
    }

    pub(super) fn reason(_sig: i32, prebuild: bool) -> String {
        if prebuild {
            "prebuild cancelled".into()
        } else {
            "cancelled".into()
        }
    }
}

fn fallback_safe(flushed: bool, submitted: bool) -> bool {
    !flushed && !submitted
}

type RunReporter = Arc<dyn Fn(&str, Option<i32>, Option<&crate::failure::Wire>) + Send + Sync>;

/// Owns hosted admission through completion, including early `?` returns.
/// Reporting only dispatches detached requests. Joining wakes the worker, not
/// a network request, and prevents new dispatches after the terminal event.
struct HostedRun {
    report: Option<RunReporter>,
    heartbeat: Option<(mpsc::Sender<()>, std::thread::JoinHandle<()>)>,
    finished: bool,
    running: bool,
}

fn heartbeat_loop(mut wait: impl FnMut() -> bool, report: impl Fn()) {
    while wait() {
        report();
    }
}

impl HostedRun {
    fn new(hosted: bool, run_id: &str, started: Instant) -> Self {
        let run_id = run_id.to_owned();
        Self::with_reporter(hosted.then(|| {
            Arc::new(
                move |event: &str, code, failure: Option<&crate::failure::Wire>| {
                    crate::remote_hosted::report_run_event(
                        &run_id,
                        event,
                        code,
                        Some(started.elapsed().as_secs_f64()),
                        failure,
                    );
                },
            ) as RunReporter
        }))
    }

    fn with_reporter(report: Option<RunReporter>) -> Self {
        Self {
            report,
            heartbeat: None,
            finished: false,
            running: false,
        }
    }

    fn running(&mut self) {
        if self.running || self.finished {
            return;
        }
        self.running = true;
        let Some(report) = &self.report else { return };
        report("running", None, None);
        let report = Arc::clone(report);
        let (stop, rx) = mpsc::channel();
        // Failure to create telemetry machinery must not change the verdict.
        if let Ok(worker) = std::thread::Builder::new()
            .name("hosted-run-heartbeat".into())
            .spawn(move || {
                heartbeat_loop(
                    || {
                        matches!(
                            rx.recv_timeout(Duration::from_secs(60)),
                            Err(mpsc::RecvTimeoutError::Timeout)
                        )
                    },
                    || report("running", None, None),
                );
            })
        {
            self.heartbeat = Some((stop, worker));
        }
    }

    fn finish(&mut self, event: &str, code: Option<i32>, failure: Option<&crate::failure::Wire>) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some((stop, worker)) = self.heartbeat.take() {
            let _ = stop.send(());
            let _ = worker.join();
        }
        if let Some(report) = &self.report {
            report(event, code, failure);
        }
        // Already dispatched requests can arrive out of order. The server
        // treats terminal states as absorbing, including late heartbeats.
    }
}

impl Drop for HostedRun {
    fn drop(&mut self) {
        self.finish("abandoned", None, None);
    }
}

/// Run `justrust <sub> <args>` remotely. `Ok(Err(Infra))` means nothing was
/// built and the caller may build locally instead. Output is streamed to
/// our stdout/stderr as it arrives.
pub fn run_remote(
    sub: &str,
    args: &[String],
    start: bool,
) -> Result<std::result::Result<Outcome, Infra>> {
    let _cancel_guard = cancel_signal::Guard::install();
    let started = Instant::now();
    let mut diagnostic = RemoteDiagnostic::default();
    let run_id = crate::record::new_run_id();
    let mut lifecycle = crate::telemetry::BuildLifecycle::start(&run_id, sub, "remote");
    crate::e2e::mark("remote_preflight");
    let mut hosted_run = None;
    let mut overlap_admit_rx: Option<OverlapAdmitRx> = None;
    let mut recovered = BTreeSet::new();
    let result = loop {
        let attempt_started = crate::paths::now() as u64;
        let result = run_remote_inner(
            sub,
            args,
            start,
            &mut diagnostic,
            &run_id,
            &mut lifecycle,
            &mut hosted_run,
            &mut overlap_admit_rx,
        );
        // If run_remote_inner returned early (e.g. remote_transport failure or
        // cancellation) while overlapped admission was still in flight, settle
        // it now so the queued row is marked abandoned (or reused on retry)
        // instead of staying orphaned in build_runs.
        settle_pending_overlap_admit(&mut hosted_run, &mut overlap_admit_rx, &run_id, started);
        if cancel_signal::received().is_some() {
            break result;
        }
        let disconnected = matches!(&result, Err(e) if e.is::<RemoteConnectionLost>())
            || matches!(&result, Ok(Err(Infra(msg))) if msg == "sync daemon closed the connection"
                || msg.starts_with("connection to the remote machine was lost"));
        if !disconnected
            || !matches!(
                remote_backend::load(),
                Some(remote_backend::Backend::Hosted)
            )
        {
            break result;
        }
        let disconnected_at = crate::paths::now() as u64;
        let update = match crate::remote_hosted::current_update() {
            Ok(Some(update)) if update.overlaps(attempt_started, disconnected_at) => update,
            _ => break result,
        };
        if recovered.len() >= 2 || !recovered.insert(update.id.clone()) {
            break Err(anyhow::anyhow!(
                "hosted update {} ({}): automatic recovery limit reached, no local fallback",
                update.id,
                update.label()
            ));
        }
        eprintln!(
            "justrust remote: interrupted by planned hosted update {}: {}; waiting for recovery (no local fallback)",
            update.id,
            update.label()
        );
        if let Err(e) = wait_for_hosted_update(update) {
            break Err(e);
        }
        eprintln!(
            "justrust remote: update complete, restarting the interrupted hosted command using retained Cargo artifacts; earlier output belongs to the interrupted attempt"
        );
        crate::telemetry::record(
            "hosted_update_recovery",
            serde_json::json!({
                "run_id": run_id, "updates": recovered.len(), "action": "restart_hosted"
            }),
        );
        diagnostic = RemoteDiagnostic::default();
        // Keep the admission and heartbeat alive across attempts. The same run
        // is charged once and gets one final result, not an abandoned result.
    };
    if let Some(sig) = cancel_signal::received() {
        let code = 128 + sig;
        if let Some(run) = hosted_run.as_mut() {
            let why = (
                crate::failure::Class::Interrupted,
                cancel_signal::reason(sig, crate::buildd::is_guess()),
            );
            run.finish("abandoned", Some(code), Some(&why));
        }
        return Ok(Ok(Outcome {
            code,
            run_id,
            cancelled: true,
        }));
    }
    let result = match result {
        Ok(Err(Infra(msg))) if !recovered.is_empty() => Err(anyhow::anyhow!(
            "hosted update recovery could not reconnect: {msg}; no local fallback"
        )),
        other => other,
    };
    if let Some(code) = diagnostic.exit {
        let logs = crate::paths::runs_dir()
            .ok()
            .is_some_and(|d| std::fs::File::open(d.join(&run_id).join("output.jsonl")).is_ok());
        lifecycle.done(code, logs);
    }
    if !matches!(&result, Ok(Ok(_))) {
        if diagnostic.failure_kind == "unknown" {
            diagnostic.failure_kind = match &result {
                Ok(Err(Infra(msg))) => transport_failure_kind(msg),
                Err(error) => transport_error_kind(error),
                _ => "unknown",
            };
        }
        // An admitted run that never produced an exit status died on our
        // side. Say where, instead of leaving a bare "abandoned" row.
        if let Some(run) = hosted_run.as_mut() {
            let refused = match &result {
                Ok(Err(Infra(msg))) => service_refusal(msg),
                _ => None,
            };
            let why = (
                crate::failure::Class::Infra,
                match refused {
                    Some(cause) => format!("hosted service refused the run: {cause}"),
                    None => format!(
                        "hosted attempt failed at {}: {} ({})",
                        diagnostic.stage, diagnostic.failure_kind, diagnostic.transport_phase
                    ),
                },
            );
            run.finish("abandoned", None, Some(&why));
        }
        report_remote_failure(sub, &diagnostic, started.elapsed().as_secs_f64());
    }
    result
}

#[derive(Debug)]
struct RemoteConnectionLost;

impl std::fmt::Display for RemoteConnectionLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("connection to the remote machine was lost")
    }
}

impl std::error::Error for RemoteConnectionLost {}

#[derive(Debug)]
struct RemoteCancelled;

impl std::fmt::Display for RemoteCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("remote build cancelled by signal")
    }
}

impl std::error::Error for RemoteCancelled {}

fn wait_for_hosted_update(mut update: crate::remote_hosted::HostedUpdate) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(360);
    loop {
        if cancel_signal::received().is_some() {
            return Err(RemoteCancelled.into());
        }
        if update.ready_to_retry() {
            return Ok(());
        }
        if update.state == "failed" || Instant::now() >= deadline {
            bail!(
                "hosted update {} ({}): recovery {}; work was not restarted locally",
                update.id,
                update.label(),
                if update.state == "failed" {
                    "failed"
                } else {
                    "timed out"
                }
            );
        }
        for _ in 0..20 {
            if cancel_signal::received().is_some() {
                return Err(RemoteCancelled.into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Ok(Some(next)) = crate::remote_hosted::current_update() {
            if next.id != update.id {
                bail!(
                    "hosted update {} was superseded, refusing to duplicate interrupted work",
                    update.id
                );
            }
            update = next;
        }
    }
}

struct RemoteDiagnostic {
    stage: &'static str,
    may_have_started: bool,
    exit: Option<i32>,
    run_id: Option<String>,
    transport_phase: &'static str,
    failure_kind: &'static str,
}

impl Default for RemoteDiagnostic {
    fn default() -> Self {
        Self {
            stage: "remote_setup",
            may_have_started: false,
            exit: None,
            run_id: None,
            transport_phase: "daemon_connect",
            failure_kind: "unknown",
        }
    }
}

/// A fixed label when the hosted service answered and refused the run, so
/// the run row does not blame the transport for a service-side outage (the
/// 2026-11-12 storage outage showed as 56 "remote_transport" failures).
/// Never the server's text: the label is shown to operators.
fn service_refusal(msg: &str) -> Option<&'static str> {
    if msg.contains(crate::remote_hosted::AT_CAPACITY) {
        Some("fleet at capacity")
    } else if msg.contains("Hosted storage") || msg.contains("workspace host") {
        Some("host storage unavailable")
    } else if msg.contains("workspace filesystem") {
        Some("workspace storage needs recovery")
    } else if msg.contains(crate::remote_hosted::UNAVAILABLE) {
        Some("service unavailable")
    } else {
        None
    }
}

// Classify locally, upload only a fixed label. Never upload the input text:
// daemon errors can contain tenant paths, command arguments and credentials.
pub(crate) fn transport_failure_kind(message: &str) -> &'static str {
    if let Some(kind) = message.strip_prefix("connection to the remote machine was lost: ") {
        return match kind {
            "connection_closed" => "connection_closed",
            "connection_reset" => "connection_reset",
            "timeout" => "timeout",
            "protocol" => "protocol",
            "io" => "io",
            "connection_refused" => "connection_refused",
            "authentication" => "authentication",
            _ => "unknown",
        };
    }
    if service_refusal(message).is_some() {
        return "service_refused";
    }
    let message = message.to_ascii_lowercase();
    if message.contains("timed out") || message.contains("timeout") {
        "timeout"
    } else if message.contains("connection refused") {
        "connection_refused"
    } else if message.contains("connection reset") || message.contains("broken pipe") {
        "connection_reset"
    } else if message.contains("closed the connection")
        || message.contains("connection to the remote machine was lost")
        || message.contains("unexpected eof")
    {
        "connection_closed"
    } else if message.contains("permission denied") || message.contains("authentication") {
        "authentication"
    } else if message.contains("resolve hostname") || message.contains("name resolution") {
        "dns"
    } else if message.contains("protocol") {
        "protocol"
    } else if message.contains("connection_closed") {
        "connection_closed"
    } else if message.contains("connection_reset") {
        "connection_reset"
    } else {
        "unknown"
    }
}

pub(crate) fn transport_error_kind(error: &anyhow::Error) -> &'static str {
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return match error.kind() {
            std::io::ErrorKind::ConnectionRefused => "connection_refused",
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe => {
                "connection_reset"
            }
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionAborted => {
                "connection_closed"
            }
            std::io::ErrorKind::TimedOut => "timeout",
            std::io::ErrorKind::PermissionDenied => "authentication",
            _ => "io",
        };
    }
    if error.downcast_ref::<serde_json::Error>().is_some() {
        return "protocol";
    }
    transport_failure_kind(&format!("{error:#}"))
}

fn remote_failure_data(sub: &str, d: &RemoteDiagnostic, elapsed: f64) -> serde_json::Value {
    let sub = match sub {
        "check" | "test" | "build" | "clippy" | "run" | "bench" | "doc" | "rustc" => sub,
        _ => "other",
    };
    serde_json::json!({ "schema_version": 3, "sub": sub, "stage": d.stage,
        "run_id": d.run_id, "transport_phase": d.transport_phase,
        "failure_kind": d.failure_kind,
        "may_have_started": d.may_have_started,
        "exit": d.exit, "elapsed_s": (elapsed * 100.0).round() / 100.0 })
}

fn report_remote_failure(sub: &str, d: &RemoteDiagnostic, elapsed: f64) {
    crate::telemetry::record("build_failure", remote_failure_data(sub, d, elapsed));
    crate::telemetry::maybe_spawn_background();
}

type OverlapAdmitRx = std::sync::mpsc::Receiver<Result<std::result::Result<bool, String>>>;

fn settle_pending_overlap_admit(
    hosted_run: &mut Option<HostedRun>,
    overlap_admit_rx: &mut Option<OverlapAdmitRx>,
    run_id: &str,
    started: Instant,
) {
    if hosted_run.is_none()
        && let Some(rx) = overlap_admit_rx.take()
        && let Ok(Ok(Ok(hosted))) = rx.recv_timeout(Duration::from_secs(5))
    {
        *hosted_run = Some(HostedRun::new(hosted, run_id, started));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_remote_inner(
    sub: &str,
    args: &[String],
    start: bool,
    diagnostic: &mut RemoteDiagnostic,
    run_id: &str,
    lifecycle: &mut crate::telemetry::BuildLifecycle,
    admitted: &mut Option<HostedRun>,
    overlap_admit_rx: &mut Option<OverlapAdmitRx>,
) -> Result<std::result::Result<Outcome, Infra>> {
    diagnostic.run_id = Some(run_id.to_owned());
    let total = Instant::now();
    let trace = std::env::var_os("JUSTRUST_REMOTE_TRACE").is_some();
    crate::e2e::mark("routed");
    let mark = |what: &str| {
        if trace {
            eprintln!(
                "justrust trace: {:>7.1} ms {what}",
                total.elapsed().as_secs_f64() * 1000.0
            );
        }
    };
    let started_at = crate::paths::now();
    let cwd = std::env::current_dir()?;
    let infra = |m: String| Ok(Err(Infra(m)));
    let roots = match source_roots(&cwd) {
        Ok(r) => r,
        Err(e) => return infra(format!("{CLIENT_SIDE}{e:#}")),
    };
    if !roots.iter().any(|r| cwd.starts_with(r)) {
        return infra(format!(
            "{CLIENT_SIDE}{} is not inside a synced root",
            cwd.display()
        ));
    }
    let mut label = vec![sub.to_string()];
    label.extend(args.iter().cloned());
    let project = roots
        .iter()
        .find(|r| cwd.starts_with(r))
        .and_then(|r| r.file_name())
        .and_then(|n| n.to_str())
        .map(str::to_string);
    mark("roots");
    crate::e2e::mark("remote_roots");
    diagnostic.stage = "remote_admission";
    let overlap_admit = admitted.is_none()
        && remote_backend::load() == Some(remote_backend::Backend::Hosted)
        && !matches!(crate::remote_windows::selected(), Ok(true))
        && std::env::var("JUSTRUST_HOSTED_OVERLAP_ADMIT").as_deref() != Ok("0")
        && crate::remote_hosted::creds()
            .ok()
            .is_some_and(|c| c.warm_admit_ready());
    if !overlap_admit
        && admitted.is_none()
        && remote_backend::load() == Some(remote_backend::Backend::Hosted)
        && !matches!(crate::remote_windows::selected(), Ok(true))
    {
        let pre_roots = roots.clone();
        std::thread::spawn(move || {
            let _ = crate::remote_daemon::pre_sync(&pre_roots);
        });
    }
    if admitted.is_none() {
        if overlap_admit {
            let (tx, rx) = std::sync::mpsc::channel();
            let rid = run_id.to_owned();
            let cmd = label.join(" ");
            let proj = project.clone();
            std::thread::spawn(move || {
                let _ = tx.send(charge_hosted(&rid, &cmd, proj.as_deref()));
            });
            *overlap_admit_rx = Some(rx);
            mark("charged (overlapped)");
            crate::e2e::mark("admitted");
        } else {
            let hosted = match charge_hosted(run_id, &label.join(" "), project.as_deref())? {
                Ok(h) => h,
                Err(m) => return infra(m),
            };
            *admitted = Some(HostedRun::new(hosted, run_id, total));
        }
    }
    if cancel_signal::received().is_some() {
        return Err(RemoteCancelled.into());
    }
    if crate::remote_windows::selected()? {
        let hosted_run = admitted.as_mut().expect("windows run admitted above");
        if remote_backend::load() != Some(remote_backend::Backend::Hosted) {
            return infra(format!(
                "{CLIENT_SIDE}native Windows execution requires the hosted backend"
            ));
        }
        diagnostic.stage = "windows_transport";
        let prepared = crate::remote_windows::prepare(sub, args, run_id, &cwd, &roots)?;
        hosted_run.running();
        diagnostic.may_have_started = true;
        let native = crate::remote_windows::execute(prepared, run_id)?;
        diagnostic.exit = Some(native.code);
        let failure = native
            .summary
            .as_ref()
            .and_then(|summary| {
                serde_json::from_value::<crate::summary::Summary>(summary.clone()).ok()
            })
            .and_then(|summary| summary.failure_wire());
        hosted_run.finish("done", Some(native.code), failure.as_ref());
        if let Some(summary) = native.summary {
            let mut summary: crate::summary::Summary = serde_json::from_value(summary)?;
            summary.id = run_id.to_owned();
            summary.cwd = cwd.display().to_string();
            let _ = record_remote(
                run_id,
                &serde_json::to_vec(&summary)?,
                &[],
                "Windows x86_64 (MSVC)",
                "hosted",
                0.0,
                None,
                None,
                total.elapsed().as_secs_f64(),
            );
        }
        return Ok(Ok(Outcome {
            code: native.code,
            run_id: run_id.to_owned(),
            cancelled: false,
        }));
    }
    if !overlap_admit {
        mark("charged");
        crate::e2e::mark("admitted");
    }
    // Visible to `justrust status` (and the status bar) while it runs.
    let mut live = LiveRun::start(run_id, &cwd, &label, started_at);
    diagnostic.stage = "remote_transport";
    let sock = match crate::remote_daemon::connect() {
        Ok(s) => s,
        Err(e) => return infra(format!("{e:#}")),
    };
    crate::e2e::mark("daemon_connected");
    // Do not join the speculative PreSync thread here: if it is still holding
    // the daemon sync lock, RunSync will wait on the lock (attributed to
    // sync.lock_wait, not unattributed sync: relay) and reuse its PreSyncStamp.
    let mut env = Vec::new();
    env.push((crate::remote_proto::CLIENT_LIVE_CPU.into(), "1".into()));
    env.extend(forwarded_env(|name| std::env::var(name).ok()));
    // SSH login defaults can differ from the caller (Ubuntu: 0002 versus
    // local 0022), changing permissions and even security-sensitive tests.
    // This metadata is consumed before exec and never passed to Cargo.
    if let Some(mask) = crate::procfs::current_umask() {
        env.push((
            crate::remote_proto::CLIENT_UMASK.to_string(),
            format!("{mask:04o}"),
        ));
    }
    // The machine builds with its cargo home at our path, so dependency
    // artifacts are interchangeable with ours (peer cache).
    if let Some(h) = crate::depcache::cargo_home()
        .to_str()
        .filter(|h| !h.is_empty())
    {
        env.push((
            crate::remote_agent::CLIENT_CARGO_HOME.to_string(),
            h.to_string(),
        ));
    }
    if let Some(repo) = roots
        .iter()
        .find(|r| cwd == **r)
        .and_then(|r| crate::remote_sync::repo_id(r))
        .and_then(|id| id.root_commits.into_iter().next())
    {
        env.push(("JUSTRUST_CANONICAL_REPO".to_string(), repo));
    }
    let mut w = BufWriter::new(sock.try_clone()?);
    let mut r = BufReader::new(sock);
    let setup_watch = SetupWatch::start(r.get_ref().try_clone().ok());
    // Declared after `r` so it clears the registered fd before `r` closes it.
    let _cancel_sock = cancel_signal::ActiveSocket::register(r.get_ref());
    let mut full = vec![sub.to_string()];
    full.extend(args.iter().cloned());
    let compile_only = crate::remote_artifacts::test_no_run(&full);
    let json_output = full
        .iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a.starts_with("--message-format=") && a.contains("json"))
        || full
            .windows(2)
            .take_while(|w| w[0] != "--")
            .any(|w| w[0] == "--message-format" && w[1].contains("json"));
    let remote_args = if compile_only {
        compile_only_args(&full)
    } else {
        full.clone()
    };
    diagnostic.transport_phase = "request_send";
    write_frame(
        &mut w,
        &Msg::RunSync {
            id: 0,
            cwd: cwd.display().to_string(),
            args: remote_args,
            env,
            roots: roots.iter().map(|r| r.display().to_string()).collect(),
            start,
            run_id: run_id.to_owned(),
        },
        &[],
    )?;
    diagnostic.transport_phase = "source_sync";
    let mut sync = None;
    let mut remote_prefix = String::new();
    let mut sync_timing = None;
    let mut submitted = false;
    let mut first_out = false;
    let mut run_files: Vec<u8> = Vec::new();
    let mut compile_stdout = CompileOutput::default();
    let mut pending_out: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut so = std::io::stdout();
    let mut se = std::io::stderr();
    let emit_out = |stream: u8,
                    data: &[u8],
                    remote_prefix: &str,
                    compile_only: bool,
                    json_output: bool,
                    compile_stdout: &mut CompileOutput,
                    live: &mut LiveRun,
                    so: &mut std::io::Stdout,
                    se: &mut std::io::Stderr|
     -> Result<()> {
        let data: std::borrow::Cow<'_, [u8]> = if !remote_prefix.is_empty()
            && let Ok(s) = std::str::from_utf8(data)
        {
            if s.contains(remote_prefix) {
                std::borrow::Cow::Owned(s.replace(remote_prefix, "").into_bytes())
            } else {
                std::borrow::Cow::Borrowed(data)
            }
        } else {
            std::borrow::Cow::Borrowed(data)
        };
        if stream == 1 {
            // Selected tests are emitted only after their local files exist.
            // Remote JSON contains host paths and cannot be passed through.
            if !compile_only {
                so.write_all(&data)?;
                so.flush()?;
            } else {
                compile_stdout.push(&data, so, se, json_output)?;
            }
        } else {
            live.stderr(&data);
            se.write_all(&data)?;
            se.flush()?;
        }
        Ok(())
    };
    macro_rules! settle_overlap_admit {
        ($block:expr) => {{
            if let Some(rx) = overlap_admit_rx.as_ref() {
                let outcome = if $block {
                    rx.recv().ok()
                } else {
                    match rx.try_recv() {
                        Ok(v) => Some(v),
                        Err(std::sync::mpsc::TryRecvError::Empty) => None,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Ok(Err(
                            "hosted admission thread exited unexpectedly".into(),
                        ))),
                    }
                };
                if let Some(res) = outcome {
                    let _ = overlap_admit_rx.take();
                    match res {
                        Ok(Ok(hosted)) => {
                            let mut hr = HostedRun::new(hosted, run_id, total);
                            if sync.is_some() || submitted {
                                hr.running();
                            }
                            *admitted = Some(hr);
                            mark("admit settled");
                            for (stream, data) in pending_out.drain(..) {
                                emit_out(
                                    stream,
                                    &data,
                                    &remote_prefix,
                                    compile_only,
                                    json_output,
                                    &mut compile_stdout,
                                    &mut live,
                                    &mut so,
                                    &mut se,
                                )?;
                            }
                        }
                        Ok(Err(m)) => {
                            let _ = write_frame(&mut w, &Msg::Cancel { id: 0 }, &[]);
                            pending_out.clear();
                            return infra(m);
                        }
                        Err(e) => {
                            let _ = write_frame(&mut w, &Msg::Cancel { id: 0 }, &[]);
                            pending_out.clear();
                            return Err(e);
                        }
                    }
                }
            }
        }};
    }
    loop {
        let frame = match read_frame(&mut r) {
            Ok(Some(f)) => f,
            failure => {
                if cancel_signal::received().is_some() {
                    return Err(RemoteCancelled.into());
                }
                settle_overlap_admit!(true);
                if let Some(secs) = setup_watch.gave_up() {
                    diagnostic.failure_kind = "timeout";
                    return infra(format!(
                        "timed out connecting to the remote machine after {secs}s (JUSTRUST_REMOTE_CONNECT_TIMEOUT)"
                    ));
                }
                diagnostic.failure_kind = match &failure {
                    Ok(None) => "connection_closed",
                    Err(error) => {
                        let kind = transport_error_kind(error);
                        if kind == "unknown" { "protocol" } else { kind }
                    }
                    _ => unreachable!(),
                };
                if fallback_safe(sync.is_some(), submitted) {
                    return infra("sync daemon closed the connection".into());
                }
                // Preserve the no-duplicate-execution policy after submission.
                return Err(RemoteConnectionLost.into());
            }
        };
        match &frame.0 {
            Msg::RunState {
                may_have_started: false,
                phase: Some(phase),
                ..
            } => setup_watch.phase(phase),
            _ => setup_watch.done(),
        }
        match frame {
            (
                Msg::RunState {
                    may_have_started,
                    phase,
                    ..
                },
                _,
            ) => {
                submitted = may_have_started;
                if let Some(phase) = phase {
                    live.phase(&phase, "");
                }
                if may_have_started {
                    diagnostic.transport_phase = "remote_execution";
                    lifecycle.running();
                    if let Some(hr) = admitted.as_mut() {
                        hr.running();
                    }
                }
                diagnostic.may_have_started |= may_have_started;
                settle_overlap_admit!(false);
            }
            (
                Msg::Flushed {
                    files,
                    ms,
                    machine,
                    prefix,
                    note,
                    timing,
                    ..
                },
                _,
            ) => {
                if !note.is_empty() {
                    eprintln!("justrust remote: {note}");
                }
                if !prefix.is_empty() {
                    remote_prefix = prefix;
                }
                mark("flushed");
                crate::e2e::mark("flushed");
                sync_timing = timing;
                if sync.is_none() {
                    lifecycle.running();
                    if let Some(hr) = admitted.as_mut() {
                        hr.running();
                    }
                    live.phase("building", &machine);
                }
                sync = Some((files, ms, machine));
                diagnostic.transport_phase = "remote_execution";
                diagnostic.may_have_started = true;
                settle_overlap_admit!(false);
            }
            (Msg::Out { stream, .. }, data) => {
                if stream == crate::remote_proto::LIVE_CPU_STREAM {
                    if let Ok(cpu) = serde_json::from_slice::<crate::record::LiveCpu>(&data)
                        && let Some(dir) = &live.dir
                    {
                        crate::record::write_live_cpu(dir, &cpu);
                    }
                    continue;
                }
                diagnostic.may_have_started = true;
                diagnostic.transport_phase = "remote_execution";
                if trace && !first_out {
                    first_out = true;
                    mark("first output");
                }
                settle_overlap_admit!(false);
                if admitted.is_some() {
                    emit_out(
                        stream,
                        &data,
                        &remote_prefix,
                        compile_only,
                        json_output,
                        &mut compile_stdout,
                        &mut live,
                        &mut so,
                        &mut se,
                    )?;
                } else {
                    pending_out.push((stream, data));
                }
            }
            (Msg::RunFiles { .. }, data) => {
                mark(&format!("run files {} KB", data.len() / 1024));
                run_files = data
            }
            (Msg::LinkOutage { ms, .. }, _) => {
                // The daemon's transport dropped and came back under this run.
                mark(&format!("link outage {ms:.0} ms"));
                crate::e2e::note("link_outage", ms / 1000.0);
            }
            (Msg::Exit { code, timing, .. }, summary) => {
                settle_overlap_admit!(true);
                diagnostic.exit = Some(code);
                diagnostic.may_have_started = true;
                mark("exit");
                crate::e2e::mark("exit");
                // The machine classified the failure in its summary.
                let failure = (code != 0)
                    .then(|| serde_json::from_slice::<crate::summary::Summary>(&summary).ok())
                    .flatten()
                    .and_then(|s| s.failure_wire());
                if let Some(hr) = admitted.as_mut() {
                    hr.finish("done", Some(code), failure.as_ref());
                }
                if code != 0 {
                    for file in learn_missing(&String::from_utf8_lossy(&summary), &cwd, &roots) {
                        eprintln!(
                            "justrust remote: {} exists here but was outside the mirrored roots, so the machine did not have it. It is mirrored from the next build on: run this again.",
                            file.display()
                        );
                    }
                }
                if compile_only {
                    compile_stdout.finish(&mut so, &mut se, json_output)?;
                    // A prebuild only warms the machine's target dir: the
                    // request it guessed runs its tests there.
                    if code == 0 && !crate::buildd::is_guess() {
                        compile_stdout.require_success()?;
                        diagnostic.stage = "remote_artifacts";
                        diagnostic.transport_phase = "artifact_download";
                        let (line, artifacts) = crate::remote_artifacts::fetch_tests(
                            &full, started_at, run_id,
                        )
                        .context(
                            "compiled tests remotely, but returning selected executables failed",
                        )?;
                        let dir = crate::paths::runs_dir()?.join(run_id);
                        std::fs::create_dir_all(&dir)?;
                        let manifest = dir.join("selected-artifacts.json");
                        std::fs::write(&manifest, serde_json::to_vec_pretty(&artifacts)?)?;
                        for artifact in artifacts {
                            if json_output {
                                serde_json::to_writer(&mut so, &artifact)?;
                                so.write_all(b"\n")?;
                            } else if let Some(path) = artifact["executable"].as_str() {
                                writeln!(se, "justrust test executable: {path}")?;
                            }
                        }
                        writeln!(
                            se,
                            "justrust remote: {line}; selected manifest {}",
                            manifest.display()
                        )?;
                    }
                    if json_output {
                        serde_json::to_writer(
                            &mut so,
                            &serde_json::json!({"reason": "build-finished", "success": code == 0}),
                        )?;
                        so.write_all(b"\n")?;
                    }
                    so.flush()?;
                    se.flush()?;
                }
                // A build is only done once its outputs are here. Counted in
                // the client wall the router compares against local.
                if code == 0 && matches!(sub, "build" | "zigbuild") {
                    diagnostic.stage = "remote_artifacts";
                    diagnostic.transport_phase = "artifact_download";
                    match crate::remote_artifacts::fetch(&full, started_at) {
                        Ok(line) => eprintln!("justrust remote: {line}"),
                        Err(e) => {
                            return Ok(Err(Infra(format!(
                                "built remotely, but bringing the binary back failed: {e:#}"
                            ))));
                        }
                    }
                }
                let (files, ms, machine) = sync.unwrap_or((0, 0.0, String::new()));
                crate::e2e::mark("artifacts");
                mark("recording");
                diagnostic.stage = "remote_summary";
                diagnostic.transport_phase = "summary_download";
                if !record_remote(
                    run_id,
                    &summary,
                    &run_files,
                    &machine,
                    crate::summary::execution_place(None, Some(&machine)),
                    ms,
                    sync_timing,
                    timing,
                    total.elapsed().as_secs_f64(),
                ) {
                    report_remote_failure(sub, diagnostic, total.elapsed().as_secs_f64());
                }
                eprintln!(
                    "justrust remote: {} on {machine} in {:.1}s (sync {:.0} ms, {files} file{} at build time)",
                    if code == 0 { "ok" } else { "failed" },
                    total.elapsed().as_secs_f64(),
                    ms,
                    if files == 1 { "" } else { "s" },
                );
                return Ok(Ok(Outcome {
                    code,
                    run_id: run_id.to_owned(),
                    cancelled: false,
                }));
            }
            // A refusal is the answer, not an outage: a local build would
            // hide the instruction to update behind a different compiler.
            (Msg::Error { msg, .. }, _) if msg.contains(crate::toolchain_policy::REFUSED) => {
                settle_overlap_admit!(true);
                bail!("{msg}")
            }
            (Msg::Error { msg, .. }, _) if fallback_safe(sync.is_some(), submitted) => {
                settle_overlap_admit!(true);
                return infra(msg);
            }
            (Msg::Error { msg, .. }, _) => {
                settle_overlap_admit!(true);
                diagnostic.failure_kind = transport_failure_kind(&msg);
                if msg.starts_with("connection to the remote machine was lost") {
                    return Err(RemoteConnectionLost.into());
                }
                bail!("remote build failed mid-run: {msg}")
            }
            _ => {}
        }
    }
}

/// Only one incomplete Cargo JSON line is buffered. Diagnostic records stream
/// immediately, but remote artifact paths never reach the local Cargo consumer.
#[derive(Default)]
struct CompileOutput {
    partial: Vec<u8>,
    successful: bool,
    failed: bool,
}

impl CompileOutput {
    const MAX_LINE: usize = 16 * 1024 * 1024;

    fn push(
        &mut self,
        bytes: &[u8],
        out: &mut impl Write,
        err: &mut impl Write,
        json: bool,
    ) -> Result<()> {
        for piece in bytes.split_inclusive(|b| *b == b'\n') {
            if self.partial.len().saturating_add(piece.len()) > Self::MAX_LINE {
                bail!("remote Cargo JSON line exceeds the 16 MiB safety limit");
            }
            self.partial.extend_from_slice(piece);
            if piece.ends_with(b"\n") {
                self.emit(out, err, json)?;
            }
        }
        Ok(())
    }

    fn emit(&mut self, out: &mut impl Write, err: &mut impl Write, json: bool) -> Result<()> {
        if self.partial.is_empty() {
            return Ok(());
        }
        match serde_json::from_slice::<serde_json::Value>(&self.partial) {
            Ok(value) if value["reason"] == "compiler-message" => {
                if json {
                    serde_json::to_writer(&mut *out, &value)?;
                    out.write_all(b"\n")?;
                    out.flush()?;
                } else if let Some(rendered) = value["message"]["rendered"].as_str() {
                    err.write_all(rendered.as_bytes())?;
                    err.flush()?;
                }
            }
            Ok(value) if value["reason"] == "build-finished" => {
                self.successful |= value["success"] == true;
                self.failed |= value["success"] != true;
            }
            // Ordinary text is a diagnostic, never part of the local JSON stream.
            Err(_) => {
                err.write_all(&self.partial)?;
                if !self.partial.ends_with(b"\n") {
                    err.write_all(b"\n")?;
                }
                err.flush()?;
            }
            _ => (),
        }
        self.partial.clear();
        Ok(())
    }

    fn finish(&mut self, out: &mut impl Write, err: &mut impl Write, json: bool) -> Result<()> {
        self.emit(out, err, json)
    }

    fn require_success(&self) -> Result<()> {
        if self.failed || !self.successful {
            bail!(
                "remote test artifacts require an exclusively successful Cargo build-finished stream"
            );
        }
        Ok(())
    }
}

/// Force Cargo's machine-readable artifact stream without changing selectors.
fn compile_only_args(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            result.push("--message-format=json-render-diagnostics".into());
            result.push(arg.clone());
            result.extend(iter.cloned());
            return result;
        }
        if arg == "--message-format" {
            let _ = iter.next();
        } else if !arg.starts_with("--message-format=") {
            result.push(arg.clone());
        }
    }
    result.push("--message-format=json-render-diagnostics".into());
    result
}

/// Where a remote build runs, for `justrust status`: "hosted" or "remote".
pub fn place_name() -> &'static str {
    match remote_backend::load() {
        Some(remote_backend::Backend::Hosted) => "hosted",
        _ => "remote",
    }
}

/// Placeholder run directory (`meta.json` with our pid, `live.json`) for a
/// remote build in flight, so `justrust status` lists it like a local one.
/// Removed on drop unless the run was recorded (`summary.json` written), so
/// a fallback to a local build does not leave a phantom active run.
struct LiveRun {
    dir: Option<PathBuf>,
    sniffer: crate::live::Sniffer,
}

impl LiveRun {
    fn start(run_id: &str, cwd: &Path, args: &[String], start: f64) -> LiveRun {
        let Ok(dir) = crate::paths::runs_dir().map(|d| d.join(run_id)) else {
            return LiveRun {
                dir: None,
                sniffer: crate::live::Sniffer::new(None, true),
            };
        };
        let meta = crate::record::Meta {
            id: run_id.to_string(),
            args: args.to_vec(),
            subcommand: args.first().cloned().unwrap_or_default(),
            cwd: cwd.display().to_string(),
            start,
            pid: std::process::id(),
            agent_session: std::env::var("JCODE_SESSION_ID").ok(),
            place: Some(place_name().into()),
            ..Default::default()
        };
        let ok = std::fs::create_dir_all(&dir).is_ok()
            && serde_json::to_vec(&meta)
                .is_ok_and(|b| std::fs::write(dir.join("meta.json"), b).is_ok());
        let dir = ok.then_some(dir);
        let sniffer = crate::live::Sniffer::new(dir.as_deref(), true);
        let mut r = LiveRun { dir, sniffer };
        r.phase("syncing", "");
        r
    }

    fn phase(&mut self, phase: &str, machine: &str) {
        let Some(dir) = &self.dir else { return };
        let state = &mut self.sniffer.state;
        state.phase = phase.into();
        if !machine.is_empty() {
            state.active = vec![machine.to_string()];
            state.active_since = vec![crate::paths::now()];
        }
        state.updated = crate::paths::now();
        if let Ok(b) = serde_json::to_vec(state) {
            let path = dir.join(crate::live::FILE);
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(tmp, path);
            }
        }
    }

    fn stderr(&mut self, data: &[u8]) {
        // Observe only. Forward the original bytes unchanged to preserve
        // diagnostics and the remote command's output contract.
        self.sniffer.feed(data, &mut Vec::new());
    }
}

impl Drop for LiveRun {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join(crate::live::FILE));
            if !dir.join("summary.json").exists() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Keep a local copy of the remote run so `justrust runs/show/log` and the
/// router see it. Marked `remote` in the index. `run_files` (from a v4+
/// agent) carries the output log, units, and samples.
#[allow(clippy::too_many_arguments)]
fn record_remote(
    run_id: &str,
    summary: &[u8],
    run_files: &[u8],
    machine: &str,
    place: &str,
    sync_ms: f64,
    sync_timing: Option<crate::e2e::SyncTiming>,
    run_timing: Option<crate::remote_proto::RunTiming>,
    wall: f64,
) -> bool {
    let Ok(mut s) = serde_json::from_slice::<crate::summary::Summary>(summary) else {
        return false;
    };
    let Ok(dir) = crate::paths::runs_dir().map(|d| d.join(run_id)) else {
        return false;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    if !run_files.is_empty()
        && let Err(e) = crate::remote_proto::unpack_run_files(run_files, &dir)
    {
        crate::remote::log_event(&format!("run {run_id}: remote run files: {e:#}"));
    }
    // The machine recorded the run as local to itself. Persist the client's
    // provenance in every representation, not just the live-run metadata.
    s.place = Some(place.into());
    s.client_version = Some(crate::update::VERSION.into());
    // Older hosted recorders return raw unit observations without the aggregate.
    if s.dashboard_profile.is_none() && !run_files.is_empty() {
        let (profile, opt) = crate::dashboard::DashboardProfile::returned(&dir);
        s.dashboard_profile = Some(profile);
        s.units.opt_level = s.units.opt_level.or(opt);
    }
    if s.git.dirty_rs_lines.is_none() && !s.cwd.is_empty() {
        let cwd = std::path::Path::new(&s.cwd);
        if cwd.is_dir() {
            s.git.dirty_rs_lines = crate::record::dirty_rs_lines_in(cwd);
        }
    }
    if s.theoretical_floor.is_none() {
        s.theoretical_floor = Some(crate::theoretical::compute(
            &s.subcommand,
            &s.args,
            &s.git,
            &s.units,
            &s.phases,
            &s.tests,
        ));
    }
    let meta_path = dir.join("meta.json");
    if let Some(mut m) = std::fs::read(&meta_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::record::Meta>(&b).ok())
    {
        m.place = s.place.clone();
        if let Ok(b) = serde_json::to_vec_pretty(&m) {
            let _ = std::fs::write(&meta_path, b);
        }
    }
    s.remote = Some(crate::summary::RemoteInfo {
        machine: machine.to_string(),
        sync_ms,
        client_wall: wall,
        sync: sync_timing,
        run: run_timing,
    });
    // The host wrote a waterfall for its own process. Keep what it measured
    // around cargo before the end-to-end one replaces it.
    let host_own = crate::e2e::HostOwn::of(s.e2e.as_ref());
    s.e2e = Some(crate::e2e::remote(
        &crate::e2e::marks(),
        crate::e2e::elapsed(),
        &crate::e2e::RemoteTimes {
            sync: sync_timing,
            run: run_timing,
            host_wall: s.wall,
            slot_wait: s.slot.as_ref().map_or(0.0, |slot| slot.join_wait_secs),
            prebuild_wait: s.prebuild.as_ref().map_or(0.0, |p| p.wait_secs),
            host_own,
            host_phases: s.phases.clone(),
            link_outage: crate::e2e::noted("link_outage"),
            artifacts: crate::e2e::ArtifactTimes::noted(),
        },
    ));
    // Hints are local state: attach them here, not on the remote machine
    // (which mirrors the workspace at the same absolute path).
    crate::split_index::attach(&mut s);
    let attached = !s.id.is_empty() && s.id != run_id;
    let _ = std::fs::write(
        dir.join("summary.json"),
        serde_json::to_vec_pretty(&s).unwrap_or_default(),
    );
    if let (Ok(idx), Ok(line)) = (
        crate::paths::index_file(),
        serde_json::to_string(&s.index_entry()),
    ) && (!attached || !crate::runs::index_has_recent_id(&idx, &s.id))
    {
        let _ = crate::paths::append_line(&idx, &line);
    }
    crate::split_index::record(&s);
    crate::split_index::maybe_refresh(&s);
    crate::test_reuse::store(&s, wall);
    if !attached {
        crate::telemetry::record_run(&s, true);
        crate::export::upload_run(&s);
    }
    crate::telemetry::maybe_spawn_background();
    // The machine printed its own report, which cannot see the client's side
    // of the wait (sync, admission, artifacts) or this machine's history.
    eprint!("{}", client_side_report(&s));
    true
}

/// Waste lines for time outside the remote build plus recurring-cost hints,
/// appended under the machine's report.
fn client_side_report(s: &crate::summary::Summary) -> String {
    let mut o = String::new();
    for f in crate::findings::e2e_findings(s) {
        o.push_str(&format!(
            "  waste   ~{:>5.1}s  {}\n",
            f.cost_secs, f.message
        ));
    }
    o.push_str(&crate::hints::render(&crate::hints::for_run(s)));
    o
}

/// `justrust remote check|test|...`: always remote (starting the machine if
/// it is stopped). Infrastructure failures are errors here, not fallbacks.
pub fn run(sub: &str, args: Vec<String>) -> Result<()> {
    if remote_backend::load().is_none() {
        report_remote_failure(sub, &RemoteDiagnostic::default(), 0.0);
        bail!("no remote machine configured. Sign in to hosted builds: `justrust login`");
    }
    match run_remote(sub, &args, true)? {
        Ok(o) => std::process::exit(o.code),
        Err(Infra(m)) => bail!("remote build unavailable: {m}"),
    }
}

#[cfg(test)]
#[path = "remote_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn remote_live_progress_tracks_split_stderr_without_consuming_output() {
        let mut live = super::LiveRun {
            dir: None,
            sniffer: crate::live::Sniffer::new(None, true),
        };
        live.stderr(b"    Blocking waiting for file ");
        live.stderr(b"lock on build directory\n");
        assert_eq!(live.sniffer.state.phase, "waiting for lock");
        live.stderr(b"    Compiling fixture v0.1.0\n");
        assert_eq!(live.sniffer.state.phase, "compiling");
        live.stderr(b"    Finished `test` profile\n     Running unittests src/lib.rs (fixture)\n");
        assert_eq!(live.sniffer.state.phase, "testing");
        assert!(live.sniffer.state.finished_at.is_some());
    }

    /// A sandbox with source but no Rust toolchain still finds what to
    /// mirror: the repository, plus a path dependency outside it.
    #[test]
    fn source_roots_without_cargo_follow_manifest_paths() {
        let t = std::env::temp_dir().join(format!("jr-roots-nocargo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (app, dep) = (t.join("app"), t.join("dep"));
        for d in [app.join("sub/src"), app.join(".git"), dep.join("src")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            app.join("sub/Cargo.toml"),
            "[package]\nname='sub'\nversion='0.1.0'\n[dependencies]\ndep={path=\"../../dep\"}\n",
        )
        .unwrap();
        std::fs::write(
            dep.join("Cargo.toml"),
            "[package]\nname='dep'\nversion='0.1.0'\n",
        )
        .unwrap();
        let missing = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::NotFound))
            .context("cargo metadata");
        assert!(super::cargo_missing(&missing));
        assert!(!super::cargo_missing(&anyhow::anyhow!(
            "cargo metadata failed"
        )));
        let mut roots = super::manifest_roots(&app.join("sub"));
        roots.sort();
        assert_eq!(roots, vec![app.clone(), dep.clone()]);
        let _ = std::fs::remove_dir_all(&t);
    }
    #[test]
    fn client_side_failures_are_distinguished_from_remote_ones() {
        use super::{CLIENT_SIDE, Infra};
        assert!(
            Infra(format!("{CLIENT_SIDE}cargo metadata failed: no Cargo.toml")).is_client_side()
        );
        assert!(!Infra("the hosted build service is unavailable".into()).is_client_side());
        assert!(!Infra(format!("{}signed out", super::BILLING)).is_client_side());
    }

    #[test]
    fn compile_output_streams_diagnostics_but_not_remote_artifacts() {
        let mut stream = super::CompileOutput::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        stream.push(b"{\"reason\":\"compiler-message\",\"message\":{\"rendered\":\"warning now\\n\"}}\n", &mut out, &mut err, false).unwrap();
        assert_eq!(err, b"warning now\n");
        assert!(stream.partial.is_empty());
        stream.push(b"{\"reason\":\"compiler-artifact\",\"executable\":\"/remote/test\"}\n{\"reason\":\"build-", &mut out, &mut err, false).unwrap();
        assert!(out.is_empty());
        assert!(stream.require_success().is_err());
        stream
            .push(b"finished\",\"success\":true}\n", &mut out, &mut err, false)
            .unwrap();
        assert!(stream.require_success().is_ok());
        assert!(stream.partial.is_empty());
        stream.push(b"{\"reason\":\"build-finished\",\"success\":false}\n{\"reason\":\"build-finished\",\"success\":true}\n", &mut out, &mut err, false).unwrap();
        assert!(stream.require_success().is_err());
        assert!(!String::from_utf8_lossy(&err).contains("/remote/test"));
    }

    #[test]
    fn compile_output_handles_json_chunks_and_bounded_lines() {
        let mut stream = super::CompileOutput::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let message =
            b"{\"reason\":\"compiler-message\",\"message\":{\"rendered\":\"error details\"}}";
        for byte in message {
            stream.push(&[*byte], &mut out, &mut err, true).unwrap();
        }
        assert!(out.is_empty());
        stream.finish(&mut out, &mut err, true).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out).unwrap()["message"]["rendered"],
            "error details"
        );
        assert!(err.is_empty());
        assert!(
            stream
                .push(
                    &vec![b'x'; super::CompileOutput::MAX_LINE + 1],
                    &mut out,
                    &mut err,
                    true
                )
                .is_err()
        );
    }

    #[test]
    fn compile_only_json_preserves_test_selection_and_separator() {
        let args = [
            "test",
            "-p",
            "fixture",
            "--no-run",
            "--message-format",
            "human",
            "--",
            "ignored",
        ]
        .map(str::to_owned);
        assert_eq!(
            super::compile_only_args(&args),
            [
                "test",
                "-p",
                "fixture",
                "--no-run",
                "--message-format=json-render-diagnostics",
                "--",
                "ignored"
            ]
            .map(str::to_owned)
        );
    }

    #[test]
    fn hosted_heartbeat_timeout_ticks_are_deterministic() {
        let mut ticks = [true, true, true, false].into_iter();
        let reports = std::cell::Cell::new(0);
        super::heartbeat_loop(|| ticks.next().unwrap(), || reports.set(reports.get() + 1));
        assert_eq!(reports.get(), 3);
    }

    #[test]
    fn hosted_heartbeat_terminal_states_stop_worker_and_preserve_verdict() {
        for code in [0, 101] {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut run = super::HostedRun::with_reporter(Some(std::sync::Arc::new(
                move |event: &str, code, failure: Option<&crate::failure::Wire>| {
                    tx.send((event.to_owned(), code, failure.cloned())).unwrap();
                },
            )));
            let why =
                (code != 0).then(|| (crate::failure::Class::UserCode, "2 tests failed".to_owned()));
            run.running();
            run.running();
            assert!(run.heartbeat.is_some());
            run.finish("done", Some(code), why.as_ref());
            assert!(run.heartbeat.is_none());
            run.running();
            run.finish("abandoned", None, None);
            drop(run);
            assert_eq!(
                rx.into_iter().collect::<Vec<_>>(),
                vec![
                    ("running".into(), None, None),
                    ("done".into(), Some(code), why),
                ]
            );
        }
    }

    #[test]
    fn hosted_heartbeat_drop_abandons_even_before_running() {
        for running in [false, true] {
            let (tx, rx) = std::sync::mpsc::channel();
            let result: anyhow::Result<()> = (|| {
                let mut run = super::HostedRun::with_reporter(Some(std::sync::Arc::new(
                    move |event: &str, code, _: Option<&crate::failure::Wire>| {
                        tx.send((event.to_owned(), code)).unwrap();
                    },
                )));
                if running {
                    run.running();
                }
                anyhow::bail!("unhandled transport error")
            })();
            assert!(result.is_err());
            let events = rx.into_iter().collect::<Vec<_>>();
            assert_eq!(events.last(), Some(&("abandoned".into(), None)));
            assert_eq!(events.len(), if running { 2 } else { 1 });
        }
    }

    #[test]
    fn hosted_heartbeat_non_hosted_never_spawns() {
        let mut run = super::HostedRun::with_reporter(None);
        run.running();
        assert!(run.heartbeat.is_none());
        run.finish("done", Some(101), None);
    }

    #[cfg(unix)]
    #[test]
    fn cancel_signal_unblocks_socket_and_abandons_hosted_run() {
        use std::io::Read;
        use std::os::unix::net::UnixStream;
        let (mut reader, _writer) = UnixStream::pair().unwrap();
        let _guard = super::cancel_signal::Guard::install();
        let _active = super::cancel_signal::ActiveSocket::register(&reader);
        // SAFETY: pthread_self() returns the handle for this test thread only,
        // so pthread_kill wakes only this thread without disturbing parallel tests.
        let target = unsafe { libc::pthread_self() } as usize;
        let sender = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(25));
            unsafe {
                libc::pthread_kill(target as libc::pthread_t, libc::SIGTERM);
            }
        });
        let started = std::time::Instant::now();
        let mut buf = [0u8; 8];
        let read_result = reader.read(&mut buf);
        sender.join().unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "SIGTERM should immediately unblock the daemon socket read"
        );
        assert!(
            matches!(read_result, Ok(0) | Err(_)),
            "shutdown socket should return EOF or error, got {read_result:?}"
        );
        let sig = super::cancel_signal::received();
        assert_eq!(sig, Some(libc::SIGTERM));
        assert_eq!(
            super::cancel_signal::reason(libc::SIGTERM, true),
            "prebuild cancelled (SIGTERM)"
        );
        assert_eq!(
            super::cancel_signal::reason(libc::SIGINT, false),
            "cancelled (SIGINT)"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let mut run = super::HostedRun::with_reporter(Some(std::sync::Arc::new(
            move |event: &str, code, failure: Option<&crate::failure::Wire>| {
                tx.send((event.to_owned(), code, failure.cloned())).unwrap();
            },
        )));
        run.running();
        let sig = sig.unwrap();
        let why = (
            crate::failure::Class::Interrupted,
            super::cancel_signal::reason(sig, true),
        );
        run.finish("abandoned", Some(128 + sig), Some(&why));
        drop(run);
        assert_eq!(
            rx.into_iter().collect::<Vec<_>>(),
            vec![
                ("running".into(), None, None),
                (
                    "abandoned".into(),
                    Some(128 + libc::SIGTERM),
                    Some((
                        crate::failure::Class::Interrupted,
                        "prebuild cancelled (SIGTERM)".into()
                    ))
                ),
            ]
        );
    }

    #[test]
    fn service_refusals_are_labeled_apart_from_transport_failures() {
        use crate::remote_hosted::{AT_CAPACITY, UNAVAILABLE};
        let storage = format!(
            "{UNAVAILABLE} (server: Hosted storage capacity is being verified. Retry shortly.)"
        );
        assert_eq!(service_refusal(&storage), Some("host storage unavailable"));
        let corrupt =
            format!("{UNAVAILABLE} (server: Your workspace filesystem requires recovery.)");
        assert_eq!(
            service_refusal(&corrupt),
            Some("workspace storage needs recovery")
        );
        assert_eq!(service_refusal(AT_CAPACITY), Some("fleet at capacity"));
        assert_eq!(service_refusal(UNAVAILABLE), Some("service unavailable"));
        assert_eq!(service_refusal("sync daemon closed the connection"), None);
        assert_eq!(service_refusal("ssh: connection refused"), None);
    }

    #[test]
    fn remote_failure_diagnostics_preserve_known_outcome_without_private_text() {
        let d = RemoteDiagnostic {
            stage: "remote_artifacts",
            may_have_started: true,
            exit: Some(0),
            ..RemoteDiagnostic::default()
        };
        let data = remote_failure_data("private-subcommand", &d, 1.234);
        assert_eq!(data["sub"], "other");
        assert_eq!(data["stage"], "remote_artifacts");
        assert_eq!(data["exit"], 0);
        assert_eq!(data["elapsed_s"], 1.23);
        assert_eq!(data["may_have_started"], true);
        assert!(!data.to_string().contains("private"));
        assert!(data.get("fallback_safe").is_none());
    }

    #[test]
    fn transport_error_labels_preserve_typed_causes_through_context() {
        for (kind, expected) in [
            (std::io::ErrorKind::ConnectionReset, "connection_reset"),
            (std::io::ErrorKind::ConnectionRefused, "connection_refused"),
            (std::io::ErrorKind::UnexpectedEof, "connection_closed"),
            (std::io::ErrorKind::TimedOut, "timeout"),
            (std::io::ErrorKind::Other, "io"),
        ] {
            let error = anyhow::Error::new(std::io::Error::new(kind, "private/path token=secret"))
                .context("private endpoint");
            assert_eq!(transport_error_kind(&error), expected);
        }
        let error =
            serde_json::from_str::<serde_json::Value>("private malformed data").unwrap_err();
        assert_eq!(transport_error_kind(&error.into()), "protocol");
        assert_eq!(
            transport_failure_kind("connection to the remote machine was lost: connection_reset"),
            "connection_reset"
        );
        assert_eq!(
            transport_failure_kind("connection to the remote machine was lost: secret"),
            "unknown"
        );
    }

    #[test]
    fn transport_failure_labels_never_include_private_error_text() {
        for (message, expected) in [
            (
                "ssh /home/private/token: Connection refused",
                "connection_refused",
            ),
            (
                "read: Connection reset by peer secret=abc",
                "connection_reset",
            ),
            ("write: Broken pipe /private/workspace", "connection_reset"),
            ("connect timed out to private.example", "timeout"),
            ("ssh: Could not resolve hostname private.example", "dns"),
            ("Permission denied (publickey) secret", "authentication"),
            ("sync daemon closed the connection", "connection_closed"),
            ("unrecognized error with source and secrets", "unknown"),
        ] {
            assert_eq!(transport_failure_kind(message), expected);
            let diagnostic = RemoteDiagnostic {
                run_id: Some("20261116-185118312-1459018-pre".into()),
                failure_kind: transport_failure_kind(message),
                transport_phase: "source_sync",
                ..RemoteDiagnostic::default()
            };
            let data = remote_failure_data("test", &diagnostic, 64.48);
            assert_eq!(data["run_id"], "20261116-185118312-1459018-pre");
            assert_eq!(data["failure_kind"], expected);
            assert_eq!(data["transport_phase"], "source_sync");
            assert!(!data.to_string().contains(message));
        }
    }

    #[test]
    fn malformed_remote_summary_is_reportable_not_silently_successful() {
        assert!(!record_remote(
            "unused",
            b"not-json",
            &[],
            "private-machine",
            "remote",
            0.0,
            None,
            None,
            0.0
        ));
    }

    #[test]
    fn attached_remote_run_does_not_duplicate_index_entry() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap();
        let home =
            std::env::temp_dir().join(format!("justrust-attached-run-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let prev = std::env::var_os("JUSTRUST_HOME");
        unsafe { std::env::set_var("JUSTRUST_HOME", &home) };
        let summary = crate::summary::Summary {
            id: "20261126-180033110-1442668".into(),
            subcommand: "test".into(),
            args: vec!["test".into(), "--lib".into()],
            cwd: "/tmp/proj".into(),
            wall: 5.0,
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&summary).unwrap();
        assert!(record_remote(
            "20261126-180033110-1442668",
            &bytes,
            &[],
            "hosted",
            "hosted",
            10.0,
            None,
            None,
            5.1,
        ));
        assert!(record_remote(
            "20261126-180033989-1442833",
            &bytes,
            &[],
            "hosted",
            "hosted",
            10.0,
            None,
            None,
            4.2,
        ));
        let index = std::fs::read_to_string(home.join("index.jsonl")).unwrap();
        assert_eq!(
            index.lines().count(),
            1,
            "attached run must not append a duplicate index line: {index}"
        );
        match prev {
            Some(v) => unsafe { std::env::set_var("JUSTRUST_HOME", v) },
            None => unsafe { std::env::remove_var("JUSTRUST_HOME") },
        }
        let _ = std::fs::remove_dir_all(&home);
    }
    #[test]
    fn ambiguous_submission_never_falls_back_but_confirmed_rejection_can() {
        assert!(super::fallback_safe(false, false));
        assert!(!super::fallback_safe(false, true));
        assert!(!super::fallback_safe(true, true));
        assert!(!super::fallback_safe(true, false));
        let mut submitted = false;
        assert!(super::fallback_safe(false, submitted));
        submitted = true;
        assert!(!super::fallback_safe(false, submitted));
        // An explicit failed-sync acknowledgement proves no run launched.
        submitted = false;
        assert!(super::fallback_safe(false, submitted));
    }
    use super::*;

    #[test]
    fn hosted_release_env_preserves_exact_values_over_transport() {
        let controls = [
            ("CARGO_INCREMENTAL", "0"),
            ("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3"),
            ("CARGO_PROFILE_RELEASE_DEBUG", "line-tables-only"),
            ("CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO", "packed"),
            ("CARGO_PROFILE_RELEASE_STRIP", "symbols"),
            ("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "false"),
            ("CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS", "true"),
            ("CARGO_PROFILE_RELEASE_LTO", "thin"),
            ("CARGO_PROFILE_RELEASE_PANIC", "abort"),
            ("CARGO_PROFILE_RELEASE_INCREMENTAL", "false"),
            ("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1"),
            ("CARGO_PROFILE_RELEASE_RPATH", "false"),
            ("CARGO_BUILD_RUSTC_WRAPPER", ""),
            (
                "JCODE_BUILD_GIT_HASH",
                "0123456789abcdef0123456789abcdef01234567",
            ),
            ("JCODE_BUILD_GIT_DATE", "2026-10-26T21:24:12Z"),
            ("JCODE_BUILD_GIT_TAG", "v1.2.3"),
            ("JCODE_BUILD_GIT_DIRTY", "false"),
        ];
        let expected: Vec<_> = controls
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env = forwarded_env(|name| {
            controls
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        });
        let mut sorted_env = env.clone();
        sorted_env.sort();
        let mut expected = expected;
        expected.sort();
        assert_eq!(sorted_env, expected);
        for msg in [
            Msg::Run {
                id: 1,
                cwd: "/project".into(),
                args: vec!["build".into(), "--release".into()],
                env: env.clone(),
                roots: vec![],
                start: false,
                run_id: "release-env".into(),
            },
            Msg::RunSync {
                id: 1,
                cwd: "/project".into(),
                args: vec!["build".into(), "--release".into()],
                env: env.clone(),
                roots: vec![],
                start: false,
                run_id: "release-env".into(),
            },
        ] {
            let mut wire = Vec::new();
            write_frame(&mut wire, &msg, &[]).unwrap();
            let (decoded, _) = read_frame(&mut std::io::Cursor::new(wire))
                .unwrap()
                .unwrap();
            assert_eq!(decoded, msg);
        }
    }

    #[test]
    fn hosted_env_omits_missing_controls_secrets_and_nonempty_wrappers() {
        assert!(forwarded_env(|_| None).is_empty());
        for wrapper in ["/home/user/bin/sccache", "sccache", " "] {
            let input = [
                ("CARGO_BUILD_RUSTC_WRAPPER", wrapper),
                ("RUSTC_WRAPPER", wrapper),
                ("RUSTC_WORKSPACE_WRAPPER", wrapper),
                ("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", wrapper),
                ("CARGO_PROFILE_RELEASE_UNKNOWN", "secret"),
                ("JCODE_BUILD_GIT_TOKEN", "secret"),
                ("JCODE_BUILD_GIT_DIR", "/private/repo/.git"),
                ("CARGO_REGISTRIES_PRIVATE_TOKEN", "secret"),
                ("AWS_SECRET_ACCESS_KEY", "secret"),
            ];
            assert!(
                forwarded_env(|name| {
                    input
                        .iter()
                        .find(|(k, _)| *k == name)
                        .map(|(_, v)| v.to_string())
                })
                .is_empty()
            );
        }
        // False, zero, and empty values must not be mistaken for absent values.
        for value in ["0", "false", "", "fat", "z"] {
            assert_eq!(
                forwarded_env(|name| (name == "CARGO_PROFILE_RELEASE_LTO").then(|| value.into())),
                vec![("CARGO_PROFILE_RELEASE_LTO".into(), value.into())],
            );
        }
    }

    #[test]
    fn hosted_preserves_parallelism_limits_and_opt_outs_verbatim() {
        for name in [
            "CARGO_BUILD_JOBS",
            "RUST_TEST_THREADS",
            "JCODE_FRONTEND_THREADS",
            "JCODE_PARALLEL_FRONTEND",
            "JUSTRUST_FRONTEND_THREADS",
            "JUSTRUST_TEST_PARALLEL",
            "JUSTRUST_TEST_JOBS",
            "JUSTRUST_TEST_SPLIT",
            "JUSTRUST_CARGO_SUBTREE_CACHE",
            "JUSTRUST_SLOTS",
            "JUSTRUST_SLOT_REFRESH",
            "JUSTRUST_SLOTS_GC",
            "JUSTRUST_SLOTS_MAX_AGE_DAYS",
            "JUSTRUST_SLOTS_MAX_GB",
            "JUSTRUST_JOIN",
            "JUSTRUST_JOIN_WAIT",
            "JUSTRUST_SLOT_WARM_REUSE",
            "JUSTRUST_SLOT_SESSION",
        ] {
            for value in ["1", "8", "0", "off", ""] {
                assert_eq!(
                    forwarded_env(|key| (key == name).then(|| value.to_owned())),
                    vec![(name.to_owned(), value.to_owned())],
                    "{name}={value}"
                );
            }
        }
    }

    #[test]
    fn hosted_forwards_explicit_optimization_controls_not_secrets_or_local_paths() {
        for var in [
            "JUSTRUST_RELEASE_BUILD",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_JOBS",
            "RUST_TEST_THREADS",
            "JCODE_FRONTEND_THREADS",
            "JCODE_PARALLEL_FRONTEND",
            "JUSTRUST_FRONTEND_THREADS",
            "JUSTRUST_TEST_PARALLEL",
            "JUSTRUST_TEST_JOBS",
            "JUSTRUST_TEST_SPLIT",
            "RUSTUP_TOOLCHAIN",
            "JCODE_DESKTOP_VERSION",
            "JCODE_DESKTOP_BUILD_METADATA",
            "SOURCE_DATE_EPOCH",
            "JCODE_RELEASE_BUILD",
            "JCODE_BUILD_METADATA_FILE",
            "JCODE_BUILD_SEMVER",
            "JCODE_BUILD_GIT_HASH",
            "JCODE_BUILD_GIT_DATE",
            "JCODE_BUILD_GIT_TAG",
            "JCODE_BUILD_GIT_DIRTY",
            "JCODE_BUILD_CHANGELOG_RAW",
            "JUSTRUST_TOOLCHAIN_DIST",
            "JUSTRUST_TOOLCHAIN_REQUIRED",
            "JUSTRUST_TOOLCHAIN_AUTO_INSTALL",
            "JUSTRUST_SHADOW",
            "JUSTRUST_LOCALCACHE",
            "JUSTRUST_LOCALCACHE_TARGET_BOUND",
            "JUSTRUST_CACHE_PROC_MACROS",
            "JUSTRUST_CUTOFF",
            "JUSTRUST_CARGO_CUTOFF",
            "JUSTRUST_CARGO_SUBTREE_CACHE",
            "JUSTRUST_CARGO_FORK",
            "JUSTRUST_CARGO_DAEMON",
            "JUSTRUST_CARGO_DAEMON_TRACE",
            "JUSTRUST_UNIFY_FEATURES",
            "JUSTRUST_EARLY",
            "JUSTRUST_EARLY_EXPERIMENTAL",
        ] {
            assert!(
                FORWARDED_ENV.contains(&var),
                "missing hosted control: {var}"
            );
        }
        for var in [
            "JCODE_API_KEY",
            "CACHE_PUBLISH_TOKEN",
            "BUILDS_INTERNAL_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "JUSTRUST_HOME",
            "JUSTRUST_LOCALCACHE_DIR",
            "CARGO_TARGET_DIR",
            "CARGO_BUILD_TARGET_DIR",
            "CARGO_BUILD_BUILD_DIR",
            "JUSTRUST_RUN_DIR",
            "JUSTRUST_REAL_RUSTC",
            "JUSTRUST_SHADOW_CHILD",
            "JUSTRUST_EARLY_EXIT",
            "JUSTRUST_EARLY_LAUNCHER",
            "JUSTRUST_EARLY_RESOURCE_LIMITED",
        ] {
            assert!(
                !FORWARDED_ENV.contains(&var),
                "unsafe hosted control: {var}"
            );
        }
        let unique: BTreeSet<_> = FORWARDED_ENV.iter().collect();
        assert_eq!(unique.len(), FORWARDED_ENV.len());
    }

    #[test]
    fn durable_hosted_sessions_never_trust_fleet_semver_as_capability() {
        for hosted in [false, true] {
            for dev_mode in [false, true] {
                assert!(!fleet_policy_allows(hosted, dev_mode, true));
                assert_eq!(
                    fleet_policy_allows(hosted, dev_mode, false),
                    hosted && !dev_mode
                );
            }
        }
        // An equal advertised release can predate the private session command.
        assert!(fleet_usable("0.1.0-alpha.23", "0.1.0-alpha.23"));
        assert!(!fleet_policy_allows(true, false, true));
        assert!(fleet_policy_allows(true, false, false));
    }

    #[cfg(unix)]
    #[test]
    fn pinned_session_executable_receipts_reject_identity_and_temp_tampering() {
        let digest = "a".repeat(64);
        let pin = PinnedSessionExecutable {
            path: format!("/home/test/.justrust/remote/session-executables/{digest}/justrust"),
            sha256: digest.clone(),
        };
        pin.validate().unwrap();
        let parent = Path::new(&pin.path).parent().unwrap().display();
        let temp = format!("{parent}/.upload.Abc1234567");
        let script = session_executable_publish_script(&pin, &temp).unwrap();
        assert!(script.contains("ln \"$tmp\" \"$f\""));
        assert!(!script.contains("mv "));
        assert!(script.contains("[ ! -L \"$tmp\" ]"));
        assert!(script.contains("[ -O \"$f\" ]"));
        assert!(script.contains(&format!("= '{digest}'")));
        for path in [
            "/tmp/.upload.Abc1234567".into(),
            format!("{parent}/../.upload.Abc1234567"),
            format!("{parent}/justrust"),
        ] {
            assert!(session_executable_publish_script(&pin, &path).is_err());
        }
        for path in [
            "/home/test/.cargo/bin/justrust".into(),
            pin.path.replace(&digest, &"b".repeat(64)),
            format!("/tmp/../{}", pin.path),
        ] {
            assert!(
                PinnedSessionExecutable {
                    path,
                    sha256: digest.clone()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[cfg(target_os = "linux")]
    struct SessionExecutableFixture(PathBuf);

    #[cfg(target_os = "linux")]
    impl SessionExecutableFixture {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "jr-session-executable-{}-{nonce}",
                std::process::id()
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for SessionExecutableFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn private_session_executable_survives_shared_replacement_and_refuses_corruption() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let home = SessionExecutableFixture::new();
        let bin = home.path().join(".cargo/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let shared = bin.join("justrust");
        std::fs::write(&shared, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        let digest = crate::toolchain::sha256(&shared).unwrap();
        let script = session_executable_prepare_script(&digest);
        let run = || {
            Command::new("sh")
                .args(["-c", &script])
                .env("HOME", home.path())
                .output()
                .unwrap()
        };
        let first = run();
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        let receipt = String::from_utf8(first.stdout).unwrap();
        let fields: Vec<_> = receipt.lines().collect();
        assert_eq!(fields[0], "READY");
        let pinned = Path::new(fields[1]);
        let parent = pinned.parent().unwrap();
        assert_eq!(
            std::fs::metadata(parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(pinned).unwrap().permissions().mode() & 0o777,
            0o500
        );
        std::fs::write(&shared, "legacy replacement").unwrap();
        assert!(run().status.success());
        assert_eq!(crate::toolchain::sha256(pinned).unwrap(), digest);
        assert_eq!(std::fs::read_dir(parent).unwrap().count(), 1);
        std::fs::remove_file(pinned).unwrap();
        symlink(&shared, pinned).unwrap();
        assert!(!run().status.success());
        std::fs::remove_file(pinned).unwrap();
        std::fs::write(pinned, "corrupt").unwrap();
        std::fs::set_permissions(pinned, std::fs::Permissions::from_mode(0o500)).unwrap();
        assert!(!run().status.success());
        std::fs::remove_file(pinned).unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!run().status.success());
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir(parent).unwrap();
        let unrelated = home.path().join("unrelated");
        std::fs::create_dir(&unrelated).unwrap();
        symlink(&unrelated, parent).unwrap();
        assert!(!run().status.success());
        assert_eq!(std::fs::read_dir(unrelated).unwrap().count(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn private_session_executable_upload_is_verified_and_temp_is_consumed() {
        let home = SessionExecutableFixture::new();
        let source = home.path().join("exact-client");
        std::fs::write(&source, "exact immutable executable bytes").unwrap();
        let digest = crate::toolchain::sha256(&source).unwrap();
        let result = Command::new("sh")
            .args(["-c", &session_executable_prepare_script(&digest)])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(result.status.success());
        let receipt = String::from_utf8(result.stdout).unwrap();
        let fields: Vec<_> = receipt.lines().collect();
        assert_eq!(fields[0], "UPLOAD");
        let pin = PinnedSessionExecutable {
            path: fields[1].into(),
            sha256: digest,
        };
        let publish = session_executable_publish_script(&pin, fields[2]).unwrap();
        std::fs::copy(&source, fields[2]).unwrap();
        assert!(
            Command::new("sh")
                .args(["-c", &publish])
                .status()
                .unwrap()
                .success()
        );
        assert!(!Path::new(fields[2]).exists());
        assert_eq!(
            crate::toolchain::sha256(Path::new(&pin.path)).unwrap(),
            pin.sha256
        );
        let wrong_digest = "b".repeat(64);
        let result = Command::new("sh")
            .args(["-c", &session_executable_prepare_script(&wrong_digest)])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(result.status.success());
        let receipt = String::from_utf8(result.stdout).unwrap();
        let fields: Vec<_> = receipt.lines().collect();
        let wrong_pin = PinnedSessionExecutable {
            path: fields[1].into(),
            sha256: wrong_digest,
        };
        std::fs::copy(&source, fields[2]).unwrap();
        let script = session_executable_publish_script(&wrong_pin, fields[2]).unwrap();
        assert!(
            !Command::new("sh")
                .args(["-c", &script])
                .status()
                .unwrap()
                .success()
        );
        assert!(!Path::new(fields[2]).exists());
        assert!(!Path::new(&wrong_pin.path).exists());
    }

    /// `sh -c script` with `home` as HOME. A script this process has just
    /// written cannot be executed while another test thread is between fork
    /// and exec with the write descriptor still open ("text file busy",
    /// exit 126): that is this test binary's doing, so it is tried again.
    #[cfg(unix)]
    fn sh_at(home: &Path, script: &str) -> std::process::Output {
        let mut tries = 0;
        loop {
            let out = Command::new("sh")
                .args(["-c", script])
                .env("HOME", home)
                .output()
                .unwrap();
            tries += 1;
            if out.status.code() != Some(126) || tries == 100 {
                return out;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    /// A fleet client (legacy transport) and a client that installs its own
    /// build share one machine and one shared path.
    #[test]
    fn a_fleet_client_and_an_own_build_alternate_without_reinstalling() {
        let home = std::env::temp_dir().join(format!("jr-fleet-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".cargo/bin")).unwrap();
        let sh = |script: &str| sh_at(&home, script);
        let shared = home.join(".cargo/bin/justrust");
        let fleet = "#!/bin/sh\necho justrust 9.9.9\n";
        let own = "#!/bin/sh\necho justrust 9.9.10-dev\n";
        let staged = home.join("staged");
        // What `release_install_script` does once its download is verified.
        let install_fleet = || {
            std::fs::write(&staged, fleet).unwrap();
            crate::sys::set_mode(&staged, 0o755).unwrap();
            let done = sh(&format!(
                "set -e; f=\"$HOME/staged\"; {}mv \"$f\" ~/.cargo/bin/justrust",
                keep_fleet_copy_step("9.9.9")
            ));
            assert!(done.status.success(), "{done:?}");
        };
        let install_own = || {
            std::fs::write(&shared, own).unwrap();
            crate::sys::set_mode(&shared, 0o755).unwrap();
        };
        let script = release_install_script("Linux x86_64", Some("9.9.9"), true).unwrap();
        assert!(script.contains(&keep_fleet_copy_step("9.9.9")));
        assert!(
            script.find("fleet-executables").unwrap() < script.find("mv \"$stage\"").unwrap(),
            "the copy is taken before the shared path is replaced"
        );
        assert!(
            !cross_release_install_script("Linux x86_64", Some("9.9.9"))
                .unwrap()
                .contains("fleet-executables")
        );
        // One home, the stamp names the last installer.
        let (fleet_stamp, own_stamp) = ("host:22 fleet 9.9.9", "host:22 sha256-abc");
        let mut stamp = "";
        let (mut before, mut after) = (0, 0);
        for turn in [fleet_stamp, own_stamp, fleet_stamp, own_stamp, fleet_stamp] {
            if stamp != turn {
                stamp = turn;
                if turn == fleet_stamp {
                    before += 1;
                    if !sh(&fleet_copy_script("9.9.9")).status.success() {
                        after += 1;
                        install_fleet();
                    }
                } else {
                    // No own copy without a durable session: the shared path.
                    install_own();
                }
            }
            let run = sh(&machine_justrust_from(Some(turn), None));
            assert_eq!(
                String::from_utf8_lossy(&run.stdout).trim(),
                if turn == fleet_stamp {
                    "justrust 9.9.9"
                } else {
                    "justrust 9.9.10-dev"
                },
                "each client reaches its own justrust, whoever installed last"
            );
        }
        assert_eq!(
            (before, after),
            (3, 1),
            "fleet installs: every turn before, once now"
        );
        // A fleet version the machine was never given is not there, and its
        // client falls back to the shared path until it is installed.
        assert!(!sh(&fleet_copy_script("9.9.8")).status.success());
        let run = sh(&machine_justrust_from(Some("host:22 fleet 9.9.8"), None));
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "justrust 9.9.10-dev"
        );
        // A copy that is not the version its directory names does not count.
        let forged = home.join(".justrust/remote/fleet-executables/9.9.8");
        std::fs::create_dir_all(&forged).unwrap();
        std::fs::write(forged.join("justrust"), fleet).unwrap();
        crate::sys::set_mode(&forged.join("justrust"), 0o500).unwrap();
        assert!(!sh(&fleet_copy_script("9.9.8")).status.success());
        assert!(sh(&fleet_copy_script("9.9.9")).status.success());
        // Only a well-formed fleet stamp selects a fleet copy.
        assert_eq!(fleet_of_stamp("host:22 fleet 0.1.69"), Some("0.1.69"));
        assert_eq!(fleet_of_stamp("host:22 sha256-abc"), None);
        assert_eq!(fleet_of_stamp("host:22 fleet 1.0; rm -rf ~"), None);
        assert_eq!(fleet_of_stamp("host:22 fleet "), None);
        assert_eq!(
            machine_justrust_from(Some("host:22 sha256-abc"), None),
            "~/.cargo/bin/justrust"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn two_client_versions_alternate_without_reinstalling() {
        let home = std::env::temp_dir().join(format!("jr-own-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let copies = home.join(".justrust/remote/session-executables");
        std::fs::create_dir_all(home.join(".cargo/bin")).unwrap();
        std::fs::create_dir_all(&copies).unwrap();
        let sh = |script: &str| sh_at(&home, script);
        let digest = |body: &str| {
            use sha2::Digest;
            sha2::Sha256::digest(body)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let shared = home.join(".cargo/bin/justrust");
        // What an install does on the machine: replace the shared path, and
        // (session preparation) keep an exact copy under its digest.
        let install = |body: &str| {
            std::fs::write(&shared, body).unwrap();
            crate::sys::set_mode(&shared, 0o755).unwrap();
            let dir = copies.join(digest(body));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("justrust"), body).unwrap();
            crate::sys::set_mode(&dir.join("justrust"), 0o500).unwrap();
        };
        let (a, b) = ("#!/bin/sh\necho client-a\n", "#!/bin/sh\necho client-b\n");
        // One home, two clients taking turns: the stamp names the last one.
        let mut stamp = String::new();
        let (mut before, mut after) = (0, 0);
        for body in [a, b, a, b, a, b] {
            let sha = digest(body);
            if stamp != sha {
                before += 1;
                if !sh(&own_copy_script(&sha)).status.success() {
                    after += 1;
                    install(body);
                }
                stamp = sha.clone();
            }
            // Each client reaches its own bytes, whoever installed last.
            let run = sh(&format!("{} x", machine_justrust_for(Some(&sha))));
            assert_eq!(
                String::from_utf8_lossy(&run.stdout).trim(),
                if body == a { "client-a" } else { "client-b" }
            );
        }
        assert_eq!(
            (before, after),
            (6, 2),
            "installs: every turn before, once each now"
        );
        // The shared path still works, for whatever else calls it.
        assert!(sh("~/.cargo/bin/justrust").status.success());
        // A client the machine has never seen falls back to the shared path.
        let unknown = digest("never installed");
        assert!(!sh(&own_copy_script(&unknown)).status.success());
        let run = sh(&format!("{} x", machine_justrust_for(Some(&unknown))));
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "client-b");
        assert_eq!(machine_justrust_for(None), "~/.cargo/bin/justrust");
        // A copy whose bytes are not its name is not ours.
        let forged = copies.join(&unknown);
        std::fs::create_dir_all(&forged).unwrap();
        std::fs::write(forged.join("justrust"), a).unwrap();
        crate::sys::set_mode(&forged.join("justrust"), 0o500).unwrap();
        assert!(!sh(&own_copy_script(&unknown)).status.success());
        // Copies unused for two weeks go, the one in use stays.
        std::fs::File::open(copies.join(digest(b)))
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::SystemTime::now() - Duration::from_secs(20 * 86400)),
            )
            .unwrap();
        assert!(sh(&own_copy_script(&digest(a))).status.success());
        assert!(copies.join(digest(a)).join("justrust").exists());
        assert!(!copies.join(digest(b)).exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_silent_setup_speaks_within_seconds_and_only_connecting_is_bounded() {
        let s = Duration::from_secs;
        let bound = Some(s(300));
        assert_eq!(setup_silence("connecting", s(9), 0, bound), Silence::Quiet);
        assert_eq!(setup_silence("connecting", s(10), 0, bound), Silence::Say);
        assert_eq!(setup_silence("connecting", s(11), 1, bound), Silence::Quiet);
        assert_eq!(setup_silence("connecting", s(30), 1, bound), Silence::Say);
        assert_eq!(setup_silence("connecting", s(60), 2, bound), Silence::Say);
        assert_eq!(
            setup_silence("connecting", s(119), 3, bound),
            Silence::Quiet
        );
        assert_eq!(setup_silence("connecting", s(120), 3, bound), Silence::Say);
        assert_eq!(
            setup_silence("connecting", s(300), 6, bound),
            Silence::GiveUp
        );
        assert_eq!(
            setup_silence("connecting", s(9000), 200, None),
            Silence::Quiet
        );
        // A long lock wait or mirror is announced, never cut off.
        for phase in ["waiting for sync lock", "syncing", "applying sources"] {
            assert_eq!(setup_silence(phase, s(10), 0, bound), Silence::Say);
            assert_eq!(setup_silence(phase, s(1500), 26, bound), Silence::Say);
            assert_eq!(setup_silence(phase, s(1500), 27, bound), Silence::Quiet);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_connect_past_its_bound_wakes_the_blocked_read() {
        use std::io::Read;
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let watch = SetupWatch(Arc::new((
            Mutex::new(SetupState {
                phase: "connecting".into(),
                since: Instant::now() - Duration::from_secs(400),
                notices: 99,
                done: false,
                gave_up: None,
            }),
            Condvar::new(),
        )));
        // The same step the watch thread takes on its bound.
        {
            let mut st = watch.0.0.lock().unwrap();
            assert_eq!(
                setup_silence(
                    &st.phase,
                    st.since.elapsed(),
                    st.notices,
                    Some(Duration::from_secs(300))
                ),
                Silence::GiveUp
            );
            st.gave_up = Some(st.since.elapsed().as_secs());
            ours.shutdown(std::net::Shutdown::Both).unwrap();
        }
        let mut byte = [0u8; 1];
        assert_eq!((&ours).read(&mut byte).unwrap(), 0, "the read returns");
        assert!(watch.gave_up().is_some_and(|secs| secs >= 400));
        // A new phase restarts the clock and the notices.
        watch.phase("syncing");
        let st = watch.0.0.lock().unwrap();
        assert!(st.since.elapsed() < Duration::from_secs(5) && st.notices == 0);
    }

    #[test]
    fn stamp_survives_unless_the_machine_reports_another_binary() {
        let own = ("Linux x86_64", "aaaa", "justrust 0.1.68");
        let reply = |platform: &str, sha: &str, version: &str| {
            format!("{platform}\n{sha}\n{version}\nanswered\n")
        };
        // Fleet stamp: the version decides, whatever the bytes.
        let fleet = "host:22 fleet 0.1.68";
        assert!(stamp_describes(
            fleet,
            "host:22",
            own,
            &reply("Linux x86_64", "bbbb", "justrust 0.1.68")
        ));
        assert!(!stamp_describes(
            fleet,
            "host:22",
            own,
            &reply("Linux x86_64", "aaaa", "justrust 0.1.67")
        ));
        // Own build: exact bytes, or this version built for another platform.
        let mine = "host:22 0.1.68-abc";
        assert!(stamp_describes(
            mine,
            "host:22",
            own,
            &reply("Linux x86_64", "aaaa", "justrust 0.1.68")
        ));
        assert!(!stamp_describes(
            mine,
            "host:22",
            own,
            &reply("Linux x86_64", "bbbb", "justrust 0.1.68")
        ));
        assert!(stamp_describes(
            mine,
            "host:22",
            own,
            &reply("Darwin arm64", "bbbb", "justrust 0.1.68")
        ));
        assert!(!stamp_describes(
            mine,
            "host:22",
            own,
            &reply("Darwin arm64", "bbbb", "justrust 0.1.67")
        ));
        // No binary there, a stamp for another machine, a short reply.
        assert!(!stamp_describes(
            mine,
            "host:22",
            own,
            &reply("Linux x86_64", "-", "-")
        ));
        assert!(!stamp_describes(
            mine,
            "other:22",
            own,
            &reply("Linux x86_64", "aaaa", "justrust 0.1.68")
        ));
        assert!(!stamp_describes(mine, "host:22", own, "Linux x86_64\n"));
    }

    #[test]
    fn fleet_install_only_when_not_behind_the_client() {
        assert!(fleet_usable("0.1.0-alpha.21", "0.1.0-alpha.21"));
        assert!(fleet_usable("0.1.0-alpha.22", "0.1.0-alpha.21"));
        assert!(!fleet_usable("0.1.0-alpha.20", "0.1.0-alpha.21"));
        let s = cross_release_install_script("Linux x86_64", Some("0.1.0-alpha.9")).unwrap();
        assert!(s.contains("/v0.1.0-alpha.9/justrust-x86_64-unknown-linux-musl.tar.gz"));
        assert!(s.contains("'justrust 0.1.0-alpha.9'") || s.contains("justrust 0.1.0-alpha.9"));
    }

    #[test]
    fn cross_release_install_uses_host_target_and_verifies_before_replacing() {
        for (arch, target) in [
            ("Linux x86_64", "x86_64-unknown-linux-musl"),
            ("Linux aarch64", "aarch64-unknown-linux-musl"),
        ] {
            let script =
                cross_release_install_script(arch, Some(env!("CARGO_PKG_VERSION"))).unwrap();
            assert!(script.contains(&format!("/v{}/", env!("CARGO_PKG_VERSION"))));
            assert!(script.contains(&format!("justrust-{target}.tar.gz.sha256")));
            let checksum = script.find("sha256sum -c -").unwrap();
            let extract = script.find("tar xzf").unwrap();
            let verify_version = script.find("--version").unwrap();
            let replace = script.find("mv \"$f\"").unwrap();
            assert!(checksum < extract && extract < verify_version && verify_version < replace);
            assert!(script.contains(&shell_quote(&format!(
                "justrust {}",
                env!("CARGO_PKG_VERSION")
            ))));
            assert!(cross_release_install_script(arch, None).is_none());
        }
        assert!(cross_release_install_script("Darwin arm64", Some("1.0.0")).is_none());
        assert!(cross_release_install_script("Linux riscv64", Some("1.0.0")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn cross_release_install_rejects_bad_downloads_without_replacing_the_agent() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("jr-cross-release-{}", std::process::id()));
        let bin = root.join("bin");
        let home = root.join("home");
        let package = root.join("justrust-x86_64-unknown-linux-musl");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(home.join(".cargo/bin")).unwrap();
        std::fs::create_dir_all(&package).unwrap();
        let installed = home.join(".cargo/bin/justrust");
        std::fs::write(&installed, "original agent").unwrap();
        let curl = bin.join("curl");
        std::fs::write(
            &curl,
            "#!/bin/sh\nfor arg; do out=$arg; done\ncase \"$out\" in */checksum) cp \"$JR_TEST_CHECKSUM\" \"$out\";; *) cp \"$JR_TEST_ARCHIVE\" \"$out\";; esac\n",
        )
        .unwrap();
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The production script runs on Linux. A native Mac test exercises
        // it with the equivalent checksum tool instead of requiring GNU
        // coreutils to be installed on the client.
        if cfg!(target_os = "macos") {
            let sha = bin.join("sha256sum");
            std::fs::write(&sha, "#!/bin/sh\nexec shasum -a 256 \"$@\"\n").unwrap();
            std::fs::set_permissions(&sha, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let archive = root.join("release.tar.gz");
        let checksum = root.join("release.sha256");
        let executable = package.join("justrust");
        let pack = |version: &str| {
            std::fs::write(
                &executable,
                format!("#!/bin/sh\necho 'justrust {version}'\n"),
            )
            .unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                Command::new("tar")
                    .arg("czf")
                    .arg(&archive)
                    .arg("-C")
                    .arg(&root)
                    .arg("justrust-x86_64-unknown-linux-musl")
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::write(
                &checksum,
                format!(
                    "{}  release.tar.gz\n",
                    crate::toolchain::sha256(&archive).unwrap()
                ),
            )
            .unwrap();
        };
        let run = || {
            Command::new("sh")
                .arg("-c")
                .arg(
                    cross_release_install_script("Linux x86_64", Some(env!("CARGO_PKG_VERSION")))
                        .unwrap(),
                )
                .env("HOME", &home)
                .env(
                    "PATH",
                    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
                )
                .env("JR_TEST_ARCHIVE", &archive)
                .env("JR_TEST_CHECKSUM", &checksum)
                .output()
                .unwrap()
        };
        pack(env!("CARGO_PKG_VERSION"));
        std::fs::write(&checksum, format!("{}  release.tar.gz\n", "0".repeat(64))).unwrap();
        assert!(!run().status.success(), "bad checksum must fail closed");
        assert_eq!(
            std::fs::read_to_string(&installed).unwrap(),
            "original agent"
        );
        pack("0.0.0-wrong");
        assert!(!run().status.success(), "wrong version must fail closed");
        assert_eq!(
            std::fs::read_to_string(&installed).unwrap(),
            "original agent"
        );
        pack(env!("CARGO_PKG_VERSION"));
        let output = run();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            std::fs::read(&installed).unwrap(),
            std::fs::read(&executable).unwrap()
        );
        assert!(
            std::fs::read_dir(home.join(".cargo/bin"))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".justrust.upload"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn release_asset_url_names_this_version() {
        if let Some(u) = release_asset_url() {
            assert!(
                u.contains(&format!("/v{}/", env!("CARGO_PKG_VERSION"))),
                "{u}"
            );
            assert!(u.ends_with("justrust-x86_64-unknown-linux-musl.tar.gz"));
            assert_eq!(shell_quote(&u), u, "URL must need no quoting");
        }
    }

    #[test]
    fn workspace_dir_finds_the_enclosing_workspace() {
        let t = std::env::temp_dir().join(format!("jr-wsdir-{}", std::process::id()));
        let ws = t.join("ws");
        let c = ws.join("crates/a");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(
            ws.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/a\"]\n",
        )
        .unwrap();
        std::fs::write(c.join("Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();
        assert_eq!(workspace_dir(&c), ws);
        let lone = t.join("lone");
        std::fs::create_dir_all(&lone).unwrap();
        std::fs::write(lone.join("Cargo.toml"), "[package]\nname = \"b\"\n").unwrap();
        assert_eq!(workspace_dir(&lone), lone);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn outermost_drops_nested() {
        let d: BTreeSet<PathBuf> = ["/a/b", "/a", "/c/d", "/c/e"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(
            outermost(d),
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/c/d"),
                PathBuf::from("/c/e")
            ]
        );
    }

    /// A workspace inside a tree the repository's `.justrustignore` excludes
    /// is mirrored as its own root, next to the repository.
    #[test]
    fn workspace_inside_an_ignored_tree_gets_its_own_root() {
        let t = std::env::temp_dir().join(format!("jr-roots-ignored-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let repo = t.join("repo");
        let inner = repo.join("vendor/tool");
        for d in [
            repo.join(".git"),
            repo.join("src"),
            inner.join("crates/a/src"),
            repo.join("vendor/member/src"),
            repo.join("vendorish/src"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(repo.join(".justrustignore"), "/vendor/\n").unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname='repo'\nversion='0.1.0'\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(
            inner.join("Cargo.toml"),
            "[package]\nname='tool'\nversion='0.1.0'\n[workspace]\nmembers=['crates/a']\n",
        )
        .unwrap();
        std::fs::write(
            inner.join("crates/a/Cargo.toml"),
            "[package]\nname='a'\nversion='0.1.0'\n",
        )
        .unwrap();
        // Excluded: its own workspace, not the repository's and not `vendor/`.
        assert_eq!(excluded_root(&inner), Some(inner.clone()));
        assert_eq!(excluded_root(&inner.join("crates/a")), Some(inner.clone()));
        // A member of the repository's workspace under the excluded tree:
        // itself, since its workspace is the repository.
        let member = repo.join("vendor/member");
        assert_eq!(excluded_root(&member), Some(member.clone()));
        // Not excluded: no root of its own.
        assert_eq!(excluded_root(&repo), None);
        assert_eq!(excluded_root(&repo.join("src")), None);
        assert_eq!(excluded_root(&repo.join("vendorish")), None);
        let mut roots = roots_of(&inner.join("crates/a"));
        roots.sort();
        assert_eq!(roots, vec![repo.clone(), inner.clone()]);
        // Nested, but the outer mirror does not hold it: both stay.
        let d: BTreeSet<PathBuf> = [
            repo.clone(),
            inner.clone(),
            inner.join("crates/a"),
            repo.join("src"),
        ]
        .into_iter()
        .collect();
        assert_eq!(outermost(d), vec![repo.clone(), inner.clone()]);
        // The mirrors do not overlap: the repository's scan leaves the tree out.
        std::fs::write(inner.join("crates/a/src/lib.rs"), "").unwrap();
        std::fs::write(repo.join("src/lib.rs"), "").unwrap();
        let outer = crate::remote_sync::scan(&repo).unwrap().files;
        assert!(outer.contains_key(Path::new("src/lib.rs")));
        assert!(!outer.keys().any(|p| p.starts_with("vendor")));
        let nested = crate::remote_sync::scan(&inner).unwrap().files;
        assert!(nested.contains_key(Path::new("crates/a/src/lib.rs")));
        assert!(nested.contains_key(Path::new("Cargo.toml")));

        // Cached roots are per workspace: the nested one has a lockfile of
        // its own below the repository's, and the repository's roots do not
        // mirror it.
        std::fs::write(repo.join("Cargo.lock"), "").unwrap();
        std::fs::write(inner.join("Cargo.lock"), "").unwrap();
        let key = |d: &Path| roots_cache_key(d).unwrap();
        assert_ne!(key(&repo), key(&inner));
        assert_eq!(key(&inner), key(&inner.join("crates/a")));
        assert!(key(&inner).starts_with(&format!("v4:{}|", inner.display())));
        // Before the nested workspace has a lockfile, and for a member of the
        // repository's workspace under the excluded tree, the key is the
        // repository's: a cached set that leaves the directory out is not
        // used.
        assert_eq!(key(&repo), key(&member));
        let cached = vec![repo.clone()];
        assert!(mirrors(&cached, &repo));
        assert!(mirrors(&cached, &repo.join("src")));
        assert!(!mirrors(&cached, &inner));
        assert!(!mirrors(&cached, &inner.join("crates/a")));
        assert!(!mirrors(&cached, &member));
        assert!(mirrors(
            &[repo.clone(), inner.clone()],
            &inner.join("crates/a")
        ));
        // Touching mtime without changing contents keeps the same key;
        // changing the lockfile contents produces a new key.
        let before = key(&inner);
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(inner.join("Cargo.lock"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(before, key(&inner));
        std::fs::write(inner.join("Cargo.lock"), "version = 4\n").unwrap();
        assert_ne!(before, key(&inner));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("-p"), "-p");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    /// The layout that failed on the machine: a workspace copied outside git
    /// whose crate names `../shared/x.rs` by `#[path]`.
    #[test]
    fn sources_named_by_path_outside_the_roots_get_a_root() {
        let t = std::env::temp_dir().join(format!("jr-roots-escaping-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let ws = t.join("toolchain/cargo");
        let krate = ws.join("crates/util");
        let shared = t.join("toolchain/shared");
        let data = t.join("toolchain/data");
        for d in [&krate.join("src"), &shared, &data, &ws.join("target/debug")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(ws.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(krate.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(shared.join("binding.rs"), "").unwrap();
        std::fs::write(data.join("table.bin"), "").unwrap();
        std::fs::write(t.join("toolchain/stray.rs"), "").unwrap();
        std::fs::write(krate.join("other.rs"), "").unwrap();
        std::fs::write(
            krate.join("src/lib.rs"),
            concat!(
                "#[path = \"../../../../shared/binding.rs\"]\nmod binding;\n",
                "static T: &[u8] = include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../../data/table.bin\"));\n",
                "// include!(\"../../../../commented/out.rs\");\n",
                "include!(\"../../../../gone/missing.rs\");\n",
                "include!(\"../other.rs\");\n",
                "#[path = \"../../../../stray.rs\"]\nmod stray;\n",
            ),
        )
        .unwrap();
        std::fs::write(
            ws.join("target/debug/gen.rs"),
            "include!(\"../../../shared/binding.rs\");\n",
        )
        .unwrap();
        let named = crate::remote_sync::escaping_source_dirs(&krate);
        let toolchain = t.join("toolchain");
        assert_eq!(
            named.iter().collect::<Vec<_>>(),
            vec![&toolchain, &data, &shared]
        );
        // Neither build output nor a nested package is a source of the
        // workspace root package.
        assert!(crate::remote_sync::escaping_source_dirs(&ws).is_empty());
        let roots = vec![ws.clone()];
        assert_eq!(outside_root(&roots, &shared), Some(shared.clone()));
        assert_eq!(outside_root(&roots, &data), Some(data.clone()));
        // Above a root: never widened to it.
        assert_eq!(outside_root(&roots, &toolchain), None);
        // Already mirrored.
        assert_eq!(outside_root(&roots, &krate), None);

        // The same file, learned from the machine's error instead.
        let error = "error: couldn't find file `crates/util/src/../../../../shared/binding.rs`\n\
                     error: couldn't read `crates/util/src/../../../../gone/missing.rs`: No such file\n\
                     error: couldn't read `crates/util/src/../other.rs`: No such file";
        assert_eq!(
            unmirrored_missing(error, &krate, &roots),
            vec![shared.join("binding.rs")]
        );
        assert!(unmirrored_missing(error, &krate, &[ws, shared]).is_empty());
        let _ = std::fs::remove_dir_all(&t);
    }
}
