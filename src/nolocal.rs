//! `justrust local off|on|status`: a persistent switch that forbids local
//! compiles on this machine until it is turned back on (FINDINGS: battery).
//!
//! While off, `~/.justrust/no-local` exists (its text is the reason) and:
//! - every routed build is forced remote and never falls back to local
//!   (`route::remote_required`), including builds through the `cargo` proxy
//!   from plain shells;
//! - the `rustc` shim refuses to compile a crate locally, which also stops
//!   the resident build daemon's prebuilds and anything else that reaches
//!   rustc through justrust. `rustc -vV` style probes still pass.
//!
//! Hosted builds run their rustc on the build machine (with
//! `JUSTRUST_ON_REMOTE_AGENT` set there), so they are unaffected.

use anyhow::Result;
use std::path::PathBuf;

fn flag_file() -> Option<PathBuf> {
    crate::paths::home().ok().map(|h| h.join("no-local"))
}

/// Local compiles are forbidden (`justrust local off`). One stat, no parse.
pub fn disabled() -> bool {
    std::env::var_os(crate::route::ON_AGENT).is_none()
        && flag_file().is_some_and(|f| f.exists())
}

pub fn reason() -> Option<String> {
    let text = std::fs::read_to_string(flag_file()?).ok()?;
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

pub fn command(action: &str, reason: Option<String>) -> Result<()> {
    let f = flag_file().ok_or_else(|| anyhow::anyhow!("no justrust home"))?;
    command_at(&f, action, reason)
}

fn command_at(f: &std::path::Path, action: &str, reason: Option<String>) -> Result<()> {
    let read_reason = || {
        std::fs::read_to_string(f)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    match action {
        "off" | "disable" => {
            if let Some(d) = f.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(f, reason.unwrap_or_else(|| "disabled by user".into()) + "\n")?;
            println!("local compiles: OFF. All builds go to hosted machines, no local fallback.");
            println!("turn back on: justrust local on");
        }
        "on" | "enable" => {
            match std::fs::remove_file(f) {
                Ok(()) => println!("local compiles: ON"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("local compiles: already ON")
                }
                Err(e) => return Err(e.into()),
            }
        }
        "status" | "" => {
            if f.exists() {
                println!(
                    "local compiles: OFF ({})",
                    read_reason().unwrap_or_else(|| "no reason given".into())
                );
            } else {
                println!("local compiles: ON");
            }
        }
        other => anyhow::bail!("unknown action `{other}`: use off, on or status"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_flag_file() {
        let dir = std::env::temp_dir().join(format!("jr-nolocal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("no-local");
        command_at(&f, "off", Some("battery".into())).unwrap();
        assert!(f.exists());
        assert_eq!(std::fs::read_to_string(&f).unwrap().trim(), "battery");
        command_at(&f, "status", None).unwrap();
        command_at(&f, "on", None).unwrap();
        assert!(!f.exists());
        command_at(&f, "on", None).unwrap();
        assert!(command_at(&f, "sideways", None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
