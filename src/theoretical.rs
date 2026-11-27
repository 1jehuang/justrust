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
/// Optimistic realistic warm clippy base (lint passes run over the edited crate).
const EST_CLIPPY_BASE_SECS: f64 = 1.20;
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

fn resolved_subcommand<'a>(subcommand: &'a str, args: &'a [String]) -> &'a str {
    if subcommand.is_empty() {
        args.iter()
            .map(String::as_str)
            .find(|a| !a.starts_with('+'))
            .unwrap_or("")
    } else {
        subcommand
    }
}

fn needs_codegen(subcommand: &str, args: &[String]) -> bool {
    !matches!(
        resolved_subcommand(subcommand, args),
        "check" | "c" | "clippy" | "metadata" | "tree" | "fmt"
    )
}

fn is_clippy(subcommand: &str, args: &[String]) -> bool {
    resolved_subcommand(subcommand, args) == "clippy"
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
        // Summed over binaries, which the pool runs side by side: more than
        // the run took is not a floor.
        if phases.test_run > 0.0 {
            tests.libtest_secs.min(phases.test_run)
        } else {
            tests.libtest_secs
        }
    } else if phases.test_run > 0.0 {
        phases.test_run
    } else {
        0.0
    };

    let total_secs = overhead_secs + frontend_secs + codegen_secs + link_secs + test_secs;

    let release = is_release(args, units.opt_level.as_deref());
    let clippy = is_clippy(subcommand, args);
    // An optimized crate built without incremental compilation is redone
    // whole whatever the edit was: that is the estimate's work, while the
    // floor above stays what an ideal compiler would need.
    let est_release_lines = dirty_lines.max(units.full_rebuild_lines as usize);
    let local_scale = units.local.min(16) as f64 + (units.local.saturating_sub(16) as f64) * 0.25;
    let dep_miss_secs = if units.depcache_misses > 0 {
        let per_dep = if !needs_codegen(subcommand, args) {
            0.03
        } else if release {
            0.25
        } else {
            0.08
        };
        (units.depcache_misses as f64) * per_dep
    } else {
        0.0
    };

    let est_compile_secs = if units.compiled == 0 || units.local == 0 {
        dep_miss_secs
    } else if !needs_codegen(subcommand, args) {
        let base = if clippy {
            if cold {
                1.20 + local_scale * 0.20
            } else {
                EST_CLIPPY_BASE_SECS + (units.local.saturating_sub(1).min(8) as f64) * 0.15
            }
        } else if cold {
            0.40 + local_scale * 0.10
        } else {
            EST_CHECK_BASE_SECS
        };
        base + (dirty_lines as f64 / 50_000.0) + dep_miss_secs
    } else if release {
        if cold {
            1.20 + local_scale * 0.30
                + (est_release_lines as f64 / 5_000.0)
                + EST_LINK_COLD_SECS
                + dep_miss_secs
        } else {
            EST_RELEASE_BASE_SECS
                + (est_release_lines as f64 / 5_000.0)
                + EST_LINK_COLD_SECS
                + dep_miss_secs
        }
    } else if cold {
        0.60 + local_scale * 0.20
            + (dirty_lines as f64 / 20_000.0)
            + EST_LINK_COLD_SECS
            + dep_miss_secs
    } else {
        EST_DEBUG_BASE_SECS + (dirty_lines as f64 / 20_000.0) + EST_LINK_INCR_SECS + dep_miss_secs
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
    fn summed_libtest_time_never_exceeds_the_test_phase() {
        // 23 binaries side by side: libtest's totals sum to 36.84 s of an
        // 8.74 s test phase (run 20261126-155311534-171924).
        let tests = TestStats {
            libtest_secs: 36.84,
            ..Default::default()
        };
        let phases = Phases {
            test_run: 8.74,
            ..Default::default()
        };
        let floor = |phases: &Phases| {
            compute(
                "test",
                &["test".into()],
                &GitInfo::default(),
                &UnitStats::default(),
                phases,
                &tests,
            )
        };
        assert_eq!(floor(&phases).test_secs, 8.74);
        assert_eq!(floor(&Phases::default()).test_secs, 36.84);
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

    #[test]
    fn clippy_and_depcache_misses_scale_optimistic_estimate_without_changing_floor() {
        let git = GitInfo {
            dirty_rs_lines: Some(50),
            ..Default::default()
        };
        let check_units = UnitStats {
            compiled: 1,
            local: 1,
            fresh: 40,
            ..Default::default()
        };
        let check_f = compute(
            "check",
            &["check".into()],
            &git,
            &check_units,
            &Phases::default(),
            &TestStats::default(),
        );
        let clippy_f = compute(
            "clippy",
            &["clippy".into()],
            &git,
            &check_units,
            &Phases::default(),
            &TestStats::default(),
        );
        assert!((check_f.total_secs - clippy_f.total_secs).abs() < 1e-9);
        assert!(clippy_f.optimistic_est_secs > check_f.optimistic_est_secs + 0.8);

        let cold_rel = UnitStats {
            compiled: 200,
            local: 10,
            fresh: 0,
            depcache_misses: 100,
            ..Default::default()
        };
        let rel_f = compute(
            "build",
            &["build".into(), "--release".into()],
            &GitInfo::default(),
            &cold_rel,
            &Phases::default(),
            &TestStats::default(),
        );
        // Theoretical floor still assumes 0s for third-party deps, while optimistic estimate
        // accounts for the 100 compiled depcache misses.
        assert!(rel_f.total_secs < 1.0);
        assert!(rel_f.optimistic_est_secs > 25.0);
    }

    /// CI release run 20261126-060749442-2620: a version bump rebuilt the
    /// 115k-line `justrust` crate optimized and non-incremental in 26 s, and
    /// the estimate said 1.28 s as if 50 edited lines were the work.
    #[test]
    fn a_non_incremental_optimized_crate_is_estimated_whole() {
        let args = ["build".to_string(), "--release".into()];
        let units = |full_rebuild_lines| UnitStats {
            compiled: 2,
            local: 2,
            fresh: 200,
            opt_level: Some("3".into()),
            full_rebuild_lines,
            ..Default::default()
        };
        let est = |u: &UnitStats| {
            compute(
                "build",
                &args,
                &GitInfo::default(),
                u,
                &Phases::default(),
                &TestStats::default(),
            )
        };
        let edited = est(&units(0));
        let whole = est(&units(115_000));
        assert!((edited.optimistic_est_secs - 1.28).abs() < 1e-6);
        assert!((whole.optimistic_est_secs - (1.27 + 23.0)).abs() < 1e-6);
        // The floor is first principles: it does not grow with how rustc works.
        assert_eq!(edited.total_secs, whole.total_secs);

        // A debug build of the same crate is incremental: nothing changes.
        let debug = compute(
            "build",
            &["build".into()],
            &GitInfo::default(),
            &UnitStats {
                opt_level: Some("0".into()),
                ..units(115_000)
            },
            &Phases::default(),
            &TestStats::default(),
        );
        assert!(debug.optimistic_est_secs < 1.0);
    }
}
