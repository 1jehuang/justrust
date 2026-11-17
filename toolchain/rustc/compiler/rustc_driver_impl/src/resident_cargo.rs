//! Cargo's serial native compiler worker. Environment/cwd are launch-time inputs.
//! The default path retains snapshots only. Explicit owned selection scopes real
//! SessionGlobals and compiler owners across serial Cargo requests, without typed reuse.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use jr_resident_inputs::Manifest;
use serde_json::json;

const MAX_REQUEST: usize = 1024 * 1024;
const IDLE_MS: i32 = 60_000;
const RSS_LIMIT: usize = 2 * 1024 * 1024 * 1024;

fn error() -> String {
    io::Error::last_os_error().to_string()
}

/// Only the native dedicated-worker entry point may transfer this descriptor.
pub(super) unsafe fn run() -> ExitCode {
    match unsafe { serve() } {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("jr-rustc Cargo resident: {e}");
            ExitCode::FAILURE
        }
    }
}

fn eligible(args: &[String], external: bool) -> Result<(), String> {
    // Reject aliases/duplicates as well as overrides. The private compiler pool
    // has one implicit token, while Cargo holds its scheduler slot for this RPC.
    for option in ["--jobs-backend", "--jobs-linker"] {
        let matching: Vec<_> =
            args.iter().filter(|a| *a == option || a.starts_with(&format!("{option}="))).collect();
        if matching.len() != 1 || matching[0].as_str() != format!("{option}=1") {
            return Err(format!("requires exactly {option}=1"));
        }
    }
    for (i, arg) in args.iter().enumerate() {
        if (!external && (arg == "--extern" || arg.starts_with("--extern=")))
            || arg == "--target"
            || arg.starts_with("--target=")
            || arg == "--jobs"
            || arg.starts_with("--jobs=")
            || arg.starts_with("-j")
            || arg == "--jobs-frontend"
            || arg.starts_with("--jobs-frontend=")
            || arg.starts_with('@')
            || arg == "-"
            || arg.contains("proc-macro")
            || arg.starts_with("--jr-resident")
        {
            return Err("unsupported resident compiler input".into());
        }
        // No parallel frontend pool or LLVM backend override, including split -Z.
        let z = arg
            .strip_prefix("-Z")
            .or_else(|| (i > 0 && args[i - 1] == "-Z").then_some(arg.as_str()));
        if z.is_some_and(|z| z.starts_with("threads") || z.starts_with("codegen-backend")) {
            return Err("custom compiler pools/backends are unsupported".into());
        }
    }
    Ok(())
}

fn poll_read(fd: i32, timeout: i32) -> Result<bool, String> {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    loop {
        let n = unsafe { libc::poll(&mut p, 1, timeout) };
        if n >= 0 {
            return Ok(n != 0);
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(error());
        }
    }
}

// recvmsg consumes exactly the first JSON byte, so ancillary rights cannot be
// lost in a buffered reader. OwnedFd closes even malformed/truncated transfers.
struct Request {
    id: u64,
    args: Vec<String>,
    outputs: Vec<OwnedFd>,
    external: Option<Manifest>,
    external_verify_nanos: u64,
}

/// An external manifest must describe the actual invocation, not just a
/// caller-selected subset of dependency files. Recapture before admission.
fn decode_request(
    line: &[u8],
    external_enabled: bool,
    cwd: &std::path::Path,
) -> Result<(u64, Vec<String>, Option<Manifest>, u64), String> {
    let mut value: serde_json::Value = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    let external = value
        .as_object_mut()
        .ok_or("request must be an object")?
        .remove("external")
        .map(serde_json::from_value::<Manifest>)
        .transpose()
        .map_err(|e| e.to_string())?;
    let (id, args) =
        super::resident::request(&serde_json::to_vec(&value).map_err(|e| e.to_string())?)?;
    eligible(&args, external.is_some())?;
    let start = Instant::now();
    if let Some(expected) = &external {
        if !external_enabled {
            return Err("external resident inputs require explicit selection".into());
        }
        if jr_resident_inputs::capture(&args, cwd)?.as_ref() != Some(expected) {
            return Err("external manifest does not match actual compiler inputs".into());
        }
    }
    let nanos = if external.is_some() {
        u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    } else {
        0
    };
    Ok((id, args, external, nanos))
}

fn receive(stream: &mut UnixStream) -> Result<Request, String> {
    let mut first = [0u8];
    let mut iov = libc::iovec { iov_base: first.as_mut_ptr().cast(), iov_len: 1 };
    let mut control = [0usize; 32];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = std::mem::size_of_val(&control);
    let n = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n != 1 {
        return Err("missing first request byte".into());
    }
    let mut fds = Vec::new();
    let mut invalid = msg.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0;
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&msg);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*header).cmsg_len.saturating_sub(libc::CMSG_LEN(0) as usize);
                if bytes % std::mem::size_of::<i32>() != 0 {
                    invalid = true;
                }
                for i in 0..bytes / std::mem::size_of::<i32>() {
                    let fd = libc::CMSG_DATA(header).cast::<i32>().add(i).read_unaligned();
                    fds.push(OwnedFd::from_raw_fd(fd));
                }
            } else {
                invalid = true;
            }
            header = libc::CMSG_NXTHDR(&msg, header);
        }
    }
    if invalid || fds.len() != 2 {
        return Err("requires exactly two SCM_RIGHTS descriptors".into());
    }
    for fd in &fds {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if fd.as_raw_fd() < 3
            || flags < 0
            || unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) } != 0
        {
            return Err("invalid output descriptor".into());
        }
        if unsafe { stat.assume_init() }.st_mode & libc::S_IFMT != libc::S_IFIFO
            || flags & libc::O_ACCMODE != libc::O_WRONLY
        {
            return Err("output descriptors must be pipe writers".into());
        }
    }
    let mut line = first.to_vec();
    while !line.ends_with(b"\n") {
        if line.len() >= MAX_REQUEST {
            return Err("oversized request".into());
        }
        let mut b = [0];
        stream.read_exact(&mut b).map_err(|e| e.to_string())?;
        line.push(b[0]);
    }
    let (id, args, external, external_verify_nanos) = decode_request(
        &line,
        std::env::var_os("JUSTRUST_CARGO_RESIDENT_EXTERNAL").as_deref()
            == Some(std::ffi::OsStr::new("1")),
        &std::env::current_dir().map_err(|e| e.to_string())?,
    )?;
    Ok(Request { id, args, outputs: fds, external, external_verify_nanos })
}

// Shutdown is explicit and joined. No detached thread can observe a reused fd.
struct Cancellation {
    event: OwnedFd,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Cancellation {
    fn start(stream: &UnixStream) -> Result<Self, String> {
        let group = unsafe { libc::getpgrp() };
        if group != unsafe { libc::getpid() } {
            return Err("worker must lead a dedicated process group".into());
        }
        let peer = stream.try_clone().map_err(|e| e.to_string())?;
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(error());
        }
        let event = unsafe { OwnedFd::from_raw_fd(fd) };
        let thread = std::thread::Builder::new()
            .name("resident-cancel".into())
            .spawn(move || {
                let mut p = [
                    libc::pollfd {
                        fd: peer.as_raw_fd(),
                        events: libc::POLLRDHUP | libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd { fd, events: libc::POLLIN, revents: 0 },
                ];
                loop {
                    let n = unsafe { libc::poll(p.as_mut_ptr(), 2, -1) };
                    if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    // A peer close wins even if normal completion races the wakeup.
                    if n < 0 || p[0].revents != 0 {
                        unsafe {
                            libc::kill(-group, libc::SIGKILL);
                        }
                        return;
                    }
                    if p[1].revents != 0 {
                        return;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { event, thread: Some(thread) })
    }
}
impl Drop for Cancellation {
    fn drop(&mut self) {
        let one = 1u64;
        loop {
            let n = unsafe { libc::write(self.event.as_raw_fd(), (&one as *const u64).cast(), 8) };
            if n == 8 {
                break;
            }
            if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            // A failed wake must not leave an unjoinable watcher behind.
            unsafe {
                libc::_exit(1);
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Outputs {
    null: File,
}
impl Outputs {
    fn attach(fds: &[OwnedFd]) -> Result<Self, String> {
        io::stdout().flush().map_err(|e| e.to_string())?;
        io::stderr().flush().map_err(|e| e.to_string())?;
        if unsafe { libc::fflush(std::ptr::null_mut()) } != 0 {
            return Err("could not flush pre-request streams".into());
        }
        let guard = Self {
            null: OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/null")
                .map_err(|e| e.to_string())?,
        };
        for (fd, target) in fds.iter().zip([1, 2]) {
            if unsafe { libc::dup2(fd.as_raw_fd(), target) } < 0 {
                return Err(error());
            }
        }
        Ok(guard)
    }
    fn finish(self) -> Result<(), String> {
        let out = io::stdout().flush();
        let err = io::stderr().flush();
        let c = unsafe { libc::fflush(std::ptr::null_mut()) };
        drop(self);
        out.and(err).map_err(|e| e.to_string())?;
        if c != 0 {
            return Err("could not flush native compiler streams".into());
        }
        Ok(())
    }
}
impl Drop for Outputs {
    fn drop(&mut self) {
        for fd in [1, 2] {
            if unsafe { libc::dup2(self.null.as_raw_fd(), fd) } < 0 {
                // Never send a completion while a pipe writer is still held.
                unsafe {
                    libc::_exit(1);
                }
            }
        }
    }
}

unsafe fn serve() -> Result<(), String> {
    if unsafe { libc::getpgrp() != libc::getpid() } {
        return Err("worker must lead a dedicated process group".into());
    }
    for name in [
        "CARGO_MAKEFLAGS",
        "MAKEFLAGS",
        "MFLAGS",
        "JUSTRUST_RUSTC_PHASES_FILE",
        "JUSTRUST_RUSTC_EARLY_REPLY",
    ] {
        if std::env::var_os(name).is_some() {
            return Err(format!("{name} must not be inherited"));
        }
    }
    let fd: i32 = std::env::var("JUSTRUST_RUSTC_RESIDENT_CARGO_FD")
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|_| "invalid listener fd")?;
    if fd < 3 {
        return Err("listener must not alias stdio".into());
    }
    let mut accept = 0i32;
    let mut len = std::mem::size_of_val(&accept) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ACCEPTCONN,
            (&mut accept as *mut i32).cast(),
            &mut len,
        )
    } != 0
        || accept != 1
    {
        return Err("descriptor must be a listening socket".into());
    }
    let mut address: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&address) as libc::socklen_t;
    if unsafe {
        libc::getsockname(fd, (&mut address as *mut libc::sockaddr_storage).cast(), &mut len)
    } != 0
        || address.ss_family as i32 != libc::AF_UNIX
    {
        return Err("listener must be AF_UNIX".into());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(error());
    }
    let listener = unsafe { UnixListener::from_raw_fd(fd) };
    let null =
        OpenOptions::new().read(true).write(true).open("/dev/null").map_err(|e| e.to_string())?;
    for stdio in [0, 1, 2] {
        if unsafe { libc::dup2(null.as_raw_fd(), stdio) } < 0 {
            return Err(error());
        }
    }
    // If stdio was initially closed, opening /dev/null can occupy fd 0..2.
    // Keep that particular owner alive rather than closing restored stdio.
    if null.as_raw_fd() < 3 {
        std::mem::forget(null);
    }
    rustc_data_structures::jobserver::initialize(1, |_| {});
    rustc_session::resident::enable_process();
    serve_requests(
        listener,
        None,
        None,
        owned_selection(std::env::var_os("JUSTRUST_CARGO_RESIDENT_OWNED").as_deref()),
    )
}

type PreparedRequest = (UnixStream, Request, crate::PreparedCompiler, crate::TimePassesCallbacks);

/// Preparation is admission, not execution. In particular help/version requests
/// and callback-owned configurations must fall back before the R/C boundary.
#[allow(rustc::bad_opt_access)]
fn owned_supported(prepared: &crate::PreparedCompiler) -> bool {
    prepared.has_input
        && !prepared.help_only
        && prepared.config.query_admission.is_none()
        && prepared.config.make_frontend_admission.is_none()
        && prepared.config.file_loader.is_none()
        && prepared.config.opts.jobs.frontend.is_none()
}

fn owned_selection(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("1"))
}

fn disabled_experiment(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_none_or(|value| value == "0")
}

fn owned_capacity_exhausted(counts: Option<(usize, usize)>) -> bool {
    counts.is_some_and(|(revisions, contexts)| revisions >= 64 || contexts >= 64)
}

fn owned_environment_supported() -> bool {
    // The Cargo experiment does not inherit the isolated typed/decoder/cache
    // experiments. They have different admission and lifetime contracts.
    [
        "JUSTRUST_RUSTC_RESIDENT_TYPED",
        "JUSTRUST_RUSTC_RESIDENT_DECODERS",
        "JUSTRUST_RUSTC_RESIDENT_LEXER",
        "JUSTRUST_RUSTC_RESIDENT_AST",
        "JUSTRUST_RUSTC_RESIDENT_VALUES",
        "JUSTRUST_RUSTC_RESIDENT_FRONTEND",
    ]
    .iter()
    .all(|name| disabled_experiment(std::env::var_os(name).as_deref()))
}

fn serve_requests(
    listener: UnixListener,
    batch: Option<&rustc_interface::resident::ResidentCompilerBatch<'_>>,
    mut pending: Option<PreparedRequest>,
    owned: bool,
) -> Result<(), String> {
    let mut frozen = None;
    let mut revision = 0;
    loop {
        let (mut peer, request, mut prepared) =
            if let Some((peer, request, prepared, callbacks)) = pending.take() {
                (peer, request, Some((prepared, callbacks)))
            } else {
                if !poll_read(listener.as_raw_fd(), IDLE_MS)? {
                    return Ok(());
                }
                let (mut peer, _) = listener.accept().map_err(|e| e.to_string())?;
                peer.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
                peer.set_write_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
                let Ok(request) = receive(&mut peer) else {
                    let _ = peer.write_all(b"F");
                    continue;
                };
                (peer, request, None)
            };
        let Request { id, args, outputs: fds, external, mut external_verify_nanos } = request;
        if frozen.as_ref().is_some_and(|(old, inputs)| old != &args || inputs != &external) {
            peer.write_all(b"F").map_err(|e| e.to_string())?;
            continue;
        }
        let rss = rustc_data_structures::profiling::get_resident_set_size();
        if rss.is_none_or(|rss| rss >= RSS_LIMIT) {
            let _ = peer.write_all(b"F");
            return Ok(());
        }
        if owned {
            if !owned_environment_supported() {
                peer.write_all(b"F").map_err(|e| e.to_string())?;
                continue;
            }
            if prepared.is_none() {
                let mut argv = vec!["jr-rustc-resident-cargo".to_owned()];
                argv.extend(args.iter().cloned());
                let mut callbacks = crate::TimePassesCallbacks::default();
                // Stdio is still /dev/null, so a refused preparation cannot leak
                // speculative diagnostics into Cargo's eventual fallback output.
                let candidate = crate::catch_fatal_errors(|| {
                    crate::prepare_compiler(&argv, &mut callbacks, None)
                });
                match candidate {
                    Ok(Some(candidate)) if owned_supported(&candidate) => {
                        prepared = Some((candidate, callbacks));
                    }
                    _ => {
                        peer.write_all(b"F").map_err(|e| e.to_string())?;
                        continue;
                    }
                }
            }
            if batch.is_none() {
                let (first, callbacks) =
                    prepared.take().expect("owned admission prepared compiler");
                let crate::PreparedCompiler { config, has_input, help_only, registered_lints } =
                    first;
                // Keep the accept loop inside this continuation. The actual
                // compiler arena and SessionGlobals outlive every admitted RPC.
                // Incremental control uses fresh query state with the active
                // source revision view, never external live typed queries.
                return crate::catch_fatal_errors(|| {
                    rustc_interface::resident::with_resident_compiler_batch_options(
                        config,
                        rustc_interface::resident::ResidentBatchOptions {
                            incremental_control: true,
                            ..Default::default()
                        },
                        move |batch, config| {
                            serve_requests(
                                listener,
                                Some(&batch),
                                Some((
                                    peer,
                                    Request {
                                        id,
                                        args,
                                        outputs: fds,
                                        external,
                                        external_verify_nanos,
                                    },
                                    crate::PreparedCompiler {
                                        config,
                                        has_input,
                                        help_only,
                                        registered_lints,
                                    },
                                    callbacks,
                                )),
                                true,
                            )
                        },
                    )
                })
                .map_err(|_| "Cargo owned batch construction failed".to_owned())?;
            }
        }
        // Batch construction and preparation can be lengthy. Re-capture the
        // complete actual invocation immediately before offering commit, not
        // merely the paths selected by the peer's manifest.
        if owned && external.is_some() {
            let verify_start = Instant::now();
            let actual = std::env::current_dir()
                .map_err(|e| e.to_string())
                .and_then(|cwd| jr_resident_inputs::capture(&args, &cwd));
            external_verify_nanos = external_verify_nanos.saturating_add(
                u64::try_from(verify_start.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
            if actual.as_ref().ok() != Some(&external) {
                peer.write_all(b"F").map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        peer.write_all(b"R").map_err(|e| e.to_string())?;
        let mut commit = [0];
        if peer.read_exact(&mut commit).is_err() || commit != *b"C" {
            // An uncommitted initial owned request has nevertheless selected
            // immutable SessionGlobals. Retire rather than admit a different
            // invocation into that universe without a frozen committed argv.
            if batch.is_some() {
                return Ok(());
            }
            continue;
        }
        // All failures from here are committed failures, never fallback permission.
        revision += 1;
        frozen.get_or_insert_with(|| (args.clone(), external.clone()));
        let lease = rustc_data_structures::jobserver::resident::RequestLease::begin_serial()
            .map_err(|e| format!("could not reserve private scheduler: {e:?}"))?;
        let cancel = Cancellation::start(&peer)?;
        let outputs = Outputs::attach(&fds)?;
        drop(fds);
        let mut argv = vec!["jr-rustc-resident-cargo".to_owned()];
        argv.extend(args);
        let start = Instant::now();
        let mut callbacks = crate::TimePassesCallbacks::default();
        let result = lease
            .with_current(|| {
                crate::catch_fatal_errors(|| {
                    if let Some(batch) = batch {
                        let (prepared, initial_callbacks) =
                            prepared.take().expect("owned preflight");
                        callbacks = initial_callbacks;
                        batch.run(prepared.config, |compiler, domain| {
                            crate::run_compiler_pipeline_with_live_identity(
                                compiler,
                                Some(domain),
                                &mut callbacks,
                                prepared.has_input,
                                prepared.help_only,
                                prepared.registered_lints,
                                None,
                            );
                        });
                    } else {
                        crate::compiler_entrypoint(&argv, &mut callbacks);
                    }
                })
            })
            .map_err(|e| format!("scheduler selection failed: {e:?}"))?;
        let status = if result.is_ok() { 0 } else { 1 };
        if let Some(batch) = batch {
            batch.finish_live_revision(status == 0);
            batch.finish_decoder_revision(status == 0);
        }
        if let Some(format) = callbacks.time_passes {
            rustc_data_structures::profiling::print_time_passes_entry(
                "total",
                start.elapsed(),
                None,
                None,
                format,
            );
        }
        outputs.finish()?;
        lease.finish().map_err(|e| format!("request scheduler ownership did not retire: {e:?}"))?;
        drop(cancel);
        let (hits, misses, entries) = rustc_query_impl::resident_stats();
        let graph = rustc_middle::dep_graph::resident_graph_stats();
        let archive = rustc_middle::query::on_disk_cache::OnDiskCache::resident_stats();
        let rss = rustc_data_structures::profiling::get_resident_set_size();
        let verify_start = Instant::now();
        // A concurrent artifact replacement cannot seed a later revision. Keep
        // the executed compiler's result, but retire this entire worker owner.
        let external_changed = external.as_ref().is_some_and(|inputs| inputs.verify().is_err());
        if external.is_some() {
            external_verify_nanos = external_verify_nanos.saturating_add(
                u64::try_from(verify_start.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
        }
        let owned_counts = batch.map(|batch| batch.counts());
        let retire = revision == 64
            || external_changed
            || rss.is_none_or(|rss| rss >= RSS_LIMIT)
            || owned_capacity_exhausted(owned_counts);
        let owned_stats = batch.map(|batch| {
            let (revisions, contexts) = batch.counts();
            let live = batch.live_stats();
            json!({"revisions":revisions,"contexts":contexts,
                "live_queries":false,"external_decoders":false,"incremental_control":true,
                "retained_entries":live.retained_entries,
                "retained_estimated_bytes":live.retained_estimated_bytes})
        });
        let mut reply = json!({"id":id,"status":status,"pid":std::process::id(),
            "external_inputs_verified":external.is_some(),"external_inputs_changed":external_changed,
            "external_verify_nanos":external_verify_nanos,
            "elapsed_seconds":start.elapsed().as_secs_f64(),"resident_hits":hits,
            "resident_misses":misses,"resident_entries":entries,"rss_bytes":rss,"retire":retire,
            "frontend":{"graph_hits":graph.hits,"graph_misses":graph.misses,"graph_bytes":graph.retained_bytes,
                "archive_hits":archive.hits,"archive_misses":archive.misses,"archive_bytes":archive.current_bytes,
                "value_loads":archive.value_loads,"typeck_loads":archive.typeck_loads,"mir_loads":archive.mir_loads}});
        if let Some(owned_stats) = owned_stats {
            reply["owned"] = owned_stats;
        }
        serde_json::to_writer(&mut peer, &reply).map_err(|e| e.to_string())?;
        peer.write_all(b"\n").map_err(|e| e.to_string())?;
        if retire {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_selection_is_strictly_opt_in() {
        use std::ffi::OsStr;
        assert!(!owned_selection(None));
        assert!(owned_selection(Some(OsStr::new("1"))));
        for value in ["", "0", "true", "01", "1 ", "live", "control"] {
            assert!(!owned_selection(Some(OsStr::new(value))), "{value}");
        }
    }

    #[test]
    fn owned_does_not_inherit_isolated_cache_experiments() {
        use std::ffi::OsStr;
        assert!(disabled_experiment(None));
        assert!(disabled_experiment(Some(OsStr::new("0"))));
        for value in ["", "1", "control", "selective-leaf", "false"] {
            assert!(!disabled_experiment(Some(OsStr::new(value))), "{value}");
        }
    }

    #[test]
    fn owned_retires_before_either_native_capacity_is_exhausted() {
        assert!(!owned_capacity_exhausted(None));
        assert!(!owned_capacity_exhausted(Some((0, 0))));
        assert!(!owned_capacity_exhausted(Some((63, 63))));
        for counts in [(64, 0), (0, 64), (64, 64), (65, 1), (1, 65)] {
            assert!(owned_capacity_exhausted(Some(counts)), "{counts:?}");
        }
    }

    #[test]
    fn owned_preflight_rejects_help_and_accepts_real_serial_compilation() {
        let mut callbacks = crate::TimePassesCallbacks::default();
        let argv = ["jr-rustc", "a.rs", "--jobs-backend=1", "--jobs-linker=1"].map(str::to_owned);
        let mut prepared = crate::prepare_compiler(&argv, &mut callbacks, None).unwrap();
        assert!(owned_supported(&prepared));
        prepared.has_input = false;
        assert!(!owned_supported(&prepared));
        prepared.has_input = true;
        prepared.help_only = true;
        assert!(!owned_supported(&prepared));
        prepared.help_only = false;
        // A custom source owner is not admitted merely because argv was serial.
        prepared.config.file_loader = Some(Box::new(rustc_span::source_map::RealFileLoader));
        assert!(!owned_supported(&prepared));
    }

    #[test]
    fn external_manifest_is_opt_in_exact_and_content_verified() {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "jr-resident-input-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let dependency = root.join("dep.rlib");
        std::fs::write(&dependency, b"first").unwrap();
        let mut args = args();
        args.push(format!("--extern=dep={}", dependency.display()));
        let manifest = jr_resident_inputs::capture(&args, &root).unwrap().unwrap();
        let request = serde_json::json!({"id":1,"args":args,"external":manifest});
        let bytes = serde_json::to_vec(&request).unwrap();
        assert!(decode_request(&bytes, true, &root).is_ok());
        assert!(decode_request(&bytes, false, &root).is_err());
        let missing = serde_json::to_vec(&serde_json::json!({"id":1,"args":args})).unwrap();
        assert!(decode_request(&missing, true, &root).is_err());
        let mut unknown = request.clone();
        unknown["unexpected"] = true.into();
        assert!(decode_request(&serde_json::to_vec(&unknown).unwrap(), true, &root).is_err());
        std::fs::write(&dependency, b"other").unwrap();
        assert!(decode_request(&bytes, true, &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn args() -> Vec<String> {
        ["a.rs", "--jobs-backend=1", "--jobs-linker=1"].map(str::to_owned).to_vec()
    }
    #[test]
    fn serial_budget_is_explicit_and_unambiguous() {
        assert!(eligible(&args(), false).is_ok());
        for extra in [
            "--jobs-backend=2",
            "--jobs-linker=1",
            "--extern=foo=x.rlib",
            "--target=host",
            "@flags",
            "--crate-type=proc-macro",
            "-Zthreads=4",
            "--jobs=4",
            "--jobs-frontend=4",
            "-j4",
        ] {
            let mut a = args();
            a.push(extra.into());
            assert!(eligible(&a, false).is_err(), "{extra}");
        }
        assert!(eligible(&args()[..2], false).is_err());
    }
    #[test]
    fn watcher_normal_shutdown_is_joined() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "resident_cargo::tests::cancellation_child", "--nocapture"])
            .env("JR_RESIDENT_CANCEL_CHILD", "normal")
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn cancellation_child() {
        let Some(mode) = std::env::var_os("JR_RESIDENT_CANCEL_CHILD") else {
            return;
        };
        assert_eq!(unsafe { libc::setpgid(0, 0) }, 0);
        if mode == "outputs" {
            let (out_r, out_w) = pipe();
            let (err_r, err_w) = pipe();
            let rights = vec![out_w, err_w];
            let outputs = Outputs::attach(&rights).unwrap();
            drop(rights);
            io::stdout().write_all(b"rust").unwrap();
            io::stderr().write_all(b"diagnostic").unwrap();
            unsafe {
                libc::printf(c"native".as_ptr());
            }
            outputs.finish().unwrap();
            let mut out = String::new();
            let mut err = String::new();
            File::from(out_r).read_to_string(&mut out).unwrap();
            File::from(err_r).read_to_string(&mut err).unwrap();
            assert_eq!(out, "rustnative");
            assert_eq!(err, "diagnostic");
            return;
        }
        let (peer, client) = UnixStream::pair().unwrap();
        let cancel = Cancellation::start(&peer).unwrap();
        if mode == "normal" {
            drop(cancel);
            return;
        }
        // A linker-like descendant inherits both the dedicated group and stdout.
        // The parent checks EOF, so killing only this worker cannot pass.
        let _descendant = std::process::Command::new("sleep").arg("10").spawn().unwrap();
        drop(client);
        std::thread::sleep(Duration::from_secs(10));
        panic!("watcher failed to kill disconnected worker");
    }

    #[test]
    fn outputs_flush_rust_and_native_buffers_and_close_before_completion() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "resident_cargo::tests::cancellation_child", "--nocapture"])
            .env("JR_RESIDENT_CANCEL_CHILD", "outputs")
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn disconnect_kills_worker_immediately() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "resident_cargo::tests::cancellation_child", "--nocapture"])
            .env("JR_RESIDENT_CANCEL_CHILD", "1")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(status.signal(), Some(libc::SIGKILL));
                let mut stdout = child.stdout.take().unwrap();
                let fd = stdout.as_raw_fd();
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
                loop {
                    match stdout.read(&mut [0u8; 4096]) {
                        Ok(0) => break,
                        Ok(_) => continue,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            if start.elapsed() > Duration::from_secs(3) {
                                unsafe {
                                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                                }
                                panic!("worker descendant survived group cancellation");
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => panic!("reading descendant output: {e}"),
                    }
                }
                break;
            }
            if start.elapsed() > Duration::from_secs(3) {
                child.kill().unwrap();
                let _ = child.wait();
                panic!("disconnect did not kill worker promptly");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    fn send_request(stream: &mut UnixStream, fds: &[i32]) {
        let line = serde_json::to_vec(&json!({"id":7,"args":args()})).unwrap();
        let mut control = [0usize; 32];
        let mut byte = [line[0]];
        let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen =
            unsafe { libc::CMSG_SPACE(std::mem::size_of_val(fds) as u32) } as usize;
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&msg);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(fds) as u32) as usize;
            std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(header).cast(), fds.len());
            assert_eq!(libc::sendmsg(stream.as_raw_fd(), &msg, libc::MSG_NOSIGNAL), 1);
        }
        stream.write_all(&line[1..]).unwrap();
        stream.write_all(b"\n").unwrap();
    }

    #[test]
    fn rights_are_exactly_two_cloexec_pipe_writers() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let (_out_r, out_w) = pipe();
        let (_err_r, err_w) = pipe();
        send_request(&mut client, &[out_w.as_raw_fd(), err_w.as_raw_fd()]);
        let Request { id, args: argv, outputs: rights, external, .. } =
            receive(&mut server).unwrap();
        assert!(external.is_none());
        assert_eq!(id, 7);
        assert_eq!(argv, args());
        for fd in rights {
            assert_ne!(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
        }
        for fds in [
            vec![out_w.as_raw_fd()],
            vec![out_w.as_raw_fd(), err_w.as_raw_fd(), out_w.as_raw_fd()],
            vec![_out_r.as_raw_fd(), err_w.as_raw_fd()],
        ] {
            let (mut client, mut server) = UnixStream::pair().unwrap();
            send_request(&mut client, &fds);
            assert!(receive(&mut server).is_err());
        }
    }

    #[test]
    fn missing_rights_are_refused() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        a.write_all(b"{\"id\":1,\"args\":[] }\n").unwrap();
        assert!(receive(&mut b).is_err());
    }
}
