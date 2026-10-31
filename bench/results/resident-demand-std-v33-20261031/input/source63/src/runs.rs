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

pub fn list(limit: usize, here: bool, json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    let mut runs: Vec<IndexEntry> = load_index()
        .into_iter()
        .filter(|r| !here || r.cwd.starts_with(&cwd))
        .collect();
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
        "{:<26} {:>8} {:>8} {:>8} {:>6} {:>5}  {:<10} COMMAND",
        "ID", "WALL", "COMPILE", "TESTS", "UNITS", "EXIT", "WHEN"
    );
    for r in &runs {
        println!(
            "{:<26} {:>7.1}s {:>7.1}s {:>7.1}s {:>6} {:>5}  {:<10} cargo {}",
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
    let s: Summary = serde_json::from_slice(&raw)?;
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
    let status = if s.diagnostics.compile_failed {
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
    let _ = writeln!(
        o,
        "time     {:.2}s wall, {:.1}s CPU ({:.1} of {} cores on average)\n",
        s.wall, s.cpu_secs, s.resources.avg_build_cores, s.ncpu
    );

    let _ = writeln!(o, "Where the wall time went");
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
                let _ = writeln!(o, "    {:<44} {:>7.2}s", pass.name, pass.secs);
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
    if !t.binaries.is_empty() {
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
    let _ = writeln!(
        o,
        "  stalls  cpu {:.2}s, memory {:.2}s, io {:.2}s (Linux pressure stall info, machine-wide)",
        r.psi_cpu_secs, r.psi_mem_secs, r.psi_io_secs
    );
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
