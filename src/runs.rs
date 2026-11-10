//! `justrust runs` and `justrust show`: read back recorded runs.

use crate::paths;
use crate::summary::{IndexEntry, Summary};
use anyhow::{Context, Result, bail};
use std::fmt::Write as _;

pub fn load_index() -> Vec<IndexEntry> {
    let Ok(path) = paths::index_file() else {
        return Vec::new();
    };
    let Ok(s) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    s.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// The last parseable index entry, reading only the file's tail. The index
/// grows without bound (tens of MB), and `status --watch` asks for the latest
/// run every second while idle.
pub fn last_index_entry() -> Option<IndexEntry> {
    last_entry_in(&paths::index_file().ok()?)
}

fn last_entry_in(path: &std::path::Path) -> Option<IndexEntry> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut window: u64 = 64 * 1024;
    loop {
        let start = len.saturating_sub(window);
        f.seek(SeekFrom::Start(start)).ok()?;
        let mut buf = Vec::new();
        f.by_ref().take(len - start).read_to_end(&mut buf).ok()?;
        let text = String::from_utf8_lossy(&buf);
        // The first line of a partial window may be cut; skip it unless the
        // window starts at the beginning of the file.
        let skip = usize::from(start > 0);
        let lines: Vec<&str> = text.lines().skip(skip).collect();
        if let Some(e) = lines
            .iter()
            .rev()
            .find_map(|l| serde_json::from_str(l).ok())
        {
            return Some(e);
        }
        if start == 0 {
            return None;
        }
        window *= 4;
    }
}

fn ago(t: f64) -> String {
    let d = (paths::now() - t).max(0.0);
    match d {
        d if d < 90.0 => format!("{d:.0}s ago"),
        d if d < 5400.0 => format!("{:.0}m ago", d / 60.0),
        d if d < 129600.0 => format!("{:.0}h ago", d / 3600.0),
        d => format!("{:.0}d ago", d / 86400.0),
    }
}

fn short_args(args: &[String]) -> String {
    let s = args.join(" ");
    if s.len() > 60 {
        format!("{}…", &s[..s.floor_char_boundary(59)])
    } else {
        s
    }
}

/// Which failed runs `justrust runs` lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FailFilter {
    #[default]
    All,
    /// Every failed run.
    Failed,
    /// Failures that are not the code's fault: toolchain, infra, unknown.
    Ours,
}

/// The failure class of an indexed run. Runs recorded before classification
/// existed are classified from their saved summary and output.
fn failure_class(r: &IndexEntry) -> Option<crate::failure::Class> {
    if r.exit == 0 {
        return None;
    }
    r.failure.or_else(|| classify_saved(&r.id).map(|f| f.class))
}

/// Classify a recorded run from its run directory.
pub fn classify_saved(id: &str) -> Option<crate::failure::Failure> {
    let dir = paths::runs_dir().ok()?.join(id);
    let s: Summary = serde_json::from_slice(&std::fs::read(dir.join("summary.json")).ok()?).ok()?;
    if let Some(f) = s.diagnostics.failure {
        return Some(f);
    }
    let raw = std::fs::read_to_string(dir.join("output.jsonl")).unwrap_or_default();
    let lines: Vec<crate::record::OutputLine> = raw
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let stderr: Vec<&str> = lines
        .iter()
        .filter(|l| l.s == "e")
        .map(|l| l.l.as_str())
        .collect();
    let units = crate::summary::load_units(&dir);
    let unit_exits: Vec<i32> = units.iter().map(|u| u.exit).filter(|&c| c != 0).collect();
    crate::failure::classify(&crate::failure::Input {
        exit: s.exit,
        subcommand: &s.subcommand,
        lines: &stderr,
        compile_failed: s.diagnostics.compile_failed,
        compile_errors: s.diagnostics.compile_errors,
        environment_fault: s.diagnostics.environment_fault.as_deref(),
        tests_failed: s.tests.failed,
        unit_exits: &unit_exits,
        units_ran: units.len(),
    })
}

pub fn list(limit: usize, here: bool, json: bool, filter: FailFilter) -> Result<()> {
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    let mut runs: Vec<IndexEntry> = load_index()
        .into_iter()
        .filter(|r| !here || r.cwd.starts_with(&cwd))
        .collect();
    if filter != FailFilter::All {
        // Classify newest first and stop once `limit` matches are found:
        // old runs may need their saved output read.
        let mut kept = Vec::new();
        for mut r in runs.into_iter().rev() {
            if kept.len() >= limit {
                break;
            }
            r.failure = failure_class(&r);
            let keep = match (filter, r.failure) {
                (FailFilter::Failed, _) => r.exit != 0,
                (FailFilter::Ours, Some(c)) => c.is_ours(),
                _ => false,
            };
            if keep {
                kept.push(r);
            }
        }
        kept.reverse();
        runs = kept;
    }
    let skip = runs.len().saturating_sub(limit);
    runs.drain(..skip);
    if json {
        for r in &runs {
            println!("{}", serde_json::to_string(r)?);
        }
        return Ok(());
    }
    if runs.is_empty() {
        println!("No recorded runs yet. Run `justrust cargo build` or `justrust install`.");
        return Ok(());
    }
    println!(
        "{:<26} {:>8} {:>8} {:>8} {:>6} {:>5} {:<9}  {:<10} COMMAND",
        "ID", "WALL", "COMPILE", "TESTS", "UNITS", "EXIT", "CAUSE", "WHEN"
    );
    for r in &runs {
        println!(
            "{:<26} {:>7.1}s {:>7.1}s {:>7.1}s {:>6} {:>5} {:<9}  {:<10} cargo {}",
            r.id,
            r.wall,
            r.compile + 0.0,
            r.test_run + 0.0,
            r.units_compiled,
            if r.compile_failed {
                "build!".to_owned()
            } else {
                r.exit.to_string()
            },
            r.failure
                .map(|c| match c {
                    crate::failure::Class::UserCode => "code",
                    crate::failure::Class::Toolchain => "TOOLCHAIN",
                    crate::failure::Class::Infra => "INFRA",
                    crate::failure::Class::Interrupted => "intr",
                    crate::failure::Class::Unknown => "UNKNOWN",
                })
                .unwrap_or(if r.exit == 0 { "" } else { "?" }),
            ago(r.start),
            short_args(&r.args)
        );
    }
    Ok(())
}

fn resolve_id(id: Option<&str>) -> Result<String> {
    let index = load_index();
    match id {
        None | Some("last") => index
            .last()
            .map(|r| r.id.clone())
            .context("no recorded runs"),
        Some(prefix) => {
            let m: Vec<&IndexEntry> = index.iter().filter(|r| r.id.starts_with(prefix)).collect();
            match m.len() {
                0 => {
                    // Runs that crashed before writing an index line still have a directory.
                    if paths::runs_dir()?.join(prefix).is_dir() {
                        Ok(prefix.to_owned())
                    } else {
                        bail!("no run matching {prefix}")
                    }
                }
                1 => Ok(m[0].id.clone()),
                _ => Ok(m.last().unwrap().id.clone()),
            }
        }
    }
}

/// Print the full saved output of a run, optionally only lines matching `grep`.
pub fn log(id: Option<&str>, grep: Option<&str>, tail: Option<usize>) -> Result<()> {
    let id = resolve_id(id)?;
    let path = paths::runs_dir()?.join(&id).join("output.jsonl");
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("run {id} has no saved output"))?;
    let mut lines: Vec<String> = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<crate::record::OutputLine>(l).ok())
        .map(|l| l.l)
        .filter(|l| grep.is_none_or(|g| l.contains(g)))
        .collect();
    if let Some(n) = tail {
        let skip = lines.len().saturating_sub(n);
        lines.drain(..skip);
    }
    for l in lines {
        println!("{l}");
    }
    Ok(())
}

pub fn show(id: Option<&str>, json: bool) -> Result<()> {
    let id = resolve_id(id)?;
    let dir = paths::runs_dir()?.join(&id);
    let raw = std::fs::read(dir.join("summary.json"))
        .with_context(|| format!("run {id} has no summary"))?;
    if json {
        println!("{}", String::from_utf8_lossy(&raw));
        return Ok(());
    }
    let mut s: Summary = serde_json::from_slice(&raw)?;
    if s.diagnostics.failure.is_none() {
        s.diagnostics.failure = classify_saved(&id);
    }
    print!("{}", render(&s, &dir));
    Ok(())
}

fn bar(v: f64, total: f64, width: usize) -> String {
    let n = if total > 0.0 {
        ((v / total) * width as f64).round() as usize
    } else {
        0
    };
    "█".repeat(n.min(width))
}

pub fn render(s: &Summary, dir: &std::path::Path) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "run      {}", s.id);
    let _ = writeln!(o, "command  cargo {}", s.args.join(" "));
    let _ = writeln!(o, "cwd      {}", s.cwd);
    if let Some(head) = &s.git.head {
        let _ = writeln!(
            o,
            "git      {} ({} modified files)",
            head,
            s.git.dirty_files.unwrap_or(0)
        );
    }
    if let Some(a) = &s.agent_session {
        let _ = writeln!(o, "agent    {a}");
    }
    if let Some(r) = &s.remote {
        let _ = writeln!(o, "where    remote on {}", r.machine);
    } else if let Some(why) = &s.route {
        let _ = writeln!(o, "where    local ({why})");
    }
    let status = if let (true, Some(fault)) = (
        s.diagnostics.compile_failed,
        s.diagnostics.environment_fault.as_ref(),
    ) {
        format!(
            "exit {} (build environment fault: {fault}, not a code error)",
            s.exit
        )
    } else if s.diagnostics.compile_failed {
        format!(
            "exit {} (compile failed, {} errors)",
            s.exit, s.diagnostics.compile_errors
        )
    } else if s.tests.failed > 0 {
        format!("exit {} ({} tests failed)", s.exit, s.tests.failed)
    } else {
        format!("exit {}", s.exit)
    };
    let _ = writeln!(o, "result   {status}");
    if let Some(f) = &s.diagnostics.failure {
        let _ = writeln!(o, "cause    {}: {}", f.class.as_str(), f.reason);
        if let Some(e) = &f.evidence {
            let _ = writeln!(o, "         {e}");
        }
        if let Some(v) = crate::stock_verify::load(dir) {
            let _ = writeln!(o, "stock    {}", v.line());
        } else if f.class == crate::failure::Class::Toolchain {
            let _ = writeln!(o, "stock    not verified (or still running)");
        }
    }
    let _ = writeln!(
        o,
        "time     {:.2}s wall, {:.1}s CPU ({:.1} of {} cores on average)\n",
        s.wall, s.cpu_secs, s.resources.avg_build_cores, s.ncpu
    );

    if let Some(note) = &s.cpu_secs_note {
        let _ = writeln!(o, "CPU accounting: {note}");
    }
    if let Some(e) = s.e2e.clone().or_else(|| crate::e2e::derive(s)) {
        let _ = writeln!(o, "{}", crate::e2e::render(&e));
    }
    let _ = writeln!(o, "Where cargo's wall time went");
    let p = &s.phases;
    for (label, v, note) in [
        (
            "cargo startup",
            p.startup,
            "resolve, fingerprints, planning",
        ),
        (
            "lock wait",
            p.lock_wait,
            "another cargo held the build directory",
        ),
        ("compiling", p.compile, "at least one rustc running"),
        (
            "build gaps",
            p.build_gaps,
            "build scripts, scheduling, no rustc running",
        ),
        ("running tests", p.test_run, "test binaries"),
        ("after build", p.tail, "doc-test setup, run, shutdown"),
    ] {
        if v < 0.005 && label != "compiling" {
            continue;
        }
        let pct = if s.wall > 0.0 {
            100.0 * v / s.wall
        } else {
            0.0
        };
        let _ = writeln!(
            o,
            "  {label:<14} {v:>7.2}s {pct:>4.0}%  {:<24} {note}",
            bar(v, s.wall, 24)
        );
    }

    let u = &s.units;
    if let Some(savings) = s.savings.clone().or_else(|| crate::savings::from_dir(dir)) {
        let _ = writeln!(o, "\nKnown skipped work: {}", savings.line());
    }
    let _ = writeln!(
        o,
        "\nCompilation units: {} compiled ({} local, {} dependencies, {} build scripts){}",
        u.compiled,
        u.local,
        u.dependencies,
        u.build_scripts,
        if u.fresh > 0 {
            format!(", {} fresh", u.fresh)
        } else {
            String::new()
        }
    );
    if u.compiled > 0 {
        let _ = writeln!(
            o,
            "  rustc CPU {:.1}s, rustc wall sum {:.1}s, pass timings for {} of {}",
            u.rustc_cpu_secs, u.rustc_wall_sum, u.with_pass_timings, u.compiled
        );
        if let Some(sp) = &u.phase_split {
            let all = (sp.frontend + sp.codegen + sp.link + sp.incremental + sp.other).max(1e-9);
            let _ = writeln!(
                o,
                "\n  Compiler phases, all {} timed units ({:.1}s of {:.1}s rustc wall explained)",
                u.with_pass_timings, u.phase_wall_secs, u.rustc_wall_sum
            );
            for (name, secs, what) in [
                (
                    "frontend",
                    sp.frontend,
                    "parse, expand, resolve, type and borrow check, lints, metadata",
                ),
                (
                    "codegen",
                    sp.codegen,
                    "monomorphize, LLVM IR, wait for LLVM",
                ),
                ("link", sp.link, "linker and output files"),
                (
                    "incremental",
                    sp.incremental,
                    "load and save the incremental cache",
                ),
                (
                    "other",
                    sp.other,
                    "compiler start, crate loading, untimed work",
                ),
            ] {
                let _ = writeln!(
                    o,
                    "    {:<12} {:>7.2}s {:>4.0}%  {}",
                    name,
                    secs,
                    100.0 * secs / all,
                    what
                );
            }
            if u.proc_macro_secs >= 0.005 {
                let _ = writeln!(
                    o,
                    "    of frontend: {:.2}s running proc macros",
                    u.proc_macro_secs
                );
            }
            if u.llvm_thread_secs >= 0.005 {
                let _ = writeln!(
                    o,
                    "    LLVM optimize and emit: {:.2}s summed over worker threads",
                    u.llvm_thread_secs
                );
            }
        } else {
            let _ = writeln!(
                o,
                "  no compiler phase split: needs a jr toolchain with the phase ledger, or JUSTRUST_PASSES=on"
            );
        }
        let _ = writeln!(
            o,
            "\n  {:<34} {:>7} {:>7} {:>7} {:>7} {:>8}  SPLIT (frontend/codegen/link/incr/other)",
            "UNIT", "SHARE", "WALL", "CPU", "RMETA", "PEAK MB"
        );
        for t in s
            .top_units
            .iter()
            .filter(|t| t.wall_share >= 0.01 || t.wall >= 0.5)
            .take(10)
        {
            let split = t.split.as_ref().map_or_else(
                || "-".to_owned(),
                |sp| {
                    format!(
                        "{:.2} / {:.2} / {:.2} / {:.2} / {:.2}",
                        sp.frontend, sp.codegen, sp.link, sp.incremental, sp.other
                    )
                },
            );
            let rmeta = t
                .rmeta_secs
                .map_or_else(|| "-".to_owned(), |r| format!("{r:.1}s"));
            let name = format!("{}{}", t.name, if t.local { "" } else { " [dep]" });
            let _ = writeln!(
                o,
                "  {:<34} {:>6.1}s {:>6.1}s {:>6.1}s {:>7} {:>8.0}  {}",
                trunc(&name, 34),
                t.wall_share,
                t.wall,
                t.cpu,
                rmeta,
                t.max_rss_mb,
                split
            );
        }
        if let Some(t) = s.top_units.first().filter(|t| !t.top_passes.is_empty()) {
            let _ = writeln!(o, "\n  Slowest rustc passes in {}", t.name);
            for pass in &t.top_passes {
                let calls = if pass.calls > 1 {
                    format!("  x{}", pass.calls)
                } else {
                    String::new()
                };
                let _ = writeln!(o, "    {:<44} {:>7.2}s{calls}", pass.name, pass.secs);
            }
        }
    }

    if s.link.link_pass_secs > 0.0 {
        let _ = writeln!(
            o,
            "\nLinking: {:.2}s inside rustc link passes ({} linker processes caught by the 200 ms sampler)",
            s.link.link_pass_secs, s.link.linker_processes
        );
    }

    let t = &s.tests;
    if t.has_results() {
        let _ = writeln!(
            o,
            "\nTests: {} passed, {} failed, {} ignored, {} filtered out (libtest reports {:.2}s)",
            t.passed, t.failed, t.ignored, t.filtered_out, t.libtest_secs
        );
        for b in t
            .binaries
            .iter()
            .filter(|b| b.wall >= 0.05 || b.passed + b.failed > 0)
            .take(10)
        {
            let _ = writeln!(
                o,
                "  {:<40} {:>6.2}s  {} passed, {} failed, {} filtered out",
                trunc(&b.name, 40),
                b.wall,
                b.passed,
                b.failed,
                b.filtered_out
            );
        }
        if !t.slowest.is_empty() {
            let _ = writeln!(
                o,
                "  slowest of {} timed tests ({:.1}s summed):",
                t.timed, t.timed_secs
            );
            for x in t.slowest.iter().take(10) {
                let _ = writeln!(
                    o,
                    "    {:>7.2}s  {}{}",
                    x.secs,
                    x.name,
                    if x.passed { "" } else { " (FAILED)" }
                );
            }
        }
    }

    let r = &s.resources;
    let _ = writeln!(o, "\nResources");
    let _ = writeln!(
        o,
        "  CPU     avg {:.1} cores, peak {:.1} cores. Other processes used {:.1} cores on average",
        r.avg_build_cores, r.peak_build_cores, r.avg_other_cores
    );
    let _ = writeln!(
        o,
        "  memory  build peak {:.0} MB, largest rustc {:.0} MB, lowest available {:.1} GB",
        r.peak_build_rss_mb,
        r.peak_unit_rss_mb,
        r.min_mem_available_mb as f64 / 1024.0
    );
    if r.psi_unavailable {
        let _ = writeln!(
            o,
            "  stalls  not measured: this machine has no pressure stall info (PSI)"
        );
    } else {
        let _ = writeln!(
            o,
            "  stalls  cpu {:.2}s, memory {:.2}s, io {:.2}s (Linux pressure stall info, machine-wide)",
            r.psi_cpu_secs, r.psi_mem_secs, r.psi_io_secs
        );
    }
    if r.max_foreign_rustc > 0 {
        let _ = writeln!(
            o,
            "  contention: up to {} other rustc processes were running",
            r.max_foreign_rustc
        );
    }

    if let Some(e) = &s.diagnostics.first_error {
        let _ = writeln!(o, "\nFirst error: {e}");
    }
    let findings = crate::findings::analyze(s);
    if !findings.is_empty() {
        let _ = writeln!(o, "\nWaste");
        for f in &findings {
            let _ = writeln!(o, "  ~{:.1}s {}", f.cost_secs, f.message);
        }
    }
    let hints = crate::hints::for_run(s);
    if !hints.is_empty() {
        let _ = writeln!(o, "\nRecurring costs (this workspace, last 7 days)");
        for h in &hints {
            let _ = writeln!(
                o,
                "  ~{:.0}s over {} runs: {}",
                h.total_secs, h.runs, h.message
            );
        }
    }
    let _ = writeln!(o, "\nRaw data: {}", dir.display());
    o
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tail_tests {
    use super::*;

    fn line(id: &str) -> String {
        serde_json::to_string(&IndexEntry {
            id: id.into(),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn last_entry_reads_tail_and_skips_torn_lines() {
        let dir = std::env::temp_dir().join(format!("jr-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("index.jsonl");
        // Larger than the first 64 KiB window, so the cut first line is exercised.
        let mut s: String = (0..3000)
            .map(|i| line(&format!("run-{i}")) + "\n")
            .collect();
        std::fs::write(&p, &s).unwrap();
        assert_eq!(last_entry_in(&p).unwrap().id, "run-2999");
        // A partially written trailing record falls back to the previous one.
        s.push_str("{\"id\":\"half");
        std::fs::write(&p, &s).unwrap();
        assert_eq!(last_entry_in(&p).unwrap().id, "run-2999");
        // A single line with no newline (window covers the whole file).
        std::fs::write(&p, line("only")).unwrap();
        assert_eq!(last_entry_in(&p).unwrap().id, "only");
        std::fs::write(&p, "").unwrap();
        assert!(last_entry_in(&p).is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
