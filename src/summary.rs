//! Turn a recorded run into a breakdown of where the time went.

use crate::record::{GitInfo, Meta, OutputLine, ProcRecord, Sample};
use crate::shim::{Pass, Unit};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Summary {
    pub id: String,
    pub args: Vec<String>,
    pub subcommand: String,
    pub cwd: String,
    pub git: GitInfo,
    pub agent_session: Option<String>,
    pub start: f64,
    pub wall: f64,
    pub exit: i32,
    /// CPU seconds used by cargo and everything it ran.
    pub cpu_secs: f64,
    pub ncpu: usize,
    pub phases: Phases,
    pub units: UnitStats,
    pub top_units: Vec<UnitBreakdown>,
    pub link: LinkStats,
    pub tests: TestStats,
    pub resources: Resources,
    pub diagnostics: Diagnostics,
    /// Why cargo rebuilt each unit, from its `Dirty` lines.
    #[serde(default)]
    pub rebuild_reasons: Vec<RebuildReason>,
    /// Every unit that ran, newest-first by share (`top_units` is the first 12).
    #[serde(default)]
    pub all_units: usize,
    /// Per-agent build slot (private target dir) used for this run.
    #[serde(default)]
    pub slot: Option<crate::slots::SlotInfo>,
    /// Which cargo locks the run waited on ("build directory", "package cache", ...).
    #[serde(default)]
    pub locks_waited: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RebuildReason {
    pub package: String,
    pub reason: String,
}

/// Non-overlapping split of wall time. Sums to `wall`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Phases {
    /// Cargo resolving, reading fingerprints, and planning before any compiler ran.
    pub startup: f64,
    /// Waiting on another cargo for the build directory lock.
    pub lock_wait: f64,
    /// At least one rustc was running.
    pub compile: f64,
    /// Inside the build, but no rustc was running (build scripts, scheduling gaps).
    pub build_gaps: f64,
    /// Running test binaries (after the build).
    pub test_run: f64,
    /// Everything else after the build (doc-tests setup, cargo shutdown, `run`).
    pub tail: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct UnitStats {
    /// rustc invocations that actually ran.
    pub compiled: usize,
    pub local: usize,
    pub dependencies: usize,
    pub build_scripts: usize,
    /// Units cargo considered up to date (only visible with `-v`).
    pub fresh: usize,
    pub rustc_cpu_secs: f64,
    /// Sum of unit wall times (exceeds compile phase when units overlap).
    pub rustc_wall_sum: f64,
    pub with_pass_timings: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct UnitBreakdown {
    pub name: String,
    pub kind: String,
    pub local: bool,
    pub start: f64,
    pub wall: f64,
    /// Share of the run's wall time this unit was responsible for. Time where
    /// several units overlap is divided between them.
    pub wall_share: f64,
    pub cpu: f64,
    pub max_rss_mb: f64,
    pub frontend_threads: Option<u32>,
    pub rmeta_secs: Option<f64>,
    /// Present when rustc pass timings were captured.
    pub split: Option<Split>,
    pub top_passes: Vec<Pass>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Split {
    /// Parsing, macro expansion, name resolution, type and borrow checking, lints.
    pub frontend: f64,
    /// Monomorphization, LLVM IR generation, and waiting on LLVM optimization.
    pub codegen: f64,
    pub link: f64,
    /// Loading and saving the incremental cache.
    pub incremental: f64,
    pub other: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LinkStats {
    pub linker_processes: usize,
    pub linker_cpu_secs: f64,
    /// Sum of `link` pass time across units with pass timings.
    pub link_pass_secs: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TestStats {
    pub binaries: Vec<TestBinary>,
    pub passed: u64,
    pub failed: u64,
    pub ignored: u64,
    pub filtered_out: u64,
    /// libtest's own "finished in" total.
    pub libtest_secs: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TestBinary {
    pub name: String,
    pub start: f64,
    pub wall: f64,
    pub passed: u64,
    pub failed: u64,
    pub filtered_out: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Resources {
    pub avg_build_cores: f64,
    pub peak_build_cores: f64,
    pub peak_build_rss_mb: f64,
    pub peak_unit_rss_mb: f64,
    pub min_mem_available_mb: u64,
    /// Average cores used by other processes on the machine during the run.
    pub avg_other_cores: f64,
    pub max_foreign_rustc: usize,
    /// Seconds where some task stalled waiting for CPU, memory, or IO.
    pub psi_cpu_secs: f64,
    pub psi_mem_secs: f64,
    pub psi_io_secs: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Diagnostics {
    pub compile_errors: usize,
    pub warnings: usize,
    pub first_error: Option<String>,
    /// The build failed to compile (as opposed to tests failing).
    pub compile_failed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct IndexEntry {
    pub id: String,
    pub start: f64,
    pub cwd: String,
    pub args: Vec<String>,
    pub exit: i32,
    pub wall: f64,
    pub cpu_secs: f64,
    pub compile: f64,
    pub test_run: f64,
    pub units_compiled: usize,
    pub top_unit: Option<String>,
    pub compile_failed: bool,
    pub agent_session: Option<String>,
}

pub fn load_units(run_dir: &Path) -> Vec<Unit> {
    let Ok(s) = std::fs::read_to_string(run_dir.join("units.jsonl")) else {
        return Vec::new();
    };
    s.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Map rustc `-Ztime-passes` output onto frontend/codegen/link/incremental.
///
/// Passes nest, so only passes that run directly on the main thread and do not
/// contain each other are summed. LLVM optimization runs on worker threads in
/// parallel with `codegen_crate`. The main thread's wait for it shows up as
/// `finish_ongoing_codegen`.
pub fn split_passes(passes: &[Pass]) -> Option<Split> {
    let get = |n: &str| {
        passes
            .iter()
            .filter(|p| p.name == n)
            .map(|p| p.secs)
            .sum::<f64>()
    };
    let total = get("total");
    if total <= 0.0 {
        return None;
    }
    let codegen = get("codegen_crate") + get("finish_ongoing_codegen");
    let link = get("link");
    let incremental = passes
        .iter()
        .filter(|p| {
            matches!(
                p.name.as_str(),
                "incr_comp_prepare_session_directory"
                    // Wraps incr_comp_persist_dep_graph and
                    // incr_comp_persist_result_cache, so those are not added.
                    | "serialize_dep_graph"
                    | "incr_comp_garbage_collect_session_directories"
                    | "incr_comp_finalize_session_directory"
                    | "load_dep_graph"
                    | "serialize_work_products"
                    | "copy_all_cgu_workproducts_to_incr_comp_cache_dir"
            )
        })
        .map(|p| p.secs)
        .sum::<f64>();
    let frontend_parts = [
        "parse_crate",
        "expand_crate",
        "resolve_crate",
        "AST_validation",
        "misc_checking_1",
        "coherence_checking",
        "type_check_crate",
        "MIR_borrow_checking",
        "MIR_effect_checking",
        "lint_checking",
        "privacy_checking_modules",
        "misc_checking_3",
        "generate_crate_metadata",
        "drop_ast",
        "maybe_building_test_harness",
    ];
    let frontend: f64 = frontend_parts.iter().map(|n| get(n)).sum();
    let other = (total - frontend - codegen - link - incremental).max(0.0);
    Some(Split {
        frontend,
        codegen,
        link,
        incremental,
        other,
    })
}

fn unit_kind(u: &Unit) -> String {
    if u.build_script {
        return "build-script".into();
    }
    if u.test {
        return "test".into();
    }
    if u.crate_types.iter().any(|t| t == "bin") {
        return "bin".into();
    }
    if u.crate_types.iter().any(|t| t == "proc-macro") {
        return "proc-macro".into();
    }
    if !u.emit.contains("link") {
        return "check".into();
    }
    "lib".into()
}

/// Split each moment of `[lo, hi]` evenly between the intervals covering it.
/// Returns (per-interval share, total time covered by at least one interval).
fn share_time(intervals: &[(f64, f64)], lo: f64, hi: f64) -> (Vec<f64>, f64) {
    let mut events: Vec<(f64, i32, usize)> = Vec::new();
    for (i, &(s, e)) in intervals.iter().enumerate() {
        let s = s.clamp(lo, hi);
        let e = e.clamp(lo, hi);
        if e > s {
            events.push((s, 1, i));
            events.push((e, -1, i));
        }
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut active: Vec<usize> = Vec::new();
    let mut share = vec![0.0; intervals.len()];
    let mut covered = 0.0;
    let mut last = lo;
    for (t, kind, i) in events {
        if !active.is_empty() && t > last {
            let each = (t - last) / active.len() as f64;
            for &a in &active {
                share[a] += each;
            }
            covered += t - last;
        }
        last = t;
        if kind > 0 {
            active.push(i);
        } else if let Some(pos) = active.iter().position(|&a| a == i) {
            active.swap_remove(pos);
        }
    }
    (share, covered)
}

fn parse_test_result(l: &str) -> Option<(u64, u64, u64, u64, f64)> {
    let rest = l.trim().strip_prefix("test result: ")?;
    let num = |key: &str| -> u64 {
        rest.split(';')
            .find_map(|part| part.trim().strip_suffix(key).map(|n| n.trim()))
            .and_then(|n| n.rsplit(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    let secs = rest
        .rsplit("finished in ")
        .next()
        .and_then(|s| s.trim().strip_suffix('s'))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    Some((
        num(" passed"),
        num(" failed"),
        num(" ignored"),
        num(" filtered out"),
        secs,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    meta: &Meta,
    git: GitInfo,
    start: f64,
    end: f64,
    exit: i32,
    cpu_secs: f64,
    lines: &[OutputLine],
    samples: &[Sample],
    procs: &[ProcRecord],
    units: &[Unit],
) -> Summary {
    let wall = (end - start).max(0.0);
    let rel = |t: f64| (t - start).clamp(0.0, wall);

    // Units and compile window.
    let intervals: Vec<(f64, f64)> = units.iter().map(|u| (rel(u.start), rel(u.end))).collect();
    let first_unit = intervals.iter().map(|i| i.0).fold(f64::INFINITY, f64::min);
    let last_unit = intervals.iter().map(|i| i.1).fold(0.0, f64::max);
    let (shares, compile_covered) = share_time(&intervals, 0.0, wall);

    // Lock waits: from "Blocking waiting for file lock" to the next output line.
    // Consecutive Blocking lines are one wait.
    let mut lock_wait = 0.0;
    let mut locks_waited: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some(what) = l.l.split("Blocking waiting for file lock on ").nth(1) {
            let what = what.trim().to_owned();
            if !locks_waited.contains(&what) {
                locks_waited.push(what);
            }
            if i > 0 && lines[i - 1].l.contains("Blocking waiting for file lock") {
                continue;
            }
            let next = lines[i + 1..]
                .iter()
                .find(|n| !n.l.contains("Blocking"))
                .map(|n| n.t)
                .unwrap_or(end);
            lock_wait += (next - l.t).max(0.0);
        }
    }

    // Test binaries: "Running <path>" until the next Running/Doc-tests or end.
    let mut tests = TestStats::default();
    let mut current: Option<TestBinary> = None;
    let close = |b: Option<TestBinary>, t: f64, tests: &mut TestStats| {
        if let Some(mut b) = b {
            b.wall = (rel(t) - b.start).max(0.0);
            tests.binaries.push(b);
        }
    };
    let mut first_test_line: Option<f64> = None;
    for l in lines {
        let text = l.l.trim();
        // Test binaries appear as `Running unittests src/lib.rs (target/.../x-hash)`
        // or, with verbose output, as "Running `/.../deps/x-hash args`". Verbose
        // cargo also prints "Running `rustc ...`" for compiler invocations.
        let running_test = text.strip_prefix("Running ").and_then(|r| {
            if let Some(cmd) = r.strip_prefix('`') {
                let exe = cmd.trim_end_matches('`').split_whitespace().next()?;
                let is_test =
                    exe.contains("/deps/") && !exe.ends_with("rustc") && !exe.contains("rustdoc");
                is_test.then_some(exe)
            } else {
                r.ends_with(')').then_some(r)
            }
        });
        if let Some(rest) = running_test.or_else(|| text.strip_prefix("Doc-tests ")) {
            close(current.take(), l.t, &mut tests);
            first_test_line.get_or_insert(rel(l.t));
            let name = rest
                .rsplit('(')
                .next()
                .map(|s| s.trim_end_matches(')'))
                .and_then(|p| p.rsplit('/').next())
                .unwrap_or(rest)
                .to_owned();
            let name = if text.starts_with("Doc-tests") {
                format!("doctest {rest}")
            } else {
                name
            };
            current = Some(TestBinary {
                name,
                start: rel(l.t),
                ..Default::default()
            });
        } else if let Some((p, f, ig, fo, secs)) = parse_test_result(text) {
            tests.passed += p;
            tests.failed += f;
            tests.ignored += ig;
            tests.filtered_out += fo;
            tests.libtest_secs += secs;
            if let Some(b) = current.as_mut() {
                b.passed += p;
                b.failed += f;
                b.filtered_out += fo;
                let t = l.t;
                close(current.take(), t, &mut tests);
            }
        }
    }
    close(current.take(), end, &mut tests);

    // Phases.
    let build_end = if units.is_empty() {
        first_test_line.unwrap_or_else(|| {
            lines
                .iter()
                .find(|l| l.l.trim_start().starts_with("Finished"))
                .map(|l| rel(l.t))
                .unwrap_or(wall)
        })
    } else {
        last_unit.max(
            lines
                .iter()
                .find(|l| l.l.trim_start().starts_with("Finished"))
                .map(|l| rel(l.t))
                .unwrap_or(last_unit),
        )
    };
    let startup = if units.is_empty() {
        build_end
    } else {
        first_unit.min(build_end)
    };
    let startup_lock = lock_wait.min(startup);
    let test_run: f64 = tests
        .binaries
        .iter()
        .map(|b| b.wall)
        .sum::<f64>()
        .min((wall - build_end).max(0.0))
        .max(0.0);
    let build_window = (build_end - startup).max(0.0);
    let compile = compile_covered.min(build_window);
    let phases = Phases {
        startup: startup - startup_lock,
        lock_wait: startup_lock,
        compile,
        build_gaps: (build_window - compile).max(0.0),
        test_run,
        tail: (wall - build_end - test_run).max(0.0),
    };

    // Units.
    let mut stats = UnitStats {
        compiled: units.len(),
        local: units.iter().filter(|u| u.local && !u.build_script).count(),
        dependencies: units.iter().filter(|u| !u.local && !u.build_script).count(),
        build_scripts: units.iter().filter(|u| u.build_script).count(),
        fresh: lines
            .iter()
            .filter(|l| l.l.trim_start().starts_with("Fresh "))
            .count(),
        rustc_cpu_secs: units.iter().map(|u| u.user + u.sys).sum(),
        rustc_wall_sum: units.iter().map(|u| u.wall).sum(),
        with_pass_timings: units.iter().filter(|u| !u.passes.is_empty()).count(),
    };
    stats.rustc_cpu_secs = (stats.rustc_cpu_secs * 100.0).round() / 100.0;

    let mut top: Vec<UnitBreakdown> = units
        .iter()
        .zip(&shares)
        .map(|(u, &share)| {
            let mut passes: Vec<Pass> = u
                .passes
                .iter()
                .filter(|p| {
                    !matches!(
                        p.name.as_str(),
                        "total" | "link" | "link_crate" | "link_binary"
                    )
                })
                .cloned()
                .collect();
            passes.sort_by(|a, b| b.secs.total_cmp(&a.secs));
            passes.truncate(8);
            UnitBreakdown {
                name: if u.test {
                    format!("{} (test)", u.crate_name)
                } else {
                    u.crate_name.clone()
                },
                kind: unit_kind(u),
                local: u.local,
                start: rel(u.start),
                wall: u.wall,
                wall_share: share,
                cpu: u.user + u.sys,
                max_rss_mb: u.max_rss_mb,
                frontend_threads: u.frontend_threads,
                rmeta_secs: u.rmeta_secs,
                split: split_passes(&u.passes),
                top_passes: passes,
            }
        })
        .collect();
    top.sort_by(|a, b| b.wall_share.total_cmp(&a.wall_share));
    top.truncate(12);

    let link = LinkStats {
        linker_processes: procs.iter().filter(|p| p.kind == "linker").count(),
        linker_cpu_secs: procs
            .iter()
            .filter(|p| p.kind == "linker")
            .map(|p| p.cpu_secs)
            .sum(),
        link_pass_secs: units
            .iter()
            .flat_map(|u| &u.passes)
            .filter(|p| p.name == "link")
            .map(|p| p.secs)
            .sum::<f64>()
            + 0.0,
    };

    // Resources. Skip the baseline sample.
    let s = if samples.len() > 1 {
        &samples[1..]
    } else {
        samples
    };
    let n = s.len().max(1) as f64;
    let resources = Resources {
        avg_build_cores: if wall > 0.0 { cpu_secs / wall } else { 0.0 },
        peak_build_cores: s.iter().map(|x| x.build_cores).fold(0.0, f64::max),
        peak_build_rss_mb: s.iter().map(|x| x.build_rss_mb).fold(0.0, f64::max),
        peak_unit_rss_mb: units.iter().map(|u| u.max_rss_mb).fold(0.0, f64::max),
        min_mem_available_mb: s
            .iter()
            .map(|x| x.mem_available_mb)
            .filter(|&m| m > 0)
            .min()
            .unwrap_or(0),
        avg_other_cores: s
            .iter()
            .map(|x| (x.sys_cores - x.build_cores).max(0.0))
            .sum::<f64>()
            / n,
        max_foreign_rustc: s.iter().map(|x| x.foreign_rustc_procs).max().unwrap_or(0),
        psi_cpu_secs: s.iter().map(|x| x.psi_cpu_ms).sum::<f64>() / 1000.0,
        psi_mem_secs: s.iter().map(|x| x.psi_mem_ms).sum::<f64>() / 1000.0,
        psi_io_secs: s.iter().map(|x| x.psi_io_ms).sum::<f64>() / 1000.0,
    };

    // Diagnostics. "error: test failed" and "error: N target failed" are test
    // failures, not compile errors.
    let mut diagnostics = Diagnostics::default();
    for l in lines {
        let t = l.l.trim_start();
        if t.starts_with("error[E") || (t.starts_with("error:") && is_compile_error(t)) {
            diagnostics.compile_errors += 1;
            if diagnostics.first_error.is_none() {
                diagnostics.first_error = Some(t.chars().take(200).collect());
            }
        } else if t.starts_with("warning:")
            && !t.contains("generated")
            && !t.starts_with("warning: unused manifest key")
        {
            diagnostics.warnings += 1;
        }
    }
    diagnostics.compile_failed = units.iter().any(|u| u.exit != 0)
        || lines.iter().any(|l| l.l.contains("could not compile"));

    let rebuild_reasons = lines
        .iter()
        .filter_map(|l| parse_dirty(&l.l))
        .collect::<Vec<_>>();

    Summary {
        id: meta.id.clone(),
        args: meta.args.clone(),
        subcommand: meta.subcommand.clone(),
        cwd: meta.cwd.clone(),
        git,
        agent_session: meta.agent_session.clone(),
        start,
        wall,
        exit,
        cpu_secs,
        ncpu: meta.ncpu,
        phases,
        units: stats,
        top_units: top,
        link,
        tests,
        resources,
        diagnostics,
        rebuild_reasons,
        all_units: units.len(),
        slot: meta.slot.clone(),
        locks_waited,
    }
}

/// Parse cargo's `Dirty <pkg> v<ver> (...): <reason>` line.
fn parse_dirty(line: &str) -> Option<RebuildReason> {
    let rest = line.trim_start().strip_prefix("Dirty ")?;
    let package = rest.split_whitespace().next()?.to_owned();
    let (_, reason) = rest.split_once(": ")?;
    // Drop the trailing "(1791435000.83s, 58ms after last build at ...)" detail.
    let reason = match reason.find(" (") {
        Some(i) if reason[i..].contains("after last build") => &reason[..i],
        _ => reason,
    };
    Some(RebuildReason {
        package,
        reason: reason.to_owned(),
    })
}

fn is_compile_error(t: &str) -> bool {
    !(t.starts_with("error: test failed")
        || t.contains("target failed")
        || t.contains("targets failed")
        || t.starts_with("error: could not compile")
        || t.starts_with("error: aborting due to")
        || t.starts_with("error: process didn't exit successfully")
        || t.starts_with("error: no test target")
        || t.starts_with("error: Recipe"))
}

impl Summary {
    pub fn index_entry(&self) -> IndexEntry {
        IndexEntry {
            id: self.id.clone(),
            start: self.start,
            cwd: self.cwd.clone(),
            args: self.args.clone(),
            exit: self.exit,
            wall: self.wall,
            cpu_secs: self.cpu_secs,
            compile: self.phases.compile,
            test_run: self.phases.test_run,
            units_compiled: self.units.compiled,
            top_unit: self.top_units.first().map(|u| u.name.clone()),
            compile_failed: self.diagnostics.compile_failed,
            agent_session: self.agent_session.clone(),
        }
    }

    /// Compact status block printed after agent-mode runs.
    pub fn agent_footer(&self, hidden: &crate::agent_output::Hidden) -> String {
        let mut out = String::from("\n");
        let verdict = if self.diagnostics.compile_failed {
            format!(
                "FAILED to compile ({} error{})",
                self.diagnostics.compile_errors,
                if self.diagnostics.compile_errors == 1 {
                    ""
                } else {
                    "s"
                }
            )
        } else if self.tests.failed > 0 {
            format!(
                "FAILED: {} of {} tests failed",
                self.tests.failed,
                self.tests.passed + self.tests.failed
            )
        } else if self.exit != 0 {
            format!("FAILED (exit {})", self.exit)
        } else if !self.tests.binaries.is_empty() {
            let mut v = format!(
                "ok: {} test{} passed",
                self.tests.passed,
                if self.tests.passed == 1 { "" } else { "s" }
            );
            if self.tests.passed == 0 && self.tests.filtered_out > 0 {
                v = "ok, but 0 tests matched the filter".to_owned();
            }
            v
        } else {
            "ok".to_owned()
        };
        out.push_str(&self.agent_report(&verdict, hidden));
        out
    }

    /// The detailed report shown after every agent-mode run: verdict, the full
    /// wall-time breakdown, the slowest units with their compiler phases, test
    /// and resource figures, and every detected inefficiency with its cost.
    /// Bounded to roughly 20 lines. Set `JUSTRUST_REPORT=brief` for 2 lines.
    fn agent_report(&self, verdict: &str, hidden: &crate::agent_output::Hidden) -> String {
        use std::fmt::Write as _;
        let mut o = String::new();
        let findings = crate::findings::analyze(self);
        let _ = writeln!(o, "justrust: {verdict} in {:.1}s", self.wall);
        let brief = std::env::var("JUSTRUST_REPORT").as_deref() == Ok("brief");
        if brief {
            if let Some(f) = findings.first() {
                let _ = writeln!(
                    o,
                    "justrust: slowest issue: {} (~{:.1}s)",
                    f.message, f.cost_secs
                );
            }
            let _ = writeln!(o, "justrust: details `justrust show {}`", self.id);
            return o;
        }

        // Wall-time breakdown, always complete.
        let p = &self.phases;
        let mut parts = Vec::new();
        for (label, v) in [
            ("startup", p.startup),
            ("lock wait", p.lock_wait),
            ("compile", p.compile),
            ("gaps", p.build_gaps),
            ("tests", p.test_run),
            ("after", p.tail),
        ] {
            if v >= 0.01 || label == "compile" {
                parts.push(format!("{label} {v:.2}s"));
            }
        }
        let _ = writeln!(o, "  time    {}", parts.join(" | "));
        if let Some(slot) = &self.slot {
            let _ = writeln!(o, "  target  {}", slot.report_line());
        }

        // Units.
        let u = &self.units;
        let reasons = self.rebuild_summary();
        let _ = writeln!(
            o,
            "  units   {} compiled ({} local, {} deps, {} build scripts), {} fresh{}",
            u.compiled,
            u.local,
            u.dependencies,
            u.build_scripts,
            u.fresh,
            reasons
                .map(|r| format!(". Rebuilt because: {r}"))
                .unwrap_or_default()
        );
        for t in self
            .top_units
            .iter()
            .filter(|t| t.wall_share >= 0.1)
            .take(4)
        {
            let mut line = format!(
                "          {:<32} {:>6.2}s  cpu {:>5.1}s  {:>5.0} MB",
                trunc(&t.name, 32),
                t.wall_share,
                t.cpu,
                t.max_rss_mb
            );
            if let Some(sp) = &t.split {
                line.push_str(&format!(
                    "  frontend {:.2} codegen {:.2} link {:.2} incr {:.2}",
                    sp.frontend, sp.codegen, sp.link, sp.incremental
                ));
                if let Some(pass) = t.top_passes.first() {
                    line.push_str(&format!("  top {} {:.2}s", pass.name, pass.secs));
                }
            }
            let _ = writeln!(o, "{line}");
        }

        // Tests.
        let t = &self.tests;
        if !t.binaries.is_empty() {
            let _ = writeln!(
                o,
                "  tests   {} passed, {} failed, {} ignored, {} filtered out, {} binar{} in {:.2}s",
                t.passed,
                t.failed,
                t.ignored,
                t.filtered_out,
                t.binaries.len(),
                if t.binaries.len() == 1 { "y" } else { "ies" },
                p.test_run
            );
        }

        // Resources.
        let r = &self.resources;
        let _ = writeln!(
            o,
            "  cpu     {:.1}s total, avg {:.1}/peak {:.1} of {} cores; others used {:.1} cores{}",
            self.cpu_secs,
            r.avg_build_cores,
            r.peak_build_cores,
            self.ncpu,
            r.avg_other_cores,
            if r.max_foreign_rustc > 0 {
                format!(" ({} other rustc)", r.max_foreign_rustc)
            } else {
                String::new()
            }
        );
        let _ = writeln!(
            o,
            "  memory  peak {:.0} MB build, {:.0} MB largest rustc, {:.1} GB free; stalls cpu {:.2}s mem {:.2}s io {:.2}s",
            r.peak_build_rss_mb,
            r.peak_unit_rss_mb,
            r.min_mem_available_mb as f64 / 1024.0,
            r.psi_cpu_secs,
            r.psi_mem_secs,
            r.psi_io_secs
        );
        if hidden.warnings_hidden > 0 || hidden.warnings_shown > 0 {
            let _ = writeln!(
                o,
                "  output  {} warnings shown, {} hidden; {} progress and {} passing-test lines hidden",
                hidden.warnings_shown,
                hidden.warnings_hidden,
                hidden.progress_lines,
                hidden.passing_test_lines
            );
        }

        // Inefficiencies.
        if findings.is_empty() {
            let _ = writeln!(o, "  waste   none detected");
        } else {
            let _ = writeln!(
                o,
                "  waste   {} issue{} (estimated cost each; they can overlap):",
                findings.len(),
                if findings.len() == 1 { "" } else { "s" }
            );
            for f in findings.iter().take(6) {
                let _ = writeln!(o, "          ~{:>5.1}s  {}", f.cost_secs, f.message);
            }
            if findings.len() > 6 {
                let _ = writeln!(
                    o,
                    "          ... {} more in `justrust show`",
                    findings.len() - 6
                );
            }
        }
        let _ = writeln!(
            o,
            "  more    `justrust log {id}` full output, `justrust show {id}` full profile",
            id = self.id
        );
        o
    }

    /// Short summary of why units were rebuilt, grouped by reason.
    fn rebuild_summary(&self) -> Option<String> {
        if self.rebuild_reasons.is_empty() {
            return None;
        }
        let mut counts: std::collections::BTreeMap<String, Vec<&str>> = Default::default();
        for r in &self.rebuild_reasons {
            let key = if r.reason.starts_with("the dependency ") {
                r.reason
                    .split('`')
                    .nth(1)
                    .map(|d| format!("{d} changed"))
                    .unwrap_or_else(|| r.reason.clone())
            } else if r.reason.starts_with("the file ") {
                r.reason
                    .split('`')
                    .nth(1)
                    .map(|f| format!("edited {f}"))
                    .unwrap_or_else(|| r.reason.clone())
            } else {
                r.reason.chars().take(60).collect()
            };
            counts.entry(key).or_default().push(&r.package);
        }
        let mut v: Vec<_> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        Some(
            v.iter()
                .take(3)
                .map(|(reason, pkgs)| {
                    if pkgs.len() == 1 {
                        format!("{} ({})", reason, pkgs[0])
                    } else {
                        format!("{} ({} crates)", reason, pkgs.len())
                    }
                })
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    /// One-line footer printed after every recorded run.
    pub fn one_line(&self) -> String {
        let p = &self.phases;
        let mut parts = vec![format!("{:.1}s", self.wall)];
        let mut phase = |label: &str, v: f64| {
            if v >= 0.05 {
                parts.push(format!("{label} {v:.1}s"));
            }
        };
        phase("startup", p.startup);
        phase("lock", p.lock_wait);
        phase("compile", p.compile);
        phase("gaps", p.build_gaps);
        phase("tests", p.test_run);
        phase("tail", p.tail);
        let mut s = parts.join(" · ");
        if let Some(u) = self.top_units.first().filter(|u| u.wall_share >= 0.05) {
            s.push_str(&format!(" | top {} {:.1}s", u.name, u.wall_share));
            if let Some(sp) = &u.split {
                s.push_str(&format!(
                    " (frontend {:.1} codegen {:.1} link {:.1})",
                    sp.frontend, sp.codegen, sp.link
                ));
            }
        }
        s.push_str(&format!(
            " | cpu {:.0}s ({:.1} cores) | `justrust show {}`",
            self.cpu_secs, self.resources.avg_build_cores, self.id
        ));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_overlapping_time() {
        let (share, covered) = share_time(&[(0.0, 4.0), (2.0, 6.0)], 0.0, 10.0);
        assert_eq!(covered, 6.0);
        assert_eq!(share, vec![3.0, 3.0]);
    }

    #[test]
    fn splits_passes_without_double_counting() {
        let p = |n: &str, s: f64| Pass {
            name: n.into(),
            secs: s,
            rss_end_mb: 0.0,
        };
        let passes = vec![
            p("type_check_crate", 2.0),
            p("MIR_borrow_checking", 1.0),
            p("codegen_crate", 3.0),
            p("codegen_to_LLVM_IR", 2.5),
            p("LLVM_passes", 4.0),
            p("finish_ongoing_codegen", 1.0),
            p("run_linker", 0.9),
            p("link", 1.0),
            p("serialize_dep_graph", 0.5),
            p("incr_comp_persist_dep_graph", 0.3),
            p("total", 9.0),
        ];
        let s = split_passes(&passes).unwrap();
        assert_eq!(s.frontend, 3.0);
        assert_eq!(s.codegen, 4.0);
        assert_eq!(s.link, 1.0);
        assert_eq!(s.incremental, 0.5);
        assert_eq!(s.other, 0.5);
    }

    #[test]
    fn parses_libtest_results() {
        let r = parse_test_result(
            "test result: ok. 12 passed; 1 failed; 2 ignored; 0 measured; 900 filtered out; finished in 0.31s",
        )
        .unwrap();
        assert_eq!(r, (12, 1, 2, 900, 0.31));
    }

    #[test]
    fn test_failures_are_not_compile_errors() {
        assert!(!is_compile_error(
            "error: test failed, to rerun pass `--lib`"
        ));
        assert!(!is_compile_error(
            "error: could not compile `x` (lib) due to 2 previous errors"
        ));
        assert!(is_compile_error(
            "error: cannot find value `x` in this scope"
        ));
    }
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
