//! Opt-in, single-owner resident compiler experiment.
//!
//! The process environment, working directory and compiler arguments are frozen.
//! Each request creates a fresh session. Only explicitly safe, dependency-validated
//! query values survive in rustc_query_impl, never arena references or diagnostics.

use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::os::fd::FromRawFd;
use std::process::ExitCode;
use std::time::Instant;

use serde_json::{Value, json};

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

fn request(line: &[u8]) -> Result<(u64, Vec<String>), String> {
    let value: Value = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    let object = value.as_object().ok_or("request must be an object")?;
    if object.len() != 2 || !object.contains_key("id") || !object.contains_key("args") {
        return Err("request requires exactly id and args".into());
    }
    let id = value["id"].as_u64().ok_or("id must be an unsigned integer")?;
    let args = value["args"].as_array().ok_or("args must be an array")?;
    if args.is_empty() || args.len() > 4096 {
        return Err("args must contain 1..4096 arguments".into());
    }
    let args: Vec<String> = args
        .iter()
        .map(|arg| {
            let arg = arg.as_str().ok_or("arguments must be strings")?;
            // Stdin is the protocol transport. Response files could change global
            // backend options without changing the frozen argument vector.
            if arg == "-" || arg.starts_with('@') || arg.contains('\0') {
                return Err("stdin, response files and NUL arguments are unsupported");
            }
            // LLVM initialization is process-global. This first protocol supports
            // only the host target and owned LLVM backend, not mutable custom
            // target specifications or dynamically loaded codegen backends.
            if arg == "--target" || arg.starts_with("--target=") {
                return Err("resident experiment currently supports only the host target");
            }
            let backend = arg.strip_prefix("-Z").unwrap_or(arg);
            if backend.starts_with("codegen-backend") && backend != "codegen-backend=llvm" {
                return Err("resident experiment currently supports only the owned LLVM backend");
            }
            Ok(arg.to_owned())
        })
        .collect::<Result<_, _>>()?;
    Ok((id, args))
}

/// # Safety
/// Called only by native process main. The launcher transfers exclusive ownership
/// of a writable response pipe through JUSTRUST_RUSTC_RESIDENT_FD.
pub(super) unsafe fn run() -> ExitCode {
    match unsafe { serve() } {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jr-rustc resident: {error}");
            ExitCode::FAILURE
        }
    }
}

unsafe fn serve() -> Result<(), String> {
    // Existing recorders assume one compiler session per process. Do not emit
    // misleading phase totals or acknowledge an early reply in this experiment.
    for name in ["JUSTRUST_RUSTC_PHASES_FILE", "JUSTRUST_RUSTC_EARLY_REPLY"] {
        if std::env::var_os(name).is_some() {
            return Err(format!("{name} is unsupported in resident experiment mode"));
        }
    }
    let fd: i32 = std::env::var("JUSTRUST_RUSTC_RESIDENT_FD")
        .map_err(|_| "JUSTRUST_RUSTC_RESIDENT_FD is required")?
        .parse()
        .map_err(|_| "invalid response descriptor")?;
    if fd < 3 {
        return Err("response descriptor must be distinct from stdin/stdout/stderr".into());
    }
    // SAFETY: fcntl and fstat only inspect the supplied descriptor. No ownership
    // is taken until it is confirmed to be a writable pipe.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if flags < 0 || unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err("invalid response descriptor".into());
    }
    let stat = unsafe { stat.assume_init() };
    if flags & libc::O_ACCMODE == libc::O_RDONLY || stat.st_mode & libc::S_IFMT != libc::S_IFIFO {
        return Err("response descriptor must be a writable pipe".into());
    }
    // Never leak the protocol response pipe into linker/proc-macro children.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err("could not protect response descriptor from child inheritance".into());
    }
    // SAFETY: the native launch protocol transfers this validated pipe to us.
    let mut response = unsafe { File::from_raw_fd(fd) };
    rustc_session::resident::enable_process();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut frozen_args: Option<Vec<String>> = None;
    let mut last_id = None;
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take(MAX_REQUEST_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        if count as u64 > MAX_REQUEST_BYTES || !line.ends_with(b"\n") {
            return Err("oversized or unterminated resident request".into());
        }
        let (id, args) = request(&line)?;
        if last_id.is_some_and(|last| id <= last) {
            return Err("request ids must increase strictly".into());
        }
        last_id = Some(id);
        match &frozen_args {
            Some(frozen) if frozen != &args => {
                return Err("compiler arguments cannot change within a resident worker".into());
            }
            None => frozen_args = Some(args.clone()),
            _ => {}
        }
        let mut argv = vec!["jr-rustc-resident".to_owned()];
        argv.extend(args);
        let start = Instant::now();
        let mut callbacks = crate::TimePassesCallbacks::default();
        let status = if crate::catch_fatal_errors(|| {
            crate::compiler_entrypoint(&argv, &mut callbacks);
        })
        .is_ok()
        {
            0
        } else {
            1
        };
        if let Some(format) = callbacks.time_passes {
            rustc_data_structures::profiling::print_time_passes_entry(
                "total",
                start.elapsed(),
                None,
                None,
                format,
            );
        }
        // A completion frame must follow every byte of compiler output.
        io::stdout().flush().map_err(|e| e.to_string())?;
        io::stderr().flush().map_err(|e| e.to_string())?;
        let (hits, misses, entries) = rustc_query_impl::resident_stats();
        let reply = json!({"id": id, "status": status, "pid": std::process::id(),
            "elapsed_seconds": start.elapsed().as_secs_f64(),
            "resident_hits": hits, "resident_misses": misses, "resident_entries": entries});
        serde_json::to_writer(&mut response, &reply).map_err(|e| e.to_string())?;
        response.write_all(b"\n").map_err(|e| e.to_string())?;
        response.flush().map_err(|e| e.to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::request;

    #[test]
    fn accepts_fixed_compiler_request() {
        let (id, args) = request(br#"{"id":1,"args":["fixture.rs","--edition=2024"]}"#).unwrap();
        assert_eq!(id, 1);
        assert_eq!(args, ["fixture.rs", "--edition=2024"]);
    }

    #[test]
    fn rejects_ambiguous_or_mutable_request_inputs() {
        for input in [
            r#"{"id":1,"args":["-"]}"#,
            r#"{"id":1,"args":["@flags"]}"#,
            r#"{"id":1,"args":[]}"#,
            r#"{"id":-1,"args":["a.rs"]}"#,
            r#"{"id":1,"args":[1]}"#,
            r#"{"id":1,"args":["a.rs"],"cwd":"/"}"#,
            r#"{"id":1,"args":["a.rs","--target=custom.json"]}"#,
            r#"{"id":1,"args":["a.rs","-Zcodegen-backend=cranelift"]}"#,
        ] {
            assert!(request(input.as_bytes()).is_err(), "{input}");
        }
    }
}
