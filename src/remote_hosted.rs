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
pub const SUBSCRIBE: &str = "remote builds need a plan ($20/mo, 5,000 builds): run `justrust upgrade --no-wait` and ask your user to open the checkout URL it prints. Builds run locally until then.";
pub const START_TRIAL: &str = "remote builds need a card on file: run `justrust upgrade --no-wait` and ask your user to open the URL it prints to start the free first month (card required, $20/mo after, 5,000 builds). Builds run locally until then.";
pub const UPGRADE_PLAN: &str = "remote builds need the $20 plan or higher (5,000 builds a month): ask your user to switch plans at https://jcode.sh/rust#subscribe. Builds run locally until then.";
pub const RAISE_LIMIT: &str = "monthly usage limit reached: ask your user to raise it at https://jcode.sh/account. Builds run locally until then.";
pub const END_TRIAL: &str = "free trial builds used up: ask your user to start their plan now at https://jcode.sh/account (it starts on its own when the trial ends). Builds run locally until then.";
pub const NO_CREDITS: &str = "out of Jcode cloud-compute credits (shared with cloud agents). Builds run locally until credits renew. There are no automatic top-ups.";
pub const TIME_EXHAUSTED: &str = "hosted build machine time for this month is used up (it grows with each charged build). Builds run locally until it resets.";
pub const UNAVAILABLE: &str = "the hosted build service is unavailable right now. This is not a problem with your subscription. Builds run locally, retry later.";
pub const NOT_OFFERED: &str = "hosted builds are not available on your Jcode account yet. This is not a problem with your subscription. Builds run locally.";
pub const RATE_LIMITED: &str = "the Jcode API is rate limiting requests. Retry in a moment.";
pub const AT_CAPACITY: &str = "every hosted build machine is busy right now (the fleet is at capacity). This is not a problem with your subscription. Builds run locally and remote is retried in a few minutes.";

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

    /// The curl `--config -` text carrying the bearer key (stdin only).
    pub(crate) fn curl_auth_config(&self) -> String {
        if self.key.is_empty() {
            return String::new();
        }
        let k = self.key.replace('\\', "\\\\").replace('"', "\\\"");
        format!("header = \"Authorization: Bearer {k}\"\n")
    }

    pub(crate) fn url(&self, suffix: &str) -> String {
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
    curl_with_timing(c, method, path, body, false)
}

fn admission_timing_enabled(method: &str, path: &str, trace: bool) -> bool {
    trace && method == "POST" && path == "build/runs"
}

fn curl_with_timing(
    c: &Creds,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timing: bool,
) -> (Command, String) {
    let mut cmd = Command::new("curl");
    let writeout = if timing {
        "\njustrust-admission-timing:%{http_code}\n%header{server-timing}"
    } else {
        "\n%{http_code}"
    };
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
        writeout,
        "--config",
        "-",
    ]);
    // %header is supported by curl 7.84+. Empty or unsupported header
    // output is ignored without changing the HTTP body or verdict.
    if timing {
        cmd.args(["-H", "X-Justrust-Timing: 1"]);
    }
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

/// Parse only bounded numeric fleet metrics. Never echo arbitrary response
/// headers, metric names, descriptions, IDs or control characters.
fn admission_timing(header: &str) -> Option<String> {
    const NAMES: [&str; 5] = [
        "jr_identity",
        "jr_charge",
        "jr_identity_wait",
        "jr_queue",
        "jr_total",
    ];
    if header.is_empty()
        || header.len() > 256
        || !header.is_ascii()
        || header.bytes().any(|b| b.is_ascii_control())
    {
        return None;
    }
    let mut values = [None; 5];
    for metric in header.split(',') {
        let (name, duration) = metric.trim().split_once(";dur=")?;
        let index = NAMES.iter().position(|candidate| *candidate == name)?;
        if values[index].is_some()
            || duration.is_empty()
            || !duration.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        {
            return None;
        }
        let duration: f64 = duration.parse().ok()?;
        if !duration.is_finite() || !(0.0..=60_000.0).contains(&duration) {
            return None;
        }
        values[index] = Some(duration);
    }
    let metrics: Vec<String> = NAMES
        .iter()
        .zip(values)
        .filter_map(|(name, value)| {
            value.map(|v| format!("{} {v:.1} ms", name.strip_prefix("jr_").unwrap()))
        })
        .collect();
    if metrics.is_empty() {
        None
    } else {
        Some(format!(
            "{} (awaited paths, identity overlaps)",
            metrics.join(" | ")
        ))
    }
}

fn curl_response(stdout: &str, timing: bool) -> (&str, u16, Option<String>) {
    if timing {
        // A valid JSON body cannot contain this unquoted literal newline
        // marker. Split before the header so hostile multiline metrics can
        // never remove body bytes or replace the authoritative status.
        let (body, footer) = stdout
            .split_once("\njustrust-admission-timing:")
            .unwrap_or((stdout, ""));
        let (code, header) = footer.split_once('\n').unwrap_or((footer, ""));
        (
            body,
            code.trim().parse().unwrap_or(0),
            admission_timing(header),
        )
    } else {
        let (body, code) = stdout.rsplit_once('\n').unwrap_or(("", stdout));
        (body, code.trim().parse().unwrap_or(0), None)
    }
}

pub fn request(c: &Creds, method: &str, path: &str, body: Option<&Value>) -> Result<Resp> {
    let timing = admission_timing_enabled(
        method,
        path,
        std::env::var_os("JUSTRUST_REMOTE_TRACE").is_some(),
    );
    let (mut cmd, config) = curl_with_timing(c, method, path, body, timing);
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
    let (text, code, metrics) = curl_response(&stdout, timing);
    if !out.status.success() || code == 0 {
        // Name the host actually contacted: build/... paths go to the fleet.
        let url = c.url(path);
        let host = url.split('/').take(3).collect::<Vec<_>>().join("/");
        bail!(
            "could not reach {host}: {}. Builds run locally.",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    if let Some(metrics) = metrics {
        eprintln!("justrust trace: admission {metrics}");
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
        (402, "build_time_exhausted") => return anyhow!(TIME_EXHAUSTED),
        (402, _) if r.body["error"]["next_step"].as_str() == Some("raise_limit") => {
            return anyhow!(RAISE_LIMIT);
        }
        (402, _) if r.body["error"]["next_step"].as_str() == Some("upgrade_plan") => {
            return anyhow!(UPGRADE_PLAN);
        }
        (402, _) if r.body["error"]["next_step"].as_str() == Some("end_trial") => {
            return anyhow!(END_TRIAL);
        }
        (402, _) if r.body["error"]["next_step"].as_str() == Some("start_trial") => {
            return anyhow!(START_TRIAL);
        }
        (402, _) | (403, "build_not_entitled") => return anyhow!(SUBSCRIBE),
        (429, _) => RATE_LIMITED.into(),
        (_, "build_capacity") => return anyhow!(AT_CAPACITY),
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

pub(crate) fn ok(r: Resp) -> Result<Value> {
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
    /// The account's plan (`/me` `plan`); `None` from an older API.
    #[serde(default)]
    pub plan: Option<Plan>,
}

/// The `plan` object of /me.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    #[serde(default)]
    pub subscribed: bool,
    #[serde(default)]
    pub plan_usd: Option<u32>,
    #[serde(default)]
    pub max_plan_usd: Option<u32>,
    /// Complimentary or legacy: not changeable self-serve.
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub trialing: bool,
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
            Some("start_trial") => s.push_str(
                ". Remote builds need a card: `justrust upgrade` starts the free first month",
            ),
            Some("raise_limit") => s.push_str(". Raise the limit at https://jcode.sh/account"),
            Some("end_trial") => {
                s.push_str(". Trial builds used up: start the plan at https://jcode.sh/account")
            }
            Some("upgrade_plan") => {
                s.push_str(". Remote builds need the $20 plan: https://jcode.sh/rust#subscribe")
            }
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
        plan: b["plan"]
            .is_object()
            .then(|| serde_json::from_value(b["plan"].clone()).ok())
            .flatten(),
    })
}

/// Signed in and entitled to build hosts.
pub fn check_access(c: &Creds) -> Result<Me> {
    let mut m = me(c)?;
    if !m.build_hosts {
        // /me can lag the fleet (an older API deploy without the capability
        // flags). The fleet is the source of truth: a 200 from build/host
        // means entitled, and a fleet outage is reported as one, not as an
        // account problem.
        match request(c, "GET", "build/host", None) {
            Ok(r) if r.code == 200 => {
                m.build_hosts = true;
                if m.builds.is_none() {
                    m.builds = Builds::from(&r.body["builds"]);
                }
                return Ok(m);
            }
            Ok(r) if r.code >= 500 || r.body["error"]["code"] == "build_unavailable" => {
                bail!(UNAVAILABLE)
            }
            Ok(r) if r.code == 402 || r.code == 429 => return Err(api_error(&r)),
            _ => {}
        }
        // A paying subscriber on a server that does not offer hosted builds
        // yet must not be told to subscribe.
        let paid = m
            .status
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("active"))
            && m.tier
                .as_deref()
                .is_some_and(|t| !t.is_empty() && t != "none");
        if paid {
            bail!(NOT_OFFERED);
        }
        bail!(SUBSCRIBE);
    }
    Ok(m)
}

/// `POST {build}/build/runs {run_id, cmd, project, client}` before a remote
/// run. `Err` holds the one line to show (the run must then not go remote).
/// `cmd` and `project` are short labels for the operator dashboard.
#[cfg(test)]
pub fn charge_run(c: &Creds, run_id: &str) -> std::result::Result<Option<Builds>, String> {
    charge_run_labeled(c, run_id, None, None)
}

pub fn charge_run_labeled(
    c: &Creds,
    run_id: &str,
    cmd: Option<&str>,
    project: Option<&str>,
) -> std::result::Result<Option<Builds>, String> {
    let mut body = json!({ "run_id": run_id });
    if let Some(cmd) = cmd {
        body["cmd"] = json!(run_label(cmd, 160));
    }
    if let Some(p) = project {
        body["project"] = json!(run_label(p, 160));
    }
    body["client"] = json!(concat!("justrust ", env!("CARGO_PKG_VERSION")));
    // Who: the account is the key. Machine and Jcode agent session tell the
    // operator which laptop and which agent the run came from.
    let h = crate::procfs::hostname();
    if !h.is_empty() {
        body["machine"] = json!(run_label(&h, 64));
    }
    if let Ok(s) = std::env::var("JCODE_SESSION_ID") {
        body["session"] = json!(run_label(&s, 96));
    }
    let r = request(c, "POST", "build/runs", Some(&body)).map_err(|e| e.to_string())?;
    if r.code == 200 {
        Ok(Builds::from(&r.body["builds"]))
    } else {
        Err(api_error(&r).to_string())
    }
}

/// Printable ASCII, one line, at most `max` chars: the server applies the
/// same rule, this just avoids sending what it would drop.
fn run_label(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// `POST {build}/build/runs/<id>/event` (running, done, abandoned) for the
/// operator dashboard. Fire and forget: a detached curl that never delays
/// or fails the build, and errors are ignored.
pub fn report_run_event(run_id: &str, event: &str, exit_code: Option<i32>, wall_s: Option<f64>) {
    let Ok(c) = creds() else {
        return;
    };
    let mut body = json!({ "event": event });
    if let Some(e) = exit_code {
        body["exit_code"] = json!(e);
    }
    if let Some(w) = wall_s {
        body["wall_s"] = json!((w * 100.0).round() / 100.0);
    }
    post_detached(&c, &format!("build/runs/{run_id}/event"), &body);
}

/// Body of `POST {build}/build/local`: why a signed-in client kept a build
/// local, which command, which client version and machine.
fn local_build_body(reason: &str, cmd: Option<&str>) -> Value {
    let mut body = json!({
        "reason": run_label(reason, 200),
        "client": concat!("justrust ", env!("CARGO_PKG_VERSION")),
    });
    if let Some(cmd) = cmd {
        body["cmd"] = json!(run_label(cmd, 160));
    }
    let h = crate::procfs::hostname();
    if !h.is_empty() {
        body["machine"] = json!(run_label(&h, 64));
    }
    body
}

/// `POST {build}/build/local` when a signed-in client builds locally, so the
/// operator sees setups that never reach the remote machine (and why).
/// Fire and forget like `report_run_event`. Signed out: nothing is sent.
pub fn report_local_build(reason: &str, cmd: Option<&str>) {
    if cfg!(test) || !signed_in() {
        return;
    }
    let Ok(c) = creds() else {
        return;
    };
    post_detached(&c, "build/local", &local_build_body(reason, cmd));
}

/// A detached curl POST that never delays or fails the build.
fn post_detached(c: &Creds, path: &str, body: &Value) {
    spawn_detached(curl(c, "POST", path, Some(body)));
}

/// An unauthenticated detached POST to the build service (`{build}/<path>`,
/// e.g. the anonymous install funnel). No key is sent.
pub fn post_anonymous_detached(path: &str, body: &Value) {
    let Ok(c) = Creds::anonymous(build_base()) else {
        return;
    };
    let url = format!("{}/{}", c.build_base, path.trim_start_matches('/'));
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
        "10",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "--data-binary",
        "@-",
        "-o",
        crate::sys::DEV_NULL,
        &url,
    ]);
    spawn_detached((cmd, body.to_string()));
}

fn spawn_detached((mut cmd, stdin_text): (Command, String)) {
    let Ok(mut child) = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_text.as_bytes());
    }
    // Reap in the background so no zombie outlives a long build. The
    // process exits right after a final event, and curl finishes on its own.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
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
    // The current transport executes Linux commands. Missing platform denotes
    // the legacy Linux API, not permission to accept an incompatible worker.
    if b.get("platform")
        .is_some_and(|platform| platform.as_str() != Some("linux"))
    {
        bail!("the hosted build API returned an unsupported worker platform, refusing to connect");
    }
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
    let body = json!({ "public_key": public_key.trim(), "platform": "linux" });
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
        std::thread::sleep(poll_wait(
            &r.body,
            poll,
            timeout.saturating_sub(start.elapsed()),
        ));
    }
}

/// How long to wait before the next connect poll: the server's
/// `retry_after_ms` hint (a fresh launch takes minutes, so it asks for 10 s
/// instead of 2 s), clamped to [`poll`, 15 s] and to the time left.
fn poll_wait(body: &Value, poll: Duration, left: Duration) -> Duration {
    let hint = body["retry_after_ms"]
        .as_u64()
        .map(Duration::from_millis)
        .unwrap_or(poll);
    hint.clamp(poll, Duration::from_secs(15).max(poll))
        .min(left)
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
    let _ = crate::sys::set_mode(p, mode);
}

fn write_private(p: &Path, content: &[u8]) -> Result<()> {
    let tmp = p.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = crate::sys::create_private(&tmp)?;
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
        format!("GlobalKnownHostsFile={}", crate::sys::DEV_NULL),
        "StrictHostKeyChecking=yes".into(),
        "BatchMode=yes".into(),
        "ConnectTimeout=10".into(),
        "LogLevel=ERROR".into(),
        "ServerAliveInterval=15".into(),
    ] {
        opts.extend(o(s));
    }
    // Windows OpenSSH has no connection multiplexing (it rejects
    // ControlMaster): each command opens its own connection there.
    if cfg!(unix) {
        for s in [
            "ControlMaster=auto".to_string(),
            format!("ControlPath={}", crate::remote_backend::control_path(d)),
            "ControlPersist=600".into(),
        ] {
            opts.extend(o(s));
        }
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
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(d.join("lock"))?;
        let _ = f.lock();
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
        timing: Option<String>,
        body: String,
    }

    /// Serves `responses` in order (status, json body), one per connection.
    fn mock(responses: Vec<(u16, &'static str)>) -> (Creds, Arc<Mutex<Vec<Req>>>) {
        mock_headers(responses, "")
    }

    fn mock_headers(
        responses: Vec<(u16, &'static str)>,
        headers: &'static str,
    ) -> (Creds, Arc<Mutex<Vec<Req>>>) {
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
                let (mut len, mut auth, mut timing) = (0usize, None, None);
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
                        "x-justrust-timing" => timing = Some(v.trim().to_string()),
                        _ => {}
                    }
                }
                let mut b = vec![0; len];
                r.read_exact(&mut b).unwrap();
                s2.lock().unwrap().push(Req {
                    method,
                    path,
                    auth,
                    timing,
                    body: String::from_utf8(b).unwrap(),
                });
                write!(
                    sock,
                    "HTTP/1.1 {code} X\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    fn poll_wait_honors_server_hint_within_bounds() {
        let p = Duration::from_secs(2);
        let left = Duration::from_secs(300);
        assert_eq!(poll_wait(&json!({}), p, left), p);
        assert_eq!(
            poll_wait(&json!({"retry_after_ms": 10000}), p, left),
            Duration::from_secs(10)
        );
        // Never faster than the client's own poll, never past 15 s or the deadline.
        assert_eq!(poll_wait(&json!({"retry_after_ms": 1}), p, left), p);
        assert_eq!(
            poll_wait(&json!({"retry_after_ms": 600000}), p, left),
            Duration::from_secs(15)
        );
        assert_eq!(
            poll_wait(&json!({"retry_after_ms": 10000}), p, Duration::from_secs(3)),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn capacity_is_its_own_message() {
        let (c, _) = mock(vec![(
            503,
            r#"{"error":{"code":"build_capacity","message":"busy","retry_after":120}}"#,
        )]);
        let e = connect_with(
            &c,
            "k",
            Duration::from_millis(5),
            Duration::from_secs(5),
            |_| {},
        )
        .unwrap_err()
        .to_string();
        assert_eq!(e, AT_CAPACITY);
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
                402,
                r#"{"error":{"code":"build_time_exhausted","message":"used up","next_step":null}}"#,
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
        // A paying account out of machine time is not told to subscribe.
        let s = e(me(&c));
        assert!(
            s.contains("machine time") && !s.contains("Subscribe"),
            "{s}"
        );
        let s = e(me(&c));
        assert!(
            s.contains("unavailable") && s.contains("not a problem"),
            "{s}"
        );
        assert!(e(me(&c)).contains("rate limiting"));
        assert!(e(check_access(&c)).contains("justrust upgrade"));
    }

    /// Paying subscriber, server without hosted builds: never "subscribe".
    #[test]
    fn paid_account_without_capability_is_not_told_to_subscribe() {
        let (c, _) = mock(vec![(
            200,
            r#"{"email":"a@b","tier":"plus","status":"active","capabilities":{"remote_compile":false}}"#,
        )]);
        let s = check_access(&c).unwrap_err().to_string();
        assert!(s.contains("not available on your Jcode account yet"), "{s}");
        assert!(!s.contains("upgrade"), "{s}");
    }

    #[test]
    fn local_build_body_is_bounded_and_labelled() {
        let b = local_build_body(
            "remote failed\n in the last 10 minutes",
            Some("test -p a\t--lib"),
        );
        assert_eq!(b["reason"], "remote failed in the last 10 minutes");
        assert_eq!(b["cmd"], "test -p a --lib");
        assert!(b["client"].as_str().unwrap().starts_with("justrust "));
        let long = "x".repeat(500);
        assert_eq!(
            local_build_body(&long, None)["reason"]
                .as_str()
                .unwrap()
                .len(),
            200
        );
        assert!(local_build_body("r", None).get("cmd").is_none());
    }

    /// /me without capability flags (older API deploy) but the fleet says 200:
    /// entitled, with builds taken from the fleet.
    #[test]
    fn fleet_200_overrides_stale_me_capabilities() {
        let (c, seen) = mock(vec![
            (
                200,
                r#"{"email":"a@b","tier":"plus","status":"active","capabilities":{"remote_compile":false}}"#,
            ),
            (
                200,
                r#"{"state":"stopped","builds":{"trial_remaining":15,"can_build":true}}"#,
            ),
        ]);
        let m = check_access(&c).unwrap();
        assert!(m.build_hosts);
        assert_eq!(m.builds.unwrap().left(), 15);
        assert!(seen.lock().unwrap()[1].path.ends_with("/build/host"));
    }

    /// Fleet down while /me lacks the flag: an outage, not an account problem.
    #[test]
    fn fleet_503_is_unavailable_not_not_offered() {
        let (c, _) = mock(vec![
            (
                200,
                r#"{"email":"a@b","tier":"plus","status":"active","capabilities":{}}"#,
            ),
            (
                503,
                r#"{"error":{"code":"build_unavailable","message":"x"}}"#,
            ),
        ]);
        let s = check_access(&c).unwrap_err().to_string();
        assert!(s.contains("unavailable right now"), "{s}");
    }

    /// Unreachable fleet: the error names the fleet host, not the API.
    #[test]
    fn unreachable_error_names_the_contacted_host() {
        let mut c = Creds::new("http://127.0.0.1:9/v1".into(), "k".into()).unwrap();
        c.set_build_base("http://localhost:9/v1".into()).unwrap();
        let s = request(&c, "GET", "build/host", None)
            .unwrap_err()
            .to_string();
        assert!(s.contains("http://localhost:9"), "{s}");
        let s = request(&c, "GET", "me", None).unwrap_err().to_string();
        assert!(s.contains("http://127.0.0.1:9"), "{s}");
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
            (
                402,
                r#"{"error":{"code":"builds_exhausted","message":"no","next_step":"upgrade_plan"}}"#,
            ),
            (
                402,
                r#"{"error":{"code":"builds_exhausted","message":"no","next_step":"end_trial"}}"#,
            ),
            (
                402,
                r#"{"error":{"code":"builds_exhausted","message":"no","next_step":"start_trial"}}"#,
            ),
        ]);
        let api = c.base.clone();
        // Fleet paths use the build base, account paths stay on the API.
        c.set_build_base(format!("{api}/fleet")).unwrap();
        assert_eq!(charge_run(&c, "r1").unwrap().unwrap().left(), 24);
        let sub = charge_run(&c, "r2").unwrap_err();
        assert!(sub.starts_with("remote builds need a plan ($20/mo, 5,000 builds)"));
        assert!(
            sub.contains("justrust upgrade") && !sub.contains('\n'),
            "{sub}"
        );
        let lim = charge_run(&c, "r3").unwrap_err();
        assert!(lim.contains("https://jcode.sh/account"), "{lim}");
        let up = charge_run(&c, "r4").unwrap_err();
        assert!(
            up.contains("$20 plan") && up.contains("jcode.sh/rust#subscribe"),
            "{up}"
        );
        let trial = charge_run(&c, "r5").unwrap_err();
        assert!(
            trial.starts_with("free trial builds used up") && trial.contains("jcode.sh/account"),
            "{trial}"
        );
        let card = charge_run(&c, "r6").unwrap_err();
        assert!(
            card.starts_with("remote builds need a card on file")
                && card.contains("justrust upgrade --no-wait")
                && !card.contains('\n'),
            "{card}"
        );
        let s = seen.lock().unwrap();
        assert_eq!(s[0].path, "/v1/fleet/build/runs");
        let b: Value = serde_json::from_str(&s[0].body).unwrap();
        assert_eq!(b["run_id"], "r1");
        assert!(b["client"].as_str().unwrap().starts_with("justrust "));
        assert!(b.get("cmd").is_none());
        assert!(s[0].auth.as_deref() == Some("Bearer sk-secret-123"));
        assert_eq!(c.url("me"), format!("{api}/me"));
    }

    #[test]
    fn run_labels_are_printable_single_line_and_bounded() {
        assert_eq!(
            run_label("test  -p\tcore\n\u{1b}[31m", 160),
            "test -p core [31m"
        );
        assert_eq!(run_label(&"x".repeat(300), 160).len(), 160);
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
    fn admission_timing_is_numeric_bounded_and_fixed_name_only() {
        assert_eq!(admission_timing("jr_queue;dur=97, jr_identity;dur=67.2, jr_charge;dur=271.0, jr_identity_wait;dur=0, jr_total;dur=365"), Some("identity 67.2 ms | charge 271.0 ms | identity_wait 0.0 ms | queue 97.0 ms | total 365.0 ms (awaited paths, identity overlaps)".into()));
        assert!(admission_timing("jr_total;dur=60000").is_some());
        for bad in [
            "",
            "jr_charge;dur=NaN",
            "jr_charge;dur=inf",
            "jr_charge;dur=-1",
            "jr_charge;dur=60000.1",
            "jr_charge;dur=1e3",
            "jr_charge;dur=.",
            "jr_charge;dur=1..2",
            "jr_charge;dur=",
            "jr_charge;dur=1;desc=sk-secret",
            "sk-secret;dur=1",
            "jr_charge;dur=1,jr_charge;dur=2",
            "jr_charge;dur=1,",
            "jr_charge;dur=1\n200",
            "jr_charge;dur=1\r",
            "jr_charge;dur=1\t",
            "jr_charge;dur=1\u{1b}[31m",
            "jr_charge;dur=é",
            "%header{server-timing}",
        ] {
            assert!(admission_timing(bad).is_none(), "{bad:?}");
        }
        assert!(admission_timing(&format!("jr_charge;dur={}1", "0".repeat(256))).is_none());
    }

    #[test]
    fn admission_trace_is_scoped_to_existing_charge_only() {
        assert!(admission_timing_enabled("POST", "build/runs", true));
        for (method, path, trace) in [
            ("POST", "build/runs", false),
            ("GET", "build/runs", true),
            ("POST", "build/host/connect", true),
            ("POST", "build/runs/event", true),
            ("GET", "me", true),
        ] {
            assert!(!admission_timing_enabled(method, path, trace));
        }
    }

    #[test]
    fn admission_curl_keeps_default_wire_and_key_stdin_only() {
        let c = Creds::new("https://build.jcode.sh/v1".into(), "sk-secret-123".into()).unwrap();
        let body = json!({"run_id":"r1"});
        let (default, config) = curl_with_timing(&c, "POST", "build/runs", Some(&body), false);
        let args: Vec<_> = default
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let expected: Vec<String> = [
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
            "POST",
            "-H",
            "Accept: application/json",
            "-w",
            "\n%{http_code}",
            "--config",
            "-",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
        ]
        .into_iter()
        .map(String::from)
        .chain([body.to_string(), c.url("build/runs")])
        .collect();
        assert_eq!(args, expected);
        let (trace, trace_config) = curl_with_timing(&c, "POST", "build/runs", Some(&body), true);
        let trace_args: Vec<_> = trace
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(trace_config, config);
        assert!(trace_args.iter().all(|a| !a.contains("sk-secret")));
        assert!(
            trace_args
                .windows(2)
                .any(|w| w == ["-H", "X-Justrust-Timing: 1"])
        );
        assert!(
            trace_args
                .windows(2)
                .any(|w| w == ["--data-binary", &body.to_string()])
        );
        assert!(trace_args.windows(2).any(|w| w == ["--config", "-"]));
        assert_eq!(trace_args.last(), args.last());
    }

    #[test]
    fn admission_footer_preserves_body_status_and_legacy_empty_metrics() {
        let body = serde_json::to_string_pretty(&json!({"message":"\njustrust-admission-timing:200", "error":{"code":"build_time_exhausted"}})).unwrap();
        for code in [200, 400, 401, 402, 503] {
            let raw = format!("{body}\n{code}");
            let plain = curl_response(&raw, false);
            assert_eq!(plain, (body.as_str(), code, None));
            for header in [
                "jr_charge;dur=271",
                "",
                "%header{server-timing}",
                "jr_charge;dur=NaN",
                "jr_charge;dur=1\njustrust-admission-timing:200\njr_total;dur=5",
                "jr_charge;dur=1;desc=sk-secret",
            ] {
                let raw = format!("{body}\njustrust-admission-timing:{code}\n{header}");
                let (parsed_body, parsed_code, metrics) = curl_response(&raw, true);
                assert_eq!(parsed_body, body);
                assert_eq!(parsed_code, code);
                assert_eq!(metrics.is_some(), header == "jr_charge;dur=271");
                assert_eq!(
                    serde_json::from_str::<Value>(parsed_body).unwrap(),
                    serde_json::from_str::<Value>(&body).unwrap()
                );
            }
        }
        assert_eq!(curl_response("", true), ("", 0, None));
        assert_eq!(
            curl_response("{}\njustrust-admission-timing:000\n", true),
            ("{}", 0, None)
        );
    }

    #[test]
    fn admission_trace_real_curl_preserves_refusal_and_optional_header() {
        for (header, has_metrics) in [
            ("", false),
            ("Server-Timing: jr_charge;dur=271.0\r\n", true),
            ("Server-Timing: jr_charge;dur=1;desc=sk-secret\r\n", false),
        ] {
            for code in [200, 402, 503] {
                let (c, seen) = mock_headers(
                    vec![(code, r#"{"error":{"code":"build_time_exhausted"}}"#)],
                    header,
                );
                let body = json!({"run_id":"trace-fixture"});
                let (mut cmd, config) =
                    curl_with_timing(&c, "POST", "build/runs", Some(&body), true);
                let mut child = cmd
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(config.as_bytes())
                    .unwrap();
                let out = child.wait_with_output().unwrap();
                assert!(out.status.success());
                let raw = String::from_utf8(out.stdout).unwrap();
                let (text, status, metrics) = curl_response(&raw, true);
                assert_eq!(status, code);
                assert_eq!(
                    serde_json::from_str::<Value>(text).unwrap()["error"]["code"],
                    "build_time_exhausted"
                );
                assert_eq!(metrics.is_some(), has_metrics);
                let requests = seen.lock().unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].method, "POST");
                assert_eq!(requests[0].path, "/v1/build/runs");
                assert_eq!(requests[0].timing.as_deref(), Some("1"));
                assert_eq!(requests[0].auth.as_deref(), Some("Bearer sk-secret-123"));
                assert_eq!(
                    serde_json::from_str::<Value>(&requests[0].body).unwrap(),
                    body
                );
            }
        }
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
    fn worker_platform_is_checked_before_connecting() {
        let mut ready: Value = serde_json::from_str(READY).unwrap();
        assert!(parse_ready(&ready).is_ok());
        ready["platform"] = json!("linux");
        assert!(parse_ready(&ready).is_ok());
        for platform in [
            json!("windows"),
            json!("macos"),
            json!(""),
            json!(null),
            json!(42),
        ] {
            ready["platform"] = platform;
            let err = parse_ready(&ready).unwrap_err();
            assert!(err.to_string().contains("unsupported worker platform"));
        }
    }

    #[test]
    fn ssh_target_is_strict() {
        let h = Path::new("/h");
        let t = target(h, "ubuntu", "203.0.113.7", 2222);
        assert_eq!(t.dest, "ubuntu@203.0.113.7");
        let o = t.opts.join(" ");
        let mut wants = vec![
            format!("-i {}", h.join("id_ed25519").display()),
            "StrictHostKeyChecking=yes".into(),
            format!("UserKnownHostsFile={}", h.join("known_hosts").display()),
            format!("GlobalKnownHostsFile={}", crate::sys::DEV_NULL),
            "BatchMode=yes".into(),
            "ServerAliveInterval=15".into(),
            "-p 2222".into(),
        ];
        if cfg!(unix) {
            wants.extend([
                "ControlMaster=auto".into(),
                "ControlPath=/h/cm-%C".into(),
                "ControlPersist=600".into(),
            ]);
        } else {
            assert!(!o.contains("ControlMaster"), "{o}");
        }
        for want in wants {
            assert!(o.contains(&want), "{want} in {o}");
        }
        assert!(!o.contains("accept-new"));
    }
}
