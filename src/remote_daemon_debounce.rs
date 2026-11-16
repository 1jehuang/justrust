//! Bounded watcher batching. Events left in the channel form the next batch.
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const QUIET: Duration = Duration::from_millis(15);
const MAX_WAIT: Duration = Duration::from_millis(60);
const MAX_PATHS: usize = 4096;

pub(super) fn collect(first: PathBuf, rx: &Receiver<PathBuf>) -> BTreeSet<PathBuf> {
    let start = Instant::now();
    collect_with(first, || start.elapsed(), |wait| rx.recv_timeout(wait).ok())
}

fn collect_with(
    first: PathBuf,
    mut elapsed: impl FnMut() -> Duration,
    mut receive: impl FnMut(Duration) -> Option<PathBuf>,
) -> BTreeSet<PathBuf> {
    let mut touched = BTreeSet::from([first]);
    // A quiet period coalesces editor rename/chmod bursts, but unrelated writes
    // must not postpone source sync forever. Bound memory as well as time.
    while touched.len() < MAX_PATHS {
        let remaining = MAX_WAIT.saturating_sub(elapsed());
        if remaining.is_zero() {
            break;
        }
        let Some(path) = receive(QUIET.min(remaining)) else {
            break;
        };
        touched.insert(path);
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    #[test]
    fn continuous_duplicate_stream_stops_at_deadline() {
        let elapsed = Cell::new(Duration::ZERO);
        let calls = Cell::new(0);
        let touched = collect_with(
            "root/src/lib.rs".into(),
            || elapsed.get(),
            |wait| {
                assert!(wait <= QUIET);
                elapsed.set(elapsed.get() + Duration::from_millis(1));
                calls.set(calls.get() + 1);
                Some("unrelated/log".into())
            },
        );
        assert_eq!(elapsed.get(), MAX_WAIT);
        assert_eq!(calls.get(), 60);
        assert_eq!(touched.len(), 2);
        assert!(touched.contains(&PathBuf::from("root/src/lib.rs")));
    }

    #[test]
    fn burst_deduplicates_without_losing_rename_paths() {
        let mut events = VecDeque::from([
            Some(PathBuf::from("root/temp")),
            Some(PathBuf::from("root/src/lib.rs")),
            Some(PathBuf::from("root/src/lib.rs")),
            None,
            Some(PathBuf::from("next/burst")),
        ]);
        let touched = collect_with(
            "root/temp".into(),
            || Duration::ZERO,
            |wait| {
                assert_eq!(wait, QUIET);
                events.pop_front().flatten()
            },
        );
        assert_eq!(
            touched,
            BTreeSet::from([PathBuf::from("root/temp"), PathBuf::from("root/src/lib.rs"),])
        );
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn final_receive_is_limited_to_remaining_deadline() {
        let elapsed = Cell::new(Duration::from_millis(59));
        let touched = collect_with(
            "first".into(),
            || elapsed.get(),
            |wait| {
                assert_eq!(wait, Duration::from_millis(1));
                elapsed.set(MAX_WAIT);
                Some("last".into())
            },
        );
        assert_eq!(touched.len(), 2);
    }

    #[test]
    fn unique_stream_is_memory_bounded_without_dropping_next_event() {
        let mut events = (1..=MAX_PATHS).map(|n| PathBuf::from(n.to_string()));
        let touched = collect_with("0".into(), || Duration::ZERO, |_| events.next());
        assert_eq!(touched.len(), MAX_PATHS);
        assert_eq!(events.next(), Some(PathBuf::from(MAX_PATHS.to_string())));
    }

    #[test]
    fn disconnected_channel_preserves_first_event() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        assert_eq!(
            collect("first".into(), &rx),
            BTreeSet::from(["first".into()])
        );
    }
}
