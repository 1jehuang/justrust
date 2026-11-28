//! Opt-in, serial, std-only native compiler RPC. No request may be replayed after C.
//! This runs inside the scheduled Executor job, retaining its implicit Cargo permit.

use crate::CargoResult;
use cargo_util::ProcessBuilder;
#[cfg(target_os = "linux")]
use jr_resident_inputs::Manifest;

pub(super) fn candidate(cmd: &ProcessBuilder) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::candidate(cmd)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cmd;
        false
    }
}

pub(super) fn execute(
    cmd: &ProcessBuilder,
    stdout: &mut dyn FnMut(&str) -> CargoResult<()>,
    stderr: &mut dyn FnMut(&str) -> CargoResult<()>,
) -> Option<CargoResult<()>> {
    #[cfg(target_os = "linux")]
    {
        linux::execute(cmd, stdout, stderr)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (cmd, stdout, stderr);
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::{Context, bail, ensure};
    use portable_atomic::AtomicU64;
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    const JOBSERVER: &[&str] = &["CARGO_MAKEFLAGS", "MAKEFLAGS", "MFLAGS"];
    const RECORDING: &[&str] = &[
        "JUSTRUST_RUSTC_PHASE_LOG",
        "JUSTRUST_RUSTC_PHASES_FILE",
        "JUSTRUST_RUSTC_EARLY_REPLY",
        "JUSTRUST_RUSTC_RESIDENT_FD",
        "JUSTRUST_RUSTC_RESIDENT_CARGO_FD",
        "JUSTRUST_QUERY_BINDING",
        "JUSTRUST_QUERY_BINDING_FD",
    ];
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    fn flag_enabled(cmd: &ProcessBuilder, name: &str, default: bool) -> bool {
        match cmd.get_env(name).as_deref().and_then(OsStr::to_str) {
            Some("0" | "off" | "false" | "no") => false,
            Some("1" | "on" | "true" | "yes") => true,
            _ => default,
        }
    }

    pub(super) fn candidate(cmd: &ProcessBuilder) -> bool {
        cmd.get_env("JUSTRUST_CARGO_RESIDENT_DIR").is_some()
            && Path::new(cmd.get_program()).is_absolute()
            && cmd.is_jr_native_compiler()
            && cmd.is_unwrapped()
            && !cmd.has_jr_query_launch()
            && cmd.get_arg0().is_none()
            && RECORDING.iter().all(|k| cmd.get_env(k).is_none())
            && args_allowed_with_external(
                &cmd.get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                flag_enabled(cmd, "JUSTRUST_CARGO_RESIDENT_EXTERNAL", true),
            )
            && cmd.get_args().all(|a| a.to_str().is_some())
            && matches!(
                Path::new(cmd.get_program())
                    .file_name()
                    .and_then(OsStr::to_str),
                Some("rustc" | "jr-rustc")
            )
    }

    #[cfg(test)]
    fn args_allowed(args: &[String]) -> bool {
        args_allowed_with_external(args, false)
    }

    fn args_allowed_with_external(args: &[String], external: bool) -> bool {
        ["--jobs-backend=1", "--jobs-linker=1"]
            .iter()
            .all(|flag| args.iter().filter(|a| a.as_str() == *flag).count() == 1)
            && !args.iter().any(|a| {
                a.starts_with('@')
                    || (!external && (a == "--extern" || a.starts_with("--extern=")))
                    || a == "--target"
                    || a.starts_with("--target=")
                    || (a.starts_with("--jobs-backend") && a != "--jobs-backend=1")
                    || (a.starts_with("--jobs-linker") && a != "--jobs-linker=1")
                    || a == "--jobs"
                    || a.starts_with("--jobs=")
                    || a.starts_with("-j")
                    || a == "--jobs-frontend"
                    || a.starts_with("--jobs-frontend=")
                    || a == "-"
                    || a.starts_with("--jr-resident")
                    || a.strip_prefix("-Z").is_some_and(|z| {
                        z.starts_with("threads") || z.starts_with("codegen-backend")
                    })
                    || a.starts_with("threads=")
                    || a.starts_with("codegen-backend=")
                    || a.contains("proc-macro")
                    || a == "--print"
                    || a.starts_with("--print=")
            })
    }

    struct Prepared {
        stream: UnixStream,
        _lock: File,
        key: String,
        external: Option<Manifest>,
    }

    fn private_dir(path: &Path) -> CargoResult<()> {
        ensure!(path.is_absolute(), "resident directory must be absolute");
        let meta = fs::symlink_metadata(path)?;
        ensure!(
            meta.is_dir()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o7777 == 0o700,
            "resident directory must be an owned, non-symlink 0700 directory"
        );
        ensure!(
            fs::canonicalize(path)? == path,
            "resident directory must not traverse symlinks"
        );
        Ok(())
    }

    fn check_socket(path: &Path) -> CargoResult<()> {
        let meta = fs::symlink_metadata(path)?;
        ensure!(
            meta.file_type().is_socket() && meta.uid() == unsafe { libc::geteuid() },
            "untrusted resident socket"
        );
        Ok(())
    }

    fn effective_env(cmd: &ProcessBuilder) -> BTreeMap<OsString, OsString> {
        let mut env = cmd.get_effective_env();
        for key in JOBSERVER {
            env.remove(OsStr::new(key));
        }
        // No other variable is observationally invisible: even telemetry names
        // can be inputs to env!/option_env! in ordinary Rust source.
        env
    }

    fn field(hash: &mut blake3::Hasher, bytes: &[u8]) {
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }

    fn identity(hash: &mut blake3::Hasher, path: &Path) -> CargoResult<()> {
        let meta = fs::metadata(path)?;
        field(hash, path.as_os_str().as_bytes());
        for value in [
            meta.dev(),
            meta.ino(),
            meta.size(),
            meta.mtime() as u64,
            meta.mtime_nsec() as u64,
            meta.ctime() as u64,
            meta.ctime_nsec() as u64,
        ] {
            hash.update(&value.to_le_bytes());
        }
        if meta.is_dir() {
            let mut entries = fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<io::Result<Vec<_>>>()?;
            entries.sort();
            for entry in entries {
                // Follow library symlinks, but not directory symlinks (no cycles).
                if fs::symlink_metadata(&entry)?.file_type().is_symlink() && entry.is_dir() {
                    field(hash, fs::canonicalize(&entry)?.as_os_str().as_bytes());
                    bail!("directory symlinks in resident sysroot are unsupported");
                }
                identity(hash, &entry)?;
            }
        }
        Ok(())
    }

    fn prepare(cmd: &ProcessBuilder) -> CargoResult<Prepared> {
        let directory = PathBuf::from(cmd.get_env("JUSTRUST_CARGO_RESIDENT_DIR").unwrap());
        private_dir(&directory)?;
        let compiler = fs::canonicalize(cmd.get_program())?;
        let cwd = fs::canonicalize(
            cmd.get_cwd()
                .map(Path::to_path_buf)
                .unwrap_or(std::env::current_dir()?),
        )?;
        let args = cmd.get_args().collect::<Vec<_>>();
        let mut sysroot = None;
        for (index, arg) in args.iter().enumerate() {
            if *arg == "--sysroot" {
                sysroot = args.get(index + 1).map(|a| PathBuf::from(a));
            }
            if let Some(value) = arg.to_str().and_then(|a| a.strip_prefix("--sysroot=")) {
                sysroot = Some(value.into());
            }
        }
        let sysroot =
            sysroot.unwrap_or_else(|| compiler.parent().unwrap().parent().unwrap().to_path_buf());
        let sysroot = fs::canonicalize(if sysroot.is_absolute() {
            sysroot
        } else {
            cwd.join(sysroot)
        })?;
        let env = effective_env(cmd);
        let external = if flag_enabled(cmd, "JUSTRUST_CARGO_RESIDENT_EXTERNAL", true) {
            let args = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            if flag_enabled(cmd, "JUSTRUST_CARGO_RESIDENT_MACROS", true) {
                ensure!(
                    flag_enabled(cmd, "JUSTRUST_CARGO_RESIDENT_OWNED", true),
                    "macro helpers require the owned compiler batch"
                );
                ensure!(
                    cmd.get_env("JUSTRUST_CARGO_RESIDENT_DECODERS")
                        .as_deref()
                        .is_none_or(|value| value == OsStr::new("0")),
                    "macro helpers cannot retain external decoders"
                );
                jr_resident_inputs::capture_with_macros(&args, &cwd, &env)
            } else {
                jr_resident_inputs::capture(&args, &cwd)
            }
            .map_err(anyhow::Error::msg)?
        } else {
            None
        };
        let mut hash = blake3::Hasher::new();
        field(
            &mut hash,
            b"jr-cargo-resident-v3-fresh-macro-runtime-closure",
        );
        if let Some(inputs) = &external {
            // capture already validated bounded content. Hash the canonical
            // serialized manifest, not file timestamps or a caller's stale key.
            field(&mut hash, &serde_json::to_vec(inputs)?);
        }
        identity(&mut hash, &compiler)?;
        // Include runtime compiler libraries as well as the target libraries.
        let compiler_lib = compiler
            .parent()
            .and_then(Path::parent)
            .context("compiler has no installation prefix")?
            .join("lib");
        identity(&mut hash, &compiler_lib)?;
        if fs::canonicalize(&compiler_lib)? != fs::canonicalize(sysroot.join("lib"))? {
            identity(&mut hash, &sysroot.join("lib"))?;
        }
        field(&mut hash, b"cwd");
        field(&mut hash, cwd.as_os_str().as_bytes());
        field(&mut hash, b"argv");
        hash.update(&(args.len() as u64).to_le_bytes());
        for arg in &args {
            field(&mut hash, arg.as_bytes());
        }
        field(&mut hash, b"env");
        hash.update(&(env.len() as u64).to_le_bytes());
        for (k, v) in &env {
            field(&mut hash, k.as_bytes());
            field(&mut hash, v.as_bytes());
        }
        let key = hash.finalize().to_hex().to_string();
        let socket = directory.join(format!("{}.sock", &key[..32]));
        ensure!(
            socket.as_os_str().as_bytes().len() < 108,
            "resident socket path too long"
        );
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join(format!("{}.lock", &key[..32])))?;
        let meta = lock.metadata()?;
        ensure!(
            meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
            "untrusted resident lock"
        );
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
            "resident lock failed"
        );
        private_dir(&directory)?;
        if socket.exists() {
            check_socket(&socket)?;
            if let Ok(stream) = UnixStream::connect(&socket) {
                return Ok(Prepared {
                    stream,
                    _lock: lock,
                    key,
                    external,
                });
            }
            fs::remove_file(&socket)?;
        }
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        let fd = listener.as_raw_fd();
        let mut child = Command::new(&compiler);
        child
            .arg("--jr-resident-cargo")
            .current_dir(&cwd)
            .env_clear()
            .envs(&env)
            .env("JUSTRUST_RUSTC_RESIDENT_CARGO_FD", fd.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Do not configure Cargo's jobserver on this Command. Close every inherited
        // fd at exec except the listener, including a non-CLOEXEC jobserver.
        unsafe {
            child.pre_exec(move || {
                if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32) < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        worker_process_group(&mut child);
        let mut child = child.spawn()?;
        drop(listener);
        // Reap the eventual worker exit without waiting for its persistent life.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        check_socket(&socket)?;
        let stream = UnixStream::connect(&socket)?;
        Ok(Prepared {
            stream,
            _lock: lock,
            key,
            external,
        })
    }

    fn worker_process_group(command: &mut Command) {
        // The native server owns cancellation of its entire linker/descendant
        // process group. Never let it share Cargo's or the invoking agent's group.
        command.process_group(0);
    }

    fn pipe() -> io::Result<(File, OwnedFd)> {
        let mut fds = [-1; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { (File::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
    }

    fn send_request(stream: &mut UnixStream, request: &[u8], fds: [i32; 2]) -> io::Result<()> {
        let mut iov = libc::iovec {
            iov_base: request.as_ptr().cast_mut().cast(),
            iov_len: 1,
        };
        // usize storage guarantees cmsghdr alignment.
        let mut control = [0usize; 8];
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen =
            unsafe { libc::CMSG_SPACE(std::mem::size_of_val(&fds) as u32) } as usize;
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(&fds) as u32) as usize;
            std::ptr::copy_nonoverlapping(
                fds.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(cmsg),
                std::mem::size_of_val(&fds),
            );
            loop {
                if libc::sendmsg(stream.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) == 1 {
                    break;
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
        stream.write_all(&request[1..])
    }

    pub(super) fn execute(
        cmd: &ProcessBuilder,
        stdout: &mut dyn FnMut(&str) -> CargoResult<()>,
        stderr: &mut dyn FnMut(&str) -> CargoResult<()>,
    ) -> Option<CargoResult<()>> {
        if !candidate(cmd) {
            return None;
        }
        if !flag_enabled(cmd, "JUSTRUST_CARGO_RESIDENT", true) {
            trace(cmd, serde_json::json!({"event":"disabled"}));
            return None;
        }
        let started = Instant::now();
        let result = before_commit(cmd);
        let (mut prepared, out, err, id) = match result {
            Ok(value) => value,
            Err(error) => {
                trace(
                    cmd,
                    serde_json::json!({"event":"fallback","error":error.to_string()}),
                );
                return None;
            }
        };
        // From this point even a short/failed C write is an uncertain commit.
        // NEVER return None after attempting it.
        let result = committed(
            cmd,
            &mut prepared,
            (out, err, id),
            (stdout, stderr),
            started,
        );
        if let Err(error) = &result {
            trace(
                cmd,
                serde_json::json!({"event":"committed_error","error":error.to_string(),"id":id}),
            );
        }
        Some(result)
    }

    fn committed(
        cmd: &ProcessBuilder,
        prepared: &mut Prepared,
        pipes: (File, File, u64),
        callbacks: (
            &mut dyn FnMut(&str) -> CargoResult<()>,
            &mut dyn FnMut(&str) -> CargoResult<()>,
        ),
        started: Instant,
    ) -> CargoResult<()> {
        let (out, err, id) = pipes;
        let (stdout, stderr) = callbacks;
        prepared
            .stream
            .write_all(b"C")
            .context("resident commit failed, request will not be replayed")?;
        prepared.stream.set_read_timeout(None)?;
        let mut callback_error = None;
        std::thread::scope(|scope| -> CargoResult<()> {
            let (tx, rx) = std::sync::mpsc::sync_channel(64);
            for (is_out, file) in [(true, out), (false, err)] {
                let tx = tx.clone();
                scope.spawn(move || {
                    let mut reader = BufReader::new(file);
                    loop {
                        let mut bytes = Vec::new();
                        match reader.read_until(b'\n', &mut bytes) {
                            Ok(0) => break,
                            Ok(_) => {
                                if tx.send(Ok((is_out, bytes))).is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                let _ = tx.send(Err(error));
                                break;
                            }
                        }
                    }
                });
            }
            drop(tx);
            for line in rx {
                match line {
                    Ok((is_out, bytes)) if callback_error.is_none() => {
                        for line in String::from_utf8_lossy(&bytes).lines() {
                            let result = if is_out { stdout(line) } else { stderr(line) };
                            if let Err(error) = result {
                                callback_error = Some(error);
                                break;
                            }
                        }
                    }
                    Err(error) if callback_error.is_none() => callback_error = Some(error.into()),
                    _ => {}
                }
            }
            Ok(())
        })?;
        let mut completion = String::new();
        let mut reader = BufReader::new(&mut prepared.stream).take(1024 * 1024);
        reader
            .read_line(&mut completion)
            .context("resident completion lost, request will not be replayed")?;
        ensure!(
            completion.ends_with('\n'),
            "truncated resident completion, request will not be replayed"
        );
        let completion: serde_json::Value = serde_json::from_str(&completion)?;
        ensure!(
            completion["id"].as_u64() == Some(id),
            "resident completion id mismatch"
        );
        let status = completion["status"]
            .as_i64()
            .context("resident completion missing status")?;
        ensure!((0..=255).contains(&status), "invalid resident exit status");
        trace(
            cmd,
            serde_json::json!({"event":"complete","key":prepared.key,"elapsed_ns":started.elapsed().as_nanos(),"result":completion}),
        );
        if let Some(error) = callback_error {
            return Err(error);
        }
        if status != 0 {
            return Err(cargo_util::ProcessError::new_raw(
                &format!("process didn't exit successfully: {cmd}"),
                Some(status as i32),
                &format!("exit status: {status}"),
                None,
                None,
            )
            .into());
        }
        Ok(())
    }

    fn before_commit(cmd: &ProcessBuilder) -> CargoResult<(Prepared, File, File, u64)> {
        handshake(cmd, prepare(cmd)?)
    }

    fn handshake(
        cmd: &ProcessBuilder,
        mut prepared: Prepared,
    ) -> CargoResult<(Prepared, File, File, u64)> {
        prepared
            .stream
            .set_read_timeout(Some(Duration::from_secs(10)))?;
        prepared
            .stream
            .set_write_timeout(Some(Duration::from_secs(10)))?;
        let (out, out_writer) = pipe()?;
        let (err, err_writer) = pipe()?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let args = cmd
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect::<Vec<_>>();
        let mut payload = serde_json::json!({"id":id,"args":args});
        if let Some(inputs) = &prepared.external {
            payload["external"] = serde_json::to_value(inputs)?;
        }
        let mut request = serde_json::to_vec(&payload)?;
        ensure!(
            request.len() < 1024 * 1024,
            "resident request exceeds wire budget"
        );
        request.push(b'\n');
        send_request(
            &mut prepared.stream,
            &request,
            [out_writer.as_raw_fd(), err_writer.as_raw_fd()],
        )?;
        drop((out_writer, err_writer));
        let mut ready = [0];
        prepared.stream.read_exact(&mut ready)?;
        ensure!(
            ready == *b"R",
            "resident refused request before commit: {}",
            refusal_reason(ready[0])
        );
        Ok((prepared, out, err, id))
    }

    // One byte only, no peer-controlled strings or follow-up reads. Old F and
    // unknown replies remain precommit fallback, never permission to send C.
    fn refusal_reason(code: u8) -> &'static str {
        match code {
            b'1' => "receive: invalid request framing or output descriptors",
            b'2' => "decode_request: invalid or ineligible request, or initial external manifest verification failed",
            b'3' => "frozen_mismatch: argv or external inputs differ from committed worker identity",
            b'4' => "rss: worker memory unavailable or at retirement limit",
            b'5' => "owned_environment: unsupported owned compiler environment",
            b'6' => "prepared_admission: compiler preparation failed or unsupported prepared configuration",
            b'7' => "external_recapture_error: could not recapture actual inputs immediately before commit",
            b'8' => "external_recapture_changed: actual inputs differ from request manifest immediately before commit",
            b'F' => "legacy refusal (no stage supplied)",
            _ => "unknown precommit response",
        }
    }

    fn trace(cmd: &ProcessBuilder, mut value: serde_json::Value) {
        let Some(path) = cmd.get_env("JUSTRUST_CARGO_RESIDENT_TRACE") else {
            return;
        };
        let args = cmd.get_args().collect::<Vec<_>>();
        let crate_name = args.iter().enumerate().find_map(|(index, arg)| {
            if *arg == "--crate-name" {
                args.get(index + 1).and_then(|name| name.to_str())
            } else {
                arg.to_str()
                    .and_then(|arg| arg.strip_prefix("--crate-name="))
            }
        });
        if let Some(crate_name) = crate_name {
            value["crate_name"] = crate_name.into();
        }
        if let Ok(mut file) = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            if let Ok(mut bytes) = serde_json::to_vec(&value) {
                bytes.push(b'\n');
                let _ = file.write_all(&bytes);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn trace_attributes_actual_crate_name() {
            for args in [vec!["--crate-name", "root"], vec!["--crate-name=root"]] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("trace");
                let mut cmd = ProcessBuilder::new("rustc");
                cmd.args(&args).env("JUSTRUST_CARGO_RESIDENT_TRACE", &path);
                trace(&cmd, serde_json::json!({"event":"complete"}));
                let value: serde_json::Value =
                    serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                assert_eq!(value["crate_name"], "root");
            }
        }
        fn test_command() -> ProcessBuilder {
            let mut cmd = ProcessBuilder::new("/owned/bin/rustc");
            cmd.jr_native_compiler(true);
            cmd.env("JUSTRUST_CARGO_RESIDENT_DIR", "/private");
            for name in RECORDING {
                cmd.env_remove(name);
            }
            cmd.args(&["--jobs-backend=1", "--jobs-linker=1", "main.rs"]);
            cmd
        }

        fn prepared(stream: UnixStream) -> Prepared {
            Prepared {
                stream,
                _lock: tempfile::tempfile().unwrap(),
                key: "test".into(),
                external: None,
            }
        }

        // The fake peer asserts the exact ancillary framing used by the native server.
        fn receive(peer: &mut UnixStream) -> (u64, [File; 2]) {
            let mut first = [0u8];
            let mut iov = libc::iovec {
                iov_base: first.as_mut_ptr().cast(),
                iov_len: 1,
            };
            let mut control = [0usize; 8];
            let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = std::mem::size_of_val(&control);
            assert_eq!(
                unsafe { libc::recvmsg(peer.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) },
                1
            );
            assert_eq!(first, *b"{");
            assert_eq!(msg.msg_flags & libc::MSG_CTRUNC, 0);
            let files = unsafe {
                let header = libc::CMSG_FIRSTHDR(&msg);
                assert!(!header.is_null());
                assert_eq!((*header).cmsg_level, libc::SOL_SOCKET);
                assert_eq!((*header).cmsg_type, libc::SCM_RIGHTS);
                assert_eq!((*header).cmsg_len, libc::CMSG_LEN(8) as usize);
                assert!(libc::CMSG_NXTHDR(&msg, header).is_null());
                let fds = libc::CMSG_DATA(header).cast::<i32>();
                [
                    File::from_raw_fd(fds.read_unaligned()),
                    File::from_raw_fd(fds.add(1).read_unaligned()),
                ]
            };
            let mut request = first.to_vec();
            while !request.ends_with(b"\n") {
                peer.read_exact(&mut first).unwrap();
                request.push(first[0]);
            }
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(
                request["args"],
                serde_json::json!(["--jobs-backend=1", "--jobs-linker=1", "main.rs"])
            );
            (request["id"].as_u64().unwrap(), files)
        }

        #[test]
        fn handshake_refusal_never_commits() {
            let (client, mut peer) = UnixStream::pair().unwrap();
            let server = std::thread::spawn(move || {
                let (_, _writers) = receive(&mut peer);
                peer.write_all(b"F").unwrap();
                let mut byte = [0];
                assert_eq!(peer.read(&mut byte).unwrap(), 0);
            });
            assert!(handshake(&test_command(), prepared(client)).is_err());
            server.join().unwrap();
        }

        #[test]
        fn bounded_stage_refusals_never_commit() {
            for code in b"12345678F?C" {
                let (client, mut peer) = UnixStream::pair().unwrap();
                let code = *code;
                let server = std::thread::spawn(move || {
                    let (_, _writers) = receive(&mut peer);
                    peer.write_all(&[code]).unwrap();
                    let mut byte = [0];
                    assert_eq!(peer.read(&mut byte).unwrap(), 0);
                });
                let error = match handshake(&test_command(), prepared(client)) {
                    Ok(_) => panic!("refusal must not be admitted"),
                    Err(error) => error,
                };
                assert!(error.to_string().contains(refusal_reason(code)));
                assert!(refusal_reason(code).len() < 160);
                server.join().unwrap();
            }
        }

        #[test]
        fn streams_both_outputs_and_preserves_partial_lines() {
            let (client, mut peer) = UnixStream::pair().unwrap();
            let server = std::thread::spawn(move || {
                let (id, [mut out, mut err]) = receive(&mut peer);
                peer.write_all(b"R").unwrap();
                let mut byte = [0];
                peer.read_exact(&mut byte).unwrap();
                assert_eq!(byte, *b"C");
                // Larger than pipe capacity, so sequential draining would deadlock.
                for _ in 0..4096 {
                    out.write_all(b"cargo stdout line\n").unwrap();
                    err.write_all(b"cargo stderr line\n").unwrap();
                }
                err.write_all(b"trailing partial").unwrap();
                drop((out, err));
                writeln!(
                    peer,
                    "{}",
                    serde_json::json!({"id":id,"status":0,"pid":123})
                )
                .unwrap();
            });
            let cmd = test_command();
            let (mut prepared, out, err, id) = handshake(&cmd, prepared(client)).unwrap();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            committed(
                &cmd,
                &mut prepared,
                (out, err, id),
                (
                    &mut |line| {
                        stdout.push(line.to_owned());
                        Ok(())
                    },
                    &mut |line| {
                        stderr.push(line.to_owned());
                        Ok(())
                    },
                ),
                Instant::now(),
            )
            .unwrap();
            server.join().unwrap();
            assert_eq!(stdout.len(), 4096);
            assert_eq!(stderr.len(), 4097);
            assert_eq!(stderr.last().unwrap(), "trailing partial");
        }

        #[test]
        fn committed_disconnect_is_an_error_not_a_replay() {
            let (client, mut peer) = UnixStream::pair().unwrap();
            let server = std::thread::spawn(move || {
                let (_, writers) = receive(&mut peer);
                peer.write_all(b"R").unwrap();
                let mut byte = [0];
                peer.read_exact(&mut byte).unwrap();
                assert_eq!(byte, *b"C");
                drop(writers);
            });
            let cmd = test_command();
            let (mut prepared, out, err, id) = handshake(&cmd, prepared(client)).unwrap();
            let result = committed(
                &cmd,
                &mut prepared,
                (out, err, id),
                (&mut |_| Ok(()), &mut |_| Ok(())),
                Instant::now(),
            );
            assert!(result.unwrap_err().to_string().contains("completion"));
            server.join().unwrap();
        }

        #[test]
        fn namespace_rejects_permissions_and_symlinks() {
            let temp = tempfile::tempdir().unwrap();
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
            assert!(private_dir(temp.path()).is_ok());
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();
            assert!(private_dir(temp.path()).is_err());
            fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let link = temp.path().join("link");
            std::os::unix::fs::symlink(temp.path(), &link).unwrap();
            assert!(private_dir(&link).is_err());
        }

        #[test]
        fn worker_process_group_child() {
            if ProcessBuilder::new("unused")
                .get_env("JUSTRUST_TEST_RESIDENT_GROUP_CHILD")
                .is_some()
            {
                assert_eq!(unsafe { libc::getpgrp() }, unsafe { libc::getpid() });
            }
        }

        #[test]
        fn worker_starts_in_dedicated_process_group() {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "compiler::jr_resident::linux::tests::worker_process_group_child",
                ])
                .env("JUSTRUST_TEST_RESIDENT_GROUP_CHILD", "1");
            worker_process_group(&mut command);
            assert!(command.status().unwrap().success());
        }

        #[test]
        fn off_control_retains_candidate_without_attempting_rpc() {
            let mut cmd = test_command();
            cmd.env_remove("JUSTRUST_CARGO_RESIDENT_TRACE");
            let args = cmd.get_args().cloned().collect::<Vec<_>>();
            let program = cmd.get_program().clone();
            cmd.env("JUSTRUST_CARGO_RESIDENT", "off");
            assert!(candidate(&cmd));
            // /private intentionally does not exist. Off must not even prepare
            // a namespace, lock, listener or worker, nor touch output callbacks.
            assert!(
                execute(&cmd, &mut |_| panic!("unexpected stdout"), &mut |_| panic!(
                    "unexpected stderr"
                ))
                .is_none()
            );
            cmd.env("JUSTRUST_CARGO_RESIDENT", "on");
            assert!(candidate(&cmd));
            assert_eq!(cmd.get_program(), &program);
            assert_eq!(cmd.get_args().cloned().collect::<Vec<_>>(), args);
        }

        #[test]
        fn arbitrary_compiler_launchers_are_not_probed() {
            let mut cmd = test_command();
            assert!(candidate(&cmd));
            cmd.jr_native_compiler(false);
            assert!(!candidate(&cmd));
            cmd.jr_native_compiler(true);
            cmd.program("/launcher/bin/rustc");
            assert!(!candidate(&cmd));
        }

        #[test]
        fn observation_is_preserved_and_incompatible_recording_refused() {
            let mut cmd = test_command();
            cmd.env("JUSTRUST_RUN_DIR", "/run/one")
                .env("JUSTRUST_RUN_ID", "one")
                .env("JUSTRUST_CARGO_RESIDENT_TRACE", "/trace/one");
            assert!(candidate(&cmd));
            let before = effective_env(&cmd);
            for (name, expected) in [
                ("JUSTRUST_RUN_DIR", "/run/one"),
                ("JUSTRUST_RUN_ID", "one"),
                ("JUSTRUST_CARGO_RESIDENT_TRACE", "/trace/one"),
            ] {
                assert_eq!(
                    before.get(OsStr::new(name)).map(OsString::as_os_str),
                    Some(OsStr::new(expected))
                );
            }
            cmd.env("JUSTRUST_RUN_DIR", "/run/two")
                .env("JUSTRUST_RUN_ID", "two")
                .env("JUSTRUST_CARGO_RESIDENT_TRACE", "/trace/two");
            assert_ne!(before, effective_env(&cmd));
            for variable in RECORDING {
                cmd.env(variable, "present");
                assert!(!candidate(&cmd), "{variable}");
                cmd.env_remove(variable);
            }
        }

        #[test]
        fn restricts_args() {
            let base = vec![
                "--jobs-backend=1".into(),
                "--jobs-linker=1".into(),
                "main.rs".into(),
            ];
            assert!(args_allowed(&base));
            let mut external = base.clone();
            external.push("--extern=dep=dep.rlib".into());
            assert!(!args_allowed_with_external(&external, false));
            assert!(args_allowed_with_external(&external, true));
            external.push("--target=other".into());
            assert!(!args_allowed_with_external(&external, true));
            for bad in [
                "--extern",
                "--extern=x=y",
                "@args",
                "--target=x86_64-unknown-linux-gnu",
                "--target",
                "--crate-type=proc-macro",
                "--print=sysroot",
                "--jobs-backend=2",
                "--jobs-linker=8",
                "--jobs-backend",
            ] {
                let mut args = base.clone();
                args.push(bad.into());
                assert!(!args_allowed(&args), "{bad}");
            }
            assert!(!args_allowed(&base[..1]));
        }
        #[test]
        fn wrapper_is_never_unwrapped() {
            let mut cmd = ProcessBuilder::new("/owned/bin/rustc");
            cmd.jr_native_compiler(true);
            cmd.env("JUSTRUST_CARGO_RESIDENT_DIR", "/private");
            cmd.args(&["--jobs-backend=1", "--jobs-linker=1", "main.rs"]);
            let cmd = cmd.wrapped(Some("wrapper"));
            assert!(!candidate(&cmd));
        }
        #[test]
        fn effective_environment_preserves_overrides_and_removals() {
            let mut cmd = ProcessBuilder::new("rustc");
            cmd.env("JR_RESIDENT_TEST_KEY", "value")
                .env_remove("PATH")
                .env("MAKEFLAGS", "--jobserver-auth=8,9");
            let env = effective_env(&cmd);
            assert_eq!(
                env.get(OsStr::new("JR_RESIDENT_TEST_KEY")),
                Some(&OsString::from("value"))
            );
            assert!(!env.contains_key(OsStr::new("PATH")));
            assert!(!env.contains_key(OsStr::new("MAKEFLAGS")));
        }
    }
}
