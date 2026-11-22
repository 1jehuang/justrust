//! First-principles theoretical optimal wall-time floor for a Cargo run.
//!
//! Computes an irreducible lower bound from what *must* happen given the
//! workspace delta, command kind, and test execution, independent of how
//! rustc and Cargo work today. Third-party dependencies contribute 0s
//! (in a theoretically optimal world, immutable registry crates are always
//! prebuilt and available).

use crate::record::GitInfo;
use crate::summary::{Phases, TestStats, UnitStats};
use serde::{Deserialize, Serialize};

/// Irreducible stat/hash check and dispatch overhead (5 ms).
const OVERHEAD_SECS: f64 = 0.005;
/// Theoretical optimal frontend throughput (parse + typecheck + borrowcheck): 1M LOC/s.
const FRONTEND_LOC_PER_SEC: f64 = 1_000_000.0;
/// Theoretical optimal debug codegen throughput (single-pass / Cranelift-class): 500k LOC/s.
const CODEGEN_DEBUG_LOC_PER_SEC: f64 = 500_000.0;
/// Theoretical optimal release codegen throughput (optimizing backend): 100k LOC/s.
const CODEGEN_RELEASE_LOC_PER_SEC: f64 = 100_000.0;
/// Theoretical incremental/patched link time (5 ms).
const LINK_INCREMENTAL_SECS: f64 = 0.005;
/// Theoretical full link time at memory bandwidth (20 ms).
const LINK_COLD_SECS: f64 = 0.020;
/// Fallback workspace LOC per compiled local unit on a cold build.
const COLD_LOC_PER_LOCAL_UNIT: usize = 500;
/// Fallback dirty LOC when an incremental local unit compiled without git diff stats.
const DIRTY_FALLBACK_LINES: usize = 50;

/// Optimistic realistic overhead (warm hosted sync + Cargo startup/fingerprints).
const EST_OVERHEAD_SECS: f64 = 0.12;
/// Optimistic realistic warm incremental check base (frontend + incr load/save).
const EST_CHECK_BASE_SECS: f64 = 0.25;
/// Optimistic realistic warm incremental debug build/test base (frontend + codegen).
const EST_DEBUG_BASE_SECS: f64 = 0.45;
/// Optimistic realistic warm incremental release build/test base.
const EST_RELEASE_BASE_SECS: f64 = 0.85;
/// Optimistic realistic incremental link with wild/mold.
const EST_LINK_INCR_SECS: f64 = 0.18;
/// Optimistic realistic cold/release link with wild/mold.
const EST_LINK_COLD_SECS: f64 = 0.30;

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct TheoreticalFloor {
    /// Total theoretical optimal wall time in seconds.
    pub total_secs: f64,
    /// Optimistic realistic wall-time estimate in seconds (well-tuned justrust today).
    #[serde(default)]
    pub optimistic_est_secs: f64,
    /// Irreducible stat/hash/dispatch overhead.
    pub overhead_secs: f64,
    /// Frontend floor (parse + typecheck + borrowcheck) for changed workspace lines.
    pub frontend_secs: f64,
    /// Codegen floor (0 for check/clippy).
    pub codegen_secs: f64,
    /// Link floor (0 for check/clippy).
    pub link_secs: f64,
    /// Test execution floor (slowest individual test if known, else measured test time).
    pub test_secs: f64,
    /// Estimated changed workspace Rust lines used for the calculation.
    pub dirty_lines: usize,
    /// Whether this was a cold workspace build (no fresh units, local units compiled).
    pub cold: bool,
}

fn needs_codegen(subcommand: &str, args: &[String]) -> bool {
    let sub = if subcommand.is_empty() {
        args.iter()
            .map(String::as_str)
            .find(|a| !a.starts_with('+'))
            .unwrap_or("")
    } else {
        subcommand
    };
    !matches!(sub, "check" | "c" | "clippy" | "metadata" | "tree" | "fmt")
}

fn is_release(args: &[String], opt_level: Option<&str>) -> bool {
    let arg_release = args
        .iter()
        .take_while(|a| a.as_str() != "--")
        .enumerate()
        .any(|(i, arg)| {
            matches!(arg.as_str(), "--release" | "-r" | "--profile=release")
                || (arg == "--profile" && args.get(i + 1).map(String::as_str) == Some("release"))
        });
    arg_release || matches!(opt_level, Some("2" | "3" | "s" | "z"))
}

pub fn compute(
    subcommand: &str,
    args: &[String],
    git: &GitInfo,
    units: &UnitStats,
    phases: &Phases,
    tests: &TestStats,
) -> TheoreticalFloor {
    let overhead_secs = OVERHEAD_SECS;

    let (dirty_lines, cold, frontend_secs, codegen_secs, link_secs) =
        if units.compiled == 0 || units.local == 0 {
            (0, false, 0.0, 0.0, 0.0)
        } else {
            let cold = units.fresh == 0;
            let dirty_lines = if cold {
                (units.local.saturating_mul(COLD_LOC_PER_LOCAL_UNIT))
                    .max(git.dirty_rs_lines.unwrap_or(0))
            } else {
                git.dirty_rs_lines
                    .filter(|&n| n > 0)
                    .unwrap_or(DIRTY_FALLBACK_LINES)
            };
            let frontend_secs = dirty_lines as f64 / FRONTEND_LOC_PER_SEC;
            let (codegen_secs, link_secs) = if needs_codegen(subcommand, args) {
                let rate = if is_release(args, units.opt_level.as_deref()) {
                    CODEGEN_RELEASE_LOC_PER_SEC
                } else {
                    CODEGEN_DEBUG_LOC_PER_SEC
                };
                let cg = dirty_lines as f64 / rate;
                let lk = if cold {
                    LINK_COLD_SECS
                } else {
                    LINK_INCREMENTAL_SECS
                };
                (cg, lk)
            } else {
                (0.0, 0.0)
            };
            (dirty_lines, cold, frontend_secs, codegen_secs, link_secs)
        };

    let test_secs = if let Some(slowest) = tests.slowest.first() {
        slowest.secs.max(0.0)
    } else if tests.libtest_secs > 0.0 {
        tests.libtest_secs
    } else if phases.test_run > 0.0 {
        phases.test_run
    } else {
        0.0
    };

    let total_secs = overhead_secs + frontend_secs + codegen_secs + link_secs + test_secs;

    let est_compile_secs = if units.compiled == 0 || units.local == 0 {
        0.0
    } else if !needs_codegen(subcommand, args) {
        if cold {
            0.40 + (units.local.min(16) as f64) * 0.10 + (dirty_lines as f64 / 50_000.0)
        } else {
            EST_CHECK_BASE_SECS + (dirty_lines as f64 / 50_000.0)
        }
    } else if is_release(args, units.opt_level.as_deref()) {
        if cold {
            1.20 + (units.local.min(16) as f64) * 0.25
                + (dirty_lines as f64 / 5_000.0)
                + EST_LINK_COLD_SECS
        } else {
            EST_RELEASE_BASE_SECS + (dirty_lines as f64 / 5_000.0) + EST_LINK_COLD_SECS
        }
    } else if cold {
        0.60 + (units.local.min(16) as f64) * 0.15
            + (dirty_lines as f64 / 20_000.0)
            + EST_LINK_COLD_SECS
    } else {
        EST_DEBUG_BASE_SECS + (dirty_lines as f64 / 20_000.0) + EST_LINK_INCR_SECS
    };
    let optimistic_est_secs = (EST_OVERHEAD_SECS + est_compile_secs + test_secs).max(total_secs);

    TheoreticalFloor {
        total_secs,
        optimistic_est_secs,
        overhead_secs,
        frontend_secs,
        codegen_secs,
        link_secs,
        test_secs,
        dirty_lines,
        cold,
    }
}

/// Compact duration formatter for table columns (e.g. `5ms`, `20ms`, `0.2s`, `1.4s`).
pub fn format_short(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "-".into();
    }
    if secs < 0.001 {
        "<1ms".into()
    } else if secs < 0.1 {
        format!("{:.0}ms", secs * 1e3)
    } else if secs < 10.0 {
        format!("{secs:.2}s")
    } else if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        crate::status::dur(secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::TestTime;

    #[test]
    fn noop_build_is_just_overhead() {
        let f = compute(
            "check",
            &["check".into()],
            &GitInfo::default(),
            &UnitStats::default(),
            &Phases::default(),
            &TestStats::default(),
        );
        assert_eq!(f.dirty_lines, 0);
        assert!((f.total_secs - 0.005).abs() < 1e-9);
    }

    #[test]
    fn incremental_one_line_test_edit_floor() {
        let git = GitInfo {
            dirty_rs_lines: Some(2),
            ..Default::default()
        };
        let units = UnitStats {
            compiled: 1,
            local: 1,
            fresh: 40,
            ..Default::default()
        };
        let tests = TestStats {
            slowest: vec![TestTime {
                name: "fps_counter".into(),
                binary: "ui".into(),
                secs: 0.015,
                passed: true,
            }],
            ..Default::default()
        };
        let f = compute(
            "test",
            &["test".into()],
            &git,
            &units,
            &Phases::default(),
            &tests,
        );
        assert!(!f.cold);
        assert_eq!(f.dirty_lines, 2);
        // 5ms overhead + ~0ms frontend + ~0ms codegen + 5ms link + 15ms test = ~25ms
        assert!((f.total_secs - 0.025).abs() < 0.001, "got {}", f.total_secs);
    }

    #[test]
    fn check_skips_codegen_and_link() {
        let git = GitInfo {
            dirty_rs_lines: Some(1000),
            ..Default::default()
        };
        let units = UnitStats {
            compiled: 1,
            local: 1,
            fresh: 10,
            ..Default::default()
        };
        let f = compute(
            "check",
            &["check".into()],
            &git,
            &units,
            &Phases::default(),
            &TestStats::default(),
        );
        assert_eq!(f.codegen_secs, 0.0);
        assert_eq!(f.link_secs, 0.0);
        assert!((f.frontend_secs - 0.001).abs() < 1e-9);
    }

    #[test]
    fn format_short_is_compact() {
        assert_eq!(format_short(0.005), "5ms");
        assert_eq!(format_short(0.025), "25ms");
        assert_eq!(format_short(0.15), "0.15s");
        assert_eq!(format_short(12.34), "12.3s");
    }
}
