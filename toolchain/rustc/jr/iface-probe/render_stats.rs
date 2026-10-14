//! Always-on, near-zero-cost attribution of view renders.
//!
//! GPUI's frame histograms say *that* a frame was drawn and how long it took,
//! not *which* views rebuilt their element trees. Each instrumented
//! `Render::render` opens a [`scope`]: two `Instant::now()` calls and a short
//! linear scan of a thread-local table. Live profile samples report deltas of
//! this table, so an idle window that keeps redrawing names its culprit, and a
//! slow render is logged with the view responsible.
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// A render slower than this is logged (rate limited) with its view name.
const SLOW_RENDER: Duration = Duration::from_millis(12);
const SLOW_LOG_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewStat {
    pub renders: u64,
    pub nanos: u64,
    pub max_nanos: u64,
}

#[derive(Default)]
struct Table {
    views: Vec<(&'static str, ViewStat)>,
    /// Why the UI woke: bridge updates, timers, and other notify sources.
    causes: Vec<(&'static str, u64)>,
    last_slow_log: Option<Instant>,
    suppressed_slow: u64,
}

thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(Table::default());
}

pub struct Scope {
    name: &'static str,
    started: Instant,
}

/// Times one render of `name` until the returned guard drops.
#[must_use]
pub fn scope(name: &'static str) -> Scope {
    Scope {
        name,
        started: Instant::now(),
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        record(self.name, self.started.elapsed());
    }
}

fn record(name: &'static str, elapsed: Duration) {
    let nanos = elapsed.as_nanos().min(u64::MAX as u128) as u64;
    TABLE.with(|table| {
        let Ok(mut table) = table.try_borrow_mut() else {
            return;
        };
        let stat = match table.views.iter().position(|(n, _)| *n == name) {
            Some(index) => &mut table.views[index].1,
            None => {
                table.views.push((name, ViewStat::default()));
                &mut table.views.last_mut().unwrap().1
            }
        };
        stat.renders += 1; // ANCHOR
        stat.nanos += nanos;
        stat.max_nanos = stat.max_nanos.max(nanos);
        if elapsed >= SLOW_RENDER {
            let now = Instant::now();
            if table
                .last_slow_log
                .is_none_or(|at| now.duration_since(at) >= SLOW_LOG_INTERVAL)
            {
                let suppressed = std::mem::take(&mut table.suppressed_slow);
                table.last_slow_log = Some(now);
                eprintln!(
                    "jcode-desktop perf: slow render {name} {:.1} ms (+{suppressed} more slow renders since last report)",
                    elapsed.as_secs_f64() * 1_000.0
                );
            } else {
                table.suppressed_slow += 1;
            }
        }
    });
}

/// Cumulative per-view totals on this (the UI) thread.
pub fn snapshot() -> Vec<(&'static str, ViewStat)> {
    TABLE.with(|table| {
        table
            .try_borrow()
            .map(|table| table.views.clone())
            .unwrap_or_default()
    })
}

/// Counts one occurrence of a wake or notify cause, such as a bridge update
/// kind. Cheap enough for every event.
pub fn note(cause: &'static str) {
    TABLE.with(|table| {
        let Ok(mut table) = table.try_borrow_mut() else {
            return;
        };
        match table.causes.iter_mut().find(|(name, _)| *name == cause) {
            Some((_, count)) => *count += 1,
            None => table.causes.push((cause, 1)),
        }
    });
}

/// Cumulative cause counts on this (the UI) thread.
pub fn causes() -> Vec<(&'static str, u64)> {
    TABLE.with(|table| {
        table
            .try_borrow()
            .map(|table| table.causes.clone())
            .unwrap_or_default()
    })
}

/// Counts a cause named by a value's `Debug` variant name, e.g. `TextDelta`
/// from `TextDelta { .. }`. Names are interned once; variant sets are finite.
pub fn note_variant(prefix: &'static str, value: &impl std::fmt::Debug) {
    thread_local! {
        static NAMES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    }
    /// Collects the leading identifier, then aborts formatting so a large
    /// payload (history, tool output) is never rendered to a string.
    struct Head(String);
    impl std::fmt::Write for Head {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for c in s.chars() {
                if !(c.is_alphanumeric() || c == '_') || self.0.len() >= 48 {
                    return Err(std::fmt::Error);
                }
                self.0.push(c);
            }
            Ok(())
        }
    }
    let mut head = Head(String::from(prefix));
    let _ = std::fmt::write(&mut head, format_args!("{value:?}"));
    let name = head.0;
    let interned = NAMES.with(|names| {
        let mut names = names.borrow_mut();
        match names.iter().find(|known| **known == name) {
            Some(known) => *known,
            None if names.len() < 256 => {
                let leaked: &'static str = Box::leak(name.into_boxed_str());
                names.push(leaked);
                leaked
            }
            None => "other",
        }
    });
    note(interned);
}

/// Causes that occurred between two [`causes`] snapshots, most frequent first.
pub fn cause_delta(
    before: &[(&'static str, u64)],
    after: &[(&'static str, u64)],
) -> Vec<(&'static str, u64)> {
    let mut changes = after
        .iter()
        .filter_map(|(name, count)| {
            let previous = before
                .iter()
                .find(|(n, _)| n == name)
                .map_or(0, |(_, c)| *c);
            (count > &previous).then_some((*name, count - previous))
        })
        .collect::<Vec<_>>();
    changes.sort_by(|a, b| b.1.cmp(&a.1));
    changes
}

/// Per-view activity between two snapshots, busiest first. `max_nanos` is the
/// cumulative maximum, so it is reported only for views that rendered.
pub fn delta(
    before: &[(&'static str, ViewStat)],
    after: &[(&'static str, ViewStat)],
) -> Vec<(&'static str, ViewStat)> {
    let mut changes = after
        .iter()
        .filter_map(|(name, now)| {
            let previous = before
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, stat)| *stat)
                .unwrap_or_default();
            let renders = now.renders.saturating_sub(previous.renders);
            (renders > 0).then_some((
                *name,
                ViewStat {
                    renders,
                    nanos: now.nanos.saturating_sub(previous.nanos),
                    max_nanos: now.max_nanos,
                },
            ))
        })
        .collect::<Vec<_>>();
    changes.sort_by(|a, b| b.1.nanos.cmp(&a.1.nanos));
    changes
}

/// Per-entity render counts for cache tests. Views record here, not in a
/// workspace test module, so views do not depend on the workspace.
#[cfg(any(test, feature = "test-support"))]
pub mod test_renders {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static RENDERS: RefCell<HashMap<gpui::EntityId, usize>> = RefCell::new(HashMap::new());
    }

    pub fn record(id: gpui::EntityId) {
        RENDERS.with_borrow_mut(|renders| *renders.entry(id).or_default() += 1);
    }

    pub fn count(id: gpui::EntityId) -> usize {
        RENDERS.with_borrow(|renders| renders.get(&id).copied().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_accumulate_per_view_and_delta_reports_only_active_views() {
        let before = snapshot();
        record("TestViewA", Duration::from_micros(300));
        record("TestViewA", Duration::from_micros(100));
        record("TestViewB", Duration::from_micros(50));
        let middle = snapshot();
        let changes = delta(&before, &middle);
        let a = changes.iter().find(|(n, _)| *n == "TestViewA").unwrap().1;
        assert_eq!(a.renders, 2);
        assert_eq!(a.nanos, 400_000);
        assert_eq!(changes[0].0, "TestViewA", "busiest view first");
        record("TestViewB", Duration::from_micros(20));
        let changes = delta(&middle, &snapshot());
        assert!(changes.iter().all(|(n, _)| *n != "TestViewA"));
        assert_eq!(
            changes
                .iter()
                .find(|(n, _)| *n == "TestViewB")
                .unwrap()
                .1
                .renders,
            1
        );
    }

    #[test]
    fn scope_guard_records_on_drop() {
        let before = snapshot();
        drop(scope("TestScopeGuard"));
        let changes = delta(&before, &snapshot());
        assert_eq!(changes[0].0, "TestScopeGuard");
        assert_eq!(changes[0].1.renders, 1);
    }

    #[test]
    fn causes_count_and_delta_only_new_occurrences() {
        let before = causes();
        note("test-cause-a");
        note("test-cause-a");
        note("test-cause-b");
        let changes = cause_delta(&before, &causes());
        assert_eq!(changes[0], ("test-cause-a", 2));
        assert!(changes.contains(&("test-cause-b", 1)));
    }
}
