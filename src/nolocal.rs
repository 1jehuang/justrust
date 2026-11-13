//! `justrust local hosted-first|on|off|status`: where compiles may run on
//! this machine.
//!
//! - **hosted-first** (the default, no file): for an account signed in to
//!   hosted builds, every build hosted can run goes there, whatever the
//!   history says, and a hosted failure is *not* answered with a local
//!   compile. The client prints what went wrong and how to act on it
//!   ([`triage`]). An explicit `--local="<why>"` still compiles here, and so
//!   do commands hosted cannot serve (`run`, an unsupported target).
//! - **on** (`~/.justrust/local-on`): the router may keep a build local
//!   when that is predicted faster, and a hosted failure falls back to a
//!   local build.
//! - **off** (`~/.justrust/no-local`, its text is the reason): strict.
//!   - every routed build is forced remote and never falls back to local
//!     (`route::remote_required`), including builds through the `cargo`
//!     proxy from plain shells and an explicit `--local`;
//!   - the `rustc` shim refuses to compile a crate locally, which stops
//!     anything else that reaches rustc through justrust. `rustc -vV` style
//!     probes still pass;
//!   - the resident build daemon keeps running: its builds and prebuilds
//!     are routed like any other, so they compile on hosted machines.
//!
//! `JUSTRUST_LOCAL=hosted-first|on|off` overrides the files for one command.
//!
//! Hosted builds run their rustc on the build machine (with
//! `JUSTRUST_ON_REMOTE_AGENT` set there), so they are unaffected.

use anyhow::Result;
use std::path::{Path, PathBuf};

const OFF_FILE: &str = "no-local";
const ON_FILE: &str = "local-on";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Policy {
    HostedFirst,
    On,
    Off,
}

fn parse(v: &str) -> Option<Policy> {
    match v.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" | "allow" | "enable" => Some(Policy::On),
        "off" | "0" | "false" | "no" | "never" | "disable" => Some(Policy::Off),
        "hosted-first" | "hosted_first" | "hosted" | "default" => Some(Policy::HostedFirst),
        _ => None,
    }
}

/// The policy from an override (`JUSTRUST_LOCAL`) and the two flag files.
/// Strict wins when both files exist: the safer reading of a torn state.
fn policy_from(env: Option<&str>, off_file: bool, on_file: bool) -> Policy {
    if let Some(p) = env.and_then(parse) {
        return p;
    }
    if off_file {
        Policy::Off
    } else if on_file {
        Policy::On
    } else {
        Policy::HostedFirst
    }
}

fn home() -> Option<PathBuf> {
    crate::paths::home().ok()
}

/// Two stats, no parse. The build machine and unit tests always compile.
pub fn policy() -> Policy {
    if cfg!(test) || std::env::var_os(crate::route::ON_AGENT).is_some() {
        return Policy::On;
    }
    let home = home();
    let has = |name: &str| home.as_ref().is_some_and(|h| h.join(name).exists());
    policy_from(
        std::env::var("JUSTRUST_LOCAL").ok().as_deref(),
        has(OFF_FILE),
        has(ON_FILE),
    )
}

/// Local compiles are forbidden outright (`justrust local off`).
pub fn disabled() -> bool {
    policy() == Policy::Off
}

pub fn reason() -> Option<String> {
    let text = std::fs::read_to_string(home()?.join(OFF_FILE)).ok()?;
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub fn refusal(what: &str) -> String {
    format!(
        "justrust: local compiles are disabled ({}); refusing {what}. \
Builds go to hosted machines. Re-enable with `justrust local on`.",
        reason().unwrap_or_else(|| "justrust local off".into())
    )
}

/// Whose problem a hosted failure is, as far as the client can tell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fault {
    /// The account: no plan, builds used up, a limit, signed out.
    Billing,
    /// This project or invocation, before the service was reached.
    ClientSide,
    /// Everything else: the network between here and the service, or the
    /// service itself.
    Service,
}

/// Hosted failure messages end with a "Builds run locally ..." sentence,
/// written for the fallback. When the fallback is refused that sentence is
/// false, so it is dropped and the rest of the message kept.
pub fn without_local_promise(message: &str) -> String {
    const PROMISE: &str = "Builds run locally";
    let Some(start) = message.find(PROMISE) else {
        return message.to_owned();
    };
    let rest = &message[start..];
    let after = rest.find('.').map_or("", |end| &rest[end + 1..]);
    format!("{}{}", message[..start].trim_end(), after)
}

/// What to do when a hosted build produced no result and no local compile
/// replaces it. Written for the agent that reads the build output: fix what
/// is fixable from here, report what is ours, and go local last.
pub fn triage(fault: Fault, strict: bool) -> String {
    let first = match fault {
        Fault::Billing => {
            "  1. This is an account limit. Relay the line above to your user: billing is their decision.".to_string()
        }
        Fault::ClientSide => {
            "  1. The cause is in this project or invocation, not the service. Fix it and rerun.".to_string()
        }
        Fault::Service => {
            "  1. Find the cause first. `justrust remote status`, `justrust log --tail 200` and a rerun \
tell a local problem (network, sign-in, a path outside the synced roots, a full disk here) from a hosted one. \
Fix what you can and rerun."
                .to_string()
        }
    };
    let report = "  2. If the hosted service itself is at fault, report it: \
`justrust feedback --kind infra-urgent \"<the command, the error line, what you tried>\"` \
when it blocks your work (this pages the maintainer), `--kind infra` when it does not.";
    let last = if strict {
        "  3. Local compiles are off on this machine (`justrust local status`). \
Only your user can turn them back on."
    } else {
        "  3. Only when it cannot be fixed from here, rerun with `--local=\"<why>\"`. \
`justrust local on` restores automatic local fallback."
    };
    format!(
        "justrust: no local compile replaces this hosted build. Next steps:\n{first}\n{report}\n{last}"
    )
}

pub fn command(action: &str, reason: Option<String>) -> Result<()> {
    let home = home().ok_or_else(|| anyhow::anyhow!("no justrust home"))?;
    command_at(&home, action, reason)
}

fn remove(f: &Path) -> Result<()> {
    match std::fs::remove_file(f) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn command_at(home: &Path, action: &str, reason: Option<String>) -> Result<()> {
    let off = home.join(OFF_FILE);
    let on = home.join(ON_FILE);
    match action {
        "off" | "disable" => {
            std::fs::create_dir_all(home)?;
            std::fs::write(
                &off,
                reason.unwrap_or_else(|| "disabled by user".into()) + "\n",
            )?;
            remove(&on)?;
            println!(
                "local compiles: OFF. All builds go to hosted machines, no local fallback, `--local` is refused."
            );
            println!("back to the default: justrust local hosted-first");
        }
        "on" | "enable" => {
            std::fs::create_dir_all(home)?;
            std::fs::write(&on, "enabled by user\n")?;
            remove(&off)?;
            println!(
                "local compiles: ON. Builds may run here when predicted faster, and a hosted failure falls back to a local build."
            );
            println!("back to the default: justrust local hosted-first");
        }
        "hosted-first" | "hosted" | "default" | "reset" => {
            remove(&off)?;
            remove(&on)?;
            println!(
                "local compiles: HOSTED-FIRST (default). Signed-in builds go to hosted machines with no automatic local fallback. `--local=\"<why>\"` compiles here on request."
            );
        }
        "status" | "" => match policy_from(None, off.exists(), on.exists()) {
            Policy::Off => println!(
                "local compiles: OFF ({})",
                std::fs::read_to_string(&off)
                    .ok()
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "no reason given".into())
            ),
            Policy::On => println!("local compiles: ON (router and local fallback allowed)"),
            Policy::HostedFirst => println!(
                "local compiles: HOSTED-FIRST (default: hosted when signed in, no automatic local fallback, `--local=\"<why>\"` on request)"
            ),
        },
        other => anyhow::bail!("unknown action `{other}`: use hosted-first, on, off or status"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_fallback_does_not_promise_a_local_build() {
        for (message, want) in [
            (
                "could not reach https://x: curl: (7) refused. Builds run locally.",
                "could not reach https://x: curl: (7) refused.",
            ),
            (
                "out of credits. Builds run locally until credits renew. There are no automatic top-ups.",
                "out of credits. There are no automatic top-ups.",
            ),
            (
                "the fleet is at capacity. Builds run locally and remote is retried in a few minutes. (server: busy)",
                "the fleet is at capacity. (server: busy)",
            ),
            ("no promise here.", "no promise here."),
        ] {
            assert_eq!(without_local_promise(message), want);
        }
        for message in [
            crate::remote_hosted::SUBSCRIBE,
            crate::remote_hosted::START_TRIAL,
            crate::remote_hosted::UPGRADE_PLAN,
            crate::remote_hosted::RAISE_LIMIT,
            crate::remote_hosted::END_TRIAL,
            crate::remote_hosted::NO_CREDITS,
            crate::remote_hosted::TIME_EXHAUSTED,
            crate::remote_hosted::UNAVAILABLE,
            crate::remote_hosted::NOT_OFFERED,
            crate::remote_hosted::AT_CAPACITY,
        ] {
            let kept = without_local_promise(message);
            assert!(!kept.contains("locally"), "{kept}");
            assert!(kept.len() > 20 && kept.ends_with('.'), "{kept}");
        }
    }

    #[test]
    fn default_is_hosted_first_and_files_or_env_change_it() {
        assert_eq!(policy_from(None, false, false), Policy::HostedFirst);
        assert_eq!(policy_from(None, false, true), Policy::On);
        assert_eq!(policy_from(None, true, false), Policy::Off);
        // A torn state reads as the strict one.
        assert_eq!(policy_from(None, true, true), Policy::Off);
        // The override wins over the files, an unknown value does not.
        assert_eq!(policy_from(Some("on"), true, false), Policy::On);
        assert_eq!(policy_from(Some("OFF"), false, true), Policy::Off);
        assert_eq!(
            policy_from(Some("hosted-first"), true, true),
            Policy::HostedFirst
        );
        assert_eq!(policy_from(Some("sideways"), false, true), Policy::On);
    }

    #[test]
    fn toggles_flag_files() {
        let dir = std::env::temp_dir().join(format!("jr-nolocal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (off, on) = (dir.join(OFF_FILE), dir.join(ON_FILE));
        command_at(&dir, "off", Some("battery".into())).unwrap();
        assert!(off.exists() && !on.exists());
        assert_eq!(std::fs::read_to_string(&off).unwrap().trim(), "battery");
        command_at(&dir, "status", None).unwrap();
        command_at(&dir, "on", None).unwrap();
        assert!(!off.exists() && on.exists());
        command_at(&dir, "on", None).unwrap();
        command_at(&dir, "off", None).unwrap();
        assert!(off.exists() && !on.exists());
        command_at(&dir, "hosted-first", None).unwrap();
        assert!(!off.exists() && !on.exists());
        command_at(&dir, "default", None).unwrap();
        command_at(&dir, "status", None).unwrap();
        assert!(command_at(&dir, "sideways", None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn triage_orders_fix_report_then_local() {
        let service = triage(Fault::Service, false);
        let fix = service.find("Find the cause first").unwrap();
        let report = service.find("--kind infra-urgent").unwrap();
        let local = service.find("--local=").unwrap();
        assert!(fix < report && report < local, "{service}");
        assert!(triage(Fault::Billing, false).contains("billing is their decision"));
        assert!(triage(Fault::ClientSide, false).contains("Fix it and rerun"));
        // Strict machines are never told to pass a flag that is refused.
        let strict = triage(Fault::Service, true);
        assert!(!strict.contains("--local="), "{strict}");
        assert!(strict.contains("justrust local status"));
    }
}
