//! Where remote builds run. Everything above this module (sync daemon,
//! protocol, routing) only needs "an ssh-reachable machine with a shell".
//!
//! ```text
//! ~/.justrust/remote/backend.json   {"kind": "hosted"} | {"kind": "aws"}
//! ```
//!
//! - `hosted`: justrust's own build fleet, tied to a Jcode subscription
//!   (see `remote_hosted`). The one remote path going forward. The user
//!   there is `ubuntu` without general sudo: mirrored paths outside /home
//!   and /Users are created with `sudo -n /usr/local/sbin/jcode-mkdir`.
//! - `aws`: the machine `justrust remote up` creates in the user's own AWS
//!   account (see `remote`). Legacy: kept until hosted matches it, then
//!   removed. The default when that machine exists.
//!
//! A `backend.json` naming the removed `ssh` backend reads as no backend.
//!
//! With no backend configured nothing remote ever happens: the router costs
//! one failed `stat`.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

use crate::remote;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Backend {
    Aws,
    Hosted,
}

fn file() -> Result<PathBuf> {
    Ok(remote::dir()?.join("backend.json"))
}

/// The configured backend, or `None` when remote builds are not set up.
/// Cheap: used on every `justrust check`.
/// A `JCODE_API_KEY` in the environment (CI, headless boxes) selects hosted
/// when nothing else is configured, so an API key from jcode.sh/account works
/// without `justrust login`. The saved credentials file alone does not:
/// Jcode writes it too, and that should not turn remote builds on.
pub fn load() -> Option<Backend> {
    let home = crate::paths::home().ok()?.join("remote");
    let env_key = std::env::var("JCODE_API_KEY").is_ok_and(|k| !k.trim().is_empty());
    resolve(
        std::fs::read(home.join("backend.json")).ok(),
        home.join("state.json").exists(),
        env_key,
    )
}

fn resolve(backend_json: Option<Vec<u8>>, aws_state: bool, env_key: bool) -> Option<Backend> {
    match backend_json {
        Some(b) => serde_json::from_slice(&b).ok(),
        // Machines created before backend.json existed.
        None if aws_state => Some(Backend::Aws),
        None => env_key.then_some(Backend::Hosted),
    }
}

pub fn save(b: &Backend) -> Result<()> {
    std::fs::write(file()?, serde_json::to_vec_pretty(b)?)?;
    Ok(())
}

pub fn clear() -> Result<()> {
    match std::fs::remove_file(file()?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

impl Backend {
    pub fn label(&self) -> String {
        match self {
            Backend::Aws => remote::describe_short().unwrap_or_else(|| "aws".into()),
            Backend::Hosted => "hosted".into(),
        }
    }

    /// Make the machine reachable (starting it if needed) and return the
    /// ssh destination.
    pub fn ensure_ready(&self) -> Result<Target> {
        match self {
            Backend::Aws => {
                let ip = remote::ensure_running()?;
                Ok(Target {
                    dest: format!("{}@{ip}", remote::SSH_USER),
                    opts: remote::ssh_opts()?,
                })
            }
            Backend::Hosted => crate::remote_hosted::ensure_ready(),
        }
    }

    /// Short backend name for status output and JSON.
    pub fn kind(&self) -> &'static str {
        match self {
            Backend::Aws => "aws",
            Backend::Hosted => "hosted",
        }
    }

    /// Reachable right now without starting anything (for auto routing).
    pub fn probably_up(&self) -> bool {
        match self {
            Backend::Aws => remote::cached_ip().is_some(),
            Backend::Hosted => crate::remote_hosted::probably_up(),
        }
    }
}

/// An ssh destination plus the options every connection uses.
#[derive(Clone, Debug)]
pub struct Target {
    pub dest: String,
    pub opts: Vec<String>,
}

impl Target {
    pub fn command(&self) -> Command {
        let mut c = Command::new("ssh");
        c.args(&self.opts).arg(&self.dest);
        c
    }

    /// `ssh <opts>` for `rsync -e`.
    pub fn rsync_shell(&self) -> String {
        let mut s = String::from("ssh");
        for o in &self.opts {
            s.push(' ');
            s.push_str(&crate::remote_build::shell_quote(o));
        }
        s
    }
}

/// ssh ControlPath for connections from justrust. `%C` expands to 40 hex
/// characters, ssh binds the master first at the path plus a 17-character
/// random suffix (`.XXXXXXXXXXXXXXXX`), and a Unix socket path must stay
/// under 108 bytes, so a long JUSTRUST_HOME falls back to the per-user
/// runtime dir (then /tmp).
pub(crate) fn control_path(dir: &std::path::Path) -> String {
    let p = format!("{}/cm-%C", dir.display());
    if p.len() - 2 + 40 + 17 < 108 {
        return p;
    }
    let uid = crate::sys::user_tag();
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/justrust-{uid}")));
    let short = base.join("justrust-cm");
    let _ = std::fs::create_dir_all(&short);
    format!("{}/%C", short.display())
}

/// `justrust remote env`: the ssh destination of the current backend as
/// shell assignments, so scripts (bench/on-remote.sh) reach whichever
/// machine remote builds use without knowing keys or addresses.
pub fn env_command() -> Result<()> {
    let Some(b) = load() else {
        bail!("no remote machine configured. Sign in to hosted builds: `justrust login`");
    };
    let t = b.ensure_ready()?;
    print!("{}", env_lines(&b, &t));
    Ok(())
}

fn env_lines(b: &Backend, t: &Target) -> String {
    use crate::remote_build::shell_quote as q;
    format!(
        "JR_REMOTE_KIND={}\nJR_REMOTE_DEST={}\nJR_REMOTE_SSH={}\n",
        b.kind(),
        q(&t.dest),
        q(&t.rsync_shell())
    )
}

/// `justrust remote use <hosted|aws|off>`.
pub fn use_command(kind: &str) -> Result<()> {
    let b = match kind {
        "aws" => {
            if remote::instance_id().is_none() {
                println!("no AWS machine yet. Create one with `justrust remote up`.");
            }
            Backend::Aws
        }
        "ssh" => bail!(
            "the ssh backend was removed: use hosted builds (`justrust login`, then `justrust remote use hosted`)"
        ),
        "hosted" => {
            crate::remote_hosted::check_use()?;
            Backend::Hosted
        }
        "off" | "none" => {
            crate::remote_daemon::stop();
            clear()?;
            println!("remote builds off: every build runs locally");
            return Ok(());
        }
        other => bail!("unknown backend {other}: use hosted, aws, or off"),
    };
    crate::remote_daemon::stop();
    save(&b)?;
    println!("remote builds use {}", b.label());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_json_shapes() {
        let s = serde_json::to_string(&Backend::Hosted).unwrap();
        assert_eq!(s, r#"{"kind":"hosted"}"#);
        let b: Backend = serde_json::from_str(r#"{"kind":"aws"}"#).unwrap();
        assert_eq!(b, Backend::Aws);
        let b: Backend = serde_json::from_str(r#"{"kind":"hosted"}"#).unwrap();
        assert_eq!(b, Backend::Hosted);
    }

    // Runs `sh` scripts.
    #[cfg(unix)]
    #[test]
    fn env_lines_eval_in_a_shell() {
        let t = Target {
            dest: "ubuntu@203.0.113.7".into(),
            opts: vec![
                "-i".into(),
                "/home/me/my keys/id".into(),
                "-o".into(),
                "ControlPath=/run/x/cm-%C".into(),
            ],
        };
        let lines = env_lines(&Backend::Hosted, &t);
        let out = Command::new("bash")
            .arg("-c")
            .arg(format!(
                "{lines}printf '%s|%s|' \"$JR_REMOTE_KIND\" \"$JR_REMOTE_DEST\"; eval \"set -- $JR_REMOTE_SSH\"; printf '%s;' \"$@\""
            ))
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "hosted|ubuntu@203.0.113.7|ssh;-i;/home/me/my keys/id;-o;ControlPath=/run/x/cm-%C;"
        );
    }

    #[test]
    fn env_api_key_selects_hosted_only_when_unconfigured() {
        assert_eq!(resolve(None, false, false), None);
        assert_eq!(resolve(None, false, true), Some(Backend::Hosted));
        assert_eq!(resolve(None, true, true), Some(Backend::Aws));
        // The removed ssh backend reads as unconfigured.
        let ssh = br#"{"kind":"ssh","host":"me@box"}"#.to_vec();
        assert_eq!(resolve(Some(ssh), false, true), None);
    }
}
