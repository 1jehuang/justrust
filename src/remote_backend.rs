//! Where remote builds run. Everything above this module (sync daemon,
//! protocol, routing) only needs "an ssh-reachable machine with a shell".
//!
//! ```text
//! ~/.justrust/remote/backend.json   {"kind": "hosted"} | {"kind": "aws"} | {"kind": "ssh", "host": "..."}
//! ```
//!
//! - `hosted`: justrust's own build fleet, tied to a Jcode subscription
//!   (see `remote_hosted`). The one remote path going forward. The user
//!   there is `ubuntu` without general sudo: mirrored paths outside /home
//!   and /Users are created with `sudo -n /usr/local/sbin/jcode-mkdir`.
//! - `aws`: the machine `justrust remote up` creates in the user's own AWS
//!   account (see `remote`). Legacy: kept until hosted matches it, then
//!   removed. The default when that machine exists.
//! - `ssh`: any SSH-reachable Linux or macOS (Apple Silicon / Intel) machine
//!   (`justrust remote use ssh <[user@]host> [ssh_opts...]`).
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
    Ssh {
        host: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        opts: Vec<String>,
    },
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
            Backend::Ssh { host, .. } => format!("ssh:{host}"),
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
                    native: None,
                })
            }
            Backend::Hosted => crate::remote_hosted::ensure_ready(),
            Backend::Ssh { host, opts } => ssh_target(host, opts),
        }
    }

    /// Short backend name for status output and JSON.
    pub fn kind(&self) -> &'static str {
        match self {
            Backend::Aws => "aws",
            Backend::Hosted => "hosted",
            Backend::Ssh { .. } => "ssh",
        }
    }

    /// Reachable right now without starting anything (for auto routing).
    pub fn probably_up(&self) -> bool {
        match self {
            Backend::Aws => remote::cached_ip().is_some(),
            Backend::Hosted => crate::remote_hosted::probably_up(),
            Backend::Ssh { .. } => true,
        }
    }
}

pub fn ssh_target(host: &str, extra_opts: &[String]) -> Result<Target> {
    let d = remote::dir()?.join("ssh");
    std::fs::create_dir_all(&d)?;
    let mut opts = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        "ControlPersist=10m".into(),
        "-o".into(),
        format!("ControlPath={}", control_path(&d)),
    ];
    opts.extend(extra_opts.iter().cloned());
    Ok(Target {
        dest: host.to_owned(),
        opts,
        native: None,
    })
}

/// A remote destination with an explicitly negotiated transport.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub dest: String,
    pub opts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<crate::remote_native::Endpoint>,
}

impl Target {
    pub fn command(&self) -> Command {
        if let Some(endpoint) = &self.native {
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("justrust"));
            let mut c = Command::new(exe);
            c.args([
                "__native",
                "client",
                &serde_json::to_string(endpoint).expect("native endpoint JSON"),
            ]);
            return c;
        }
        self.ssh_command()
    }

    /// Explicit compatibility path for SSH control operations, never used for native exec.
    pub fn ssh_command(&self) -> Command {
        let mut c = Command::new("ssh");
        c.args(&self.opts).arg(&self.dest);
        c
    }

    /// `ssh <opts>` for `rsync -e`.
    pub fn rsync_shell(&self) -> String {
        if let Some(endpoint) = &self.native {
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("justrust"));
            return format!(
                "{} __native rsync {}",
                crate::remote_sync::rsync_shell_quote(&exe.to_string_lossy()),
                crate::remote_sync::rsync_shell_quote(
                    &serde_json::to_string(endpoint).expect("native endpoint JSON")
                )
            );
        }
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
/// random suffix (`.XXXXXXXXXXXXXXXX`), and a Unix socket path must fit in
/// `sun_path` ([`SUN_PATH_MAX`], including the NUL), so a long JUSTRUST_HOME
/// falls back to the per-user runtime dir (then /tmp).
pub(crate) fn control_path(dir: &std::path::Path) -> String {
    let p = format!("{}/cm-%C", dir.display());
    if fits_sun_path(p.len() - 2 + 40 + 17) {
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

/// `sun_path` size: 108 on Linux, 104 on macOS and the BSDs. Was a fixed
/// 108, so a 46-character macOS home (`/Users/<17 chars>/.justrust/remote/hosted`)
/// made a 107-byte path, ssh failed with `unix_listener: path ... too long`,
/// and a new user's first hosted build fell back to local (production
/// `local_builds`, 2026-10-13).
const SUN_PATH_MAX: usize = if cfg!(target_os = "linux") { 108 } else { 104 };

/// A socket path of `len` bytes plus its NUL fits.
fn fits_sun_path(len: usize) -> bool {
    len < SUN_PATH_MAX
}

/// `justrust remote env`: the ssh destination of the current backend as
/// shell assignments, so scripts (bench/on-remote.sh) reach whichever
/// machine remote builds use without knowing keys or addresses.
pub fn env_command(refresh: bool) -> Result<()> {
    let Some(b) = load() else {
        bail!("no remote machine configured. Sign in to hosted builds: `justrust login`");
    };
    let t = if refresh && matches!(b, Backend::Hosted) {
        crate::remote_hosted::ensure_ready_with_refresh(true)?
    } else {
        b.ensure_ready()?
    };
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

/// `justrust remote use <hosted|aws|ssh <host> [ssh_opts...]|off>`.
pub fn use_command(kind: &str, args: &[String]) -> Result<()> {
    let b = match kind {
        "aws" => {
            if !args.is_empty() {
                bail!("unexpected arguments for `remote use aws`");
            }
            if remote::instance_id().is_none() {
                println!("no AWS machine yet. Create one with `justrust remote up`.");
            }
            Backend::Aws
        }
        "ssh" => {
            let Some(host) = args.first().map(|h| h.trim()).filter(|h| !h.is_empty()) else {
                bail!("usage: justrust remote use ssh <[user@]host> [ssh_opts...]");
            };
            // The host is passed to ssh and rsync as a destination: it must
            // never parse as an option.
            if host.starts_with('-') || host.contains(char::is_whitespace) {
                bail!(
                    "invalid ssh host {host:?}: usage: justrust remote use ssh <[user@]host> [ssh_opts...]"
                );
            }
            Backend::Ssh {
                host: host.to_owned(),
                opts: args[1..].to_vec(),
            }
        }
        "hosted" => {
            let platform = crate::remote_windows::setup_args(args)?;
            crate::remote_hosted::check_use()?;
            crate::remote_windows::select(&platform)?;
            Backend::Hosted
        }
        "off" | "none" => {
            if !args.is_empty() {
                bail!("unexpected arguments for `remote use {kind}`");
            }
            crate::remote_daemon::stop();
            clear()?;
            println!("remote builds off: every build runs locally");
            return Ok(());
        }
        other => bail!("unknown backend {other}: use hosted, aws, ssh, or off"),
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
    fn control_path_fits_the_socket_limit() {
        // ssh binds `<path>.<17 random chars>` with %C as 40 hex characters.
        let bound = |p: &str| p.replace("%C", &"a".repeat(40)).len() + 17;
        // The production failure: a 17-character macOS user name.
        let mac = std::path::Path::new("/Users/jonathanfishner/.justrust/remote/hosted");
        assert!(
            bound(&control_path(mac)) < SUN_PATH_MAX,
            "{}",
            control_path(mac)
        );
        // Short homes keep the socket next to the keys.
        let short = std::path::Path::new("/home/me/.justrust/remote/hosted");
        assert_eq!(
            control_path(short),
            "/home/me/.justrust/remote/hosted/cm-%C"
        );
        assert!(!fits_sun_path(SUN_PATH_MAX) && fits_sun_path(SUN_PATH_MAX - 1));
        // On macOS (104) the old fixed 108 accepted the 107-byte production
        // path; the per-OS limit must reject it there.
        let raw_mac = bound("/Users/jonathanfishner/.justrust/remote/hosted/cm-%C");
        assert_eq!(raw_mac, 107);
        assert!(raw_mac >= 104);
    }

    #[test]
    fn backend_json_shapes() {
        let s = serde_json::to_string(&Backend::Hosted).unwrap();
        assert_eq!(s, r#"{"kind":"hosted"}"#);
        let b: Backend = serde_json::from_str(r#"{"kind":"aws"}"#).unwrap();
        assert_eq!(b, Backend::Aws);
        let b: Backend = serde_json::from_str(r#"{"kind":"hosted"}"#).unwrap();
        assert_eq!(b, Backend::Hosted);
        let b: Backend = serde_json::from_str(r#"{"kind":"ssh","host":"wayamac"}"#).unwrap();
        assert_eq!(
            b,
            Backend::Ssh {
                host: "wayamac".into(),
                opts: Vec::new(),
            }
        );
        assert_eq!(b.label(), "ssh:wayamac");
        assert_eq!(b.kind(), "ssh");
    }

    // Runs `sh` scripts.
    #[cfg(unix)]
    #[test]
    fn env_lines_eval_in_a_shell() {
        let t = Target {
            dest: "ubuntu@203.0.113.7".into(),
            native: None,
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
    fn native_target_keeps_scripts_and_endpoint_as_separate_arguments() {
        let endpoint = crate::remote_native::Endpoint {
            address: "203.0.113.7".into(),
            port: 443,
            server_name: "slot-1.pool.example".into(),
            certificate_sha256: "a".repeat(64),
            key_path: PathBuf::from("/home/me/key with 'quotes'"),
            resume_path: PathBuf::from("/home/me/resume"),
        };
        let target = Target {
            dest: "ubuntu@203.0.113.7".into(),
            opts: Vec::new(),
            native: Some(endpoint),
        };
        let mut command = target.command();
        let script = "printf '%s' 'unchanged; $(literal)'";
        command.arg(script);
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(args[..2], ["__native", "client"]);
        assert_eq!(args[3], script);
        let encoded: serde_json::Value = serde_json::from_str(args[2]).unwrap();
        assert_eq!(encoded["key_path"], "/home/me/key with 'quotes'");
        assert_ne!(command.get_program(), "ssh");
        assert!(target.rsync_shell().contains("__native rsync"));
        let roundtrip: Target =
            serde_json::from_value(serde_json::to_value(&target).unwrap()).unwrap();
        assert_eq!(roundtrip.native, target.native);
        let legacy: Target =
            serde_json::from_str(r#"{"dest":"ubuntu@203.0.113.7","opts":["-p","22001"]}"#).unwrap();
        assert!(legacy.native.is_none());
        assert_eq!(legacy.command().get_program(), "ssh");
    }

    #[test]
    fn env_api_key_selects_hosted_only_when_unconfigured() {
        assert_eq!(resolve(None, false, false), None);
        assert_eq!(resolve(None, false, true), Some(Backend::Hosted));
        assert_eq!(resolve(None, true, true), Some(Backend::Aws));
        // An explicit ssh backend overrides the fallback env key.
        let ssh = br#"{"kind":"ssh","host":"me@box"}"#.to_vec();
        assert_eq!(
            resolve(Some(ssh), false, true),
            Some(Backend::Ssh {
                host: "me@box".into(),
                opts: Vec::new(),
            })
        );
    }
}
