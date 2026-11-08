//! Reliable delivery for account-private records (`justrust records`).
//!
//! A record is first written to `~/.justrust/records/pending/<kind>.<id>.json`
//! (one file per record, so re-recording the same id just replaces it), then
//! a detached `justrust __records` process sends every pending file. A file
//! is removed once the server stores it (2xx) or rejects it for good (400,
//! 413, 404). Offline, auth, rate-limit, and server failures keep it for the
//! next sender, which starts after any later build or benchmark. Records
//! older than `MAX_AGE` or beyond `MAX_PENDING` are dropped oldest first, so
//! a machine that is never online cannot fill its disk.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

pub const ARG: &str = "__records";
const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 3600);
const MAX_PENDING: usize = 2000;
/// One sender pass gives up after this many consecutive transient failures.
const MAX_FAILURES: usize = 3;

fn dir() -> Option<PathBuf> {
    let d = crate::paths::home().ok()?.join("records").join("pending");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn file_name(kind: &str, id: &str) -> String {
    let clean = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    format!("{}.{}.json", clean(kind), clean(id).trim_start_matches('.'))
}

/// Write one record body for delivery. Never fails the caller.
pub fn enqueue_in(d: &Path, kind: &str, id: &str, body: &Value) -> Option<PathBuf> {
    let path = d.join(file_name(kind, id));
    let tmp = d.join(format!(".{}.tmp", std::process::id()));
    std::fs::write(&tmp, body.to_string()).ok()?;
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// Queue a record and start a detached sender.
pub fn submit(kind: &str, id: &str, body: &Value) {
    let Some(d) = dir() else { return };
    if enqueue_in(&d, kind, id, body).is_some() {
        spawn_sender();
    }
}

pub fn spawn_sender() {
    if cfg!(test) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use crate::sys::Detach;
    let _ = std::process::Command::new(exe)
        .arg(ARG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .detached()
        .spawn();
}

/// What to do with a pending file after one POST.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    Rejected,
    Retry,
}

pub fn classify(code: Option<u16>) -> Outcome {
    match code {
        Some(200..=299) => Outcome::Delivered,
        Some(400 | 404 | 413 | 422) => Outcome::Rejected,
        _ => Outcome::Retry,
    }
}

/// Pending files, oldest first, after dropping expired and excess ones.
pub fn pending(d: &Path, now: SystemTime) -> Vec<PathBuf> {
    let mut files: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?;
            if name.starts_with('.') || !name.ends_with(".json") {
                // Stale temp files from a crashed writer.
                if name.ends_with(".tmp") && older(&p, now, Duration::from_secs(3600)) {
                    let _ = std::fs::remove_file(&p);
                }
                return None;
            }
            Some((e.metadata().ok()?.modified().ok()?, p))
        })
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(MAX_PENDING);
    let mut keep = Vec::new();
    for (i, (t, p)) in files.into_iter().enumerate() {
        let expired = now.duration_since(t).is_ok_and(|age| age > MAX_AGE);
        if i < excess || expired {
            let _ = std::fs::remove_file(&p);
        } else {
            keep.push(p);
        }
    }
    keep
}

fn older(p: &Path, now: SystemTime, limit: Duration) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|a| a > limit)
}

/// Send everything pending with `post`. Returns (delivered, rejected, left).
pub fn drain(d: &Path, post: &mut dyn FnMut(&Value) -> Option<u16>) -> (usize, usize, usize) {
    let files = pending(d, SystemTime::now());
    let (mut sent, mut rejected, mut failures) = (0, 0, 0);
    let total = files.len();
    for p in files {
        let Some(body) = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            let _ = std::fs::remove_file(&p);
            rejected += 1;
            continue;
        };
        // Replaced while sending: the new body is sent on the next pass.
        let before = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        match classify(post(&body)) {
            Outcome::Retry => {
                failures += 1;
                if failures >= MAX_FAILURES {
                    break;
                }
                continue;
            }
            outcome => {
                if std::fs::metadata(&p).and_then(|m| m.modified()).ok() == before {
                    let _ = std::fs::remove_file(&p);
                }
                if outcome == Outcome::Delivered {
                    sent += 1;
                } else {
                    rejected += 1;
                }
                failures = 0;
            }
        }
    }
    (sent, rejected, total - sent - rejected)
}

/// `justrust __records`: one sender at a time, lowest priority.
pub fn sender_main() -> ! {
    crate::sys::lower_priority();
    let Some(d) = dir() else {
        std::process::exit(0)
    };
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(d.join(".lock"))
        .ok()
        .filter(|f| f.try_lock().is_ok());
    if lock.is_some()
        && let Ok(c) = crate::remote_hosted::creds()
    {
        // Records queued while this pass runs are picked up by a second one.
        for _ in 0..2 {
            let (sent, _, _) = drain(&d, &mut |body| {
                crate::remote_hosted::request(&c, "POST", "build/records", Some(body))
                    .ok()
                    .map(|r| r.code)
            });
            if sent == 0 {
                break;
            }
        }
    }
    std::process::exit(0)
}

/// Number of records waiting for delivery (for `justrust records`).
pub fn pending_count() -> usize {
    dir().map_or(0, |d| pending(&d, SystemTime::now()).len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> PathBuf {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("jr-records-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn delivered_and_rejected_are_removed_transient_kept() {
        let d = scratch("drain");
        for id in ["a", "b", "c"] {
            enqueue_in(&d, "run", id, &json!({"id": id})).unwrap();
        }
        let mut seen = Vec::new();
        let (sent, rej, left) = drain(&d, &mut |b| {
            let id = b["id"].as_str().unwrap().to_string();
            seen.push(id.clone());
            match id.as_str() {
                "a" => Some(200),
                "b" => Some(413),
                _ => None,
            }
        });
        assert_eq!((sent, rej, left), (1, 1, 1));
        assert_eq!(seen.len(), 3);
        let rest = pending(&d, SystemTime::now());
        assert_eq!(rest, vec![d.join("run.c.json")]);
        // Back online: delivered.
        assert_eq!(drain(&d, &mut |_| Some(201)), (1, 0, 0));
        assert!(pending(&d, SystemTime::now()).is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn same_id_replaces_and_outage_stops_early() {
        let d = scratch("same");
        enqueue_in(&d, "bench", "x", &json!({"v": 1})).unwrap();
        enqueue_in(&d, "bench", "x", &json!({"v": 2})).unwrap();
        for i in 0..10 {
            enqueue_in(&d, "run", &format!("r{i}"), &json!({})).unwrap();
        }
        let mut calls = 0;
        drain(&d, &mut |_| {
            calls += 1;
            Some(503)
        });
        assert_eq!(calls, MAX_FAILURES);
        assert_eq!(pending(&d, SystemTime::now()).len(), 11);
        let v: Value =
            serde_json::from_slice(&std::fs::read(d.join("bench.x.json")).unwrap()).unwrap();
        assert_eq!(v["v"], 2);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn expired_records_are_dropped() {
        let d = scratch("age");
        let p = enqueue_in(&d, "run", "old", &json!({})).unwrap();
        enqueue_in(&d, "run", "new", &json!({})).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(SystemTime::now() - MAX_AGE - Duration::from_secs(60))
            .unwrap();
        assert_eq!(pending(&d, SystemTime::now()), vec![d.join("run.new.json")]);
        assert!(!p.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn ids_cannot_escape_the_queue() {
        for id in ["../../x", "a/b", "/etc/passwd"] {
            let n = file_name("run", id);
            assert!(!n.contains('/') && n.starts_with("run."), "{n}");
        }
    }

    #[test]
    fn classify_codes() {
        assert_eq!(classify(Some(200)), Outcome::Delivered);
        assert_eq!(classify(Some(413)), Outcome::Rejected);
        for c in [None, Some(401), Some(429), Some(500)] {
            assert_eq!(classify(c), Outcome::Retry);
        }
    }
}
