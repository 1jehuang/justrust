use justrust_engine::compiler;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(feature = "allocation-profile")]
mod allocation_profile;
mod compiler_profile;
#[cfg(feature = "allocation-profile")]
#[global_allocator]
static ALLOCATION_PROFILE: allocation_profile::ProfilingAllocator<std::alloc::System> =
    allocation_profile::ProfilingAllocator::new(std::alloc::System);

const HELP: &str = "jr-rust SOURCE [-o NEW_EXECUTABLE] [--workers N] [--check] [--debug-assertions yes|no] [--cfg NAME_OR_KEY_VALUE] [--trace NEW_TRACE] [--profile NEW_JSON]\n\nClean-room Rust subset compiler. Direct Linux x86-64 ELF output, no rustc delegation.\nSee COMPILER.md for supported syntax and limitations. Existing files are never overwritten.\nAccounting is always enabled. --check performs semantic checks without code generation.\n--cfg may be repeated for user names or key=\"value\" predicates; native target cfg is fixed.\n--debug-assertions yes|no sets native debug assertions (default: yes); may be supplied once.\n--source-lock PATH --source-item NAME adds a verified source declaration probe, not a library crate.\n--profile writes bounded task timings and exact first budget failure without CFG dumps.\nAllocator counters require a binary built with --features allocation-profile.\n";

fn allocator_json() -> String {
    #[cfg(feature = "allocation-profile")]
    {
        ALLOCATION_PROFILE.snapshot().to_json()
    }
    #[cfg(not(feature = "allocation-profile"))]
    {
        "null".into()
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jr-rust: {error}");
        std::process::exit(1);
    }
}

fn new_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn set_debug_assertions_option(
    cfg: &mut compiler::source::CfgSet,
    seen: &mut bool,
    value: Option<&std::ffi::OsStr>,
) -> Result<(), &'static str> {
    if *seen {
        return Err("--debug-assertions may only be supplied once");
    }
    let value = value.ok_or("missing --debug-assertions value (expected yes or no)")?;
    let enabled = match value.to_str() {
        Some("yes") => true,
        Some("no") => false,
        _ => return Err("invalid --debug-assertions value (expected yes or no)"),
    };
    cfg.set_debug_assertions(enabled);
    *seen = true;
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut source = None;
    let mut output: Option<PathBuf> = None;
    let mut trace: Option<PathBuf> = None;
    let mut profile: Option<PathBuf> = None;
    let mut workers = 1usize;
    let mut check = false;
    let mut source_options = compiler::source::SourceOptions::default();
    let mut debug_assertions_seen = false;
    let mut source_lock: Option<PathBuf> = None;
    let mut source_item: Option<String> = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                print!("{HELP}");
                return Ok(());
            }
            Some("--check") => check = true,
            Some("--source-lock") => {
                let path = args.next().ok_or("missing source lock path")?;
                if source_lock.replace(path.into()).is_some() {
                    return Err("--source-lock may only be supplied once".into());
                }
            }
            Some("--source-item") => {
                let item = args.next().ok_or("missing source item name")?;
                let item = item.to_str().ok_or("source item must be UTF-8")?;
                if item.is_empty() || item.len() > 1024 {
                    return Err("source item must contain 1..1024 bytes".into());
                }
                if source_item.replace(item.to_owned()).is_some() {
                    return Err("--source-item may only be supplied once".into());
                }
            }
            Some("--debug-assertions") => {
                let value = args.next();
                set_debug_assertions_option(
                    &mut source_options.cfg,
                    &mut debug_assertions_seen,
                    value.as_deref(),
                )?;
            }
            Some(s) if s.starts_with("--debug-assertions=") => {
                set_debug_assertions_option(
                    &mut source_options.cfg,
                    &mut debug_assertions_seen,
                    Some(std::ffi::OsStr::new(&s[19..])),
                )?;
            }
            Some("--cfg") => {
                let specification = args.next().ok_or("missing cfg specification")?;
                source_options.cfg.insert_user(
                    specification
                        .to_str()
                        .ok_or("cfg specification must be UTF-8")?,
                )?;
            }
            Some("-o" | "--output") => {
                output = Some(args.next().ok_or("missing output path")?.into())
            }
            Some("--trace") => trace = Some(args.next().ok_or("missing trace path")?.into()),
            Some("--profile") => profile = Some(args.next().ok_or("missing profile path")?.into()),
            Some("--workers") => {
                workers = args
                    .next()
                    .ok_or("missing workers")?
                    .to_str()
                    .ok_or("invalid worker count")?
                    .parse()?
            }
            Some(s) if s.starts_with("--cfg=") => {
                source_options.cfg.insert_user(&s[6..])?;
            }
            Some(s) if s.starts_with('-') => return Err(format!("unknown option {s}").into()),
            _ => {
                if source.replace(PathBuf::from(arg)).is_some() {
                    return Err("expected one source file".into());
                }
            }
        }
    }
    let source = source.ok_or("missing Rust source file, use --help")?;
    let probe = match (source_lock, source_item) {
        (Some(lock_path), Some(item)) => {
            Some(compiler::source::SourceProbeRequest { lock_path, item })
        }
        (None, None) => None,
        _ => return Err("--source-lock and --source-item must be supplied together".into()),
    };
    if !(1..=4096).contains(&workers) {
        return Err("workers must be 1..4096".into());
    }
    if check && output.is_some() {
        return Err("--check does not accept -o".into());
    }
    if !check && output.is_none() {
        return Err("missing -o NEW_EXECUTABLE".into());
    }
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("native output currently supports Linux x86-64 only".into());
    }
    // Source loading is an observed graph operation and retains immutable input.
    let start = Instant::now();
    if let Some(path) = &output
        && path.try_exists()?
    {
        return Err(format!("output already exists: {}", path.display()).into());
    }
    let compiled = match probe {
        Some(request) => compiler::compile_probe_file_observed_with_options(
            source.clone(),
            workers,
            check,
            source_options,
            request,
        ),
        None => compiler::compile_file_observed_with_options(
            source.clone(),
            workers,
            check,
            source_options,
        ),
    };
    let compiler_wall_ns = start.elapsed().as_nanos();
    // Snapshot before profile serialization so its own allocations are excluded.
    let allocation = profile.as_ref().map(|_| allocator_json());
    // Never create a trace before loading: it might alias a missing module.
    let mut trace_file = trace.as_deref().map(new_file).transpose()?;
    let compiled = match compiled {
        Ok(compiled) => compiled,
        Err(error) => {
            if let Some(path) = &profile {
                compiler_profile::Profile {
                    mode: if check { "check" } else { "build" },
                    workers,
                    compiler_wall_ns,
                    report: error.report.as_deref(),
                    resource_usage: &error.resource_usage,
                    allocator_json: allocation.as_deref().unwrap_or("null"),
                    error: Some(&error.to_string()),
                }
                .write_json(&mut new_file(path)?)?;
            }
            if let Some(file) = &mut trace_file {
                writeln!(file, "compilation failed: {error}")?;
                writeln!(file, "initialization/move analyses: {:#?}", error.analyses)?;
                if let Some(report) = &error.report {
                    writeln!(file, "{report:#?}")?;
                }
            }
            return Err(format!("{}: {error}\nnote: source preparation and syntax diagnostics use original file locations; semantic diagnostic source spans remain incomplete", source.display()).into());
        }
    };
    let source_bytes = compiled.source_bytes;
    let source_files = compiled.source_files;
    let bytes = compiled.executable.as_ref().map_or(0, Vec::len);
    if let (Some(path), Some(data)) = (&output, &compiled.executable) {
        let mut file = new_file(path)?;
        let written = (|| -> std::io::Result<()> {
            file.write_all(data)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
            }
            Ok(())
        })();
        if let Err(error) = written {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(error.into());
        }
    }
    let report = &compiled.report;
    if let Some(path) = &profile {
        compiler_profile::Profile {
            mode: if check { "check" } else { "build" },
            workers,
            compiler_wall_ns,
            report: Some(report),
            resource_usage: &compiled.resource_usage,
            allocator_json: allocation.as_deref().unwrap_or("null"),
            error: None,
        }
        .write_json(&mut new_file(path)?)?;
    }
    if let Some(file) = &mut trace_file {
        writeln!(file, "{report:#?}")?;
        writeln!(file, "control-flow graphs: {:#?}", compiled.control_flow)?;
        writeln!(
            file,
            "initialization/move analyses: {:#?}",
            compiled.analyses
        )?;
    }
    let analysis_worklist_pops: usize = compiled
        .analyses
        .iter()
        .map(|a| a.stats.worklist_pops)
        .sum();
    let analysis_move_paths: usize = compiled.analyses.iter().map(|a| a.stats.move_paths).sum();
    let analysis_moves: usize = compiled.analyses.iter().map(|a| a.stats.moves).sum();
    let analysis_variant_proof_checks: usize = compiled
        .analyses
        .iter()
        .map(|a| a.stats.variant_proof_checks)
        .sum();
    let analysis_variant_refinements: usize = compiled
        .analyses
        .iter()
        .map(|a| a.stats.variant_refinements)
        .sum();
    let analysis_variant_proofs_killed: usize = compiled
        .analyses
        .iter()
        .map(|a| a.stats.variant_proofs_killed)
        .sum();
    println!(
        "{{\"schema\":1,\"mode\":\"{}\",\"source_bytes\":{source_bytes},\"source_files\":{source_files},\"functions\":{},\"structs\":{},\"enums\":{},\"cfg_blocks\":{},\"cfg_edges\":{},\"cfg_locals\":{},\"analysis_worklist_pops\":{analysis_worklist_pops},\"analysis_move_paths\":{analysis_move_paths},\"analysis_moves\":{analysis_moves},\"analysis_variant_proof_checks\":{analysis_variant_proof_checks},\"analysis_variant_refinements\":{analysis_variant_refinements},\"analysis_variant_proofs_killed\":{analysis_variant_proofs_killed},\"workers\":{workers},\"executable_bytes\":{bytes},\"total_ns\":{},\"engine_ns\":{},\"nodes\":{},\"edges\":{},\"intern_hits\":{},\"failures\":{},\"events_dropped\":{},\"capture_complete\":{},\"measurement\":\"contended/uncontrolled\"}}",
        if check { "check" } else { "build" },
        compiled.functions,
        compiled.structs,
        compiled.enums,
        compiled.cfg_blocks,
        compiled.cfg_edges,
        compiled.cfg_locals,
        start.elapsed().as_nanos(),
        report.elapsed_ns,
        report.nodes,
        report.edges,
        report.intern_hits,
        report.failures,
        report.events_dropped,
        report.capture_complete
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn debug_assertions_option_accepts_only_exact_yes_or_no_once() {
        for (value, expected) in [("yes", true), ("no", false)] {
            let mut cfg = compiler::source::CfgSet::native_x86_64_linux();
            let mut seen = false;
            set_debug_assertions_option(&mut cfg, &mut seen, Some(OsStr::new(value))).unwrap();
            assert_eq!(cfg.debug_assertions(), expected);
            assert!(cfg.validate_native().is_ok());
            assert_eq!(
                set_debug_assertions_option(&mut cfg, &mut seen, Some(OsStr::new(value))),
                Err("--debug-assertions may only be supplied once")
            );
            assert_eq!(cfg.debug_assertions(), expected);
        }
    }

    #[test]
    fn debug_assertions_option_rejects_missing_and_invalid_without_mutation() {
        let mut cfg = compiler::source::CfgSet::native_x86_64_linux();
        let mut seen = false;
        assert_eq!(
            set_debug_assertions_option(&mut cfg, &mut seen, None),
            Err("missing --debug-assertions value (expected yes or no)")
        );
        for value in [
            "", "true", "false", "1", "0", "YES", "No", " yes", "no ", "--check",
        ] {
            assert_eq!(
                set_debug_assertions_option(&mut cfg, &mut seen, Some(OsStr::new(value))),
                Err("invalid --debug-assertions value (expected yes or no)")
            );
            assert!(!seen);
            assert!(cfg.debug_assertions());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert!(
                set_debug_assertions_option(&mut cfg, &mut seen, Some(OsStr::from_bytes(b"\xff")))
                    .is_err()
            );
        }
        assert!(HELP.contains("--debug-assertions yes|no"));
        assert!(HELP.contains("default: yes"));
    }
}
