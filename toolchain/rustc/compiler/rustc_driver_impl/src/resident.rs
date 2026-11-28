//! Opt-in, single-owner resident compiler experiment.
//!
//! The process environment, working directory and compiler arguments are frozen.
//! Legacy requests create fresh owners. The separate owned mode scopes native
//! arenas and compiler sessions across requests, but does not reuse typed queries.

use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::os::fd::FromRawFd;
use std::process::ExitCode;
use std::time::Instant;

use serde_json::{Value, json};

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

// Boundary admission budget, not a hard peak allocation cap: a single compilation
// may exceed it before finalization. Retiring returns from the whole owner scope.
const OWNED_RSS_BUDGET_BYTES: usize = 2 * 1024 * 1024 * 1024;
const OWNED_MAX_REVISIONS: usize = 64;

fn owned_retirement(rss: Option<usize>, revisions: usize, contexts: usize) -> Option<&'static str> {
    match rss {
        None => Some("rss_unavailable"),
        Some(bytes) if bytes >= OWNED_RSS_BUDGET_BYTES => Some("rss_budget"),
        Some(_) if revisions >= OWNED_MAX_REVISIONS || contexts >= OWNED_MAX_REVISIONS => {
            Some("context_capacity")
        }
        Some(_) => None,
    }
}

// None means preflight refused the request, not a compiler result. Retirement
// after execution must never replace the actual success/failure status.
fn owned_completion(
    status: Option<i32>,
    rss: Option<usize>,
    revisions: usize,
    contexts: usize,
) -> Value {
    json!({
        "status": status.unwrap_or(1),
        "executed": status.is_some(),
        "owned": {
            "revisions": revisions, "contexts": contexts,
            "retirement_reason": owned_retirement(rss, revisions, contexts),
            "rss_bytes": rss,
        },
    })
}

pub(super) fn request(line: &[u8]) -> Result<(u64, Vec<String>), String> {
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

/// Idle controls never alter compiler arguments or the frozen environment.
/// They are handled only between completed compiler sessions.
fn idle_request(line: &[u8]) -> Result<Option<(u64, String)>, String> {
    let value: Value = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    let object = value.as_object().ok_or("request must be an object")?;
    if !object.contains_key("tier") {
        return Ok(None);
    }
    if object.len() != 2 || !object.contains_key("id") {
        return Err("idle request requires exactly id and tier".into());
    }
    let id = value["id"].as_u64().ok_or("id must be an unsigned integer")?;
    let tier = value["tier"].as_str().ok_or("tier must be a string")?;
    if !matches!(tier, "hot" | "compressed" | "cold") {
        return Err("unknown resident memory tier".into());
    }
    Ok(Some((id, tier.to_owned())))
}

/// # Safety
/// Called only by native process main. The launcher transfers exclusive ownership
/// of a writable response pipe through JUSTRUST_RUSTC_RESIDENT_FD.
pub(super) unsafe fn run(owned: bool) -> ExitCode {
    match unsafe { serve(owned) } {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jr-rustc resident: {error}");
            ExitCode::FAILURE
        }
    }
}

unsafe fn serve(owned: bool) -> Result<(), String> {
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
    if owned {
        return serve_owned(response);
    }
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
        if let Some((id, tier)) = idle_request(&line)? {
            if last_id.is_some_and(|last| id <= last) {
                return Err("request ids must increase strictly".into());
            }
            last_id = Some(id);
            use rustc_middle::query::on_disk_cache::{OnDiskCache, ResidentIdleTier};
            let start = Instant::now();
            let archive = match tier.as_str() {
                "compressed" => OnDiskCache::resident_idle(ResidentIdleTier::Compressed),
                "cold" => OnDiskCache::resident_idle(ResidentIdleTier::Cold),
                _ => OnDiskCache::resident_tier_stats(),
            };
            let graph = if tier == "hot" {
                rustc_middle::dep_graph::resident_graph_stats()
            } else {
                rustc_middle::dep_graph::resident_graph_idle()
            };
            let reply = json!({"id":id,"status":0,"pid":std::process::id(),"tier":tier,
                "elapsed_seconds":start.elapsed().as_secs_f64(),
                "memory":{"graph_bytes":graph.retained_bytes,
                    "archive_bytes":archive.hot_bytes + archive.compressed_bytes,
                    "hot_entries":archive.hot_entries,"hot_bytes":archive.hot_bytes,
                    "compressed_entries":archive.compressed_entries,
                    "compressed_bytes":archive.compressed_bytes,
                    "pinned_entries":archive.active_entries}});
            serde_json::to_writer(&mut response, &reply).map_err(|e| e.to_string())?;
            response.write_all(b"\n").map_err(|e| e.to_string())?;
            response.flush().map_err(|e| e.to_string())?;
            continue;
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
        let graph = rustc_middle::dep_graph::resident_graph_stats();
        let archive = rustc_middle::query::on_disk_cache::OnDiskCache::resident_stats();
        let frontend = json!({
            "graph_hits": graph.hits,
            "graph_misses": graph.misses,
            "graph_bytes": graph.retained_bytes,
            "archive_hits": archive.hits,
            "archive_misses": archive.misses,
            "archive_bytes": archive.current_bytes,
            "value_loads": archive.value_loads,
            "typeck_loads": archive.typeck_loads,
            "mir_loads": archive.mir_loads,
        });
        let reply = json!({"id": id, "status": status, "pid": std::process::id(),
            "elapsed_seconds": start.elapsed().as_secs_f64(),
            "resident_hits": hits, "resident_misses": misses, "resident_entries": entries,
            "frontend": frontend});
        serde_json::to_writer(&mut response, &reply).map_err(|e| e.to_string())?;
        response.write_all(b"\n").map_err(|e| e.to_string())?;
        response.flush().map_err(|e| e.to_string())?;
    }
}

struct OwnedRequests {
    input: io::Stdin,
    args: Option<Vec<String>>,
    last_id: Option<u64>,
}

impl OwnedRequests {
    fn next(&mut self) -> Result<Option<(u64, Vec<String>)>, String> {
        let mut line = Vec::new();
        let count = self
            .input
            .lock()
            .take(MAX_REQUEST_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(None);
        }
        if count as u64 > MAX_REQUEST_BYTES || !line.ends_with(b"\n") {
            return Err("oversized or unterminated resident request".into());
        }
        let (id, args) = request(&line)?;
        if self.last_id.is_some_and(|last| id <= last) {
            return Err("request ids must increase strictly".into());
        }
        if self.args.as_ref().is_some_and(|frozen| frozen != &args) {
            return Err("compiler arguments cannot change within a resident worker".into());
        }
        self.last_id = Some(id);
        self.args.get_or_insert_with(|| args.clone());
        Ok(Some((id, args)))
    }
}

fn typed_selection(value: Option<&std::ffi::OsStr>) -> Result<(bool, bool, bool), String> {
    match value {
        None => Ok((false, false, false)),
        Some(value) if value == "0" => Ok((false, false, false)),
        Some(value) if value == "control" => Ok((false, true, false)),
        Some(value) if value == "1" => Ok((true, false, false)),
        Some(value) if value == "leaf" => Ok((true, false, true)),
        Some(_) => Err("invalid native typed experiment selection".into()),
    }
}

fn ast_selection(value: Option<&std::ffi::OsStr>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some(value) if value == "0" => Ok(false),
        Some(value) if value == "1" => Ok(true),
        Some(_) => Err("invalid native AST experiment selection".into()),
    }
}

fn decoder_selection(value: Option<&std::ffi::OsStr>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some(value) if value == "0" => Ok(false),
        Some(value) if value == "1" => Ok(true),
        Some(_) => Err("invalid native external decoder experiment selection".into()),
    }
}

/// Actual common native owners. Persistence and typed queries require separate
/// explicit development selections, never a production compiler default change.
fn serve_owned(mut response: File) -> Result<(), String> {
    let mode = std::env::var_os("JUSTRUST_RUSTC_RESIDENT_TYPED");
    let (live_queries, incremental_control, selective_leaf) = typed_selection(mode.as_deref())?;
    let lexer_cache = match std::env::var_os("JUSTRUST_RUSTC_RESIDENT_LEXER") {
        None => false,
        Some(value) if value == "0" => false,
        Some(value) if value == "1" => true,
        Some(_) => return Err("invalid native lexer experiment selection".into()),
    };
    let ast_cache = ast_selection(std::env::var_os("JUSTRUST_RUSTC_RESIDENT_AST").as_deref())?;
    let external_decoders =
        decoder_selection(std::env::var_os("JUSTRUST_RUSTC_RESIDENT_DECODERS").as_deref())?;
    for name in ["JUSTRUST_RUSTC_RESIDENT_VALUES", "JUSTRUST_RUSTC_RESIDENT_FRONTEND"] {
        if std::env::var_os(name).is_some_and(|value| value == "1") {
            return Err(format!("{name} cannot be combined with the native owned batch"));
        }
    }
    let mut requests = OwnedRequests { input: io::stdin(), args: None, last_id: None };
    let Some((first_id, first_args)) = requests.next()? else {
        return Ok(());
    };
    let mut first_argv = vec!["jr-rustc-resident-owned".to_owned()];
    first_argv.extend(first_args);
    let invocation = if live_queries || incremental_control {
        Some(frozen_semantic_invocation(&first_argv)?)
    } else {
        None
    };
    let mut first_callbacks = crate::TimePassesCallbacks::default();
    let first = crate::catch_fatal_errors(|| {
        crate::prepare_compiler(&first_argv, &mut first_callbacks, None)
    })
    .map_err(|_| "could not prepare native owned batch")?
    .ok_or("owned batch requires a compilation, not a help-only request")?;
    let crate::PreparedCompiler { config, has_input, help_only, registered_lints } = first;
    if !has_input || help_only {
        return Err("owned batch requires a source input".into());
    }
    crate::catch_fatal_errors(|| {
        rustc_interface::resident::with_resident_compiler_batch_options_and_decoders(
            config,
            rustc_interface::resident::ResidentBatchOptions {
                live_queries,
                incremental_control,
                selective_leaf,
                lexer_cache,
                ast_cache,
            },
            external_decoders,
            move |batch, config| {
                let mut first = Some((config, first_callbacks, registered_lints));
                let mut id = first_id;
                let mut argv = first_argv;
                loop {
                    if let Some(expected) = &invocation {
                        if frozen_semantic_invocation(&argv)? != *expected {
                            return Err("native typed worker semantic invocation changed".into());
                        }
                    }
                    let start = Instant::now();
                    let (revisions, contexts) = batch.counts();
                    let rss = rustc_data_structures::profiling::get_resident_set_size();
                    if owned_retirement(rss, revisions, contexts).is_some() {
                        let mut reply = owned_completion(None, rss, revisions, contexts);
                        reply["id"] = json!(id);
                        reply["pid"] = json!(std::process::id());
                        reply["elapsed_seconds"] = json!(start.elapsed().as_secs_f64());
                        io::stdout().flush().map_err(|e| e.to_string())?;
                        io::stderr().flush().map_err(|e| e.to_string())?;
                        serde_json::to_writer(&mut response, &reply).map_err(|e| e.to_string())?;
                        response.write_all(b"\n").map_err(|e| e.to_string())?;
                        response.flush().map_err(|e| e.to_string())?;
                        return Ok(());
                    }
                    let mut callbacks = crate::TimePassesCallbacks::default();
                    let status = if crate::catch_fatal_errors(|| {
                        let prepared = if let Some((config, initial_callbacks, registered_lints)) =
                            first.take()
                        {
                            callbacks = initial_callbacks;
                            crate::PreparedCompiler {
                                config,
                                has_input: true,
                                help_only: false,
                                registered_lints,
                            }
                        } else {
                            crate::prepare_compiler(&argv, &mut callbacks, None)
                                .expect("frozen compilation arguments cannot become help-only")
                        };
                        batch.run(prepared.config, |compiler, domain| {
                            crate::run_compiler_pipeline_with_live_identity(
                                compiler,
                                Some(domain),
                                &mut callbacks,
                                prepared.has_input,
                                prepared.help_only,
                                prepared.registered_lints,
                                invocation.as_deref(),
                            );
                        });
                    })
                    .is_ok()
                    {
                        0
                    } else {
                        1
                    };
                    // The batch has now finished native execution, linker work and
                    // diagnostics. No failed or unfinished revision can publish.
                    batch.finish_live_revision(status == 0);
                    batch.finish_decoder_revision(status == 0);
                    if let Some(format) = callbacks.time_passes {
                        rustc_data_structures::profiling::print_time_passes_entry(
                            "total",
                            start.elapsed(),
                            None,
                            None,
                            format,
                        );
                    }
                    io::stdout().flush().map_err(|e| e.to_string())?;
                    io::stderr().flush().map_err(|e| e.to_string())?;
                    let (revisions, contexts) = batch.counts();
                    // batch.run has returned through normal revision finalization.
                    let rss = rustc_data_structures::profiling::get_resident_set_size();
                    let mut reply = owned_completion(Some(status), rss, revisions, contexts);
                    let decoder = batch.decoder_stats();
                    reply["decoder"] = json!({
                        "hits": decoder.hits,
                        "misses": decoder.misses,
                        "rejected": decoder.rejected,
                        "publications": decoder.publications,
                        "retained_entries": decoder.retained_entries,
                        "retired": decoder.retired,
                        "retirement_requests": decoder.retirement_requests,
                        "selection_nanos": decoder.selection_nanos,
                        "publication_nanos": decoder.publication_nanos,
                    });
                    if decoder.retired && reply["owned"]["retirement_reason"].is_null() {
                        reply["owned"]["retirement_reason"] = json!("external_decoder");
                    }
                    let live = batch.live_stats();
                    reply["live"] = json!({
                        "identity_begin_rejections": live.identity_begin_rejections,
                        "identity_complete_rejections": live.identity_complete_rejections,
                        "published_revisions": live.published_revisions,
                        "aborted_revisions": live.aborted_revisions,
                        "typeck_hits": live.typeck_hits,
                        "typeck_misses": live.typeck_misses,
                        "mir_hits": live.mir_hits,
                    "mir_leaf_hits": live.mir_leaf_hits,
                    "mir_leaf_candidate_misses": live.mir_leaf_candidate_misses,
                    "mir_leaf_last_rejection": live.mir_leaf_last_rejection.map(|reason| format!("{reason:?}")),
                        "mir_misses": live.mir_misses,
                        "retained_entries": live.retained_entries,
                        "retained_estimated_bytes": live.retained_estimated_bytes,
                    });
                    let ast = batch.ast_stats();
                    reply["ast"] = json!({
                        "hits": ast.hits,
                        "misses": ast.misses,
                        "rejected": ast.rejected,
                        "retained_entries": ast.retained_entries,
                        "retained_estimated_bytes": ast.retained_estimated_bytes,
                    });
                    let lexer = batch.lexer_stats();
                    reply["lexer"] = json!({
                        "hits": lexer.hits,
                        "misses": lexer.misses,
                        "rejected": lexer.rejected,
                        "retained_entries": lexer.retained_entries,
                        "retained_estimated_bytes": lexer.retained_estimated_bytes,
                    });
                    reply["id"] = json!(id);
                    reply["pid"] = json!(std::process::id());
                    reply["elapsed_seconds"] = json!(start.elapsed().as_secs_f64());
                    serde_json::to_writer(&mut response, &reply).map_err(|e| e.to_string())?;
                    response.write_all(b"\n").map_err(|e| e.to_string())?;
                    response.flush().map_err(|e| e.to_string())?;
                    if decoder.retired || owned_retirement(rss, revisions, contexts).is_some() {
                        // Do not read another request. Returning from this callback
                        // drops the actual batch owners before process termination.
                        return Ok(());
                    }
                    let Some((next_id, args)) = requests.next()? else {
                        return Ok(());
                    };
                    id = next_id;
                    argv = vec!["jr-rustc-resident-owned".to_owned()];
                    argv.extend(args);
                }
            },
        )
    })
    .map_err(|_| "native owned batch construction failed".to_owned())?
}

/// Exact process-local inputs, never a hash or Debug representation. The cache
/// cannot outlive this executable/LLVM owner, and requests already freeze argv.
/// Recheck cwd and the complete OS environment before every controlled request.
/// These bytes remain private RAM state, never protocol diagnostics or records.
pub(super) fn frozen_semantic_invocation(args: &[String]) -> Result<Vec<u8>, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let mut environment: Vec<_> = std::env::vars_os()
        .map(|(key, value)| (key.as_encoded_bytes().to_vec(), value.as_encoded_bytes().to_vec()))
        .collect();
    environment.sort();
    serde_json::to_vec(&(args, cwd.as_os_str().as_encoded_bytes(), environment))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        OWNED_RSS_BUDGET_BYTES, ast_selection, decoder_selection, idle_request, owned_completion,
        owned_retirement, request, typed_selection,
    };

    #[test]
    fn idle_controls_are_exact_and_distinct_from_compilation() {
        for tier in ["hot", "compressed", "cold"] {
            let frame = format!(r#"{{"id":7,"tier":"{tier}"}}"#);
            assert_eq!(idle_request(frame.as_bytes()).unwrap(), Some((7, tier.into())));
            assert!(request(frame.as_bytes()).is_err());
        }
        assert!(idle_request(br#"{"id":8,"args":["a.rs"]}"#).unwrap().is_none());
        for frame in [
            r#"{"id":7,"tier":"unknown"}"#,
            r#"{"id":-1,"tier":"cold"}"#,
            r#"{"id":7,"tier":"cold","args":["a.rs"]}"#,
            r#"{"tier":"cold"}"#,
            r#"{"id":7,"tier":false}"#,
        ] {
            assert!(idle_request(frame.as_bytes()).is_err());
        }
    }

    #[test]
    fn decoder_selection_is_explicit_and_defaults_off() {
        use std::ffi::OsStr;
        assert!(!decoder_selection(None).unwrap());
        assert!(!decoder_selection(Some(OsStr::new("0"))).unwrap());
        assert!(decoder_selection(Some(OsStr::new("1"))).unwrap());
        for invalid in ["", "true", "control", "leaf", "2", " 1"] {
            assert!(decoder_selection(Some(OsStr::new(invalid))).is_err());
        }
    }

    #[test]
    fn ast_selection_is_explicit_and_defaults_off() {
        use std::ffi::OsStr;
        assert!(!ast_selection(None).unwrap());
        assert!(!ast_selection(Some(OsStr::new("0"))).unwrap());
        assert!(ast_selection(Some(OsStr::new("1"))).unwrap());
        for invalid in ["", "true", "control", "leaf", "2"] {
            assert!(ast_selection(Some(OsStr::new(invalid))).is_err());
        }
    }

    #[test]
    fn typed_selection_is_explicit_and_separates_leaf_authorization() {
        use std::ffi::OsStr;
        assert_eq!(typed_selection(None).unwrap(), (false, false, false));
        for (mode, expected) in [
            ("0", (false, false, false)),
            ("control", (false, true, false)),
            ("1", (true, false, false)),
            ("leaf", (true, false, true)),
        ] {
            assert_eq!(typed_selection(Some(OsStr::new(mode))).unwrap(), expected);
        }
        for invalid in ["", "true", "LEAF", " leaf"] {
            assert!(typed_selection(Some(OsStr::new(invalid))).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn typed_selection_rejects_non_unicode_instead_of_becoming_default() {
        use std::os::unix::ffi::OsStrExt;
        assert!(typed_selection(Some(std::ffi::OsStr::from_bytes(b"\xff"))).is_err());
    }

    #[test]
    fn owned_boundary_rss_admission() {
        assert_eq!(owned_retirement(Some(OWNED_RSS_BUDGET_BYTES - 1), 0, 0), None);
        for rss in [OWNED_RSS_BUDGET_BYTES, OWNED_RSS_BUDGET_BYTES + 1] {
            assert_eq!(owned_retirement(Some(rss), 0, 0), Some("rss_budget"));
        }
        assert_eq!(owned_retirement(None, 0, 0), Some("rss_unavailable"));
    }

    #[test]
    fn owned_boundary_count_exhaustion() {
        assert_eq!(owned_retirement(Some(1), 63, 63), None);
        for (revisions, contexts) in [(64, 0), (0, 64), (65, 65)] {
            assert_eq!(owned_retirement(Some(1), revisions, contexts), Some("context_capacity"));
        }
    }

    #[test]
    fn owned_retirement_preserves_executed_result() {
        for status in [0, 1] {
            let reply = owned_completion(Some(status), Some(OWNED_RSS_BUDGET_BYTES), 64, 64);
            assert_eq!(reply["status"], status);
            assert_eq!(reply["executed"], true);
            assert_eq!(reply["owned"]["retirement_reason"], "rss_budget");
            assert_eq!(reply["owned"]["rss_bytes"], OWNED_RSS_BUDGET_BYTES);
        }
        let refused = owned_completion(None, None, 0, 0);
        assert_eq!(refused["status"], 1);
        assert_eq!(refused["executed"], false);
        assert_eq!(refused["owned"]["retirement_reason"], "rss_unavailable");
        assert!(refused["owned"]["rss_bytes"].is_null());
    }

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
