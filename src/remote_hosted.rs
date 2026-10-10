//! The `hosted` remote backend: a build machine from justrust's own fleet,
//! paid per build (`builds` on /me), with a Jcode account.
//!
//! ```text
//! GET  /v1/me                  capabilities.build_hosts, tier, status, email
//! POST /v1/build/host/connect  {"public_key"} -> 202 while starting, 200 when ready
//! GET  /v1/build/host          {state, credits}
//! POST /v1/build/host/stop     {state}
//!
//! ~/.justrust/remote/hosted/
//!   id_ed25519(.pub)  fresh key per connect, valid for 60 s on the server
//!   known_hosts       exactly the host keys the API returned
//!   state.json        {address, user, port, ready_at}
//!   cm-%C             ssh ControlMaster, kept 10 minutes after last use
//! ```
//!
//! The key is only accepted for `key_valid_seconds`, so `ensure_ready` opens
//! the ControlMaster right away and every later ssh, rsync, and the sync
//! daemon ride on it. When it has gone away the next build reconnects with a
//! new key.
//!
//! Credentials are the ones `jcode account login` writes: `JCODE_API_KEY`
//! and `JCODE_API_BASE` from the environment, else
//! `~/.config/jcode/jcode-subscription.env`. HTTP goes through curl. The key
//! is handed to curl on stdin (`--config -`), never in argv or a URL, and
//! never printed.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::remote_backend::Target;

pub const DEFAULT_API_BASE: &str = "https://api.jcode.sh/v1";
/// The build fleet (justrust-cloud). Every `build/...` path goes here.
pub const DEFAULT_BUILD_BASE: &str = "https://build.jcode.sh/v1";
const ENV_FILE: &str = "jcode-subscription.env";
const POLL: Duration = Duration::from_secs(2);
/// First creation of a machine: boot plus cloud-init.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6 * 60);
/// A ready this recent counts as up for routing even before the master check.
const RECENT_READY_S: u64 = 60;

pub const SIGNED_OUT: &str = "not signed in for hosted builds. Run `justrust login --no-wait` and ask your user to open the URL it prints (builds stay local until then).";
pub const SUBSCRIBE: &str = "free builds used up. Subscribe ($10/mo, 500 builds): run `justrust upgrade --no-wait` and ask your user to open the checkout URL it prints. Builds run locally until then.";
pub const RAISE_LIMIT: &str = "monthly usage limit reached: ask your user to raise it at https://jcode.sh/account. Builds run locally until then.";
pub const NO_CREDITS: &str = "out of Jcode cloud-compute credits (shared with cloud agents). Builds run locally until credits renew. There are no automatic top-ups.";
pub const UNAVAILABLE: &str = "the hosted build service is unavailable right now. This is not a problem with your subscription. Builds run locally, retry later.";
pub const RATE_LIMITED: &str = "the Jcode API is rate limiting requests. Retry in a moment.";

// ---------------------------------------------------------------- credentials

#[derive(Clone)]
pub struct Creds {
    pub base: String,
    /// Base for `build/...` paths (JUSTRUST_BUILD_BASE).
    pub build_base: String,
    key: String,
}

impl std::fmt::Debug for Creds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Creds")
            .field("base", &self.base)
            .field("build_base", &self.build_base)
            .field("key", &"<redacted>")
            .finish()
    }
}

fn config_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("JCODE_HOME") {
        return Some(PathBuf::from(h).join("config").join("jcode"));
    }
    Some(dirs::config_dir()?.join("jcode"))
}

fn clean(v: &str) -> Option<String> {
    let v = v.trim();
    let v = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v)
        .trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// `KEY=value` lines (optionally `export KEY=value`, quoted values, comments).
pub fn parse_env_file(content: &str, name: &str) -> Option<String> {
    content.lines().find_map(|l| {
        let l = l.trim();
        let l = l.strip_prefix("export ").unwrap_or(l).trim_start();
        let (k, v) = l.split_once('=')?;
        (k.trim() == name).then(|| clean(v)).flatten()
    })
}

fn resolve(name: &str, file: Option<&str>) -> Option<String> {
    if let Some(v) = std::env::var(name).ok().as_deref().and_then(clean) {
        return Some(v);
    }
    parse_env_file(file?, name)
}

/// The Jcode API base and key, or the signed-out guidance.
pub fn creds() -> Result<Creds> {
    let file = config_dir().and_then(|d| std::fs::read_to_string(d.join(ENV_FILE)).ok());
    let key = resolve("JCODE_API_KEY", file.as_deref()).ok_or_else(|| anyhow!(SIGNED_OUT))?;
    let mut c = Creds::new(api_base(), key)?;
    c.set_build_base(build_base())?;
    Ok(c)
}

fn env_file_text() -> Option<String> {
    config_dir().and_then(|d| std::fs::read_to_string(d.join(ENV_FILE)).ok())
}

/// The credentials file Jcode and `justrust login` share.
pub fn env_file_path() -> Option<PathBuf> {
    Some(config_dir()?.join(ENV_FILE))
}

/// A key is present (env or file). No network.
pub fn signed_in() -> bool {
    resolve("JCODE_API_KEY", env_file_text().as_deref()).is_some()
}

pub fn api_base() -> String {
    resolve("JCODE_API_BASE", env_file_text().as_deref()).unwrap_or_else(|| DEFAULT_API_BASE.into())
}

pub fn build_base() -> String {
    std::env::var("JUSTRUST_BUILD_BASE")
        .ok()
        .as_deref()
        .and_then(clean)
        .unwrap_or_else(|| DEFAULT_BUILD_BASE.into())
}

impl Creds {
    pub fn new(base: String, key: String) -> Result<Self> {
        if key.chars().any(|c| c.is_control() || c == ' ') {
            bail!(
                "JCODE_API_KEY contains whitespace or control characters. Run `jcode account login` again."
            );
        }
        check_base(&base)?;
        let base = base.trim_end_matches('/').to_string();
        Ok(Creds {
            build_base: base.clone(),
            base,
            key,
        })
    }

    /// No key: only for the device flow, which is how a key is obtained.
    pub fn anonymous(base: String) -> Result<Self> {
        check_base(&base)?;
        let base = base.trim_end_matches('/').to_string();
        Ok(Creds {
            build_base: base.clone(),
            base,
            key: String::new(),
        })
    }

    pub fn set_build_base(&mut self, b: String) -> Result<()> {
        check_base(&b)?;
        self.build_base = b.trim_end_matches('/').to_string();
        Ok(())
    }

    fn url(&self, suffix: &str) -> String {
        let suffix = suffix.trim_start_matches('/');
        let base = if suffix.starts_with("build/") {
            &self.build_base
        } else {
            &self.base
        };
        format!("{base}/{suffix}")
    }
}

/// https, or http only on loopback. No userinfo, query, or fragment, so a
/// bearer key never goes to a plaintext public endpoint.
pub fn check_base(base: &str) -> Result<()> {
    let bad = || anyhow!("JCODE_API_BASE must be an https URL (http only on loopback): {base}");
    let (scheme, rest) = base.split_once("://").ok_or_else(bad)?;
    let authority = rest.split('/').next().unwrap_or("");
    if base.contains(['?', '#', '@', ' ', '\n', '\r', '"', '\\']) || authority.is_empty() {
        return Err(bad());
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        format!("[{}]", v6.split(']').next().unwrap_or(""))
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    match scheme {
        "https" => Ok(()),
        "http" if matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]") => Ok(()),
        _ => Err(bad()),
    }
}

// ----------------------------------------------------------------------- http

#[derive(Debug)]
pub struct Resp {
    pub code: u16,
    pub body: Value,
}

/// The curl invocation and the config it reads on stdin. Split out so tests
/// can assert the key is only on stdin.
fn curl(c: &Creds, method: &str, path: &str, body: Option<&Value>) -> (Command, String) {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--proto",
        "=https,http",
        "--max-redirs",
        "0",
        "--connect-timeout",
        "5",
        "--max-time",
        "30",
        "--max-filesize",
        "1048576",
        "-X",
        method,
        "-H",
        "Accept: application/json",
        "-w",
        "\n%{http_code}",
        "--config",
        "-",
    ]);
    if let Some(b) = body {
        cmd.args(["-H", "Content-Type: application/json", "--data-binary"])
            .arg(b.to_string());
    }
    cmd.arg(c.url(path));
    // Creds::new rejects quotes' neighbours (control chars, spaces); escape
    // the rest of curl's quoted-string syntax anyway.
    if c.key.is_empty() {
        return (cmd, String::new());
    }
    let k = c.key.replace('\\', "\\\\").replace('"', "\\\"");
    (cmd, format!("header = \"Authorization: Bearer {k}\"\n"))
}

pub fn request(c: &Creds, method: &str, path: &str, body: Option<&Value>) -> Result<Resp> {
    let (mut cmd, config) = curl(c, method, path, body);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running curl (install curl for hosted remote builds)")?;
    child
        .stdin
        .take()
        .context("curl stdin")?
        .write_all(config.as_bytes())?;
    let out = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (text, code) = stdout.rsplit_once('\n').unwrap_or(("", &stdout));
    let code: u16 = code.trim().parse().unwrap_or(0);
    if !out.status.success() || code == 0 {
        bail!(
            "could not reach the Jcode API at {}: {}. Account access could not be verified, builds run locally.",
            c.base,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(Resp {
        code,
        body: serde_json::from_str(text).unwrap_or(Value::Null),
    })
}

/// Turn a non-2xx response into user guidance.
pub fn api_error(r: &Resp) -> anyhow::Error {
    let code = r.body["error"]["code"].as_str().unwrap_or("");
    // Bounded and on one line so a hostile body cannot flood the terminal.
    let msg: String = r.body["error"]["message"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_graphic())
        .take(64)
        .collect();
    let g = match (r.code, code.as_str()) {
        (401, _) => "the Jcode sign-in is missing or expired. Run `justrust login --no-wait` and ask your user to open the URL it prints.".into(),
        (402, "insufficient_compute_credits") => NO_CREDITS.into(),
        (402, _) if r.body["error"]["next_step"].as_str() == Some("raise_limit") => {
            return anyhow!(RAISE_LIMIT);
        }
        (402, _) | (403, "build_not_entitled") => return anyhow!(SUBSCRIBE),
        (429, _) => RATE_LIMITED.into(),
        (503, _) | (_, "build_unavailable") => UNAVAILABLE.into(),
        _ if !msg.is_empty() => return anyhow!("Jcode API error {} ({code}): {msg}", r.code),
        _ => format!("Jcode API error {}", r.code),
    };
    // The server's own words help when its state is unusual.
    if msg.is_empty() || g.contains(&msg) {
        anyhow!(g)
    } else {
        anyhow!("{g} (server: {msg})")
    }
}

fn ok(r: Resp) -> Result<Value> {
    if (200..300).contains(&r.code) {
        Ok(r.body)
    } else {
        Err(api_error(&r))
    }
}

// ------------------------------------------------------------------- account

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Me {
    pub email: Option<String>,
    pub tier: Option<String>,
    pub status: Option<String>,
    pub build_hosts: bool,
    #[serde(default)]
    pub builds: Option<Builds>,
}

/// The `builds` object of /me and of build fleet responses.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Builds {
    #[serde(default)]
    pub trial_remaining: Option<i64>,
    #[serde(default)]
    pub included: Option<i64>,
    #[serde(default)]
    pub included_used: Option<i64>,
    #[serde(default)]
    pub overage_used: Option<i64>,
    #[serde(default)]
    pub overage_price_usd: Option<f64>,
    #[serde(default)]
    pub resets_at: Option<String>,
    #[serde(default)]
    pub can_build: bool,
    #[serde(default)]
    pub next_step: Option<String>,
}

impl Builds {
    pub fn from(v: &Value) -> Option<Self> {
        v.is_object()
            .then(|| serde_json::from_value(v.clone()).ok())
            .flatten()
    }

    /// Prepaid builds left: trial plus the unused monthly bundle.
    pub fn left(&self) -> i64 {
        let bundle = (self.included.unwrap_or(0) - self.included_used.unwrap_or(0)).max(0);
        self.trial_remaining.unwrap_or(0).max(0) + bundle
    }

    /// "12 builds left", plus overage or the next step.
    pub fn describe(&self) -> String {
        let n = self.left();
        let mut s = format!("{n} build{} left", if n == 1 { "" } else { "s" });
        if n == 0 && self.can_build {
            let p = self.overage_price_usd.unwrap_or(0.10);
            s.push_str(&format!(", then ${p:.2} per build"));
        }
        match self.next_step.as_deref() {
            Some("subscribe") => s.push_str(". Subscribe: `justrust upgrade`"),
            Some("raise_limit") => s.push_str(". Raise the limit at https://jcode.sh/account"),
            _ => {}
        }
        s
    }
}

pub fn me(c: &Creds) -> Result<Me> {
    let b = ok(request(c, "GET", "me", None)?)?;
    let s = |k: &str| b[k].as_str().map(str::to_string);
    Ok(Me {
        email: s("email"),
        tier: s("tier"),
        status: s("status"),
        build_hosts: b["capabilities"]["build_hosts"].as_bool().unwrap_or(false)
            || b["capabilities"]["hosted_builds"]
                .as_bool()
                .unwrap_or(false)
            || b["builds"].is_object(),
        builds: Builds::from(&b["builds"]),
    })
}

/// Signed in and entitled to build hosts.
pub fn check_access(c: &Creds) -> Result<Me> {
    let m = me(c)?;
    if !m.build_hosts {
        bail!(SUBSCRIBE);
    }
    Ok(m)
}

/// `POST {build}/build/runs {run_id}` before a remote run. `Err` holds the
/// one line to show (the run must then not go remote).
pub fn charge_run(c: &Creds, run_id: &str) -> std::result::Result<Option<Builds>, String> {
    let r = request(c, "POST", "build/runs", Some(&json!({ "run_id": run_id })))
        .map_err(|e| e.to_string())?;
    if r.code == 200 {
        Ok(Builds::from(&r.body["builds"]))
    } else {
        Err(api_error(&r).to_string())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Credits {
    pub available_microcredits: Option<f64>,
    pub rate_microcredits_per_second: Option<f64>,
}

impl Credits {
    fn from(v: &Value) -> Self {
        Credits {
            available_microcredits: v["available_microcredits"].as_f64(),
            rate_microcredits_per_second: v["rate_microcredits_per_second"].as_f64(),
        }
    }

    /// Hours of build-machine runtime the balance buys at the current rate.
    pub fn hours_left(&self) -> Option<f64> {
        let (a, r) = (
            self.available_microcredits?,
            self.rate_microcredits_per_second?,
        );
        (r > 0.0).then(|| a.max(0.0) / r / 3600.0)
    }
}

/// Everything `justrust remote status` shows for the hosted backend.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HostedStatus {
    /// Unix seconds of the API query.
    #[serde(default)]
    pub at: u64,
    pub me: Option<Me>,
    /// Server-side host state: none, provisioning, starting, running, stopped...
    pub host_state: Option<String>,
    pub credits: Credits,
    pub hours_left: Option<f64>,
    #[serde(default)]
    pub builds: Option<Builds>,
    /// The cached ssh destination, when a session was opened before.
    pub address: Option<String>,
    /// The local ControlMaster is alive.
    pub connected: bool,
    pub error: Option<String>,
}

const STATUS_TTL_S: u64 = 30;

/// Account and host state for `remote status`. `refresh`: query the API when
/// the cached answer is older than 30 s (waybar polls often). Without it,
/// only local state and the last cached answer.
pub fn status_cached(refresh: bool) -> HostedStatus {
    let file = crate::paths::home()
        .ok()
        .map(|h| h.join("remote/hosted/status.json"));
    let cached: Option<HostedStatus> = file
        .as_ref()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice(&b).ok());
    let fresh = |s: &HostedStatus| now().saturating_sub(s.at) < STATUS_TTL_S;
    let mut s = match cached {
        Some(c) if !refresh || fresh(&c) => c,
        _ if refresh => {
            let s = status();
            if let (Some(f), Ok(d)) = (&file, dir())
                && d.exists()
            {
                let _ = write_private(f, &serde_json::to_vec(&s).unwrap_or_default());
            }
            s
        }
        _ => HostedStatus::default(),
    };
    s.connected = master_alive();
    s.address = load_state().map(|st| st.address);
    s
}

pub fn status() -> HostedStatus {
    let mut s = HostedStatus {
        at: now(),
        connected: master_alive(),
        address: load_state().map(|st| st.address),
        ..Default::default()
    };
    let r = (|| -> Result<()> {
        let c = creds()?;
        s.me = Some(me(&c)?);
        let h = ok(request(&c, "GET", "build/host", None)?)?;
        s.host_state = h["state"].as_str().map(str::to_string);
        s.credits = Credits::from(&h["credits"]);
        s.hours_left = s.credits.hours_left();
        s.builds = Builds::from(&h["builds"]).or_else(|| s.me.as_ref()?.builds.clone());
        Ok(())
    })();
    s.error = r.err().map(|e| e.to_string());
    s
}

// ------------------------------------------------------------------- connect

#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    pub address: String,
    pub port: u16,
    pub user: String,
    pub host_keys: Vec<String>,
    pub credits: Credits,
    pub builds: Option<Builds>,
}

fn safe_host(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b))
}

fn safe_user(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

fn parse_ready(b: &Value) -> Result<Ready> {
    let address = b["address"].as_str().unwrap_or("").to_string();
    let user = b["user"].as_str().unwrap_or("ubuntu").to_string();
    let port = b["port"].as_u64().unwrap_or(22);
    if !safe_host(&address) || !safe_user(&user) || port == 0 || port > 65535 {
        bail!("the Jcode API returned an invalid build host address");
    }
    let host_keys: Vec<String> = b["host_keys"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|k| k.as_str())
                .map(|k| k.trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    let valid = |k: &String| {
        let mut p = k.split_whitespace();
        matches!((p.next(), p.next()), (Some(t), Some(d))
            if (t.starts_with("ssh-") || t.starts_with("ecdsa-")) && !d.is_empty())
            && !k.contains(['\n', '\r'])
    };
    if host_keys.is_empty() || !host_keys.iter().all(valid) {
        bail!("the Jcode API returned no usable host keys for the build host, refusing to connect");
    }
    Ok(Ready {
        address,
        port: port as u16,
        user,
        host_keys,
        credits: Credits::from(&b["credits"]),
        builds: Builds::from(&b["builds"]),
    })
}

/// POST connect and poll until the host is ready. `progress` sees each new
/// server message while the machine starts.
pub fn connect_with(
    c: &Creds,
    public_key: &str,
    poll: Duration,
    timeout: Duration,
    mut progress: impl FnMut(&str),
) -> Result<Ready> {
    let start = Instant::now();
    let body = json!({ "public_key": public_key.trim() });
    let mut last = String::new();
    loop {
        let r = request(c, "POST", "build/host/connect", Some(&body))?;
        match r.code {
            200 if r.body["ready"].as_bool() != Some(false) => return parse_ready(&r.body),
            200..=299 => {
                let msg = r.body["message"]
                    .as_str()
                    .or(r.body["state"].as_str())
                    .unwrap_or("starting")
                    .to_string();
                if msg != last {
                    progress(&msg);
                    last = msg;
                }
            }
            _ => return Err(api_error(&r)),
        }
        if start.elapsed() >= timeout {
            bail!(
                "the hosted build machine did not become ready within {} s (last state: {last}). Builds run locally, retry later.",
                timeout.as_secs()
            );
        }
        std::thread::sleep(poll);
    }
}

/// One line per host key, pinned to `address` (and port when not 22).
pub fn known_hosts(r: &Ready) -> String {
    let name = if r.port == 22 {
        r.address.clone()
    } else {
        format!("[{}]:{}", r.address, r.port)
    };
    r.host_keys
        .iter()
        .map(|k| format!("{name} {k}\n"))
        .collect()
}

pub fn dir() -> Result<PathBuf> {
    let d = crate::remote::dir()?.join("hosted");
    std::fs::create_dir_all(&d)?;
    set_mode(&d, 0o700);
    Ok(d)
}

fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
}

fn write_private(p: &Path, content: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = p.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(content)?;
    drop(f);
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// ssh options for the hosted machine. Strict host key checking against
/// only the pinned known_hosts.
pub fn target(d: &Path, user: &str, address: &str, port: u16) -> Target {
    let o = |s: String| ["-o".to_string(), s];
    let mut opts: Vec<String> = vec!["-i".into(), d.join("id_ed25519").display().to_string()];
    for s in [
        "IdentitiesOnly=yes".to_string(),
        format!("UserKnownHostsFile={}", d.join("known_hosts").display()),
        "GlobalKnownHostsFile=/dev/null".into(),
        "StrictHostKeyChecking=yes".into(),
        "BatchMode=yes".into(),
        "ConnectTimeout=10".into(),
        "LogLevel=ERROR".into(),
        "ControlMaster=auto".into(),
        format!("ControlPath={}", crate::remote_backend::control_path(d)),
        "ControlPersist=600".into(),
        "ServerAliveInterval=15".into(),
    ] {
        opts.extend(o(s));
    }
    opts.extend(["-p".into(), port.to_string()]);
    Target {
        dest: format!("{user}@{address}"),
        opts,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct State {
    pub address: String,
    pub user: String,
    pub port: u16,
    /// Unix seconds.
    pub ready_at: u64,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn state_file() -> Option<PathBuf> {
    Some(crate::paths::home().ok()?.join("remote/hosted/state.json"))
}

pub fn load_state() -> Option<State> {
    serde_json::from_slice(&std::fs::read(state_file()?).ok()?).ok()
}

/// The cached destination, without any network.
pub fn cached_target() -> Option<Target> {
    let st = load_state()?;
    let d = state_file()?.parent()?.to_path_buf();
    Some(target(&d, &st.user, &st.address, st.port))
}

/// `ssh -O check`: a local socket query, no network.
pub fn master_alive() -> bool {
    let Some(t) = cached_target() else {
        return false;
    };
    t.command()
        .args(["-O", "check"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// For routing on every build: the ControlMaster is up, or the host was
/// ready a moment ago. Never touches the network.
pub fn probably_up() -> bool {
    match load_state() {
        None => false,
        Some(st) => now().saturating_sub(st.ready_at) < RECENT_READY_S || master_alive(),
    }
}

/// Held for its fd: the flock is released when it drops.
struct Lock(#[allow(dead_code)] std::fs::File);

impl Lock {
    fn take(d: &Path) -> Result<Lock> {
        use std::os::fd::AsRawFd;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(d.join("lock"))?;
        // SAFETY: valid fd owned by f for the duration of the call.
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
        Ok(Lock(f))
    }
}

fn new_key(d: &Path) -> Result<String> {
    let k = d.join("id_ed25519");
    let _ = std::fs::remove_file(&k);
    let _ = std::fs::remove_file(d.join("id_ed25519.pub"));
    let out = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "justrust-hosted",
            "-f",
        ])
        .arg(&k)
        .stdin(Stdio::null())
        .output()
        .context("ssh-keygen")?;
    if !out.status.success() {
        bail!(
            "ssh-keygen failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    set_mode(&k, 0o600);
    Ok(std::fs::read_to_string(d.join("id_ed25519.pub"))?
        .trim()
        .to_string())
}

/// Open the ControlMaster while the key is still accepted. sshd may need a
/// moment after the API reports ready, so retry inside the key window. The
/// persisted master inherits stdio, so it gets a log file, not a pipe.
fn open_master(t: &Target, d: &Path, window: Duration) -> Result<()> {
    let start = Instant::now();
    let log = d.join("ssh.log");
    loop {
        let err = std::fs::File::create(&log)?;
        let ok = t
            .command()
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(err)
            .status()
            .context("ssh")?
            .success();
        if ok {
            return Ok(());
        }
        let msg = || std::fs::read_to_string(&log).unwrap_or_default();
        let hostkey = msg().contains("Host key verification failed");
        if hostkey || start.elapsed() >= window {
            bail!(
                "cannot open an ssh session to the hosted build machine {}{}: {}",
                t.dest,
                if hostkey {
                    " (its host key does not match the one the Jcode API returned, refusing to connect)"
                } else {
                    ""
                },
                msg().trim()
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Make the hosted machine reachable and return its ssh destination.
pub fn ensure_ready() -> Result<Target> {
    let d = dir()?;
    let _lock = Lock::take(&d)?;
    if master_alive()
        && let Some(t) = cached_target()
    {
        return Ok(t);
    }
    let c = creds()?;
    let pubkey = new_key(&d)?;
    eprintln!("hosted: connecting to a build machine...");
    let r = connect_with(&c, &pubkey, POLL, CONNECT_TIMEOUT, |m| {
        eprintln!("hosted: {m}")
    })?;
    write_private(&d.join("known_hosts"), known_hosts(&r).as_bytes())?;
    let t = target(&d, &r.user, &r.address, r.port);
    // A stale master for another address must not shadow the new one.
    let _ = std::fs::remove_file(d.join("state.json"));
    let _ = std::fs::remove_file(d.join("status.json"));
    open_master(&t, &d, Duration::from_secs(45))?;
    let st = State {
        address: r.address.clone(),
        user: r.user.clone(),
        port: r.port,
        ready_at: now(),
    };
    write_private(&d.join("state.json"), &serde_json::to_vec_pretty(&st)?)?;
    if let Some(b) = &r.builds {
        eprintln!("hosted: ready at {} ({})", t.dest, b.describe());
    } else if let Some(h) = r.credits.hours_left() {
        eprintln!("hosted: ready at {} ({h:.1} h of credits left)", t.dest);
    } else {
        eprintln!("hosted: ready at {}", t.dest);
    }
    Ok(t)
}

/// `remote up` for hosted.
pub fn up() -> Result<()> {
    let t = ensure_ready()?;
    println!("hosted build machine ready: {}", t.dest);
    Ok(())
}

/// `remote down` for hosted: stop the machine server-side and drop the
/// local session.
pub fn down() -> Result<()> {
    let c = creds()?;
    let b = ok(request(&c, "POST", "build/host/stop", Some(&json!({})))?)?;
    if let Some(t) = cached_target() {
        let _ = t
            .command()
            .args(["-O", "exit"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if let Some(f) = state_file() {
        let _ = std::fs::remove_file(f.with_file_name("status.json"));
        let _ = std::fs::remove_file(f);
    }
    println!(
        "hosted build machine: {}",
        b["state"].as_str().unwrap_or("stopping")
    );
    Ok(())
}

/// `remote use hosted`: signed in, entitled, then a one-line summary.
pub fn check_use() -> Result<Me> {
    let c = creds()?;
    let m = check_access(&c)?;
    let who = m.email.as_deref().unwrap_or("signed in");
    let tier = m.tier.as_deref().unwrap_or("subscription");
    let credits = request(&c, "GET", "build/host", None)
        .ok()
        .filter(|r| r.code == 200)
        .and_then(|r| Credits::from(&r.body["credits"]).hours_left());
    match credits {
        _ if m.builds.is_some() => {
            println!("{who} ({tier}): {}", m.builds.as_ref().unwrap().describe())
        }
        Some(h) => println!("{who} ({tier}): {h:.1} h of build-machine credits"),
        None => println!("{who} ({tier})"),
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    struct Req {
        method: String,
        path: String,
        auth: Option<String>,
        body: String,
    }

    /// Serves `responses` in order (status, json body), one per connection.
    fn mock(responses: Vec<(u16, &'static str)>) -> (Creds, Arc<Mutex<Vec<Req>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}/v1", l.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for (code, body) in responses {
                let (mut sock, _) = l.accept().unwrap();
                let mut r = BufReader::new(sock.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut it = line.split_whitespace();
                let method = it.next().unwrap_or("").to_string();
                let path = it.next().unwrap_or("").to_string();
                let (mut len, mut auth) = (0usize, None);
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap();
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => len = v.trim().parse().unwrap(),
                        "authorization" => auth = Some(v.trim().to_string()),
                        _ => {}
                    }
                }
                let mut b = vec![0; len];
                r.read_exact(&mut b).unwrap();
                s2.lock().unwrap().push(Req {
                    method,
                    path,
                    auth,
                    body: String::from_utf8(b).unwrap(),
                });
                write!(
                    sock,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (Creds::new(base, "sk-secret-123".into()).unwrap(), seen)
    }

    const READY: &str = r#"{"state":"running","ready":true,"address":"203.0.113.7","port":2222,"user":"ubuntu","host_keys":["ssh-ed25519 AAAAC3Nza host1","ecdsa-sha2-nistp256 AAAAE2V"],"key_valid_seconds":60,"credits":{"available_microcredits":7200000000,"rate_microcredits_per_second":1000000}}"#;

    #[test]
    fn polls_202_until_ready() {
        let (c, seen) = mock(vec![
            (
                202,
                r#"{"state":"provisioning","ready":false,"message":"creating machine"}"#,
            ),
            (
                202,
                r#"{"state":"booting","ready":false,"message":"booting"}"#,
            ),
            (200, READY),
        ]);
        let mut msgs = vec![];
        let r = connect_with(
            &c,
            "ssh-ed25519 AAAA me",
            Duration::from_millis(10),
            Duration::from_secs(10),
            |m| msgs.push(m.to_string()),
        )
        .unwrap();
        assert_eq!(msgs, ["creating machine", "booting"]);
        assert_eq!(r.address, "203.0.113.7");
        assert_eq!(r.port, 2222);
        assert_eq!(r.credits.hours_left(), Some(2.0));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].path, "/v1/build/host/connect");
        assert_eq!(seen[0].auth.as_deref(), Some("Bearer sk-secret-123"));
        let b: Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(b["public_key"], "ssh-ed25519 AAAA me");
    }

    #[test]
    fn connect_times_out() {
        let (c, _) = mock(vec![(202, r#"{"ready":false,"message":"booting"}"#); 3]);
        let e = connect_with(&c, "k", Duration::from_millis(5), Duration::ZERO, |_| {})
            .unwrap_err()
            .to_string();
        assert!(e.contains("did not become ready"), "{e}");
    }

    #[test]
    fn error_guidance() {
        let (c, _) = mock(vec![
            (
                401,
                r#"{"error":{"code":"unauthorized","message":"bad key"}}"#,
            ),
            (
                402,
                r#"{"error":{"code":"build_not_entitled","message":"no"}}"#,
            ),
            (
                402,
                r#"{"error":{"code":"insufficient_compute_credits","message":"no"}}"#,
            ),
            (
                503,
                r#"{"error":{"code":"build_unavailable","message":"down"}}"#,
            ),
            (429, r#"{"error":{"code":"rate_limited","message":"slow"}}"#),
            (
                200,
                r#"{"email":"a@b","tier":"pro","capabilities":{"build_hosts":false}}"#,
            ),
        ]);
        let e = |r: Result<Me>| r.unwrap_err().to_string();
        assert!(e(me(&c)).contains("justrust login"));
        let s = e(me(&c));
        assert!(s.contains("justrust upgrade"), "{s}");
        assert!(e(me(&c)).contains("credits"));
        let s = e(me(&c));
        assert!(
            s.contains("unavailable") && s.contains("not a problem"),
            "{s}"
        );
        assert!(e(me(&c)).contains("rate limiting"));
        assert!(e(check_access(&c)).contains("justrust upgrade"));
    }

    #[test]
    fn me_parses_capability() {
        let (c, _) = mock(vec![(
            200,
            r#"{"email":"a@b","tier":"pro","status":"active","capabilities":{"build_hosts":true}}"#,
        )]);
        let m = check_access(&c).unwrap();
        assert_eq!(m.email.as_deref(), Some("a@b"));
        assert!(m.build_hosts);
    }

    #[test]
    fn charge_run_and_402_lines() {
        let (mut c, seen) = mock(vec![
            (
                200,
                r#"{"charged":"trial","builds":{"trial_remaining":24,"can_build":true}}"#,
            ),
            (
                402,
                r#"{"error":{"code":"builds_exhausted","message":"no","next_step":"subscribe"}}"#,
            ),
            (
                402,
                r#"{"error":{"code":"builds_exhausted","message":"no","next_step":"raise_limit"}}"#,
            ),
        ]);
        let api = c.base.clone();
        // Fleet paths use the build base, account paths stay on the API.
        c.set_build_base(format!("{api}/fleet")).unwrap();
        assert_eq!(charge_run(&c, "r1").unwrap().unwrap().left(), 24);
        let sub = charge_run(&c, "r2").unwrap_err();
        assert!(sub.starts_with("free builds used up. Subscribe ($10/mo, 500 builds)"));
        assert!(
            sub.contains("justrust upgrade") && !sub.contains('\n'),
            "{sub}"
        );
        let lim = charge_run(&c, "r3").unwrap_err();
        assert!(lim.contains("https://jcode.sh/account"), "{lim}");
        let s = seen.lock().unwrap();
        assert_eq!(s[0].path, "/v1/fleet/build/runs");
        assert_eq!(s[0].body, r#"{"run_id":"r1"}"#);
        assert!(s[0].auth.as_deref() == Some("Bearer sk-secret-123"));
        assert_eq!(c.url("me"), format!("{api}/me"));
    }

    #[test]
    fn builds_describe() {
        let b = Builds {
            trial_remaining: Some(3),
            included: Some(500),
            included_used: Some(100),
            can_build: true,
            ..Default::default()
        };
        assert_eq!(b.describe(), "403 builds left");
    }

    #[test]
    fn key_never_in_argv_or_url() {
        let c = Creds::new("https://api.jcode.sh/v1/".into(), "sk-secret-123".into()).unwrap();
        let (cmd, cfg) = curl(
            &c,
            "POST",
            "build/host/connect",
            Some(&json!({"public_key": "x"})),
        );
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().all(|a| !a.contains("sk-secret")), "{args:?}");
        assert!(args.contains(&"https://api.jcode.sh/v1/build/host/connect".to_string()));
        assert!(args.windows(2).any(|w| w == ["--config", "-"]));
        assert_eq!(cfg, "header = \"Authorization: Bearer sk-secret-123\"\n");
        assert!(!format!("{c:?}").contains("sk-secret"));
    }

    #[test]
    fn base_must_be_https_or_loopback() {
        for ok in [
            "https://api.jcode.sh/v1",
            "http://127.0.0.1:8787/v1",
            "http://localhost:8787/v1",
            "http://[::1]:8787/v1",
        ] {
            check_base(ok).unwrap();
        }
        for bad in [
            "http://api.jcode.sh/v1",
            "http://127.0.0.1.evil.com/v1",
            "https://user:pw@api.jcode.sh/v1",
            "https://api.jcode.sh/v1?x=1",
            "ftp://x",
            "api.jcode.sh",
        ] {
            assert!(check_base(bad).is_err(), "{bad}");
        }
        assert!(Creds::new("https://a.b".into(), "a b".into()).is_err());
    }

    #[test]
    fn env_file_parsing() {
        let f = "# jcode\nJCODE_API_BASE=http://127.0.0.1:9/v1\nexport JCODE_API_KEY=\"sk-1\"\nOTHER=x\n";
        assert_eq!(parse_env_file(f, "JCODE_API_KEY").as_deref(), Some("sk-1"));
        assert_eq!(
            parse_env_file(f, "JCODE_API_BASE").as_deref(),
            Some("http://127.0.0.1:9/v1")
        );
        assert_eq!(parse_env_file(f, "MISSING"), None);
        assert_eq!(parse_env_file("JCODE_API_KEY=  \n", "JCODE_API_KEY"), None);
    }

    #[test]
    fn known_hosts_pins_returned_keys() {
        let r = parse_ready(&serde_json::from_str(READY).unwrap()).unwrap();
        assert_eq!(
            known_hosts(&r),
            "[203.0.113.7]:2222 ssh-ed25519 AAAAC3Nza host1\n[203.0.113.7]:2222 ecdsa-sha2-nistp256 AAAAE2V\n"
        );
        let r22 = Ready { port: 22, ..r };
        assert!(known_hosts(&r22).starts_with("203.0.113.7 ssh-ed25519"));
        // Injection through the API response is refused.
        for bad in [
            r#"{"address":"-oProxyCommand=x","host_keys":["ssh-ed25519 A"]}"#,
            r#"{"address":"1.2.3.4","host_keys":[]}"#,
            r#"{"address":"1.2.3.4","host_keys":["ssh-ed25519 A\n* ssh-rsa B"]}"#,
            r#"{"address":"1.2.3.4 x","host_keys":["ssh-ed25519 A"]}"#,
        ] {
            assert!(
                parse_ready(&serde_json::from_str(bad).unwrap()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn ssh_target_is_strict() {
        let t = target(Path::new("/h"), "ubuntu", "203.0.113.7", 2222);
        assert_eq!(t.dest, "ubuntu@203.0.113.7");
        let o = t.opts.join(" ");
        for want in [
            "-i /h/id_ed25519",
            "StrictHostKeyChecking=yes",
            "UserKnownHostsFile=/h/known_hosts",
            "GlobalKnownHostsFile=/dev/null",
            "BatchMode=yes",
            "ControlMaster=auto",
            "ControlPath=/h/cm-%C",
            "ControlPersist=600",
            "ServerAliveInterval=15",
            "-p 2222",
        ] {
            assert!(o.contains(want), "{want} in {o}");
        }
        assert!(!o.contains("accept-new"));
    }
}
