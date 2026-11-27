//! The compiler reuse ledger: what incremental compilation did with every
//! query and codegen unit of a unit, and how much of the recomputed work a
//! finer-grained architecture (Blaze: retain more compiler state, know
//! exactly what changed) would not have had to do.
//!
//! jr-rustc writes `<phases file>.reuse` beside the phase ledger:
//! `outcome\tquery\tseconds\tcount` per line, seconds exclusive of nested
//! queries on the same thread (so summed over frontend threads, not wall).
//!
//! Outcomes and how they are classed here:
//!
//! | outcome           | class       | meaning |
//! |-------------------|-------------|---------|
//! | `loaded`          | reused      | green, value loaded from the incremental cache |
//! | `same`            | avoidable   | an input changed, re-ran, produced the identical value |
//! | `green_recompute` | avoidable   | proven unchanged, but its value was never persisted |
//! | `nohash`          | opaque      | re-ran, result is never hashed, so it can never be reused |
//! | `eval_always`     | opaque      | re-runs every session by design |
//! | `changed`         | necessary   | re-ran and produced a different value |
//! | `new`             | necessary   | no node in the previous session (first build, new item) |
//!
//! "Avoidable" is the work a design with exact, finer-grained change
//! knowledge and complete value retention would skip. "Opaque" is work the
//! current design cannot classify. Codegen units count reuse only (no time).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Row {
    pub outcome: String,
    pub query: String,
    pub secs: f64,
    pub count: u64,
}

pub fn parse(text: &str) -> Vec<Row> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let outcome = f.next().filter(|s| !s.is_empty())?.to_owned();
            let query = f.next().filter(|s| !s.is_empty())?.to_owned();
            let secs = f.next()?.parse::<f64>().ok().filter(|s| s.is_finite())?;
            let count = f.next()?.parse().ok()?;
            Some(Row {
                outcome,
                query,
                secs,
                count,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Reused,
    Avoidable,
    Opaque,
    Necessary,
}

pub fn class(outcome: &str) -> Class {
    match outcome {
        "loaded" => Class::Reused,
        "same" | "green_recompute" | "eval_always_same" => Class::Avoidable,
        "changed" | "new" | "eval_always_changed" => Class::Necessary,
        _ => Class::Opaque,
    }
}

/// Rows that are evidence about the unit, not query execution time:
/// `<cgu>` counts, `<session>`, `<sibling_state>`, `<cgu_cpu>`,
/// `<cgu_avoided>`, `<link>`, and the `loaded_avoided`/`loaded_unmeasured`
/// credit of loads (jr-rustc). Kept out of the query time classes.
pub fn is_evidence(r: &Row) -> bool {
    r.query.starts_with('<') || r.outcome.starts_with("loaded_")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Cold,
    Warm,
    Unknown,
}

fn session_state(rows: &[Row]) -> SessionState {
    let mut cold = false;
    let mut warm = false;
    for r in rows.iter().filter(|r| r.count > 0) {
        match (r.query.as_str(), r.outcome.as_str()) {
            ("<session>", "cold_no_prior" | "cold_out_of_date" | "cold_io_error") => cold = true,
            ("<session>", "warm") | ("<cgu>", "loaded") => warm = true,
            (_, "loaded") if !is_evidence(r) => warm = true,
            _ => {}
        }
    }
    match (cold, warm) {
        (true, false) => SessionState::Cold,
        (false, true) => SessionState::Warm,
        _ => SessionState::Unknown,
    }
}

/// Only a positive compiler session marker establishes a cold start.
#[cfg(test)]
pub fn is_cold(rows: &[Row]) -> bool {
    session_state(rows) == SessionState::Cold
}

/// Totals for one unit, or many.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Totals {
    pub reused_secs: f64,
    pub reused_count: u64,
    pub avoidable_secs: f64,
    pub avoidable_count: u64,
    /// Of avoidable: values proven unchanged but never persisted.
    pub unpersisted_secs: f64,
    pub opaque_secs: f64,
    pub necessary_secs: f64,
    pub necessary_count: u64,
    pub cgu_reused: u64,
    pub cgu_recompiled: u64,
    pub warm_units: u64,
    /// Nonempty ledgers without conclusive session evidence.
    pub unknown_units: u64,
    /// Units explicitly reported cold, and their summed recomputed query seconds.
    pub cold_units: u64,
    pub cold_secs: f64,
}

impl Totals {
    pub fn add(&mut self, rows: &[Row]) {
        for r in rows {
            if r.query == "<cgu>" {
                match r.outcome.as_str() {
                    "loaded" => self.cgu_reused += r.count,
                    _ => self.cgu_recompiled += r.count,
                }
                continue;
            }
            if is_evidence(r) {
                continue;
            }
            match class(&r.outcome) {
                Class::Reused => {
                    self.reused_secs += r.secs;
                    self.reused_count += r.count;
                }
                Class::Avoidable => {
                    self.avoidable_secs += r.secs;
                    self.avoidable_count += r.count;
                    if r.outcome == "green_recompute" {
                        self.unpersisted_secs += r.secs;
                    }
                }
                Class::Opaque => self.opaque_secs += r.secs,
                Class::Necessary => {
                    self.necessary_secs += r.secs;
                    self.necessary_count += r.count;
                }
            }
        }
    }

    /// Add one unit, noting whether it started cold.
    pub fn add_unit(&mut self, rows: &[Row]) {
        self.add(rows);
        match session_state(rows) {
            SessionState::Warm => self.warm_units += 1,
            SessionState::Unknown => self.unknown_units += u64::from(!rows.is_empty()),
            SessionState::Cold => {
                self.cold_units += 1;
                self.cold_secs += rows
                    .iter()
                    .filter(|r| !is_evidence(r) && class(&r.outcome) != Class::Reused)
                    .map(|r| r.secs)
                    .sum::<f64>();
            }
        }
    }

    fn session_counts(&self) -> String {
        format!(
            "incremental starts (ledger units): {} cold, {} warm, {} unknown",
            self.cold_units, self.warm_units, self.unknown_units
        )
    }

    pub fn recomputed_secs(&self) -> f64 {
        self.avoidable_secs + self.opaque_secs + self.necessary_secs
    }

    /// Share of recomputed query time that produced nothing new.
    pub fn avoidable_share(&self) -> f64 {
        let r = self.recomputed_secs();
        if r > 0.0 {
            self.avoidable_secs / r
        } else {
            0.0
        }
    }
}

fn unit_label(u: &crate::shim::Unit) -> String {
    format!("{}{}", u.crate_name, if u.test { " (test)" } else { "" })
}

/// The `Reuse` section of `justrust show`. Empty when no unit has a ledger.
pub fn render_run(units: &[crate::shim::Unit]) -> String {
    let mut o = String::new();
    let with: Vec<_> = units.iter().filter(|u| !u.reuse.is_empty()).collect();
    if with.is_empty() {
        return o;
    }
    let mut all = Totals::default();
    for u in &with {
        all.add_unit(&u.reuse);
    }
    let _ = writeln!(
        o,
        "\nIncremental reuse ({} units with a reuse ledger, query seconds summed over threads)",
        with.len()
    );
    let _ = writeln!(o, "  {}", all.session_counts());
    let _ = writeln!(
        o,
        "  reused {:.2}s to load {} results; recomputed {:.2}s: avoidable {:.2}s ({:.0}%), opaque {:.2}s, necessary {:.2}s",
        all.reused_secs,
        all.reused_count,
        all.recomputed_secs(),
        all.avoidable_secs,
        100.0 * all.avoidable_share(),
        all.opaque_secs,
        all.necessary_secs
    );
    let _ = writeln!(
        o,
        "  {:<28} {:>8} {:>9} {:>8} {:>9} {:>9}",
        "UNIT", "WALL", "AVOIDABLE", "OPAQUE", "NECESSARY", "CGU REUSE"
    );
    let mut rows: Vec<_> = with
        .iter()
        .map(|u| {
            let mut t = Totals::default();
            t.add(&u.reuse);
            (u, t)
        })
        .collect();
    rows.sort_by(|a, b| b.1.avoidable_secs.total_cmp(&a.1.avoidable_secs));
    for (u, t) in rows.iter().take(8) {
        let cold_note = if t.cgu_reused == 0 && t.cgu_recompiled > 0 {
            match u.incr_prior_session {
                Some(false) => "  [empty incr dir]",
                Some(true) => "  [prior session rejected]",
                None => "",
            }
        } else {
            ""
        };
        let _ = writeln!(
            o,
            "  {:<28} {:>7.1}s {:>8.2}s {:>7.2}s {:>8.2}s {:>5}/{:<3}{}",
            unit_label(u),
            u.wall,
            t.avoidable_secs,
            t.opaque_secs,
            t.necessary_secs,
            t.cgu_reused,
            t.cgu_reused + t.cgu_recompiled,
            cold_note,
        );
    }
    let queries = by_query(with.iter().map(|u| u.reuse.as_slice()));
    let top: Vec<_> = queries
        .iter()
        .filter(|q| q.avoidable_secs >= 0.01)
        .take(6)
        .collect();
    if !top.is_empty() {
        let _ = writeln!(o, "  most avoidable queries:");
        for q in top {
            let _ = writeln!(o, "    {}", q.line());
        }
    }
    o
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct QueryTotals {
    pub query: String,
    pub avoidable_secs: f64,
    pub same_secs: f64,
    pub same_count: u64,
    pub unpersisted_secs: f64,
    pub unpersisted_count: u64,
    pub necessary_secs: f64,
    pub opaque_secs: f64,
    pub reused_count: u64,
}

impl QueryTotals {
    fn line(&self) -> String {
        let mut parts = Vec::new();
        if self.same_count > 0 {
            parts.push(format!(
                "{} re-ran to the same value ({:.2}s)",
                self.same_count, self.same_secs
            ));
        }
        if self.unpersisted_count > 0 {
            parts.push(format!(
                "{} proven unchanged but not persisted ({:.2}s)",
                self.unpersisted_count, self.unpersisted_secs
            ));
        }
        format!(
            "{:>7.2}s {:<40} {}",
            self.avoidable_secs,
            self.query,
            parts.join(", ")
        )
    }
}

fn by_query<'a>(rows: impl Iterator<Item = &'a [Row]>) -> Vec<QueryTotals> {
    let mut m: BTreeMap<&str, QueryTotals> = BTreeMap::new();
    for rs in rows {
        for r in rs {
            if is_evidence(r) {
                continue;
            }
            let q = m.entry(r.query.as_str()).or_insert_with(|| QueryTotals {
                query: r.query.clone(),
                ..Default::default()
            });
            match r.outcome.as_str() {
                "same" => {
                    q.same_secs += r.secs;
                    q.same_count += r.count;
                }
                "green_recompute" => {
                    q.unpersisted_secs += r.secs;
                    q.unpersisted_count += r.count;
                }
                "loaded" => q.reused_count += r.count,
                o => match class(o) {
                    Class::Necessary => q.necessary_secs += r.secs,
                    _ => q.opaque_secs += r.secs,
                },
            }
            q.avoidable_secs = q.same_secs + q.unpersisted_secs;
        }
    }
    let mut v: Vec<_> = m.into_values().collect();
    v.sort_by(|a, b| b.avoidable_secs.total_cmp(&a.avoidable_secs));
    v
}

/// `justrust reuse`: across recorded runs, the compile work that was not
/// reused, ranked by what a finer-grained design would have saved.
pub fn command(days: f64, here: bool, top: usize, json: bool) -> Result<()> {
    let dir = crate::paths::runs_dir()?;
    let cutoff = crate::paths::now() - days * 86400.0;
    let cwd = std::env::current_dir().ok();
    let mut runs = 0usize;
    let mut ledger_runs = 0usize;
    let mut unit_count = 0usize;
    let mut ledger_units = 0usize;
    let mut uncovered_secs = 0.0;
    let mut all = Totals::default();
    let mut crates: BTreeMap<String, (Totals, f64, usize)> = BTreeMap::new();
    let mut cascade = (0.0f64, 0usize);
    let mut rows_all: Vec<Vec<Row>> = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }
        let path = entry.path();
        let Ok(raw) = std::fs::read(path.join("summary.json")) else {
            continue;
        };
        let Ok(s) = serde_json::from_slice::<crate::summary::Summary>(&raw) else {
            continue;
        };
        if s.start < cutoff {
            continue;
        }
        if here
            && let Some(cwd) = &cwd
            && !std::path::Path::new(&s.cwd).starts_with(cwd)
        {
            continue;
        }
        runs += 1;
        let cascaded: std::collections::BTreeSet<&str> = s
            .rebuild_reasons
            .iter()
            .filter(|r| {
                r.reason.starts_with("the dependency") || r.reason.contains("info of dependency")
            })
            .map(|r| r.package.as_str())
            .collect();
        let units = crate::summary::load_units(&path);
        let mut any = false;
        for u in units.iter().filter(|u| u.local && !u.build_script) {
            unit_count += 1;
            if u.reuse.is_empty() {
                uncovered_secs += u.wall;
                continue;
            }
            any = true;
            ledger_units += 1;
            let mut t = Totals::default();
            t.add_unit(&u.reuse);
            all.add_unit(&u.reuse);
            if cascaded.contains(u.package.as_str()) {
                cascade.0 += t.avoidable_secs;
                cascade.1 += 1;
            }
            let c = crates.entry(unit_label(u)).or_default();
            c.0.add_unit(&u.reuse);
            c.1 += u.wall;
            c.2 += 1;
            rows_all.push(u.reuse.clone());
        }
        ledger_runs += any as usize;
    }
    let queries = by_query(rows_all.iter().map(Vec::as_slice));
    if json {
        let crates: Vec<_> = crates
            .iter()
            .map(|(k, (t, wall, n))| serde_json::json!({"unit": k, "builds": n, "wall": wall, "totals": t}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "days": days, "runs": runs, "ledger_runs": ledger_runs,
                "local_units": unit_count, "ledger_units": ledger_units,
                "uncovered_wall_secs": uncovered_secs, "totals": all,
                "cascade_avoidable_secs": cascade.0, "cascade_units": cascade.1,
                "crates": crates, "queries": queries.iter().take(top * 4).collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }
    let mut o = String::new();
    let _ = writeln!(
        o,
        "Incremental reuse over the last {days} days: {runs} runs, {ledger_runs} with a reuse ledger"
    );
    let _ = writeln!(
        o,
        "  local units: {ledger_units} of {unit_count} have a ledger; {uncovered_secs:.0}s of local compile wall has none (compiler without the ledger)"
    );
    if ledger_units == 0 {
        let _ = writeln!(
            o,
            "  nothing to report yet: the reuse ledger needs a jr-rustc toolchain that writes it"
        );
        print!("{o}");
        return Ok(());
    }
    let _ = writeln!(
        o,
        "\n  query seconds summed over threads: reused {:.0}s ({} loads), recomputed {:.0}s",
        all.reused_secs,
        all.reused_count,
        all.recomputed_secs()
    );
    let _ = writeln!(
        o,
        "    avoidable {:.0}s ({:.0}% of recomputed): {:.0}s re-ran to the same value, {:.0}s proven unchanged but not persisted",
        all.avoidable_secs,
        100.0 * all.avoidable_share(),
        all.avoidable_secs - all.unpersisted_secs,
        all.unpersisted_secs
    );
    let _ = writeln!(
        o,
        "    opaque {:.0}s (unhashed or eval-always), necessary {:.0}s (changed or new values)",
        all.opaque_secs, all.necessary_secs
    );
    let _ = writeln!(o, "    {}", all.session_counts());
    let _ = writeln!(
        o,
        "    codegen units: {} reused, {} recompiled",
        all.cgu_reused, all.cgu_recompiled
    );
    if cascade.1 > 0 {
        let _ = writeln!(
            o,
            "    {:.0}s of avoidable work was in {} rebuilds caused only by a dependency rebuild",
            cascade.0, cascade.1
        );
    }
    let _ = writeln!(o, "\n  By unit, most avoidable first");
    let _ = writeln!(
        o,
        "  {:<30} {:>6} {:>9} {:>10} {:>8} {:>10} {:>9}",
        "UNIT", "BUILDS", "WALL", "AVOIDABLE", "OPAQUE", "NECESSARY", "CGU REUSE"
    );
    let mut cs: Vec<_> = crates.into_iter().collect();
    cs.sort_by(|a, b| b.1.0.avoidable_secs.total_cmp(&a.1.0.avoidable_secs));
    for (name, (t, wall, n)) in cs.iter().take(top) {
        let cgu = t.cgu_reused + t.cgu_recompiled;
        let _ = writeln!(
            o,
            "  {:<30} {:>6} {:>8.0}s {:>9.0}s {:>7.0}s {:>9.0}s {:>8.0}%",
            name,
            n,
            wall,
            t.avoidable_secs,
            t.opaque_secs,
            t.necessary_secs,
            if cgu > 0 {
                100.0 * t.cgu_reused as f64 / cgu as f64
            } else {
                0.0
            }
        );
    }
    let _ = writeln!(o, "\n  By query, most avoidable first");
    for q in queries.iter().take(top * 2) {
        let _ = writeln!(o, "  {}", q.line());
    }
    let _ = writeln!(
        o,
        "\n  avoidable: work a design with exact change tracking and complete value retention would skip. Seconds are exclusive per query and summed over frontend threads, so they exceed wall time on parallel builds."
    );
    print!("{o}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_classify() {
        let rows = parse(
            "same\ttypeck\t1.5\t10\nloaded\ttypeck\t0.2\t900\ngreen_recompute\toptimized_mir\t0.5\t3\nchanged\ttypeck\t0.3\t1\nnohash\thir_crate\t0.1\t1\nloaded\t<cgu>\t0\t14\nchanged\t<cgu>\t0\t2\nbad line\n",
        );
        assert_eq!(rows.len(), 7);
        let mut t = Totals::default();
        t.add(&rows);
        assert_eq!(t.reused_count, 900);
        assert!((t.avoidable_secs - 2.0).abs() < 1e-9);
        assert!((t.unpersisted_secs - 0.5).abs() < 1e-9);
        assert!((t.opaque_secs - 0.1).abs() < 1e-9);
        assert!((t.necessary_secs - 0.3).abs() < 1e-9);
        assert_eq!((t.cgu_reused, t.cgu_recompiled), (14, 2));
        assert!((t.avoidable_share() - 2.0 / 2.4).abs() < 1e-9);
        let q = by_query(std::iter::once(rows.as_slice()));
        assert_eq!(q[0].query, "typeck");
        assert_eq!(q[0].same_count, 10);
        assert_eq!(q[0].reused_count, 900);
    }

    #[test]
    fn cold_units_are_counted_separately() {
        let cold = parse(
            "cold_no_prior\t<session>\t0\t1\nnew\ttypeck\t2.0\t50\neval_always\tanalysis\t0.5\t1\nnew\t<cgu>\t0\t4\n",
        );
        let warm = parse("loaded\ttypeck\t0.1\t49\nchanged\ttypeck\t0.2\t1\n");
        assert!(is_cold(&cold) && !is_cold(&warm) && !is_cold(&[]));
        let mut t = Totals::default();
        t.add_unit(&cold);
        t.add_unit(&warm);
        assert_eq!(t.cold_units, 1);
        assert!((t.cold_secs - 2.5).abs() < 1e-9);
        assert!((t.necessary_secs - 2.2).abs() < 1e-9);
    }

    #[test]
    fn starts_require_positive_evidence() {
        let mut totals = Totals::default();
        totals.add_unit(&[]);
        for (outcome, query, count, expected) in [
            ("warm", "<session>", 1, SessionState::Warm),
            ("loaded", "<cgu>", 2, SessionState::Warm),
            ("cold_io_error", "<session>", 1, SessionState::Cold),
            ("cold_out_of_date", "<session>", 1, SessionState::Cold),
            ("cold_future", "<session>", 1, SessionState::Unknown),
            ("cold_no_prior", "<session>", 0, SessionState::Unknown),
            ("loaded", "typeck", 0, SessionState::Unknown),
            ("new", "typeck", 5, SessionState::Unknown),
            ("present", "<sibling_state>", 1, SessionState::Unknown),
        ] {
            let rows = parse(&format!("{outcome}\t{query}\t0\t{count}"));
            assert!(
                session_state(&rows) == expected,
                "{outcome} {query} {count}"
            );
            totals.add_unit(&rows);
        }
        assert_eq!(
            totals.session_counts(),
            "incremental starts (ledger units): 2 cold, 2 warm, 5 unknown"
        );
    }

    #[test]
    fn render_run_is_empty_without_ledger() {
        assert!(render_run(&[crate::shim::Unit::default()]).is_empty());
        let u = crate::shim::Unit {
            crate_name: "a".into(),
            wall: 3.0,
            reuse: parse("same\ttypeck\t1.0\t2\n"),
            ..Default::default()
        };
        let out = render_run(&[u]);
        assert!(out.contains("avoidable 1.00s (100%)"), "{out}");
        assert!(out.contains("2 re-ran to the same value"), "{out}");
    }
}
