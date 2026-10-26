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
use std::sync::{Arc, mpsc};
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
    "RUSTUP_TOOLCHAIN",
    "JCODE_DESKTOP_VERSION",
    "JCODE_DESKTOP_BUILD_METADATA",
    "SOURCE_DATE_EPOCH",
    "JCODE_RELEASE_BUILD",
    "JCODE_BUILD_METADATA_FILE",
    "JCODE_BUILD_SEMVER",
    "JCODE_BUILD_CHANGELOG_RAW",
    "JUSTRUST_MAX_WARNINGS",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "JCODE_SESSION_ID",
    "CARGO_TERM_COLOR",
    "JUSTRUST_DEPCACHE_EXPLAIN",
    "JUSTRUST_DEPCACHE",
    "JUSTRUST_LOCALCACHE",
    "JUSTRUST_LOCALCACHE_TARGET_BOUND",
    "JUSTRUST_PEER_CACHE",
    "JUSTRUST_TOOLCHAIN",
    "JUSTRUST_TOOLCHAIN_REQUIRED",
    "JUSTRUST_TOOLCHAIN_DIST",
    "JUSTRUST_TOOLCHAIN_AUTO_INSTALL",
    "JUSTRUST_PREBUILT",
    "JUSTRUST_SHADOW",
    "JUSTRUST_TEST_FILTER_COMPILE",
    "JUSTRUST_CACHE_PROC_MACROS",
    "JUSTRUST_CUTOFF",
    "JUSTRUST_CARGO_CUTOFF",
    "JUSTRUST_CARGO_FORK",
    "JUSTRUST_CARGO_DAEMON",
    "JUSTRUST_CARGO_DAEMON_TRACE",
    "JUSTRUST_UNIFY_FEATURES",
    "JUSTRUST_EARLY",
    "JUSTRUST_EARLY_EXPERIMENTAL",
];

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
        if !roots.is_empty() && roots.iter().all(|r| r.is_dir()) {
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
    let mut out = metadata(true)?;
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
        dirs.insert(repo_root(Path::new(ws)));
    }
    for p in m["packages"].as_array().into_iter().flatten() {
        if p["source"].is_null()
            && let Some(manifest) = p["manifest_path"].as_str()
            && let Some(d) = Path::new(manifest).parent()
        {
            let root = repo_root(d);
            // Outside git (a tarball, a benchmark snapshot) the crate dir
            // alone misses its workspace's root manifest and lockfile, which
            // build scripts and cargo read: mirror the whole workspace.
            dirs.insert(if root.join(".git").exists() {
                root
            } else {
                workspace_dir(d)
            });
        }
    }
    let roots = outermost(dirs);
    if let Some(key) = cache_key {
        let mut v: serde_json::Value = std::fs::read(&cache)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        v[key] = serde_json::json!(roots);
        let _ = std::fs::write(&cache, v.to_string());
    }
    Ok(roots)
}

/// Lockfile path + mtime: path dependencies only change with the lockfile.
fn roots_cache_key(cwd: &Path) -> Option<String> {
    let lock = lockfile(cwd)?;
    let mtime = std::fs::metadata(&lock)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{}@{mtime}", lock.display()))
}

/// The outermost Cargo.lock at or above `cwd` (the workspace's).
pub fn lockfile(cwd: &Path) -> Option<PathBuf> {
    let mut d = Some(cwd);
    let mut lock = None;
    while let Some(dir) = d {
        let l = dir.join("Cargo.lock");
        if l.exists() {
            lock = Some(l);
        }
        d = dir.parent();
    }
    lock
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

/// Drop roots nested inside another root.
fn outermost(dirs: BTreeSet<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !out.iter().any(|o| d.starts_with(o)) {
            out.retain(|o| !o.starts_with(&d));
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
    let Ok(exe) = std::env::current_exe() else {
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
fn cross_release_install_script(remote_arch: &str, release_build: bool) -> Option<String> {
    if !release_build {
        return None;
    }
    let target = match remote_arch {
        "Linux x86_64" => "x86_64-unknown-linux-musl",
        "Linux aarch64" => "aarch64-unknown-linux-musl",
        _ => return None,
    };
    let version = env!("CARGO_PKG_VERSION");
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
         mkdir -p ~/.cargo/bin; chmod +x \"$f\"; \
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
    let Some(script) = cross_release_install_script(remote_arch, crate::update::release_build())
    else {
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
    let (Some(base), Ok(exe)) = (installed_copy(), std::env::current_exe()) else {
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
        "set -e; command -v zstd >/dev/null; b=~/.cargo/bin/justrust; u=~/.cargo/bin/.justrust.upload; \
         [ \"$(sha256sum \"$b\" | cut -d' ' -f1)\" = {base_sha} ]; \
         zstd -q -d -f --long=27 --patch-from=\"$b\" -o \"$u\"; \
         [ \"$(sha256sum \"$u\" | cut -d' ' -f1)\" = {want} ] || {{ rm -f \"$u\"; exit 1; }}; \
         chmod +x \"$u\"; mv \"$u\" \"$b\"",
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

/// Make sure the machine runs this exact justrust binary. Same OS and
/// architecture (the common case): fetch it from its release on the
/// machine, else copy it. Otherwise build it there from crates.io at this
/// version.
pub fn ensure_remote_justrust(t: &Target) -> Result<()> {
    let id = crate::remote_daemon::build_id();
    let stamp = crate::remote::dir()?.join("remote-justrust");
    let want = format!("{} {id}", t.dest);
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str()) {
        return Ok(());
    }
    let arch = t
        .command()
        .arg("uname -sm")
        .stdin(Stdio::null())
        .output()
        .context("ssh uname")?;
    let remote_arch = String::from_utf8_lossy(&arch.stdout).trim().to_string();
    let local_arch = format!(
        "{} {}",
        if cfg!(target_os = "linux") {
            "Linux"
        } else {
            std::env::consts::OS
        },
        std::env::consts::ARCH
    );
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
        let exe = std::env::current_exe()?;
        let tmp = ".cargo/bin/.justrust.upload";
        let ok = Command::new("rsync")
            .args(["-z", "-e", &t.rsync_shell()])
            .arg(&exe)
            .arg(format!("{}:{tmp}", t.dest))
            .stdin(Stdio::null())
            .status()?
            .success()
            && t.command()
                .arg(format!(
                    "mkdir -p ~/.cargo/bin && chmod +x ~/{tmp} && mv ~/{tmp} ~/.cargo/bin/justrust"
                ))
                .stdin(Stdio::null())
                .status()?
                .success();
        if !ok {
            bail!("copying justrust to the remote machine failed");
        }
    } else if remote_arch != local_arch && fetch_cross_release_on_machine(t, &remote_arch) {
        crate::remote::log_event(&format!(
            "justrust {} fetched its {remote_arch} release on {} in {:.1}s",
            env!("CARGO_PKG_VERSION"),
            t.dest,
            t0.elapsed().as_secs_f64()
        ));
    } else {
        eprintln!("justrust remote: building justrust on the machine ({remote_arch})...");
        let st = t
            .command()
            .arg(format!(
                ". ~/.cargo/env; cargo install justrust --version {} --locked -q",
                env!("CARGO_PKG_VERSION")
            ))
            .status()?;
        if !st.success() {
            bail!("installing justrust on the remote machine failed");
        }
    }
    // Same linker on both sides: with wild here and not there, the
    // machine linked the 670 MB Desktop test binary with mold, 1.26 s
    // against 0.25 s (FINDINGS 33). Best effort: a failure only costs speed.
    if crate::linker::is_installed() && std::env::var("JUSTRUST_LINKER").as_deref() != Ok("off") {
        let _ = t
            .command()
            .arg("~/.cargo/bin/justrust toolchain wild >/dev/null 2>&1")
            .stdin(Stdio::null())
            .status();
    }
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
    if let Ok(exe) = std::env::current_exe() {
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
}

/// Marks an [`Infra`] whose text is a billing instruction.
pub const BILLING: &str = "justrust: ";

/// Hosted only: charge this run before anything runs remotely. Err holds
/// the line to show; the build then runs locally.
/// `Ok(true)` when a hosted charge happened (the run then reports its
/// lifecycle to the operator dashboard).
fn charge_hosted(
    run_id: &str,
    cmd: &str,
    project: Option<&str>,
) -> std::result::Result<bool, String> {
    if remote_backend::load() != Some(remote_backend::Backend::Hosted) {
        return Ok(false);
    }
    let c = crate::remote_hosted::creds().map_err(|e| format!("{BILLING}{e}"))?;
    match crate::remote_hosted::charge_run_labeled(&c, run_id, Some(cmd), project) {
        Ok(_) => Ok(true),
        Err(e)
            if e.contains("justrust upgrade")
                || e.contains("jcode.sh/account")
                || e.contains("justrust login") =>
        {
            Err(format!("{BILLING}{e}"))
        }
        Err(e) => Err(e),
    }
}

pub struct Outcome {
    pub code: i32,
    #[allow(dead_code)] // used by the router (in progress)
    pub run_id: String,
}

fn fallback_safe(flushed: bool, submitted: bool) -> bool {
    !flushed && !submitted
}

type RunReporter = Arc<dyn Fn(&str, Option<i32>) + Send + Sync>;

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
            Arc::new(move |event: &str, code| {
                crate::remote_hosted::report_run_event(
                    &run_id,
                    event,
                    code,
                    Some(started.elapsed().as_secs_f64()),
                );
            }) as RunReporter
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
        report("running", None);
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
                    || report("running", None),
                );
            })
        {
            self.heartbeat = Some((stop, worker));
        }
    }

    fn finish(&mut self, event: &str, code: Option<i32>) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some((stop, worker)) = self.heartbeat.take() {
            let _ = stop.send(());
            let _ = worker.join();
        }
        if let Some(report) = &self.report {
            report(event, code);
        }
        // Already dispatched requests can arrive out of order. The server
        // treats terminal states as absorbing, including late heartbeats.
    }
}

impl Drop for HostedRun {
    fn drop(&mut self) {
        self.finish("abandoned", None);
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
    let started = Instant::now();
    let mut diagnostic = RemoteDiagnostic::default();
    let run_id = crate::record::new_run_id();
    let mut lifecycle = crate::telemetry::BuildLifecycle::start(&run_id, sub, "remote");
    let result = run_remote_inner(sub, args, start, &mut diagnostic, &run_id, &mut lifecycle);
    if let Some(code) = diagnostic.exit {
        let logs = crate::paths::runs_dir()
            .ok()
            .is_some_and(|d| std::fs::File::open(d.join(&run_id).join("output.jsonl")).is_ok());
        lifecycle.done(code, logs);
    }
    if !matches!(&result, Ok(Ok(_))) {
        report_remote_failure(sub, &diagnostic, started.elapsed().as_secs_f64());
    }
    result
}

struct RemoteDiagnostic {
    stage: &'static str,
    may_have_started: bool,
    exit: Option<i32>,
}

impl Default for RemoteDiagnostic {
    fn default() -> Self {
        Self {
            stage: "remote_setup",
            may_have_started: false,
            exit: None,
        }
    }
}

fn remote_failure_data(sub: &str, d: &RemoteDiagnostic, elapsed: f64) -> serde_json::Value {
    let sub = match sub {
        "check" | "test" | "build" | "clippy" | "run" | "bench" | "doc" | "rustc" => sub,
        _ => "other",
    };
    serde_json::json!({ "schema_version": 2, "sub": sub, "stage": d.stage,
        "may_have_started": d.may_have_started,
        "exit": d.exit, "elapsed_s": (elapsed * 100.0).round() / 100.0 })
}

fn report_remote_failure(sub: &str, d: &RemoteDiagnostic, elapsed: f64) {
    crate::telemetry::record("build_failure", remote_failure_data(sub, d, elapsed));
    crate::telemetry::maybe_spawn_background();
}

fn run_remote_inner(
    sub: &str,
    args: &[String],
    start: bool,
    diagnostic: &mut RemoteDiagnostic,
    run_id: &str,
    lifecycle: &mut crate::telemetry::BuildLifecycle,
) -> Result<std::result::Result<Outcome, Infra>> {
    let total = Instant::now();
    let trace = std::env::var_os("JUSTRUST_REMOTE_TRACE").is_some();
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
        Err(e) => return infra(format!("{e:#}")),
    };
    if !roots.iter().any(|r| cwd.starts_with(r)) {
        return infra(format!("{} is not inside a synced root", cwd.display()));
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
    diagnostic.stage = "remote_admission";
    let hosted = match charge_hosted(run_id, &label.join(" "), project.as_deref()) {
        Ok(h) => h,
        Err(m) => return infra(m),
    };
    let mut hosted_run = HostedRun::new(hosted, run_id, total);
    mark("charged");
    // Visible to `justrust status` (and the status bar) while it runs.
    let mut live = LiveRun::start(run_id, &cwd, &label, started_at);
    diagnostic.stage = "remote_transport";
    let sock = match crate::remote_daemon::connect() {
        Ok(s) => s,
        Err(e) => return infra(format!("{e:#}")),
    };
    let mut env = Vec::new();
    env.push((crate::remote_proto::CLIENT_LIVE_CPU.into(), "1".into()));
    for var in FORWARDED_ENV {
        if let Ok(v) = std::env::var(var) {
            env.push((var.to_string(), v));
        }
    }
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
    let mut w = BufWriter::new(sock.try_clone()?);
    let mut r = BufReader::new(sock);
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
    let mut sync = None;
    let mut submitted = false;
    let mut first_out = false;
    let mut run_files: Vec<u8> = Vec::new();
    let mut compile_stdout = Vec::new();
    let mut so = std::io::stdout();
    let mut se = std::io::stderr();
    loop {
        let frame = match read_frame(&mut r) {
            Ok(Some(f)) => f,
            Ok(None) | Err(_) if fallback_safe(sync.is_some(), submitted) => {
                return infra("sync daemon closed the connection".into());
            }
            Ok(None) | Err(_) => {
                // Output already started: a local rerun would duplicate it.
                bail!("lost the remote build mid-run (see ~/.justrust/remote/daemon.log)");
            }
        };
        match frame {
            (
                Msg::RunState {
                    may_have_started, ..
                },
                _,
            ) => {
                submitted = may_have_started;
                if may_have_started {
                    lifecycle.running();
                    hosted_run.running();
                }
                diagnostic.may_have_started |= may_have_started;
            }
            (
                Msg::Flushed {
                    files,
                    ms,
                    machine,
                    note,
                    ..
                },
                _,
            ) => {
                if !note.is_empty() {
                    eprintln!("justrust remote: {note}");
                }
                mark("flushed");
                if sync.is_none() {
                    lifecycle.running();
                    hosted_run.running();
                    live.phase("building", &machine);
                }
                sync = Some((files, ms, machine));
                diagnostic.may_have_started = true;
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
                if trace && !first_out {
                    first_out = true;
                    mark("first output");
                }
                if stream == 1 {
                    // Selected tests are emitted only after their local files exist.
                    // Remote JSON contains host paths and cannot be passed through.
                    if !compile_only {
                        so.write_all(&data)?;
                        so.flush()?;
                    } else {
                        compile_stdout.extend_from_slice(&data);
                    }
                } else {
                    se.write_all(&data)?;
                    se.flush()?;
                }
            }
            (Msg::RunFiles { .. }, data) => {
                mark(&format!("run files {} KB", data.len() / 1024));
                run_files = data
            }
            (Msg::Exit { code, .. }, summary) => {
                diagnostic.exit = Some(code);
                diagnostic.may_have_started = true;
                mark("exit");
                hosted_run.finish("done", Some(code));
                if compile_only {
                    // Preserve all rendered compiler diagnostics, including failed
                    // builds, without publishing unusable remote executable paths.
                    for line in compile_stdout.split(|b| *b == b'\n') {
                        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(line)
                            && value["reason"] == "compiler-message"
                        {
                            if json_output {
                                serde_json::to_writer(&mut so, &value)?;
                                so.write_all(b"\n")?;
                            } else if let Some(rendered) = value["message"]["rendered"].as_str() {
                                se.write_all(rendered.as_bytes())?;
                            }
                        }
                    }
                    if code == 0 {
                        diagnostic.stage = "remote_artifacts";
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
                mark("recording");
                diagnostic.stage = "remote_summary";
                if !record_remote(
                    run_id,
                    &summary,
                    &run_files,
                    &machine,
                    ms,
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
                }));
            }
            (Msg::Error { msg, .. }, _) if fallback_safe(sync.is_some(), submitted) => {
                return infra(msg);
            }
            (Msg::Error { msg, .. }, _) => {
                bail!("remote build failed mid-run: {msg}")
            }
            _ => {}
        }
    }
}

/// Where a remote build runs, for `justrust status`: "hosted" or "remote".
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
    state: crate::live::LiveState,
}

impl LiveRun {
    fn start(run_id: &str, cwd: &Path, args: &[String], start: f64) -> LiveRun {
        let Ok(dir) = crate::paths::runs_dir().map(|d| d.join(run_id)) else {
            return LiveRun {
                dir: None,
                state: Default::default(),
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
        let mut r = LiveRun {
            dir: ok.then_some(dir),
            state: Default::default(),
        };
        r.phase("syncing", "");
        r
    }

    fn phase(&mut self, phase: &str, machine: &str) {
        let Some(dir) = &self.dir else { return };
        self.state.phase = phase.into();
        if !machine.is_empty() {
            self.state.active = vec![machine.to_string()];
            self.state.active_since = vec![crate::paths::now()];
        }
        self.state.updated = crate::paths::now();
        if let Ok(b) = serde_json::to_vec(&self.state) {
            let _ = std::fs::write(dir.join(crate::live::FILE), b);
        }
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
fn record_remote(
    run_id: &str,
    summary: &[u8],
    run_files: &[u8],
    machine: &str,
    sync_ms: f64,
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
    // The machine recorded the run as local to itself.
    let meta_path = dir.join("meta.json");
    if let Some(mut m) = std::fs::read(&meta_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::record::Meta>(&b).ok())
    {
        m.place = Some(place_name().into());
        if let Ok(b) = serde_json::to_vec_pretty(&m) {
            let _ = std::fs::write(&meta_path, b);
        }
    }
    s.remote = Some(crate::summary::RemoteInfo {
        machine: machine.to_string(),
        sync_ms,
        client_wall: wall,
    });
    // Hints are local state: attach them here, not on the remote machine
    // (which mirrors the workspace at the same absolute path).
    crate::split_index::attach(&mut s);
    let _ = std::fs::write(
        dir.join("summary.json"),
        serde_json::to_vec_pretty(&s).unwrap_or_default(),
    );
    if let (Ok(idx), Ok(line)) = (
        crate::paths::index_file(),
        serde_json::to_string(&s.index_entry()),
    ) {
        let _ = crate::paths::append_line(&idx, &line);
    }
    crate::split_index::record(&s);
    crate::split_index::maybe_refresh(&s);
    crate::telemetry::record_run(&s, true);
    crate::telemetry::maybe_spawn_background();
    true
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
mod tests {
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
            let mut run =
                super::HostedRun::with_reporter(Some(std::sync::Arc::new(move |event, code| {
                    tx.send((event.to_owned(), code)).unwrap();
                })));
            run.running();
            run.running();
            assert!(run.heartbeat.is_some());
            run.finish("done", Some(code));
            assert!(run.heartbeat.is_none());
            run.running();
            run.finish("abandoned", None);
            drop(run);
            assert_eq!(
                rx.into_iter().collect::<Vec<_>>(),
                vec![("running".into(), None), ("done".into(), Some(code)),]
            );
        }
    }

    #[test]
    fn hosted_heartbeat_drop_abandons_even_before_running() {
        for running in [false, true] {
            let (tx, rx) = std::sync::mpsc::channel();
            let result: anyhow::Result<()> = (|| {
                let mut run = super::HostedRun::with_reporter(Some(std::sync::Arc::new(
                    move |event, code| {
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
        run.finish("done", Some(101));
    }

    #[test]
    fn remote_failure_diagnostics_preserve_known_outcome_without_private_text() {
        let d = RemoteDiagnostic {
            stage: "remote_artifacts",
            may_have_started: true,
            exit: Some(0),
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
    fn malformed_remote_summary_is_reportable_not_silently_successful() {
        assert!(!record_remote(
            "unused",
            b"not-json",
            &[],
            "private-machine",
            0.0,
            0.0
        ));
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
    fn hosted_forwards_explicit_optimization_controls_not_secrets_or_local_paths() {
        for var in [
            "JUSTRUST_RELEASE_BUILD",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTUP_TOOLCHAIN",
            "JCODE_DESKTOP_VERSION",
            "JCODE_DESKTOP_BUILD_METADATA",
            "SOURCE_DATE_EPOCH",
            "JCODE_RELEASE_BUILD",
            "JCODE_BUILD_METADATA_FILE",
            "JCODE_BUILD_SEMVER",
            "JCODE_BUILD_CHANGELOG_RAW",
            "JUSTRUST_TOOLCHAIN_DIST",
            "JUSTRUST_TOOLCHAIN_REQUIRED",
            "JUSTRUST_TOOLCHAIN_AUTO_INSTALL",
            "JUSTRUST_SHADOW",
            "JUSTRUST_LOCALCACHE",
            "JUSTRUST_LOCALCACHE_TARGET_BOUND",
            "JUSTRUST_TEST_FILTER_COMPILE",
            "JUSTRUST_CACHE_PROC_MACROS",
            "JUSTRUST_CUTOFF",
            "JUSTRUST_CARGO_CUTOFF",
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
    fn cross_release_install_uses_host_target_and_verifies_before_replacing() {
        for (arch, target) in [
            ("Linux x86_64", "x86_64-unknown-linux-musl"),
            ("Linux aarch64", "aarch64-unknown-linux-musl"),
        ] {
            let script = cross_release_install_script(arch, true).unwrap();
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
            assert!(cross_release_install_script(arch, false).is_none());
        }
        assert!(cross_release_install_script("Darwin arm64", true).is_none());
        assert!(cross_release_install_script("Linux riscv64", true).is_none());
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
                .arg(cross_release_install_script("Linux x86_64", true).unwrap())
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

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("-p"), "-p");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
