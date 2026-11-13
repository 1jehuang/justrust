//! Which compiler builds use when a project pins none, decided by the hosted
//! control plane rather than compiled into this client.
//!
//! `GET <build base>/toolchain/current` answers `{"toolchain", "protocol"}`
//! (justrust-cloud `src/toolchain_policy.js`, `BUILD_TOOLCHAIN` in its
//! `wrangler.toml`). Changing it there switches every build machine on its
//! next build, and rolls back the same way, with no client release.
//!
//! `protocol` is the version of the private interface between justrust (the
//! shim, the cache keys, the test runner) and the toolchain (jr-cargo,
//! jr-rustc). Any justrust may now be handed any toolchain, so a justrust
//! that does not speak a toolchain's protocol refuses the build and says to
//! update, instead of driving a compiler it does not understand
//! ([`Policy::check`]).
//!
//! The answer is cached in `~/.justrust/toolchain-policy.json`. A fresh
//! cache costs no request, a stale one is used while a background request
//! refreshes it, and an unreachable control plane keeps the last answer, so
//! a build never waits on this beyond the first one on a new machine. With
//! no answer at all the compiled-in default applies
//! ([`crate::depcache_prebuilt::HOSTED_TOOLCHAIN`]).
//!
//! A project pin (`justrust.toml`, a `rust-toolchain` file) and
//! `JUSTRUST_TOOLCHAIN` always win over this. `JUSTRUST_TOOLCHAIN_POLICY=off`
//! ignores the control plane.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Toolchain protocols this justrust can drive. Raise the upper bound when
/// justrust learns a new one, the lower when it drops an old one.
pub const SUPPORTED: std::ops::RangeInclusive<u32> = 1..=1;
/// A cached answer younger than this is used without asking again.
const FRESH_SECS: f64 = 60.0;
const FILE: &str = "toolchain-policy.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub toolchain: String,
    pub protocol: u32,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    #[serde(flatten)]
    policy: Policy,
    fetched_at: f64,
}

impl Policy {
    /// Parse the control plane's answer. Only an exact pinned id is accepted:
    /// this names code the machine will download and run.
    fn parse(text: &str) -> Option<Policy> {
        let p: Policy = serde_json::from_str(text).ok()?;
        (crate::toolchain::Spec::parse(&p.toolchain).ok()?.id() == p.toolchain && p.protocol >= 1)
            .then_some(p)
    }

    /// Refuse a toolchain this justrust cannot drive.
    pub fn check(&self) -> Result<(), String> {
        if SUPPORTED.contains(&self.protocol) {
            return Ok(());
        }
        let (what, fix) = if self.protocol > *SUPPORTED.end() {
            ("too old", "Update justrust (`justrust update`) and retry")
        } else {
            (
                "too new",
                "Use the current release, or pin a toolchain this version supports",
            )
        };
        Err(format!(
            "justrust {} is {what} for the hosted toolchain {}: it speaks toolchain protocol {}, \
             this justrust supports {}-{}. {fix}.",
            crate::update::VERSION,
            self.toolchain,
            self.protocol,
            SUPPORTED.start(),
            SUPPORTED.end(),
        ))
    }
}

fn off() -> bool {
    std::env::var("JUSTRUST_TOOLCHAIN_POLICY")
        .is_ok_and(|v| matches!(v.trim(), "0" | "off" | "false" | "no"))
}

fn path() -> Option<PathBuf> {
    Some(crate::paths::home().ok()?.join(FILE))
}

fn read(path: &Path) -> Option<Cached> {
    let c: Cached = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    // Re-validate: the file is only as trusted as the answer was.
    Policy::parse(&serde_json::to_string(&c.policy).ok()?)?;
    Some(c)
}

fn write(path: &Path, policy: &Policy, now: f64) {
    let c = Cached {
        policy: policy.clone(),
        fetched_at: now,
    };
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    if let Ok(bytes) = serde_json::to_vec(&c)
        && std::fs::write(&tmp, bytes).is_ok()
        && std::fs::rename(&tmp, path).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn fetch() -> Option<Policy> {
    let url = format!("{}/toolchain/current", crate::remote_hosted::build_base());
    let out = Command::new("curl")
        .args([
            "-fsS",
            "--proto",
            "=https",
            "--connect-timeout",
            "2",
            "--max-time",
            "4",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| Policy::parse(&String::from_utf8_lossy(&out.stdout)))?
}

fn refresh(path: &Path) -> Option<Policy> {
    let p = fetch()?;
    write(path, &p, crate::paths::now());
    Some(p)
}

/// What to do with the cache at `now`.
#[derive(Debug, PartialEq)]
enum Plan {
    /// Use it as is.
    Fresh(Policy),
    /// Use it, and ask again in the background.
    Stale(Policy),
    /// Nothing known: ask now.
    Ask,
}

fn plan(cached: Option<Cached>, now: f64) -> Plan {
    match cached {
        Some(c) if (0.0..FRESH_SECS).contains(&(now - c.fetched_at)) => Plan::Fresh(c.policy),
        Some(c) => Plan::Stale(c.policy),
        None => Plan::Ask,
    }
}

/// The control plane's toolchain, for a build machine: asks when nothing is
/// known yet, otherwise never waits. `None` when it has never answered.
pub fn current() -> Option<Policy> {
    if off() {
        return None;
    }
    let path = path()?;
    match plan(read(&path), crate::paths::now()) {
        Plan::Fresh(p) => Some(p),
        Plan::Stale(p) => {
            std::thread::spawn(move || refresh(&path));
            Some(p)
        }
        Plan::Ask => refresh(&path),
    }
}

/// The last answer, without ever waiting on the network: for local builds
/// and status output. A stale answer is refreshed in the background.
pub fn known() -> Option<Policy> {
    if off() {
        return None;
    }
    let path = path()?;
    match plan(read(&path), crate::paths::now()) {
        Plan::Fresh(p) => Some(p),
        Plan::Stale(p) => {
            std::thread::spawn(move || refresh(&path));
            Some(p)
        }
        Plan::Ask => {
            std::thread::spawn(move || refresh(&path));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, protocol: u32) -> Policy {
        Policy {
            toolchain: id.into(),
            protocol,
        }
    }

    #[test]
    fn only_exact_pinned_ids_are_accepted() {
        assert_eq!(
            Policy::parse(r#"{"toolchain":"jr-ac20de5e7921","protocol":1}"#),
            Some(p("jr-ac20de5e7921", 1))
        );
        assert_eq!(
            Policy::parse(r#"{"toolchain":"nightly-2026-10-08","protocol":2}"#),
            Some(p("nightly-2026-10-08", 2))
        );
        for bad in [
            r#"{"toolchain":"stable","protocol":1}"#,
            r#"{"toolchain":"nightly","protocol":1}"#,
            r#"{"toolchain":" jr-ac20de5e7921","protocol":1}"#,
            r#"{"toolchain":"jr-ac20de5e7921/../x","protocol":1}"#,
            r#"{"toolchain":"jr-ac20de5e7921","protocol":0}"#,
            r#"{"toolchain":"jr-ac20de5e7921"}"#,
            r#"{"error":{"code":"toolchain_unpublished"}}"#,
            "",
        ] {
            assert_eq!(Policy::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_toolchain_this_justrust_cannot_drive_is_refused_with_the_fix() {
        assert_eq!(p("jr-ac20de5e7921", *SUPPORTED.end()).check(), Ok(()));
        let newer = p("jr-ac20de5e7921", SUPPORTED.end() + 1)
            .check()
            .unwrap_err();
        assert!(newer.contains("too old"), "{newer}");
        assert!(newer.contains("jr-ac20de5e7921"), "{newer}");
        assert!(newer.contains("`justrust update`"), "{newer}");
        assert!(newer.contains(crate::update::VERSION), "{newer}");
    }

    #[test]
    fn the_cache_answers_without_waiting_once_anything_is_known() {
        let c = |at| {
            Some(Cached {
                policy: p("jr-ac20de5e7921", 1),
                fetched_at: at,
            })
        };
        assert_eq!(plan(c(100.0), 130.0), Plan::Fresh(p("jr-ac20de5e7921", 1)));
        // Old, or written by a clock that was ahead: use it and ask again.
        assert_eq!(plan(c(100.0), 161.0), Plan::Stale(p("jr-ac20de5e7921", 1)));
        assert_eq!(plan(c(500.0), 130.0), Plan::Stale(p("jr-ac20de5e7921", 1)));
        assert_eq!(plan(None, 130.0), Plan::Ask);
    }

    #[test]
    fn the_cache_round_trips_and_rejects_a_tampered_id() {
        let d = std::env::temp_dir().join(format!("jr-tc-policy-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(FILE);
        write(&f, &p("jr-ac20de5e7921", 1), 42.0);
        let c = read(&f).unwrap();
        assert_eq!((c.policy, c.fetched_at), (p("jr-ac20de5e7921", 1), 42.0));
        std::fs::write(
            &f,
            r#"{"toolchain":"../../bin","protocol":1,"fetched_at":1}"#,
        )
        .unwrap();
        assert!(read(&f).is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
