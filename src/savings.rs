//! Accounting for skipped work, not a counterfactual stock build or CPU time.
use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Savings {
    pub dependency_hits: usize,
    pub local_hits: usize,
    pub build_script_hits: usize,
    pub original_task_secs: f64,
    pub measured_hit_overhead_secs: f64,
    pub estimated_task_secs_avoided: f64,
    pub hits_without_full_overhead: usize,
    pub hits_without_valid_timing: usize,
    pub malformed_records: usize,
}

impl Savings {
    fn hit(&mut self, original: Option<f64>, overhead: Option<f64>, full: bool) {
        let valid = |v: &f64| v.is_finite() && *v >= 0.0;
        let Some(original) = original.filter(valid) else {
            self.hits_without_valid_timing += 1;
            return;
        };
        self.original_task_secs += original;
        let overhead = overhead.filter(valid);
        if !full || overhead.is_none() {
            self.hits_without_full_overhead += 1;
        }
        self.measured_hit_overhead_secs += overhead.unwrap_or(0.0);
        // Keep negative results: a slow cache restore is a regression, not a saving.
        self.estimated_task_secs_avoided += original - overhead.unwrap_or(0.0);
    }

    fn add(&mut self, other: &Self) {
        self.dependency_hits += other.dependency_hits;
        self.local_hits += other.local_hits;
        self.build_script_hits += other.build_script_hits;
        self.original_task_secs += other.original_task_secs;
        self.measured_hit_overhead_secs += other.measured_hit_overhead_secs;
        self.estimated_task_secs_avoided += other.estimated_task_secs_avoided;
        self.hits_without_full_overhead += other.hits_without_full_overhead;
        self.hits_without_valid_timing += other.hits_without_valid_timing;
        self.malformed_records += other.malformed_records;
    }

    pub fn line(&self) -> String {
        format!(
            "estimated task time avoided {:.2}s ({:.2}s original - {:.2}s measured hit overhead). Not CPU time, wall-clock savings, or a stock comparison.{}",
            self.estimated_task_secs_avoided,
            self.original_task_secs,
            self.measured_hit_overhead_secs,
            if self.hits_without_full_overhead
                + self.hits_without_valid_timing
                + self.malformed_records
                > 0
            {
                format!(
                    " Incomplete timing: {} hits lack full overhead, {} lack valid original timing, {} malformed records.",
                    self.hits_without_full_overhead,
                    self.hits_without_valid_timing,
                    self.malformed_records
                )
            } else {
                String::new()
            }
        )
    }
}

/// None means no raw recording survives. An empty file is valid zero-hit evidence.
pub fn from_dir(dir: &Path) -> Option<Savings> {
    let units = std::fs::read_to_string(dir.join("units.jsonl")).ok();
    let scripts = std::fs::read_to_string(dir.join("scripts.jsonl")).ok();
    if units.is_none() && scripts.is_none() {
        return None;
    }
    Some(from_records(
        units.as_deref().unwrap_or(""),
        scripts.as_deref().unwrap_or(""),
    ))
}

fn from_records(units: &str, scripts: &str) -> Savings {
    let mut total = Savings::default();
    for line in units.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            total.malformed_records += 1;
            continue;
        };
        match v["depcache"].as_str() {
            Some("local") => total.local_hits += 1,
            Some("hit" | "remote" | "peer" | "prebuilt") => total.dependency_hits += 1,
            _ => continue,
        }
        let full = v["cache_hit_overhead_secs"].as_f64();
        total.hit(
            v["depcache_saved_secs"].as_f64(),
            full.or_else(|| v["wall"].as_f64()),
            full.is_some(),
        );
    }
    for line in scripts.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            total.malformed_records += 1;
            continue;
        };
        if v["hit"].as_bool() != Some(true) {
            continue;
        }
        total.build_script_hits += 1;
        let full = v["cache_hit_overhead_secs"].as_f64();
        total.hit(
            v["saved_secs"].as_f64(),
            full.or_else(|| v["wall"].as_f64()),
            full.is_some(),
        );
    }
    total
}

#[derive(Debug, Default, Serialize)]
struct Report {
    metric: &'static str,
    limitations: &'static str,
    indexed_runs: usize,
    measured_runs: usize,
    unavailable_runs: usize,
    totals: Savings,
}

#[derive(Deserialize)]
struct SavedAccounting {
    #[serde(default)]
    savings: Option<Savings>,
}

pub fn command(days: Option<u32>, here: bool, json: bool) -> Result<()> {
    let root = paths::runs_dir()?;
    let cwd = std::env::current_dir()?;
    let cutoff = days.map(|d| paths::now() - f64::from(d) * 86400.0);
    let mut seen = HashSet::new();
    let mut report = Report {
        metric: "estimated_task_time_avoided",
        limitations: "Retained recordings only. Not CPU time, wall-clock savings, or a stock comparison. Tasks overlap and original durations may come from other machines. Misses, uploads, process startup, recorder work, and other build overhead are not deducted.",
        ..Report::default()
    };
    for entry in crate::runs::load_index() {
        if cutoff.is_some_and(|c| entry.start < c)
            || (here && !Path::new(&entry.cwd).starts_with(&cwd))
            || !seen.insert(entry.id.clone())
        {
            continue;
        }
        report.indexed_runs += 1;
        let dir = root.join(&entry.id);
        let persisted = std::fs::read(dir.join("summary.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice::<SavedAccounting>(&raw).ok())
            .and_then(|s| s.savings);
        if let Some(savings) = persisted.or_else(|| from_dir(&dir)) {
            report.measured_runs += 1;
            report.totals.add(&savings);
        } else {
            report.unavailable_runs += 1;
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Known skipped work across retained run history{}",
            days.map_or(String::new(), |d| format!(" (last {d} days)"))
        );
        println!("{}", report.totals.line());
        println!(
            "Hits: {} dependency, {} local, {} build script",
            report.totals.dependency_hits,
            report.totals.local_hits,
            report.totals.build_script_hits
        );
        println!(
            "Coverage: {}/{} indexed runs have accounting, {} unavailable. Retained history only, not lifetime savings.",
            report.measured_runs, report.indexed_runs, report.unavailable_runs
        );
        println!(
            "Original durations may come from other machines. Overlapping tasks are summed. Misses, setup, uploads, and other build overhead are not deducted."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_disjoint_hits_and_preserves_regressions() {
        let s = from_records(
            r#"{"depcache":"remote","depcache_saved_secs":10,"wall":1,"cache_hit_overhead_secs":2}
{"depcache":"local","depcache_saved_secs":1,"cache_hit_overhead_secs":3}
{"depcache":"miss","wall":200}
{"depcache":"prebuilt","depcache_saved_secs":5,"cache_hit_overhead_secs":1}"#,
            r#"{"hit":true,"saved_secs":4,"wall":0.5,"cache_hit_overhead_secs":1}
{"hit":false,"wall":20}"#,
        );
        assert_eq!(
            (s.dependency_hits, s.local_hits, s.build_script_hits),
            (2, 1, 1)
        );
        assert_eq!(s.original_task_secs, 20.0);
        assert_eq!(s.measured_hit_overhead_secs, 7.0);
        assert_eq!(s.estimated_task_secs_avoided, 13.0);
        assert_eq!(s.hits_without_full_overhead, 0);
    }

    #[test]
    fn old_and_missing_timings_are_disclosed() {
        let s = from_records(
            r#"{"depcache":"hit","depcache_saved_secs":10,"wall":2}
{"depcache":"peer","depcache_saved_secs":-1,"wall":2}
{"depcache":"local"}
not json"#,
            "",
        );
        assert_eq!(s.estimated_task_secs_avoided, 8.0);
        assert_eq!(s.hits_without_full_overhead, 1);
        assert_eq!(s.hits_without_valid_timing, 2);
        assert_eq!(s.malformed_records, 1);
        assert!(
            s.line()
                .contains("Not CPU time, wall-clock savings, or a stock comparison")
        );
    }

    #[test]
    fn aggregation_and_serialization_retain_accounting() {
        let mut s = Savings::default();
        let one = from_records(
            r#"{"depcache":"hit","depcache_saved_secs":1,"cache_hit_overhead_secs":3}"#,
            "",
        );
        s.add(&one);
        s.add(&one);
        assert_eq!(s.estimated_task_secs_avoided, -4.0);
        let roundtrip: Savings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(roundtrip.estimated_task_secs_avoided, -4.0);
        assert_eq!(roundtrip.dependency_hits, 2);
    }
}
