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
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in s
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<IndexEntry>(l).ok())
    {
        if seen.insert(entry.id.clone()) {
            out.push(entry);
        }
    }
    out.reverse();
    out
}

/// True if `id` already appears in the tail of `index.jsonl`. Used when an
/// attached remote run receives the shared run's summary so it does not append
/// a second index row for the same run id.
pub(crate) fn index_has_recent_id(path: &std::path::Path, id: &str) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut checked = 0usize;
    read_recent_entries(&mut f, 1, |e| {
        if e.id == id {
            return true;
        }
        checked += 1;
        false
    })
    .is_ok_and(|v| !v.is_empty())
        && checked < usize::MAX
}

/// The last parseable index entry, reading only the file's tail. The index
/// grows without bound (tens of MB), and `status --watch` asks for the latest
/// run every second while idle.
pub fn last_index_entry() -> Option<IndexEntry> {
    last_entry_in(&paths::index_file().ok()?)
}

fn last_entry_in(path: &std::path::Path) -> Option<IndexEntry> {
    let mut f = std::fs::File::open(path).ok()?;
    read_recent_entries(&mut f, 1, |_| true).ok()?.pop()
}

/// Visit complete records newest first, stopping once enough match. The index
/// is append-only and may be tens of MB: dashboard polling must not reparse its
/// entire history to display a bounded tail. Keep one block and, when a record
/// crosses blocks, its reversed suffix. Every byte is read at most once, even
/// for unusually long records or filters that must search the whole history.
fn read_recent_entries(
    reader: &mut (impl std::io::Read + std::io::Seek),
    limit: usize,
    mut keep: impl FnMut(&mut IndexEntry) -> bool,
) -> std::io::Result<Vec<IndexEntry>> {
    read_recent_records(reader, limit, |line| {
        let Some(mut entry) = serde_json::from_slice(line).ok() else {
            return Ok(None);
        };
        Ok(keep(&mut entry).then_some(entry))
    })
}

fn read_recent_records<T>(
    reader: &mut (impl std::io::Read + std::io::Seek),
    limit: usize,
    mut select: impl FnMut(&[u8]) -> std::io::Result<Option<T>>,
) -> std::io::Result<Vec<T>> {
    use std::io::SeekFrom;

    let mut entries = Vec::new();
    if limit == 0 {
        return Ok(entries);
    }
    let mut end = reader.seek(SeekFrom::End(0))?;
    let mut suffix = Vec::new();
    while end > 0 {
        let start = end.saturating_sub(64 * 1024);
        let mut block = vec![0; (end - start) as usize];
        reader.seek(SeekFrom::Start(start))?;
        reader.read_exact(&mut block)?;
        let mut lines = block.rsplit(|b| *b == b'\n').peekable();
        while let Some(part) = lines.next() {
            if start > 0 && lines.peek().is_none() {
                // The first bytes of a noninitial block may be a partial
                // record. Save them until the previous block supplies its
                // beginning. Reversed accumulation avoids repeatedly copying
                // a large line when it spans many blocks.
                suffix.extend(part.iter().rev().copied());
                break;
            }
            let joined;
            let line = if suffix.is_empty() {
                part
            } else {
                joined = part
                    .iter()
                    .copied()
                    .chain(suffix.drain(..).rev())
                    .collect::<Vec<_>>();
                &joined
            };
            if let Some(entry) = select(line)? {
                entries.push(entry);
                if entries.len() == limit {
                    entries.reverse();
                    return Ok(entries);
                }
            }
        }
        end = start;
    }
    entries.reverse();
    Ok(entries)
}

pub(crate) fn ago(t: f64) -> String {
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
    let mut seen = std::collections::HashSet::new();
    let runs = paths::index_file()
        .ok()
        .and_then(|p| std::fs::File::open(p).ok())
        .and_then(|mut f| {
            read_recent_entries(&mut f, limit, |r| {
                if !seen.insert(r.id.clone()) {
                    return false;
                }
                if here && !r.cwd.starts_with(&cwd) {
                    return false;
                }
                if filter == FailFilter::All {
                    return true;
                }
                // Classify only matching workspaces, newest first. Legacy
                // failures can need saved output, so never inspect runs older
                // than the requested number of matches.
                r.failure = failure_class(r);
                match (filter, r.failure) {
                    (FailFilter::Failed, _) => r.exit != 0,
                    (FailFilter::Ours, Some(c)) => c.is_ours(),
                    _ => false,
                }
            })
            .ok()
        })
        .unwrap_or_default();
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
        "{:<26} {:>8} {:>7} {:>7} {:>8} {:>8} {:>6} {:>5} {:<9}  {:<10} COMMAND",
        "ID", "WALL", "EST", "FLOOR", "COMPILE", "TESTS", "UNITS", "EXIT", "CAUSE", "WHEN"
    );
    for r in &runs {
        let est = r
            .optimistic_est_secs
            .map(crate::theoretical::format_short)
            .unwrap_or_else(|| "-".into());
        let floor = r
            .theoretical_floor_secs
            .map(crate::theoretical::format_short)
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<26} {:>7.1}s {:>7} {:>7} {:>7.1}s {:>7.1}s {:>6} {:>5} {:<9}  {:<10} cargo {}",
            r.id,
            r.wall,
            est,
            floor,
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
    match id {
        None | Some("last") => {
            let cwd = std::env::current_dir()?;
            let session = std::env::var("JCODE_SESSION_ID").ok();
            let mut file = std::fs::File::open(paths::index_file()?).context("no recorded runs")?;
            let workspace = selection_workspace(&cwd);
            latest_in_context(&mut file, &cwd, &workspace, session.as_deref())?.with_context(|| format!(
                "no recorded runs in workspace {}{}; use `justrust runs` and an explicit run ID to inspect another context",
                workspace.display(),
                session.as_ref().map(|s| format!(" for session {s}")).unwrap_or_default()
            ))
        }
        Some(prefix) => {
            if let Some(id) = paths::index_file()
                .ok()
                .and_then(|p| std::fs::File::open(p).ok())
                .and_then(|mut f| explicit_index_id(&mut f, prefix).ok().flatten())
            {
                return Ok(id);
            }
            // A crashed run can have a directory but no index entry.
            if paths::runs_dir()?.join(prefix).is_dir() {
                Ok(prefix.to_owned())
            } else {
                bail!("no run matching {prefix}")
            }
        }
    }
}

fn explicit_index_id(
    reader: &mut (impl std::io::Read + std::io::Seek),
    prefix: &str,
) -> std::io::Result<Option<String>> {
    let needle = memchr::memmem::Finder::new(prefix.as_bytes());
    Ok(read_recent_records(reader, 1, |line| {
        // With no escapes a decoded ID prefix must occur literally in the
        // row, regardless of field order. Hits and escaped rows require full
        // typed validation, not just matching bytes in an unrelated field.
        if needle.find(line).is_none() && memchr::memchr(b'\\', line).is_none() {
            return Ok(None);
        }
        Ok(serde_json::from_slice::<IndexEntry>(line)
            .ok()
            .filter(|r| r.id.starts_with(prefix))
            .map(|r| r.id))
    })?
    .pop())
}

fn selection_workspace(cwd: &std::path::Path) -> std::path::PathBuf {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    crate::early::workspace_root(&cwd).unwrap_or(cwd)
}

/// Implicit lookup must never report another workspace or agent's result.
/// Explicit IDs remain global, including for old runs without session metadata.
fn latest_in_context(
    reader: &mut (impl std::io::Read + std::io::Seek),
    cwd: &std::path::Path,
    workspace: &std::path::Path,
    session: Option<&str>,
) -> std::io::Result<Option<String>> {
    // Only negative matches repeat: the first positive match ends the scan.
    // Key by literal spelling, not Path equality or a normalized ancestor.
    let mut misses = std::collections::HashSet::<String>::new();
    Ok(read_recent_entries(reader, 1, |r| {
        if session.is_some_and(|s| r.agent_session.as_deref() != Some(s)) {
            return false;
        }
        if cwd.to_str() == Some(r.cwd.as_str()) {
            return true;
        }
        if misses.contains(&r.cwd) {
            return false;
        }
        if selection_workspace(std::path::Path::new(&r.cwd)) == workspace {
            return true;
        }
        if misses.len() == 128 {
            misses.clear();
        }
        misses.insert(std::mem::take(&mut r.cwd));
        false
    })?
    .pop()
    .map(|r| r.id))
}

/// Print the full saved output of a run, optionally only lines matching `grep`.
pub fn log(id: Option<&str>, grep: Option<&str>, tail: Option<usize>) -> Result<()> {
    let id = resolve_id(id)?;
    let path = paths::runs_dir()?.join(&id).join("output.jsonl");
    if let Some(n) = tail {
        let mut f =
            std::fs::File::open(&path).with_context(|| format!("run {id} has no saved output"))?;
        for line in
            log_tail(&mut f, grep, n).with_context(|| format!("reading output for run {id}"))?
        {
            println!("{line}");
        }
        return Ok(());
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("run {id} has no saved output"))?;
    let lines = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<crate::record::OutputLine>(l).ok())
        .map(|l| l.l)
        .filter(|l| grep.is_none_or(|g| l.contains(g)));
    for l in lines {
        println!("{l}");
    }
    Ok(())
}

/// Malformed JSON (including torn writes) is skipped. Invalid UTF-8 is an
/// error in any visited record, but a bounded tail does not validate the
/// unrequested prefix. An unbounded log still validates the entire file.
fn log_tail(
    reader: &mut (impl std::io::Read + std::io::Seek),
    grep: Option<&str>,
    n: usize,
) -> std::io::Result<Vec<String>> {
    read_recent_records(reader, n, |line| {
        let text = std::str::from_utf8(line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(serde_json::from_str::<crate::record::OutputLine>(text)
            .ok()
            .map(|record| record.l)
            .filter(|l| grep.is_none_or(|g| l.contains(g))))
    })
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
    // A successful run has no failure to reconstruct. Older records may have
    // large logs and unit ledgers, neither of which changes this verdict.
    if s.exit != 0 && s.diagnostics.failure.is_none() {
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
    let computed_floor;
    let floor = match &s.theoretical_floor {
        Some(f) => f,
        None => {
            computed_floor = crate::theoretical::compute(
                &s.subcommand,
                &s.args,
                &s.git,
                &s.units,
                &s.phases,
                &s.tests,
            );
            &computed_floor
        }
    };
    {
        let f = floor;
        // A run ended by a signal did not finish: its wall time is not a
        // build time to hold against the floor.
        let ratio = if f.total_secs > 0.0 && !crate::summary::signal_exit(s.exit) {
            format!(" ({:.1}x above theoretical optimal)", s.wall / f.total_secs)
        } else {
            String::new()
        };
        let _ = writeln!(
            o,
            "floor    {}{ratio}  [overhead {}, frontend {}, codegen {}, link {}, tests {}; {} dirty lines{}]",
            crate::theoretical::format_short(f.total_secs),
            crate::theoretical::format_short(f.overhead_secs),
            crate::theoretical::format_short(f.frontend_secs),
            crate::theoretical::format_short(f.codegen_secs),
            crate::theoretical::format_short(f.link_secs),
            crate::theoretical::format_short(f.test_secs),
            f.dirty_lines,
            if f.cold { ", cold" } else { "" },
        );
    }
    if let Some(r) = &s.remote {
        let meta = std::fs::read(dir.join("meta.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<crate::record::Meta>(&b).ok());
        let place = crate::summary::execution_place(
            s.place
                .as_deref()
                .or_else(|| meta.as_ref().and_then(|m| m.place.as_deref())),
            Some(&r.machine),
        );
        let _ = writeln!(o, "where    {place} on {}", r.machine);
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
    } else if crate::summary::signal_exit(s.exit) && !s.units.cut.is_empty() {
        format!(
            "exit {} (killed while compiling {})",
            s.exit,
            s.units.cut.join(", ")
        )
    } else if crate::summary::signal_exit(s.exit) {
        format!("exit {} (killed)", s.exit)
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
    let lock_note = if s.locks_waited.is_empty() {
        "stderr-based Cargo lock estimate; lock kind unknown".to_string()
    } else {
        format!(
            "stderr-based Cargo lock estimate: {}",
            s.locks_waited.join(", ")
        )
    };
    for (label, v, note) in [
        (
            "cargo startup",
            p.startup,
            "resolve, fingerprints, planning",
        ),
        ("lock wait", p.lock_wait, lock_note.as_str()),
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
    let reuse = crate::reuse::render_run(&crate::summary::load_units(dir));
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
            let units = crate::summary::load_units(dir);
            let _ = writeln!(o, "  no compiler phase split");
            let mut reasons = std::collections::BTreeMap::new();
            for unit in &units {
                if matches!(
                    unit.depcache.as_str(),
                    "hit" | "remote" | "peer" | "prebuilt" | "local"
                ) {
                    continue;
                }
                *reasons
                    .entry(unit.phase_coverage.as_str())
                    .or_insert(0usize) += 1;
            }
            if reasons.is_empty() {
                let _ = writeln!(
                    o,
                    "    phase coverage not recorded (older or unavailable unit records)"
                );
            }
            for (coverage, count) in reasons {
                let explanation = match coverage {
                    "native-ledger-unavailable" => {
                        "native phase ledger unavailable: compiler capability was not detected or the recording directory could not be created; JUSTRUST_PASSES=on enables time-passes fallback"
                    }
                    "disabled" => {
                        "phase timing explicitly disabled by JUSTRUST_PASSES; remove the override to enable native telemetry"
                    }
                    "native-ledger-missing" => {
                        "native phase ledger requested but the compiler did not produce a readable ledger"
                    }
                    "native-ledger-empty" => {
                        "native phase ledger read but contained no usable timings"
                    }
                    "time-passes" => {
                        "time-passes requested but no usable compiler phase timings recorded"
                    }
                    "native-ledger" => {
                        "native phase ledger recorded but no aggregate phase split available"
                    }
                    "" => "phase coverage not recorded (older client or reconstructed unit)",
                    _ => "unrecognized recorded phase coverage",
                };
                let label = if coverage.is_empty() {
                    "unknown"
                } else {
                    coverage
                };
                let _ = writeln!(o, "    {count} units [{label}]: {explanation}");
            }
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

    o.push_str(&reuse);

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

    #[test]
    fn latest_run_is_scoped_to_workspace_and_session() {
        let dir = std::env::temp_dir().join(format!("jr-selection-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.join("project");
        let member = root.join("member");
        let sibling = dir.join("project-other");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(member.join("Cargo.toml"), "[package]\n").unwrap();
        let mut data = String::new();
        for (id, cwd, session) in [
            ("ours", &member, Some("agent-a")),
            ("other-agent", &root, Some("agent-b")),
            ("legacy", &root, None),
            ("other-workspace", &sibling, Some("agent-a")),
        ] {
            data.push_str(
                &serde_json::to_string(&IndexEntry {
                    id: id.into(),
                    cwd: cwd.to_string_lossy().into_owned(),
                    agent_session: session.map(str::to_owned),
                    ..Default::default()
                })
                .unwrap(),
            );
            data.push('\n');
        }
        data.push_str("incomplete concurrent append");
        let select = |cwd: &std::path::Path, session| {
            latest_in_context(
                &mut std::io::Cursor::new(data.as_bytes()),
                cwd,
                &selection_workspace(cwd),
                session,
            )
            .unwrap()
        };
        assert_eq!(select(&root, Some("agent-a")).as_deref(), Some("ours"));
        assert_eq!(select(&member, Some("agent-a")).as_deref(), Some("ours"));
        assert_eq!(
            select(&root, Some("agent-b")).as_deref(),
            Some("other-agent")
        );
        assert_eq!(select(&root, None).as_deref(), Some("legacy"));
        assert_eq!(select(&root, Some("missing-session")), None);
        assert_eq!(select(&dir, None), None);
        #[cfg(unix)]
        {
            let alias = dir.join("alias");
            std::os::unix::fs::symlink(&root, &alias).unwrap();
            assert_eq!(select(&alias, Some("agent-a")).as_deref(), Some("ours"));
            // A recorded path can disappear or be redirected between lookups.
            std::fs::remove_dir(&sibling).unwrap();
            assert_eq!(select(&root, None).as_deref(), Some("legacy"));
            std::os::unix::fs::symlink(&root, &sibling).unwrap();
            assert_eq!(select(&root, None).as_deref(), Some("other-workspace"));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn show_phase_coverage_explains_recorded_reasons() {
        let dir = std::env::temp_dir().join(format!("jr-phase-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut summary = Summary::default();
        summary.units.compiled = 1;
        for (coverage, expected) in [
            (
                "native-ledger-unavailable",
                "compiler capability was not detected",
            ),
            ("disabled", "phase timing explicitly disabled"),
            ("native-ledger-missing", "did not produce a readable ledger"),
            ("native-ledger-empty", "contained no usable timings"),
            ("time-passes", "time-passes requested but no usable"),
            ("native-ledger", "no aggregate phase split available"),
            ("", "older client or reconstructed unit"),
            ("future-coverage", "unrecognized recorded phase coverage"),
        ] {
            let unit = crate::shim::Unit {
                phase_coverage: coverage.into(),
                ..Default::default()
            };
            let hit = crate::shim::Unit {
                depcache: "hit".into(),
                ..Default::default()
            };
            std::fs::write(
                dir.join("units.jsonl"),
                format!(
                    "{}\n{}\n",
                    serde_json::to_string(&unit).unwrap(),
                    serde_json::to_string(&hit).unwrap()
                ),
            )
            .unwrap();
            let report = render(&summary, &dir);
            assert!(report.contains(expected), "{report}");
            assert!(!report.contains("needs a jr toolchain"), "{report}");
            assert_eq!(report.matches("1 units [").count(), 1, "{report}");
            if coverage != "native-ledger-unavailable" {
                assert!(!report.contains("JUSTRUST_PASSES=on"), "{report}");
            }
        }
        std::fs::remove_file(dir.join("units.jsonl")).unwrap();
        let report = render(&summary, &dir);
        assert!(
            report.contains("older or unavailable unit records"),
            "{report}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn show_lock_wait_uses_observed_labels_without_inventing_target_contention() {
        for labels in [
            vec![],
            vec!["package cache".to_string()],
            vec!["package cache".to_string(), "build directory".to_string()],
        ] {
            let summary = Summary {
                locks_waited: labels.clone(),
                wall: 2.0,
                phases: crate::summary::Phases {
                    lock_wait: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let report = render(
                &summary,
                std::path::Path::new("/nonexistent/lock-label-fixture"),
            );
            assert!(
                report.contains("stderr-based Cargo lock estimate"),
                "{report}"
            );
            assert!(
                !report.contains("another cargo held the build directory"),
                "{report}"
            );
            if labels.is_empty() {
                assert!(report.contains("lock kind unknown"), "{report}");
            } else {
                assert!(report.contains(&labels.join(", ")), "{report}");
            }
        }
    }

    #[test]
    fn show_does_not_score_a_killed_run_and_says_what_was_cut() {
        let missing = std::path::Path::new("/nonexistent/justrust-killed-test");
        let mut s = Summary {
            subcommand: "build".into(),
            exit: 143,
            wall: 6.0,
            ..Default::default()
        };
        s.units.cut = vec!["justrust".into()];
        let report = render(&s, missing);
        assert!(!report.contains("above theoretical optimal"), "{report}");
        assert!(
            report.contains("result   exit 143 (killed while compiling justrust)"),
            "{report}"
        );
        s.units.cut.clear();
        assert!(render(&s, missing).contains("result   exit 143 (killed)"));
        // A run that ended by itself is still held against its floor.
        s.exit = 0;
        let report = render(&s, missing);
        assert!(report.contains("above theoretical optimal"), "{report}");
        assert!(report.contains("result   exit 0\n"), "{report}");
    }

    #[test]
    fn show_reports_saved_execution_location_not_generic_remote() {
        for (place, machine, expected) in [
            (Some("hosted"), "Windows x86_64 (MSVC)", "hosted"),
            (Some("hosted"), "fleet-17", "hosted"),
            (Some("remote"), "ssh:personal", "remote"),
            (None, "hosted", "hosted"),
            (None, "ssh:hosted", "remote"),
        ] {
            let summary = Summary {
                place: place.map(str::to_owned),
                remote: Some(crate::summary::RemoteInfo {
                    machine: machine.into(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let missing = std::path::Path::new("/nonexistent/justrust-provenance-test");
            let report = render(&summary, missing);
            assert!(
                report.contains(&format!("where    {expected} on {machine}")),
                "{report}"
            );
        }
    }

    #[test]
    fn output_tail_counts_valid_matching_records_not_physical_lines() {
        use std::io::Cursor;
        let long = format!("{}\nneedle", "界".repeat(75000));
        let short = "last\nneedle";
        let record = |l: &str| serde_json::json!({"t":0,"s":"e","l":l}).to_string();
        let text = format!(
            "{}\r\n{}\n{}\nnot json\n{{\"t\":",
            record(&long),
            record("other"),
            record(short)
        );
        let mut reader = Cursor::new(text.as_bytes());
        assert_eq!(
            log_tail(&mut reader, Some("\nneedle"), 2).unwrap(),
            [long, short.into()]
        );
        let bytes = [b"\xff\n".as_slice(), text.as_bytes()].concat();
        let mut reader = Cursor::new(bytes);
        assert_eq!(log_tail(&mut reader, None, 1).unwrap(), [short]);
        assert_eq!(
            log_tail(&mut reader, None, 10).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert!(log_tail(&mut reader, None, 0).unwrap().is_empty());
    }

    fn line(id: &str) -> String {
        serde_json::to_string(&IndexEntry {
            id: id.into(),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn explicit_prefix_matches_last_valid_append() {
        let mut rows = vec![
            line("x"),
            line("exact"),
            line("exact-later"),
            line("界🦀"),
            line("escaped\"quote"),
            line("uni").replace("\"uni\"", r#""\u0075ni""#),
            format!(" {}", line("spaced")),
        ];
        rows.push(line("exact-invalid").replace("\"exit\":0", "\"exit\":false"));
        rows.push(format!("{}{}", " ".repeat(150_000), line("long")));
        rows.push(r#"{"id":"exact-torn""#.into());
        let text = rows.join("\r\n");
        for prefix in [
            "",
            "exact",
            "exact-later",
            "界",
            "escaped\"",
            "uni",
            "spaced",
            "missing",
            "long",
            "x",
        ] {
            let expected = text
                .lines()
                .rev()
                .filter_map(|l| serde_json::from_str::<IndexEntry>(l).ok())
                .find(|r| r.id.starts_with(prefix))
                .map(|r| r.id);
            assert_eq!(
                explicit_index_id(&mut std::io::Cursor::new(text.as_bytes()), prefix).unwrap(),
                expected,
                "{prefix:?}"
            );
        }
    }

    #[test]
    #[ignore = "read-only explicit ID resolution timing on retained history"]
    fn explicit_index_id_history_diagnostic() {
        let path = std::env::var_os("JUSTRUST_HISTORY_BENCH_INDEX")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| paths::index_file().unwrap());
        let full = || {
            std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .filter_map(|l| serde_json::from_str::<IndexEntry>(l).ok())
                .collect::<Vec<_>>()
        };
        let ids: Vec<_> = full().into_iter().map(|e| e.id).collect();
        for (label, prefix) in [
            ("newest", ids.last().unwrap().as_str()),
            ("middle", &ids[ids.len() / 2]),
            ("oldest", &ids[0]),
            ("ambiguous", ""),
            ("absent", "nonexistent-run-id"),
        ] {
            let mut before = Vec::new();
            let mut after = Vec::new();
            for _ in 0..5 {
                let start = std::time::Instant::now();
                let expected = full()
                    .into_iter()
                    .filter(|r| r.id.starts_with(prefix))
                    .map(|r| r.id)
                    .next_back();
                before.push(start.elapsed().as_micros());
                let start = std::time::Instant::now();
                let actual =
                    explicit_index_id(&mut std::fs::File::open(&path).unwrap(), prefix).unwrap();
                after.push(start.elapsed().as_micros());
                assert_eq!(actual, expected);
            }
            before.sort_unstable();
            after.sort_unstable();
            println!("explicit {label}: full={}us tail={}us", before[2], after[2]);
        }
    }

    #[test]
    fn recent_entries_match_full_scan_with_filters_and_block_boundaries() {
        let mut text = String::new();
        for i in 0..450 {
            let entry = IndexEntry {
                id: format!("run-{i}"),
                cwd: if i % 7 == 0 { "/wanted" } else { "/other" }.into(),
                // Include multi-block records and non-ASCII bytes at block
                // boundaries, not just the usual short ASCII index lines.
                args: vec![if i % 97 == 0 {
                    "界🦀".repeat(25_000)
                } else {
                    format!("test-{i}")
                }],
                ..Default::default()
            };
            text.push_str(&serde_json::to_string(&entry).unwrap());
            text.push_str(if i % 3 == 0 { "\r\n" } else { "\n" });
            if i % 43 == 0 {
                text.push_str("not json\n\n");
            }
        }
        text.push_str("{\"id\":\"torn");
        let full: Vec<IndexEntry> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        for only_here in [false, true] {
            for limit in [0, 1, 2, 63, 200, 1000] {
                let mut expected: Vec<_> = full
                    .iter()
                    .rev()
                    .filter(|e| !only_here || e.cwd == "/wanted")
                    .take(limit)
                    .cloned()
                    .collect();
                expected.reverse();
                let actual =
                    read_recent_entries(&mut std::io::Cursor::new(text.as_bytes()), limit, |e| {
                        !only_here || e.cwd == "/wanted"
                    })
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&actual).unwrap(),
                    serde_json::to_value(&expected).unwrap(),
                    "here={only_here}, limit={limit}"
                );
            }
        }
        // No newline at EOF, exact block boundaries, blank lines, and a
        // single record spanning several blocks must all retain valid rows.
        for padding in [0, 1, 65_535, 65_536, 65_537] {
            let text = "\n".repeat(padding) + &line("only");
            let result =
                read_recent_entries(&mut std::io::Cursor::new(text.as_bytes()), 1, |_| true)
                    .unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].id, "only");
        }
        let empty = read_recent_entries(&mut std::io::Cursor::new([]), 10, |_| true).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn recent_entries_read_only_needed_blocks_and_classify_newest_first() {
        struct Counted {
            cursor: std::io::Cursor<Vec<u8>>,
            bytes: usize,
        }
        impl std::io::Read for Counted {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.cursor.read(out)?;
                self.bytes += n;
                Ok(n)
            }
        }
        impl std::io::Seek for Counted {
            fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
                self.cursor.seek(to)
            }
        }
        let text: String = (0..3000)
            .map(|i| line(&format!("run-{i}")) + "\n")
            .collect();
        assert!(text.len() > 64 * 1024);
        let mut reader = Counted {
            cursor: std::io::Cursor::new(text.into_bytes()),
            bytes: 0,
        };
        let zero =
            read_recent_entries(&mut reader, 0, |_| panic!("zero limit parsed a row")).unwrap();
        assert!(zero.is_empty());
        assert_eq!(reader.bytes, 0);
        let mut visited = Vec::new();
        let result = read_recent_entries(&mut reader, 2, |e| {
            visited.push(e.id.clone());
            // An expensive filter can annotate selected rows, as the failure
            // classifier does, without revisiting them as blocks are read.
            e.failure = Some(crate::failure::Class::Infra);
            e.id.ends_with('8') || e.id.ends_with('6')
        })
        .unwrap();
        assert_eq!(visited, ["run-2999", "run-2998", "run-2997", "run-2996"]);
        assert_eq!(result[0].id, "run-2996");
        assert_eq!(result[1].id, "run-2998");
        assert!(
            result
                .iter()
                .all(|e| e.failure == Some(crate::failure::Class::Infra))
        );
        assert_eq!(reader.bytes, 64 * 1024);

        reader.bytes = 0;
        let mut visited = 0;
        let none = read_recent_entries(&mut reader, 1, |_| {
            visited += 1;
            false
        })
        .unwrap();
        assert!(none.is_empty());
        assert_eq!(visited, 3000);
        assert_eq!(reader.bytes, reader.cursor.get_ref().len());
    }

    /// Focused retained-history diagnostic, not a compiler speed benchmark.
    #[test]
    #[ignore = "read-only full-versus-bounded history CPU diagnostic"]
    fn recent_entries_history_diagnostic() {
        let path = std::env::var_os("JUSTRUST_HISTORY_BENCH_INDEX")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| paths::index_file().unwrap());
        let mut full_samples = Vec::new();
        let mut tail_samples = Vec::new();
        for _ in 0..7 {
            let start = std::time::Instant::now();
            let text = std::fs::read_to_string(&path).unwrap();
            let mut full: Vec<IndexEntry> = text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            full.drain(..full.len().saturating_sub(200));
            full_samples.push(start.elapsed().as_nanos());
            let mut file = std::fs::File::open(&path).unwrap();
            let start = std::time::Instant::now();
            let recent = read_recent_entries(&mut file, 200, |_| true).unwrap();
            tail_samples.push(start.elapsed().as_nanos());
            assert_eq!(
                serde_json::to_value(&recent).unwrap(),
                serde_json::to_value(&full).unwrap()
            );
        }
        full_samples.sort_unstable();
        tail_samples.sort_unstable();
        println!(
            "history bytes={} limit=200 full_median_ns={} bounded_median_ns={} samples=7",
            std::fs::metadata(path).unwrap().len(),
            full_samples[3],
            tail_samples[3]
        );
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
