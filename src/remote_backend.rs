//! Where remote builds run. Everything above this module (sync daemon,
//! protocol, routing) only needs "an ssh-reachable machine with a shell".
//!
//! ```text
//! ~/.justrust/remote/backend.json   {"kind": "aws"} | {"kind": "ssh", ...} | {"kind": "hosted"}
//! ```
//!
//! - `aws`: the machine `justrust remote up` creates in the user's own AWS
//!   account (see `remote`). The default when that machine exists.
//! - `ssh`: any machine the user already has (`justrust remote use ssh
//!   user@host`). Needs a shell, a C toolchain, and rustup. justrust installs
//!   itself there.
//! - `hosted`: justrust's own build fleet, tied to a Jcode subscription.
//!   Reserved: selecting it reports that it is not available yet.
//!
//! With no backend configured nothing remote ever happens: the router costs
//! one failed `stat`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::remote;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Backend {
    Aws,
    Ssh {
        /// `user@host` or a Host alias from ~/.ssh/config.
        host: String,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        identity: Option<String>,
    },
    Hosted,
}

fn file() -> Result<PathBuf> {
    Ok(remote::dir()?.join("backend.json"))
}

/// The configured backend, or `None` when remote builds are not set up.
/// Cheap: used on every `justrust check`.
pub fn load() -> Option<Backend> {
    let home = crate::paths::home().ok()?.join("remote");
    match std::fs::read(home.join("backend.json")) {
        Ok(b) => serde_json::from_slice(&b).ok(),
        // Machines created before backend.json existed.
        Err(_) => home.join("state.json").exists().then_some(Backend::Aws),
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

pub const HOSTED_UNAVAILABLE: &str = "hosted remote builds (Jcode subscription) are not available yet. Use `justrust remote up` (your AWS account) or `justrust remote use ssh user@host`";

impl Backend {
    pub fn label(&self) -> String {
        match self {
            Backend::Aws => remote::describe_short().unwrap_or_else(|| "aws".into()),
            Backend::Ssh { host, .. } => host.clone(),
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
            Backend::Ssh {
                host,
                port,
                identity,
            } => {
                let t = ssh_target(host, *port, identity.as_deref())?;
                let ok = t
                    .command()
                    .arg("true")
                    .stdin(Stdio::null())
                    .stderr(Stdio::piped())
                    .output()
                    .context("ssh")?;
                if !ok.status.success() {
                    bail!(
                        "cannot reach {host} over ssh: {}",
                        String::from_utf8_lossy(&ok.stderr).trim()
                    );
                }
                Ok(t)
            }
            Backend::Hosted => bail!(HOSTED_UNAVAILABLE),
        }
    }

    /// Reachable right now without starting anything (for auto routing).
    pub fn probably_up(&self) -> bool {
        match self {
            Backend::Aws => remote::cached_ip().is_some(),
            Backend::Ssh { .. } => true,
            Backend::Hosted => false,
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

fn ssh_target(host: &str, port: Option<u16>, identity: Option<&str>) -> Result<Target> {
    let d = remote::dir()?;
    let mut opts: Vec<String> = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}/cm-%C", d.display()),
        "-o".into(),
        "ControlPersist=600".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
    ];
    if let Some(p) = port {
        opts.extend(["-p".into(), p.to_string()]);
    }
    if let Some(i) = identity {
        opts.extend(["-i".into(), i.to_string()]);
    }
    Ok(Target {
        dest: host.to_string(),
        opts,
    })
}

/// `justrust remote use <aws|ssh HOST|hosted|off>`.
pub fn use_command(
    kind: &str,
    host: Option<String>,
    port: Option<u16>,
    identity: Option<String>,
) -> Result<()> {
    let b = match kind {
        "aws" => {
            if remote::instance_id().is_none() {
                println!("no AWS machine yet. Create one with `justrust remote up`.");
            }
            Backend::Aws
        }
        "ssh" => Backend::Ssh {
            host: host
                .context("usage: justrust remote use ssh user@host [--port N] [--identity KEY]")?,
            port,
            identity,
        },
        "hosted" => bail!(HOSTED_UNAVAILABLE),
        "off" | "none" => {
            crate::remote_daemon::stop();
            clear()?;
            println!("remote builds off: every build runs locally");
            return Ok(());
        }
        other => bail!("unknown backend {other}: use aws, ssh, hosted, or off"),
    };
    if let Backend::Ssh { host, .. } = &b {
        println!("checking {host}...");
        b.ensure_ready()?;
    }
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
        let s = serde_json::to_string(&Backend::Ssh {
            host: "me@box".into(),
            port: None,
            identity: None,
        })
        .unwrap();
        assert!(s.contains("\"kind\":\"ssh\""), "{s}");
        let b: Backend = serde_json::from_str(r#"{"kind":"aws"}"#).unwrap();
        assert_eq!(b, Backend::Aws);
        let b: Backend = serde_json::from_str(r#"{"kind":"hosted"}"#).unwrap();
        assert!(b.ensure_ready().is_err());
    }
}
