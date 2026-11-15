//! Experimental hybrid dispatch. A positive pre-execution miss may use Cargo.
//! Once the native engine commits, no exit status or diagnostic can cause retry.
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use anyhow::{Context as _, bail};
use sha2::{Digest as _, Sha256};

const HELP: &str = "justrust hybrid <build|test|check|run> [arguments...]\n\
Experimental local hybrid compiler: a ready resident worker, otherwise the normal\n\
justrust Cargo/hosted path. Native failures are never retried with Cargo.\n\
justrust hybrid prewarm <build|test|check|run> [arguments...]\n\
Prepares native state without executing user tests or programs.\n\
justrust hybrid status|stop\n\
Requires a trusted owned engine (JUSTRUST_NATIVE_ENGINE or sibling jr-native)\n\
and JUSTRUST_HYBRID_TOOLCHAIN pointing to its matching toolchain directory.\n\
JR_ENGINE_HOME overrides the private hybrid namespace. Ordinary commands are\n\
unchanged unless JUSTRUST_HYBRID=ready explicitly opts build/test/check/run in.\n\
This is an experiment, not a production default or a Rust parity claim.\n\
JUSTRUST_HYBRID_PREWARM=after-success opts into background preparation\n\
after a successful Cargo fallback. Only one preparation runs per namespace.\n";

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Miss,
    Committed,
}

fn decision(bytes: &[u8]) -> anyhow::Result<Decision> {
    match bytes {
        b"miss\n" => Ok(Decision::Miss),
        b"committed\n" => Ok(Decision::Committed),
        _ => bail!("native hybrid decision missing or invalid, refusing an ambiguous Cargo retry"),
    }
}

fn is_workload(command: Option<&OsStr>) -> bool {
    matches!(
        command.and_then(OsStr::to_str),
        Some("build" | "test" | "check" | "run")
    )
}

#[cfg(unix)]
fn daemon_socket_path_capacity() -> usize {
    // SAFETY: sockaddr_un contains only integer fields, for which zero is valid.
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_path.len()
}

#[cfg(unix)]
fn validate_daemon_socket_path(home: &Path) -> anyhow::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    // Keep this identical to the engine's socket_in(home), without changing the
    // namespace or canonical identity. Unix pathname sockets need a trailing NUL.
    let socket = home.join("daemon.sock");
    let bytes = socket.as_os_str().as_bytes().len();
    let capacity = daemon_socket_path_capacity();
    if bytes >= capacity {
        bail!(
            "hybrid daemon socket path is too long: {} is {bytes} bytes, maximum {} bytes on this platform; set JR_ENGINE_HOME to a shorter existing canonical owner-only directory",
            socket.display(),
            capacity - 1
        );
    }
    Ok(())
}

fn private_home() -> anyhow::Result<PathBuf> {
    let explicit = std::env::var_os("JR_ENGINE_HOME");
    let home = if let Some(home) = explicit.as_ref() {
        PathBuf::from(home)
    } else {
        let parent = crate::paths::home()?;
        std::fs::create_dir_all(&parent)?;
        parent.canonicalize()?.join("hybrid")
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
        if explicit.is_none() {
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&home) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).context("create hybrid private home"),
            }
        }
        let metadata = std::fs::symlink_metadata(&home).context("read hybrid private home")?;
        // SAFETY: geteuid has no memory preconditions.
        if !home.is_absolute()
            || home.parent().is_none()
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
            || home.canonicalize()? != home
        {
            bail!("JR_ENGINE_HOME must be an existing canonical owner-only directory");
        }
        validate_daemon_socket_path(&home)?;
    }
    Ok(home)
}

fn toolchain() -> anyhow::Result<Option<PathBuf>> {
    let Some(path) = std::env::var_os("JUSTRUST_HYBRID_TOOLCHAIN") else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    if !path.is_absolute() || !path.join("bin/rustc").is_file() || !path.join("bin/cargo").is_file()
    {
        bail!("JUSTRUST_HYBRID_TOOLCHAIN must be an absolute matching toolchain directory");
    }
    Ok(Some(path.canonicalize()?))
}

fn native_command(engine: &Path, home: &Path, tc: &Path) -> anyhow::Result<Command> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt as _;
        cwd.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let bytes = cwd.as_os_str().as_encoded_bytes();
    let key = format!("{:x}", Sha256::digest(bytes));
    let mut cmd = Command::new(engine);
    cmd.env("JR_ENGINE_HOME", home)
        .env("RUSTC", tc.join("bin/rustc"))
        .env("CARGO_TARGET_DIR", home.join("targets").join(&key[..24]))
        .env(
            "CARGO_BUILD_BUILD_DIR",
            home.join("targets").join(&key[..24]).join("build"),
        )
        .env("RA_JIT_SPARE", "0")
        .env("BLAZE_SPARE", "0")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("JUSTRUST_HYBRID")
        .env_remove("JUSTRUST_REAL_RUSTC")
        .env_remove("JUSTRUST_REAL_CARGO");
    Ok(cmd)
}

fn engine() -> anyhow::Result<Option<PathBuf>> {
    let exe = std::env::current_exe()?;
    if std::env::var_os("JUSTRUST_NATIVE_ENGINE").is_none() {
        let sibling = exe
            .parent()
            .context("justrust executable parent")?
            .join("jr-native");
        if !sibling.exists() {
            return Ok(None);
        }
    }
    crate::native::engine_path(std::env::var_os("JUSTRUST_NATIVE_ENGINE"), &exe).map(Some)
}

fn fallback(args: &[OsString], prepare: Option<(&Path, &Path, &Path)>) -> anyhow::Result<()> {
    let executable = match std::env::var_os("JUSTRUST_HYBRID_FALLBACK") {
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() || !path.is_file() {
                bail!("JUSTRUST_HYBRID_FALLBACK must name an absolute justrust executable");
            }
            path
        }
        None => std::env::current_exe()?,
    };
    eprintln!("justrust hybrid: Cargo fallback (no ready resident worker)");
    let mut command = Command::new(executable);
    command
        .args(args)
        .env_remove("JUSTRUST_HYBRID")
        .env_remove("JR_HYBRID_DECISION_FD");
    if std::env::var_os("JUSTRUST_HYBRID_PREWARM").as_deref() == Some(OsStr::new("after-success"))
        && let Some((engine, home, tc)) = prepare
    {
        let status = command
            .status()
            .context("execute unchanged justrust fallback")?;
        if status.success()
            && let Err(error) = schedule_prewarm(args, engine, home, tc)
        {
            eprintln!("justrust hybrid: background prewarm not started: {error:#}");
        }
        finish(status);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        Err(command.exec()).context("execute unchanged justrust fallback")
    }
    #[cfg(not(unix))]
    {
        finish(command.status().context("execute justrust fallback")?)
    }
}

fn schedule_prewarm(
    args: &[OsString],
    engine: &Path,
    home: &Path,
    tc: &Path,
) -> anyhow::Result<()> {
    if args.first().is_some_and(|arg| arg == "check") {
        // Whole-program resident check is not implemented. Preserve ordinary
        // Cargo check and do not launch a preparation guaranteed to fail.
        eprintln!("justrust hybrid: background preparation skipped (unsupported command: check)");
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::{fs::OpenOptionsExt as _, process::CommandExt as _};
        let log_path = home.join("prewarm.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&log_path)?;
        let mut child = Command::new(std::env::current_exe()?);
        child
            .arg("__hybrid-prewarm")
            .args(args)
            .env("JR_ENGINE_HOME", home)
            .env("JUSTRUST_NATIVE_ENGINE", engine)
            .env("JUSTRUST_HYBRID_TOOLCHAIN", tc)
            .env_remove("JUSTRUST_HYBRID")
            .env_remove("JR_HYBRID_DECISION_FD")
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        // SAFETY: only process-local libc system calls run between fork and exec.
        // A dedicated session prevents terminal signals from cancelling unrelated
        // future foreground requests. Do not lower inherited process priority:
        // a newly started resident daemon would keep that priority for later edits.
        unsafe {
            child.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let process = child
            .spawn()
            .context("launch deferred native preparation")?;
        eprintln!(
            "justrust hybrid: queued background preparation (pid {}, log {})",
            process.id(),
            log_path.display()
        );
        // This child deliberately outlives the foreground CLI. Its namespace
        // lock is held by the internal supervisor until the engine exits.
        drop(process);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (args, engine, home, tc);
        bail!("background native preparation requires Unix")
    }
}

pub(crate) fn background(args: Vec<OsString>) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::MetadataExt as _;
        use std::os::{fd::AsRawFd as _, unix::fs::OpenOptionsExt as _};
        use std::time::{Duration, SystemTime};
        if !is_workload(args.first().map(OsString::as_os_str))
            || args.iter().any(|a| a.to_str().is_none())
        {
            bail!("invalid background native preparation command");
        }
        let home = private_home()?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(home.join("prewarm.lock"))?;
        // SAFETY: flock only changes the lock of this owned open descriptor.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                eprintln!("justrust hybrid: preparation already active in this namespace");
                return Ok(());
            }
            return Err(error).context("lock native preparation namespace");
        }
        // Only the automatic supervisor uses this marker, under prewarm.lock.
        // Keep the file open through the attempt and never follow links or unlink
        // a pathname on success. Empty means clear, one byte means failed.
        let mut failure = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(home.join("prewarm.failed"))
            .context("open automatic preparation failure marker")?;
        let metadata = failure.metadata()?;
        // SAFETY: geteuid has no memory preconditions.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.len() > 1
        {
            bail!(
                "automatic preparation failure marker must be an owner-only regular single-link file of at most one byte"
            );
        }
        if metadata.len() == 1 {
            let elapsed = SystemTime::now()
                .duration_since(metadata.modified()?)
                // A wall-clock rollback must not create an unbounded cooldown.
                .unwrap_or(Duration::from_secs(60));
            if elapsed < Duration::from_secs(60) {
                eprintln!(
                    "justrust hybrid: automatic preparation skipped (failure cooldown: retry in {}s, 60s per namespace)",
                    60 - elapsed.as_secs()
                );
                return Ok(());
            }
        }
        let engine = engine()?.context("owned engine unavailable for preparation")?;
        let tc = toolchain()?.context("matching native toolchain unavailable for preparation")?;
        let status = native_command(&engine, &home, &tc)?
            .args(["cargo", "hybrid-prewarm", "--"])
            .args(args)
            .status();
        if !matches!(&status, Ok(status) if status.success()) {
            let recorded = (|| -> std::io::Result<()> {
                failure.set_len(0)?;
                failure.write_all(b"1")?;
                failure.set_modified(SystemTime::now())
            })();
            match recorded {
                Ok(()) => eprintln!(
                    "justrust hybrid: automatic preparation failed; 60s failure cooldown recorded for this namespace"
                ),
                Err(error) => eprintln!(
                    "justrust hybrid: automatic preparation failed; could not record failure cooldown: {error}"
                ),
            }
        }
        let status = status.context("prepare native worker")?;
        if !status.success() {
            bail!("background native preparation ended with {status}");
        }
        failure.set_len(0)?;
        eprintln!("justrust hybrid: background prewarm finished status=0");
        drop(lock);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        bail!("background native preparation requires Unix")
    }
}

fn finish(status: ExitStatus) -> ! {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            // SAFETY: restore and deliver the child's termination signal to this process.
            unsafe {
                libc::signal(signal, libc::SIG_DFL);
                libc::raise(signal);
            }
            std::process::exit(128 + signal);
        }
    }
    std::process::exit(status.code().unwrap_or(1))
}

#[cfg(unix)]
fn attempt(mut command: Command, args: &[OsString]) -> anyhow::Result<(Decision, ExitStatus)> {
    use std::io::Read as _;
    use std::os::fd::AsRawFd as _;
    use std::os::unix::{net::UnixStream, process::CommandExt as _};
    use std::time::Duration;
    let (mut reader, writer) = UnixStream::pair().context("create native decision channel")?;
    reader.set_read_timeout(Some(Duration::from_secs(2)))?;
    let fd = writer.as_raw_fd();
    command
        .args(["cargo", "hybrid-ready", "--"])
        .args(args)
        .env("JR_HYBRID_DECISION_FD", fd.to_string());
    // SAFETY: only async-signal-safe fcntl occurs between fork and exec. The
    // writer remains owned by this scope through spawn, and is closed below.
    unsafe {
        command.pre_exec(move || {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context("start native ready-only attempt")?;
    drop(writer);
    let mut bytes = Vec::new();
    if let Err(error) = reader.by_ref().take(64).read_to_end(&mut bytes) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error)
            .context("native decision channel failed, refusing an ambiguous Cargo retry");
    }
    let decided = match decision(&bytes) {
        Ok(decided) => decided,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let status = child.wait().context("wait for native attempt")?;
    if decided == Decision::Miss && !status.success() {
        bail!("native miss did not exit successfully, refusing an ambiguous Cargo retry");
    }
    Ok((decided, status))
}

pub(crate) fn main(mut args: Vec<OsString>) -> anyhow::Result<()> {
    if args.is_empty() || (args.len() == 1 && matches!(args[0].to_str(), Some("--help" | "-h"))) {
        print!("{HELP}");
        return Ok(());
    }
    if !cfg!(unix) {
        bail!(
            "experimental hybrid dispatch currently requires Unix, normal commands are unchanged"
        );
    }
    let prewarm = args.first().is_some_and(|a| a == "prewarm");
    if prewarm {
        args.remove(0);
    }
    let lifecycle =
        !prewarm && args.len() == 1 && matches!(args[0].to_str(), Some("status" | "stop"));
    if !lifecycle && !is_workload(args.first().map(OsString::as_os_str)) {
        bail!("hybrid requires build, test, check, run, prewarm, status or stop, see --help");
    }
    if args.iter().any(|a| a.to_str().is_none()) {
        if prewarm || lifecycle {
            bail!("native hybrid operations require UTF-8 arguments");
        }
        return fallback(&args, None);
    }
    let Some(engine) = engine()? else {
        if prewarm || lifecycle {
            bail!("owned native engine is unavailable, see justrust native --help");
        }
        return fallback(&args, None);
    };
    let Some(tc) = toolchain()? else {
        if prewarm || lifecycle {
            bail!("set JUSTRUST_HYBRID_TOOLCHAIN to the matching native toolchain directory");
        }
        return fallback(&args, None);
    };
    let home = private_home()?;
    let mut command = native_command(&engine, &home, &tc)?;
    if prewarm || lifecycle {
        if prewarm {
            command.args(["cargo", "hybrid-prewarm", "--"]);
        } else {
            command.arg("cargo");
        }
        finish(
            command
                .args(args)
                .status()
                .context("execute native hybrid management command")?,
        );
    }
    #[cfg(unix)]
    {
        let (decided, status) = attempt(command, &args)?;
        match decided {
            Decision::Miss => fallback(&args, Some((&engine, &home, &tc))),
            Decision::Committed => {
                eprintln!("justrust hybrid: resident committed");
                finish(status)
            }
        }
    }
    #[cfg(not(unix))]
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn daemon_socket_path_boundary_counts_bytes_and_reserves_nul() {
        use std::os::unix::ffi::OsStringExt as _;
        let capacity = daemon_socket_path_capacity();
        for length in [capacity - 1, capacity, capacity + 1] {
            // One leading slash and the "/daemon.sock" suffix are included.
            let home = PathBuf::from(format!("/{}", "a".repeat(length - 13)));
            assert_eq!(home.join("daemon.sock").as_os_str().len(), length);
            assert_eq!(
                validate_daemon_socket_path(&home).is_ok(),
                length < capacity
            );
            // An invalid UTF-8 byte must not be replaced or expanded for counting.
            let mut bytes = home.into_os_string().into_vec();
            bytes[1] = 0xff;
            let home = PathBuf::from(OsString::from_vec(bytes));
            assert_eq!(
                validate_daemon_socket_path(&home).is_ok(),
                length < capacity
            );
        }
        let home = PathBuf::from(format!("/{}", "é".repeat(capacity / 2)));
        let error = validate_daemon_socket_path(&home).unwrap_err().to_string();
        assert!(error.contains("bytes"));
        assert!(error.contains("shorter"));
        assert!(error.contains("JR_ENGINE_HOME"));
    }

    #[test]
    fn decision_protocol_is_exact_not_a_child_exit_code() {
        assert_eq!(decision(b"miss\n").unwrap(), Decision::Miss);
        assert_eq!(decision(b"committed\n").unwrap(), Decision::Committed);
        for invalid in [
            b"".as_slice(),
            b"miss",
            b"miss\ncommitted\n",
            b"miss\nnoise",
            b"125\n",
        ] {
            assert!(decision(invalid).is_err());
        }
    }
    #[test]
    fn admits_only_explicit_compiler_workloads() {
        for command in ["build", "test", "check", "run"] {
            assert!(is_workload(Some(OsStr::new(command))));
        }
        for command in ["clippy", "install", "cargo", "--help"] {
            assert!(!is_workload(Some(OsStr::new(command))));
        }
        assert!(!is_workload(None));
    }
}
