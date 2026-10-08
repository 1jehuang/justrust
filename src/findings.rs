//! Detect inefficiencies in a recorded run.
//!
//! Each check looks at one known way builds waste time and, when it fires,
//! estimates how many seconds of this run it cost. Agent-mode output lists
//! every finding, so a run is never silently slower than it needs to be.

use crate::summary::Summary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    /// Stable identifier, for grouping across runs.
    pub kind: String,
    /// Estimated seconds of this run's wall time the issue cost.
    pub cost_secs: f64,
    pub message: String,
}

impl Finding {
    fn new(kind: &str, cost_secs: f64, message: String) -> Finding {
        Finding {
            kind: kind.to_owned(),
            cost_secs: cost_secs.max(0.0),
            message,
        }
    }
}

/// Ignore findings worth less than this many seconds.
const MIN_COST: f64 = 0.3;

/// Run every check and return findings, most expensive first.
pub fn analyze(s: &Summary) -> Vec<Finding> {
    let mut out = Vec::new();
    let p = &s.phases;
    let r = &s.resources;
    let ncpu = s.ncpu.max(1) as f64;

    // Waiting on another cargo for the build directory.
    if p.lock_wait >= MIN_COST {
        let on_dir =
            s.locks_waited.is_empty() || s.locks_waited.iter().any(|l| l.contains("directory"));
        let what = if s.locks_waited.is_empty() {
            "the build directory".to_owned()
        } else {
            s.locks_waited.join(", ")
        };
        let slot_hint = match &s.slot {
            Some(i) if i.slot.is_none() && on_dir => format!(
                "; all {} justrust build slots were busy, raise JUSTRUST_SLOTS",
                i.capacity
            ),
            Some(i) if i.slot.is_some() && on_dir => {
                "; another process used this build slot".to_owned()
            }
            _ if !on_dir => "; cargo's shared CARGO_HOME lock, not the target dir".to_owned(),
            _ => " (parallel builds in the same target dir serialize)".to_owned(),
        };
        out.push(Finding::new(
            "lock_wait",
            p.lock_wait,
            format!(
                "waited {:.1}s for another cargo to release the lock on {what}{slot_hint}",
                p.lock_wait
            ),
        ));
    }

    // Seeding a new build slot (reflink copy of the shared target dir).
    if let Some(i) = &s.slot
        && i.seed_secs >= MIN_COST
    {
        out.push(Finding::new(
            "slot_seed",
            i.seed_secs,
            format!(
                "seeded build slot {} from the shared target dir in {:.1}s \
                 (once per slot, before cargo started)",
                i.slot.unwrap_or(0),
                i.seed_secs
            ),
        ));
    }

    // All slots busy: this run shared the target dir and could block others.
    if let Some(i) = &s.slot
        && i.slot.is_none()
    {
        out.push(Finding::new(
            "slots_full",
            MIN_COST,
            format!(
                "all {} build slots were busy, so this run used the shared target dir \
                 and may have blocked or been blocked by other builds (JUSTRUST_SLOTS raises the cap)",
                i.capacity
            ),
        ));
    }

    // Other processes competing for the CPU.
    if r.avg_other_cores >= 0.5 * ncpu && s.wall >= 2.0 {
        // While others used N of C cores, this build could at best have had the
        // remainder. Estimate the slowdown as the share of the machine taken.
        let share = (r.avg_other_cores / ncpu).min(0.95);
        let cost = s.wall * share * 0.5;
        out.push(Finding::new(
            "busy_machine",
            cost,
            format!(
                "machine was busy: other processes used {:.0} of {} cores on average{} \
                 and tasks stalled {:.1}s waiting for CPU",
                r.avg_other_cores,
                s.ncpu,
                if r.max_foreign_rustc > 0 {
                    format!(" (up to {} rustc from other builds)", r.max_foreign_rustc)
                } else {
                    String::new()
                },
                r.psi_cpu_secs
            ),
        ));
    }

    // Memory pressure.
    if r.psi_mem_secs >= MIN_COST {
        out.push(Finding::new(
            "memory_pressure",
            r.psi_mem_secs,
            format!(
                "tasks stalled {:.1}s on memory (lowest available {:.1} GB, build peak {:.1} GB)",
                r.psi_mem_secs,
                r.min_mem_available_mb as f64 / 1024.0,
                r.peak_build_rss_mb / 1024.0
            ),
        ));
    }

    // Dependencies or build scripts rebuilt: usually flags, features, toolchain,
    // or target-dir changes rather than real edits.
    let deps: Vec<_> = s.top_units.iter().filter(|u| !u.local).collect();
    let dep_secs: f64 = deps.iter().map(|u| u.wall_share).sum();
    if s.units.dependencies + s.units.build_scripts > 0 && dep_secs >= MIN_COST {
        let reasons = summarize_reasons(s, false);
        out.push(Finding::new(
            "dependency_rebuild",
            dep_secs,
            format!(
                "rebuilt {} registry dependencies and {} build scripts ({:.1}s){}",
                s.units.dependencies,
                s.units.build_scripts,
                dep_secs,
                reasons.map(|r| format!(": {r}")).unwrap_or_default()
            ),
        ));
    }

    // Local crates rebuilt only because something upstream changed.
    let cascades: Vec<_> = s
        .rebuild_reasons
        .iter()
        .filter(|r| r.reason.starts_with("the dependency ") && r.reason.ends_with(" was rebuilt"))
        .filter(|r| {
            s.top_units
                .iter()
                .any(|u| u.local && unit_matches(&u.name, &r.package))
        })
        .collect();
    if !cascades.is_empty() {
        let names: Vec<&str> = cascades.iter().map(|r| r.package.as_str()).collect();
        let cost: f64 = s
            .top_units
            .iter()
            .filter(|u| names.iter().any(|n| unit_matches(&u.name, n)))
            .map(|u| u.wall_share)
            .sum();
        if cost >= MIN_COST {
            let upstream: Vec<String> = cascades
                .iter()
                .filter_map(|r| {
                    r.reason
                        .strip_prefix("the dependency `")
                        .and_then(|x| x.split('`').next())
                        .map(str::to_owned)
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            out.push(Finding::new(
                "upstream_cascade",
                cost,
                format!(
                    "{} rebuilt only because {} changed ({:.1}s). Edits to shared upstream \
                     crates force downstream recompiles",
                    names.join(", "),
                    upstream.join(", "),
                    cost
                ),
            ));
        }
    }

    // A unit whose codegen dominated: usually the incremental cache did not
    // help (upstream API changed, flags changed) or the crate is huge.
    for u in s.top_units.iter().filter(|u| u.local) {
        let Some(sp) = &u.split else { continue };
        if sp.codegen >= 3.0 && sp.codegen >= sp.frontend {
            let mono = u
                .top_passes
                .iter()
                .find(|p| p.name == "monomorphization_collector_graph_walk")
                .map(|p| p.secs)
                .unwrap_or(0.0);
            out.push(Finding::new(
                "heavy_codegen",
                sp.codegen,
                format!(
                    "{} spent {:.1}s in codegen{} vs {:.1}s type-checking. Large codegen after \
                     a small edit means little was reused from the incremental cache",
                    u.name,
                    sp.codegen,
                    if mono > 0.5 {
                        format!(" ({mono:.1}s monomorphizing)")
                    } else {
                        String::new()
                    },
                    sp.frontend
                ),
            ));
        }
        if sp.link >= 1.0 {
            out.push(Finding::new(
                "slow_link",
                sp.link,
                format!("{} spent {:.1}s linking", u.name, sp.link),
            ));
        }
        if sp.incremental >= 1.0 && sp.incremental >= 0.15 * u.wall {
            out.push(Finding::new(
                "incremental_overhead",
                sp.incremental,
                format!(
                    "{} spent {:.1}s loading and saving its incremental cache ({:.0}% of the unit)",
                    u.name,
                    sp.incremental,
                    100.0 * sp.incremental / u.wall.max(1e-9)
                ),
            ));
        }
        if let Some(pass) = u.top_passes.iter().find(|p| p.name == "macro_expand_crate")
            && pass.secs >= 1.5
            && pass.secs >= 0.2 * u.wall
        {
            out.push(Finding::new(
                "macro_expansion",
                pass.secs,
                format!(
                    "{} spent {:.1}s expanding macros on every rebuild (derives, proc macros)",
                    u.name, pass.secs
                ),
            ));
        }
    }

    // Built a test binary but ran almost none of it.
    let ran = s.tests.passed + s.tests.failed;
    let total_tests = ran + s.tests.filtered_out + s.tests.ignored;
    if ran > 0 && total_tests >= 200 && (ran as f64) < 0.02 * total_tests as f64 {
        let test_units: f64 = s
            .top_units
            .iter()
            .filter(|u| u.kind == "test")
            .map(|u| u.wall_share)
            .sum();
        if test_units >= 2.0 {
            out.push(Finding::new(
                "filtered_test_build",
                test_units * 0.5,
                format!(
                    "compiled a test binary with {} tests ({:.1}s) to run {}. Tests in a \
                     smaller crate or `justrust check` first would be faster",
                    total_tests, test_units, ran
                ),
            ));
        }
    }

    // The build used few cores for a long time: a serial bottleneck.
    if p.compile >= 5.0 && r.avg_build_cores < 0.25 * ncpu && r.avg_other_cores < 0.5 * ncpu {
        if let Some(u) = s
            .top_units
            .first()
            .filter(|u| u.wall_share >= 0.6 * p.compile)
        {
            let threads = u
                .frontend_threads
                .map(|t| format!(" (frontend threads {t})"))
                .unwrap_or_else(|| " (single-threaded frontend)".to_owned());
            out.push(Finding::new(
                "serial_bottleneck",
                u.wall_share * 0.3,
                format!(
                    "build averaged {:.1} of {} cores: {} ran {:.1}s mostly alone{}",
                    r.avg_build_cores, s.ncpu, u.name, u.wall_share, threads
                ),
            ));
        }
    }

    // Build gaps: no rustc running inside the build window.
    if p.build_gaps >= 1.0 {
        out.push(Finding::new(
            "build_gaps",
            p.build_gaps,
            format!(
                "{:.1}s inside the build with no compiler running (build scripts or scheduling)",
                p.build_gaps
            ),
        ));
    }

    // Cargo startup.
    if p.startup >= 1.5 {
        out.push(Finding::new(
            "slow_startup",
            p.startup - 0.5,
            format!(
                "cargo took {:.1}s before compiling anything (resolving, fingerprinting, \
                 lock file, registry)",
                p.startup
            ),
        ));
    }

    // A slow test binary.
    if let Some(b) = s
        .tests
        .binaries
        .iter()
        .max_by(|a, b| a.wall.total_cmp(&b.wall))
        && b.wall >= 3.0
    {
        out.push(Finding::new(
            "slow_tests",
            b.wall,
            format!("tests in {} took {:.1}s to run", b.name, b.wall),
        ));
    }

    out.retain(|f| f.cost_secs >= MIN_COST);
    out.sort_by(|a, b| b.cost_secs.total_cmp(&a.cost_secs));
    out
}

/// Summarize rebuild reasons for local (`local=true`) or non-local units.
fn summarize_reasons(s: &Summary, local: bool) -> Option<String> {
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for r in &s.rebuild_reasons {
        let is_local = s
            .top_units
            .iter()
            .any(|u| u.local && unit_matches(&u.name, &r.package));
        if is_local != local {
            continue;
        }
        *counts.entry(generalize_reason(&r.reason)).or_default() += 1;
    }
    if counts.is_empty() {
        return None;
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    Some(
        v.into_iter()
            .take(3)
            .map(|(r, n)| if n > 1 { format!("{r} (x{n})") } else { r })
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// Collapse a reason to a short category so similar reasons group together.
fn generalize_reason(r: &str) -> String {
    if r.starts_with("the dependency ") && r.ends_with(" was rebuilt") {
        return "an upstream dependency was rebuilt".into();
    }
    if r.contains("has changed") && r.starts_with("the file") {
        return "source file changed".into();
    }
    if r.contains("RUSTFLAGS") || r.contains("rustflags") {
        return "RUSTFLAGS changed".into();
    }
    if r.contains("features") {
        return "features changed".into();
    }
    if r.contains("profile") {
        return "profile changed".into();
    }
    if r.contains("rustc") {
        return "compiler changed".into();
    }
    if r.contains("env var") || r.contains("environment variable") {
        return "environment variable changed".into();
    }
    r.chars().take(80).collect()
}

/// Unit names are crate names (`jcode_desktop_ui (test)`), packages use dashes.
fn unit_matches(unit_name: &str, package: &str) -> bool {
    let base = unit_name.split(" (").next().unwrap_or(unit_name);
    base == package.replace('-', "_") || base == package
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::{RebuildReason, Split, UnitBreakdown};

    fn base() -> Summary {
        Summary {
            wall: 20.0,
            ncpu: 16,
            ..Default::default()
        }
    }

    #[test]
    fn clean_fast_run_has_no_findings() {
        let mut s = base();
        s.wall = 0.6;
        s.phases.compile = 0.5;
        assert!(analyze(&s).is_empty());
    }

    #[test]
    fn detects_upstream_cascade_and_heavy_codegen() {
        let mut s = base();
        s.top_units.push(UnitBreakdown {
            name: "jcode_desktop_ui (test)".into(),
            local: true,
            kind: "test".into(),
            wall: 35.0,
            wall_share: 35.0,
            split: Some(Split {
                frontend: 5.0,
                codegen: 23.6,
                link: 0.7,
                incremental: 1.1,
                other: 0.0,
            }),
            ..Default::default()
        });
        s.rebuild_reasons.push(RebuildReason {
            package: "jcode-desktop-ui".into(),
            reason: "the dependency `jcode_base` was rebuilt".into(),
        });
        let f = analyze(&s);
        let kinds: Vec<&str> = f.iter().map(|f| f.kind.as_str()).collect();
        assert!(kinds.contains(&"upstream_cascade"), "{kinds:?}");
        assert!(kinds.contains(&"heavy_codegen"), "{kinds:?}");
        assert!(f.iter().any(|f| f.message.contains("jcode_base")));
    }

    #[test]
    fn detects_lock_wait_first() {
        let mut s = base();
        s.phases.lock_wait = 12.0;
        let f = analyze(&s);
        assert_eq!(f[0].kind, "lock_wait");
    }

    #[test]
    fn reports_slot_seed_and_fallback() {
        let mut s = base();
        s.slot = Some(crate::slots::SlotInfo {
            slot: Some(1),
            how: "new".into(),
            seed_secs: 9.0,
            capacity: 4,
            ..Default::default()
        });
        let f = analyze(&s);
        assert_eq!(f[0].kind, "slot_seed");
        s.slot = Some(crate::slots::SlotInfo {
            slot: None,
            how: "fallback".into(),
            busy: 4,
            capacity: 4,
            ..Default::default()
        });
        s.phases.lock_wait = 5.0;
        let f = analyze(&s);
        let kinds: Vec<&str> = f.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(kinds, ["lock_wait", "slots_full"]);
        assert!(f[0].message.contains("JUSTRUST_SLOTS"));
        s.locks_waited = vec!["package cache".into()];
        let f = analyze(&s);
        assert!(f[0].message.contains("CARGO_HOME"), "{}", f[0].message);
        assert!(!f[0].message.contains("JUSTRUST_SLOTS"));
    }

    #[test]
    fn parses_unit_names() {
        assert!(unit_matches("jcode_desktop_ui (test)", "jcode-desktop-ui"));
        assert!(!unit_matches("jcode_base", "jcode-desktop-ui"));
    }
}
