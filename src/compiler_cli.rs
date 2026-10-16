use justrust_engine::compiler;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const HELP: &str = "jr-rust SOURCE [-o NEW_EXECUTABLE] [--workers N] [--check] [--trace NEW_TRACE]\n\nClean-room Rust subset compiler. Direct Linux x86-64 ELF output, no rustc delegation.\nSee COMPILER.md for supported syntax and limitations. Existing files are never overwritten.\nAccounting is always enabled. --check performs semantic checks without code generation.\n";

fn main() {
    if let Err(error) = run() {
        eprintln!("jr-rust: {error}");
        std::process::exit(1);
    }
}

fn new_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut source = None;
    let mut output: Option<PathBuf> = None;
    let mut trace: Option<PathBuf> = None;
    let mut workers = 1usize;
    let mut check = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                print!("{HELP}");
                return Ok(());
            }
            Some("--check") => check = true,
            Some("-o" | "--output") => {
                output = Some(args.next().ok_or("missing output path")?.into())
            }
            Some("--trace") => trace = Some(args.next().ok_or("missing trace path")?.into()),
            Some("--workers") => {
                workers = args
                    .next()
                    .ok_or("missing workers")?
                    .to_str()
                    .ok_or("invalid worker count")?
                    .parse()?
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
    let compiled = compiler::compile_file_observed(source.clone(), workers, check);
    // Never create a trace before loading: it might alias a missing module.
    let mut trace_file = trace.as_deref().map(new_file).transpose()?;
    let compiled = match compiled {
        Ok(compiled) => compiled,
        Err(error) => {
            if let Some(file) = &mut trace_file {
                writeln!(file, "compilation failed: {error}")?;
                writeln!(file, "initialization/move analyses: {:#?}", error.analyses)?;
                if let Some(report) = &error.report {
                    writeln!(file, "{report:#?}")?;
                }
            }
            return Err(format!("{}: {error}\nnote: filesystem modules use temporary inline expansion; frontend line/column positions refer to expanded source, not original module files", source.display()).into());
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
