//! `justrust login`, `logout`, `upgrade`: the funnel from install to hosted
//! remote builds. Built so a coding agent can drive it alone: the only human
//! steps are opening the sign-in link and paying.
//!
//! ```text
//! POST {api}/auth/device {client_name}  -> {device_code, verification_uri_complete, interval, expires_in}
//! POST {api}/auth/token  {device_code}  -> 428 authorization_pending | 429 slow_down {interval}
//!                                          | 200 {api_key, account_id, email, tier}
//! GET  {api}/me                         -> {builds:{...}}
//! POST {api}/billing/subscribe {plan_usd} -> {url}
//!
//! ~/.config/jcode/jcode-subscription.env        shared with Jcode, 0600
//! ~/.justrust/remote/hosted/pending-login.json  `login --no-wait` in flight, 0600
//! ```
//!
//! `login --no-wait` saves the device code and exits. Every later justrust
//! command calls [`poll_pending`], which sends at most one token request per
//! interval, never blocks on the human, and completes the login once it is
//! approved.

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::remote_hosted::{self as hosted, Builds, Creds};

const CLIENT_NAME: &str = "justrust-cli";
const KEY_LINES: [&str; 4] = [
    "JCODE_API_KEY",
    "JCODE_ACCOUNT_ID",
    "JCODE_ACCOUNT_EMAIL",
    "JCODE_TIER",
];
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(600);

fn now() -> u64 {
    crate::paths::now() as u64
}

fn interactive() -> bool {
    std::io::stdout().is_terminal() && std::io::stderr().is_terminal()
}

/// Best effort. Never blocks, never fails the command.
fn open_browser(url: &str) {
    if std::env::var_os("JUSTRUST_NO_BROWSER").is_some() {
        return;
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// The URL for the human, readable by an agent that relays it.
fn show_url(what: &str, url: &str) {
    if interactive() {
        println!("Open this link to {what} (opening your browser):");
    } else {
        println!("Ask your user to open this link to {what}:");
    }
    println!("{url}");
}

// ------------------------------------------------------------- env file

/// Replace the account lines in `old`, keeping every unrelated line.
pub fn merge_env(old: &str, vals: &[(&str, &str)]) -> String {
    let mut out: String = old
        .lines()
        .filter(|l| !is_key_line(l))
        .map(|l| format!("{l}\n"))
        .collect();
    for (k, v) in vals {
        if !v.is_empty() {
            out.push_str(&format!("{k}={v}\n"));
        }
    }
    out
}

fn is_key_line(l: &str) -> bool {
    let t = l.trim();
    let t = t.strip_prefix("export ").unwrap_or(t).trim_start();
    t.split_once('=')
        .is_some_and(|(k, _)| KEY_LINES.contains(&k.trim()))
}

pub fn strip_keys(old: &str) -> String {
    merge_env(old, &[])
}

fn write_0600(p: &Path, content: &str) -> Result<()> {
    use std::io::Write;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = crate::sys::create_private(&tmp)?;
    f.write_all(content.as_bytes())?;
    drop(f);
    std::fs::rename(&tmp, p)?;
    let _ = crate::sys::set_mode(p, 0o600);
    Ok(())
}

fn env_path() -> Result<PathBuf> {
    hosted::env_file_path().ok_or_else(|| anyhow!("no config directory"))
}

fn save_creds(tok: &Value) -> Result<()> {
    let key = tok["api_key"].as_str().unwrap_or("");
    if key.is_empty() || key.chars().any(|c| c.is_control() || c == ' ') {
        bail!("the Jcode API returned no usable API key");
    }
    let s = |k: &str| {
        tok[k]
            .as_str()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control() && *c != ' ')
            .collect::<String>()
    };
    let p = env_path()?;
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    let new = merge_env(
        &old,
        &[
            ("JCODE_API_KEY", key),
            ("JCODE_ACCOUNT_ID", &s("account_id")),
            ("JCODE_ACCOUNT_EMAIL", &s("email")),
            ("JCODE_TIER", &s("tier")),
        ],
    );
    write_0600(&p, &new)
}

// ----------------------------------------------------------- device flow

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    pub base: String,
    pub device_code: String,
    pub url: String,
    pub interval: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub last_poll: u64,
}

pub fn start(base: &str) -> Result<Pending> {
    let c = Creds::anonymous(base.to_string())?;
    let r = hosted::request(
        &c,
        "POST",
        "auth/device",
        Some(&json!({ "client_name": CLIENT_NAME })),
    )?;
    if r.code != 200 {
        return Err(hosted::api_error(&r));
    }
    let b = &r.body;
    let code = b["device_code"].as_str().unwrap_or("");
    let url = b["verification_uri_complete"]
        .as_str()
        .or(b["verify_url"].as_str())
        .unwrap_or("");
    if code.is_empty() || !url.starts_with("https://") && !url.starts_with("http://") {
        bail!("the Jcode API returned an unusable sign-in flow");
    }
    let p = Pending {
        base: c.base.clone(),
        device_code: code.to_string(),
        url: url.chars().filter(|c| !c.is_control()).collect(),
        interval: b["interval"].as_u64().unwrap_or(5).clamp(1, 60),
        expires_at: now() + b["expires_in"].as_u64().unwrap_or(600),
        last_poll: 0,
    };
    report_login_start(&p.url);
    Ok(p)
}

/// The install id the installer saved (`~/.justrust/install_id`), if any and
/// not opted out (`JUSTRUST_NO_TELEMETRY`, `DO_NOT_TRACK` other than 0).
fn install_id() -> Option<String> {
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    if set("JUSTRUST_NO_TELEMETRY")
        || std::env::var("DO_NOT_TRACK").is_ok_and(|v| !v.is_empty() && v != "0")
    {
        return None;
    }
    let s = std::fs::read_to_string(crate::paths::home().ok()?.join("install_id")).ok()?;
    let s = s.trim();
    is_uuid(s).then(|| s.to_string())
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// `flow=<uuid>` from the sign-in URL: joins the install to the device flow.
fn flow_id(url: &str) -> Option<&str> {
    let q = url.split_once('?')?.1;
    q.split('&')
        .find_map(|kv| kv.strip_prefix("flow="))
        .filter(|f| is_uuid(f))
}

fn login_start_body(install_id: &str, url: &str) -> Value {
    let mut b = json!({
        "install_id": install_id,
        "stage": "login_start",
        "source": "cli",
        "client": concat!("justrust ", env!("CARGO_PKG_VERSION")),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    });
    if let Some(f) = flow_id(url) {
        b["flow_id"] = json!(f);
    }
    b
}

/// Anonymous `login_start` install event (see scripts/install.sh): only when
/// the installer saved an install id, so opted-out and source installs send
/// nothing. Fire and forget.
fn report_login_start(url: &str) {
    if cfg!(test) {
        return;
    }
    let Some(id) = install_id() else { return };
    hosted::post_anonymous_detached("install/event", &login_start_body(&id, url));
}

#[derive(Debug)]
pub enum Poll {
    Pending,
    Done(Value),
}

/// One POST /auth/token. Raises `p.interval` on slow_down.
pub fn poll_once(p: &mut Pending) -> Result<Poll> {
    let c = Creds::anonymous(p.base.clone())?;
    p.last_poll = now();
    let r = hosted::request(
        &c,
        "POST",
        "auth/token",
        Some(&json!({ "device_code": p.device_code })),
    )?;
    match (r.code, r.body["error"]["code"].as_str().unwrap_or("")) {
        (200, _) => Ok(Poll::Done(r.body)),
        (428, _) | (_, "authorization_pending") => Ok(Poll::Pending),
        (429, _) | (_, "slow_down") => {
            p.interval = r.body["error"]["interval"]
                .as_u64()
                .unwrap_or(p.interval + 2)
                .clamp(p.interval, 60);
            Ok(Poll::Pending)
        }
        (_, "access_denied") => bail!("the sign-in was denied. Run `justrust login` again."),
        (_, "expired_token") => {
            bail!("the sign-in link expired. Run `justrust login --no-wait` again.")
        }
        _ => Err(hosted::api_error(&r)),
    }
}

/// Block until approved or expired.
pub fn wait(p: &mut Pending, sleep: &dyn Fn(Duration)) -> Result<Value> {
    loop {
        if now() >= p.expires_at {
            bail!("the sign-in link expired. Run `justrust login` again.");
        }
        if let Poll::Done(v) = poll_once(p)? {
            return Ok(v);
        }
        sleep(Duration::from_secs(p.interval));
    }
}

fn pending_file() -> Option<PathBuf> {
    Some(
        crate::paths::home()
            .ok()?
            .join("remote/hosted/pending-login.json"),
    )
}

pub fn load_pending() -> Option<Pending> {
    let p: Pending = serde_json::from_slice(&std::fs::read(pending_file()?).ok()?).ok()?;
    if now() >= p.expires_at {
        clear_pending();
        return None;
    }
    Some(p)
}

fn save_pending(p: &Pending) -> Result<()> {
    let f = pending_file().ok_or_else(|| anyhow!("no home directory"))?;
    write_0600(&f, &serde_json::to_string(p)?)
}

fn clear_pending() {
    if let Some(f) = pending_file() {
        let _ = std::fs::remove_file(f);
    }
}

/// After a login: select hosted.
fn finish(tok: &Value) -> Result<()> {
    save_creds(tok)?;
    clear_pending();
    select_hosted()
}

/// Signing in is the explicit way onto hosted builds, so it selects hosted
/// even over the legacy own-AWS machine, saying so once. `remote use aws`
/// switches back. `remote use off` (no backend.json, no AWS state) stays a
/// fresh start that login also fills in.
fn select_hosted() -> Result<()> {
    use crate::remote_backend::{Backend, load, save};
    match load() {
        Some(Backend::Hosted) => {}
        Some(Backend::Aws) => {
            save(&Backend::Hosted)?;
            // stderr: login_link runs inside `justrust mcp`, whose stdout is
            // the protocol.
            eprintln!(
                "remote builds now use hosted (were: your own AWS machine, `justrust remote use aws` to switch back)"
            );
        }
        None => save(&Backend::Hosted)?,
    }
    Ok(())
}

/// Called by every justrust command: complete a `login --no-wait` once the
/// human has approved it. At most one request per interval, never blocks
/// long (curl's own timeouts), silent unless it completes.
pub fn poll_pending() {
    let Some(mut p) = load_pending() else { return };
    if now() < p.last_poll + p.interval {
        return;
    }
    match poll_once(&mut p) {
        Ok(Poll::Done(tok)) => match finish(&tok) {
            Ok(()) => eprintln!(
                "justrust: signed in to Jcode{}, hosted remote builds are on",
                tok["email"]
                    .as_str()
                    .map(|e| format!(" as {e}"))
                    .unwrap_or_default()
            ),
            Err(e) => eprintln!("justrust: completing sign-in failed: {e:#}"),
        },
        Ok(Poll::Pending) => {
            let _ = save_pending(&p);
        }
        // Denied or expired: drop it silently. Network trouble: retry later.
        Err(e) if e.to_string().contains("Run `justrust login") => clear_pending(),
        Err(_) => {
            let _ = save_pending(&p);
        }
    }
}

fn print_builds() {
    let Ok(c) = hosted::creds() else { return };
    match hosted::me(&c) {
        Ok(m) => match m.builds {
            Some(b) => println!("hosted builds: {}", b.describe()),
            None => println!("signed in"),
        },
        Err(e) => println!("signed in ({e})"),
    }
}

/// `justrust login [--no-wait]`.
pub fn login(no_wait: bool) -> Result<()> {
    if hosted::signed_in() {
        println!(
            "already signed in to Jcode ({}). `justrust logout` to switch accounts.",
            env_path()?.display()
        );
        select_hosted()?;
        print_builds();
        return Ok(());
    }
    let mut p = match load_pending() {
        Some(p) => p,
        None => start(&hosted::api_base())?,
    };
    save_pending(&p)?;
    show_url("sign in to Jcode", &p.url);
    open_browser(&p.url);
    if no_wait {
        println!(
            "justrust: keep working, builds stay local. Any later justrust command finishes the sign-in once it is approved."
        );
        return Ok(());
    }
    let tok = wait(&mut p, &|d| std::thread::sleep(d))?;
    finish(&tok)?;
    println!(
        "signed in{}",
        tok["email"]
            .as_str()
            .map(|e| format!(" as {e}"))
            .unwrap_or_default()
    );
    print_builds();
    Ok(())
}

/// For `justrust mcp`: the sign-in link to relay to the human, starting or
/// resuming a device flow that later justrust commands complete. `None` when
/// already signed in.
pub fn login_link() -> Result<Option<String>> {
    poll_pending();
    if hosted::signed_in() {
        select_hosted()?;
        return Ok(None);
    }
    let p = match load_pending() {
        Some(p) => p,
        None => start(&hosted::api_base())?,
    };
    save_pending(&p)?;
    Ok(Some(p.url))
}

/// `justrust logout`: drop only the account lines.
pub fn logout() -> Result<()> {
    clear_pending();
    let p = env_path()?;
    match std::fs::read_to_string(&p) {
        Ok(old) => {
            write_0600(&p, &strip_keys(&old))?;
            println!("signed out ({} kept other settings)", p.display());
        }
        Err(_) => println!("not signed in"),
    }
    Ok(())
}

/// `justrust upgrade [--plan N] [--no-wait]`.
pub fn upgrade(plan: u32, no_wait: bool) -> Result<()> {
    let c = hosted::creds()?;
    // No card yet and the free first month is open ($20 plan only): start it.
    let trial = plan == 20
        && hosted::me(&c)
            .ok()
            .and_then(|m| m.builds)
            .and_then(|b| b.next_step)
            .as_deref()
            == Some("start_trial");
    let mut r = hosted::request(
        &c,
        "POST",
        "billing/subscribe",
        Some(&checkout_body(plan, trial)),
    )?;
    if trial && r.code == 409 && r.body["error"]["code"].as_str() == Some("trial_used") {
        r = hosted::request(
            &c,
            "POST",
            "billing/subscribe",
            Some(&checkout_body(plan, false)),
        )?;
    }
    if !(200..300).contains(&r.code) {
        if r.code == 409 {
            // Already subscribed: nothing to buy, the limit is what to raise.
            return Err(hosted::api_error(&r));
        }
        let e = hosted::api_error(&r);
        bail!(
            "{e}\njustrust: checkout could not start here. Ask your user to subscribe at https://jcode.sh/rust#subscribe (same account, $20 plan or higher), then keep working: builds go remote once it is active."
        );
    }
    let url = r.body["url"].as_str().unwrap_or("");
    if !url.starts_with("https://") {
        bail!("the Jcode API returned no checkout URL");
    }
    show_url(
        &if trial {
            "start the free first month (card required, $20/mo after)".to_string()
        } else {
            format!("subscribe (${plan}/mo)")
        },
        url,
    );
    open_browser(url);
    if no_wait {
        println!(
            "justrust: keep working, builds stay local. Remote builds start by themselves once the card is added."
        );
        return Ok(());
    }
    let b = wait_can_build(&c, UPGRADE_TIMEOUT, Duration::from_secs(5))?;
    println!("ready: {}", b.describe());
    Ok(())
}

/// `POST billing/subscribe` body. `trial` asks for the free first month
/// (card collected now, the plan renews after it).
fn checkout_body(plan: u32, trial: bool) -> Value {
    if trial {
        json!({ "plan_usd": plan, "trial": true })
    } else {
        json!({ "plan_usd": plan })
    }
}

pub fn wait_can_build(c: &Creds, timeout: Duration, every: Duration) -> Result<Builds> {
    let t0 = Instant::now();
    loop {
        if let Ok(m) = hosted::me(c)
            && let Some(b) = m.builds
            && b.can_build
        {
            return Ok(b);
        }
        if t0.elapsed() >= timeout {
            bail!(
                "no completed payment after {} minutes. Run `justrust upgrade` again, or `justrust remote status` to check.",
                timeout.as_secs() / 60
            );
        }
        std::thread::sleep(every);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// One response per connection, records "METHOD path body".
    fn mock(responses: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
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
                let (mut len, mut auth) = (0usize, false);
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    let h = h.trim_end().to_ascii_lowercase();
                    if h.is_empty() {
                        break;
                    }
                    if let Some(v) = h.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    auth |= h.starts_with("authorization");
                }
                let mut b = vec![0; len];
                r.read_exact(&mut b).unwrap();
                let mut it = line.split_whitespace();
                s2.lock().unwrap().push(format!(
                    "{} {} {}{}",
                    it.next().unwrap(),
                    it.next().unwrap(),
                    String::from_utf8(b).unwrap(),
                    if auth { " +auth" } else { "" }
                ));
                write!(
                    sock,
                    "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (base, seen)
    }

    #[test]
    fn login_start_joins_install_and_flow() {
        let id = "0b7c3c1e-1111-4abc-8def-0123456789ab";
        let b = login_start_body(
            id,
            "https://jcode.sh/account?flow=fe8c4a11-a358-47bc-aaf3-96eb7fc10165&client=justrust",
        );
        assert_eq!(b["install_id"], id);
        assert_eq!(b["stage"], "login_start");
        assert_eq!(b["flow_id"], "fe8c4a11-a358-47bc-aaf3-96eb7fc10165");
        assert!(b["client"].as_str().unwrap().starts_with("justrust "));
        let b = login_start_body(id, "https://jcode.sh/account?flow=../../x");
        assert!(b.get("flow_id").is_none());
        assert!(is_uuid(id));
        assert!(!is_uuid("0b7c3c1e-1111-4abc-8def-0123456789a"));
        assert!(!is_uuid("0b7c3c1e_1111-4abc-8def-0123456789ab"));
    }

    #[test]
    fn device_flow_honours_pending_and_slow_down() {
        let (base, seen) = mock(vec![
            (
                200,
                r#"{"device_code":"dc1","verification_uri_complete":"https://jcode.sh/account?flow=f","interval":1,"expires_in":600}"#,
            ),
            (
                428,
                r#"{"error":{"code":"authorization_pending","message":"x"}}"#,
            ),
            (
                429,
                r#"{"error":{"code":"slow_down","message":"x","interval":7}}"#,
            ),
            (
                200,
                r#"{"api_key":"jk_abc","account_id":"acc1","email":"a@b.c","tier":"free"}"#,
            ),
        ]);
        let mut p = start(&base).unwrap();
        assert_eq!(p.url, "https://jcode.sh/account?flow=f");
        let sleeps = Mutex::new(Vec::new());
        let tok = wait(&mut p, &|d| sleeps.lock().unwrap().push(d.as_secs())).unwrap();
        assert_eq!(tok["api_key"], "jk_abc");
        assert_eq!(*sleeps.lock().unwrap(), vec![1, 7]);
        let s = seen.lock().unwrap();
        assert_eq!(
            s[0],
            r#"POST /v1/auth/device {"client_name":"justrust-cli"}"#
        );
        assert_eq!(s[1], r#"POST /v1/auth/token {"device_code":"dc1"}"#);
        // No key is sent before there is one.
        assert!(s.iter().all(|l| !l.ends_with("+auth")), "{s:?}");
    }

    #[test]
    fn denied_and_expired_are_terminal() {
        let (base, _) = mock(vec![
            (400, r#"{"error":{"code":"access_denied","message":"x"}}"#),
            (400, r#"{"error":{"code":"expired_token","message":"x"}}"#),
        ]);
        let mut p = Pending {
            base,
            device_code: "d".into(),
            url: "u".into(),
            interval: 1,
            expires_at: now() + 100,
            last_poll: 0,
        };
        assert!(
            poll_once(&mut p)
                .unwrap_err()
                .to_string()
                .contains("denied")
        );
        assert!(
            poll_once(&mut p)
                .unwrap_err()
                .to_string()
                .contains("justrust login")
        );
    }

    #[test]
    fn env_merge_keeps_unrelated_lines() {
        let old =
            "# jcode\nJCODE_API_BASE=http://x\nexport JCODE_API_KEY=old\nJCODE_TIER=pro\nOTHER=1\n";
        let new = merge_env(
            old,
            &[
                ("JCODE_API_KEY", "jk_new"),
                ("JCODE_ACCOUNT_ID", "a"),
                ("JCODE_ACCOUNT_EMAIL", ""),
                ("JCODE_TIER", "free"),
            ],
        );
        assert_eq!(
            new,
            "# jcode\nJCODE_API_BASE=http://x\nOTHER=1\nJCODE_API_KEY=jk_new\nJCODE_ACCOUNT_ID=a\nJCODE_TIER=free\n"
        );
        assert_eq!(
            strip_keys(&new),
            "# jcode\nJCODE_API_BASE=http://x\nOTHER=1\n"
        );
    }

    #[test]
    fn checkout_body_asks_for_the_trial_only_when_eligible() {
        assert_eq!(
            checkout_body(20, true),
            json!({ "plan_usd": 20, "trial": true })
        );
        assert_eq!(checkout_body(50, false), json!({ "plan_usd": 50 }));
    }

    #[test]
    fn upgrade_waits_for_can_build() {
        let (base, seen) = mock(vec![
            (
                200,
                r#"{"builds":{"trial_remaining":0,"can_build":false,"next_step":"subscribe"}}"#,
            ),
            (
                200,
                r#"{"builds":{"trial_remaining":0,"included":500,"included_used":0,"can_build":true}}"#,
            ),
        ]);
        let c = Creds::new(base, "jk".into()).unwrap();
        let b = wait_can_build(&c, Duration::from_secs(5), Duration::from_millis(1)).unwrap();
        assert_eq!(b.left(), 500);
        assert!(seen.lock().unwrap()[0].starts_with("GET /v1/me"));
    }
}
