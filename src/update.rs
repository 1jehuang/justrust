//! Auto-update: release binaries keep themselves current.
//!
//! At most once a day (after a build or command, in the detached
//! `justrust __bg` process) justrust asks `{build}/release/latest` which
//! release it should run. When it is newer, it downloads that release's
//! tarball for this platform from GitHub, checks it against the published
//! sha256, checks the new binary runs and reports the expected version, and
//! atomically renames it over the running executable. Builds already running
//! keep the old binary; the next command uses the new one and says so once.
//!
//! Only binaries from the release workflow (`JUSTRUST_RELEASE_BUILD` at
//! compile time, as the installer ships them) replace themselves. Source
//! builds (`cargo install`, `cargo install --path .`) are never touched:
//! they print a one-line notice when a newer release exists.
//!
//! Off: `justrust update off`, `JUSTRUST_NO_UPDATE=1`, or
//! `JUSTRUST_AUTO_UPDATE=0`. Never in CI or on the remote build agent.
//! `justrust update` updates now.

use crate::paths;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const CHECK_SECS: f64 = 24.0 * 3600.0;
/// After a failed check or install, try again sooner than a day.
const RETRY_SECS: f64 = 3.0 * 3600.0;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Built by `.github/workflows/release.yml` (it sets the variable).
pub fn release_build() -> bool {
    option_env!("JUSTRUST_RELEASE_BUILD").is_some_and(|v| v == "1")
}

fn dir() -> Option<PathBuf> {
    Some(paths::home().ok()?.join("update"))
}

fn env_off(k: &str) -> bool {
    matches!(
        std::env::var(k).ok().as_deref().map(str::trim),
        Some("0" | "off" | "false" | "no")
    )
}

fn env_on(k: &str) -> bool {
    std::env::var(k).is_ok_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
    })
}

/// Why automatic checks are off, `None` when on.
pub fn disabled_reason() -> Option<&'static str> {
    if cfg!(test) {
        return Some("unit tests");
    }
    if env_on("JUSTRUST_NO_UPDATE") {
        return Some("JUSTRUST_NO_UPDATE is set");
    }
    if env_off("JUSTRUST_AUTO_UPDATE") {
        return Some("JUSTRUST_AUTO_UPDATE=0");
    }
    if std::env::var_os(crate::route::ON_AGENT).is_some() {
        return Some("remote build agent");
    }
    if env_on("CI") {
        return Some("CI");
    }
    if crate::telemetry::setting("auto_update") == Some(false) {
        return Some("turned off with `justrust update off`");
    }
    None
}

fn interval() -> f64 {
    std::env::var("JUSTRUST_UPDATE_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(CHECK_SECS)
}

fn mtime(p: &Path) -> Option<f64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64())
}

/// One `stat`: is a background check due?
pub fn check_due() -> bool {
    if disabled_reason().is_some() {
        return false;
    }
    let Some(d) = dir() else { return false };
    let next = std::fs::read_to_string(d.join("next-check"))
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok());
    match next {
        Some(t) => paths::now() >= t.min(paths::now() + interval()),
        None => mtime(&d.join("last-check")).is_none_or(|t| paths::now() - t >= interval()),
    }
}

fn schedule(after: f64) {
    if let Some(d) = dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(d.join("last-check"), b"");
        let _ = std::fs::write(d.join("next-check"), format!("{}", paths::now() + after));
    }
}

/// This platform's release asset name (`justrust-<target>.tar.gz`).
pub fn target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    })
}

fn release_url() -> String {
    let base = std::env::var("JUSTRUST_UPDATE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| format!("{}/release/latest", crate::remote_hosted::build_base()));
    format!("{base}?target={}&version={VERSION}", target().unwrap_or(""))
}

fn curl_text(url: &str) -> Result<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--proto",
            "=https,http,file",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
        ])
        .arg(url)
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!("GET {url}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The server's answer: `{version, update, url, sha256_url}`.
fn latest() -> Result<Value> {
    let v: Value = serde_json::from_str(&curl_text(&release_url())?)
        .context("release endpoint returned invalid JSON")?;
    if v["version"].as_str().is_none() {
        bail!("release endpoint returned no version");
    }
    Ok(v)
}

/// Semver order for our tags (X.Y.Z[-pre.N]).
pub fn newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> (Vec<u64>, Vec<String>) {
        let v = v.trim_start_matches('v');
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        (
            core.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            if pre.is_empty() {
                vec![]
            } else {
                pre.split('.').map(str::to_string).collect()
            },
        )
    }
    let (a, ap) = parse(candidate);
    let (b, bp) = parse(current);
    for i in 0..3 {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    match (ap.is_empty(), bp.is_empty()) {
        (true, true) => return false,
        (true, false) => return true,
        (false, true) => return false,
        _ => {}
    }
    for i in 0..ap.len().max(bp.len()) {
        match (ap.get(i), bp.get(i)) {
            (None, _) => return false,
            (_, None) => return true,
            (Some(x), Some(y)) if x == y => continue,
            (Some(x), Some(y)) => {
                return match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(p), Ok(q)) => p > q,
                    (Ok(_), Err(_)) => false,
                    (Err(_), Ok(_)) => true,
                    _ => x > y,
                };
            }
        }
    }
    false
}

/// Any recorded build still running on this machine? Replacing the binary
/// under it would let its later rustc shim calls run the new version.
fn builds_running() -> bool {
    crate::status::collect().is_ok_and(|s| !s.active.is_empty())
}

/// The file to replace: the running binary, symlinks resolved.
fn exe_path() -> Result<PathBuf> {
    Ok(std::env::current_exe()?.canonicalize()?)
}

/// Download, verify, and install `version`. Returns the installed path.
fn install(info: &Value) -> Result<PathBuf> {
    let version = info["version"].as_str().context("no version")?;
    let url = info["url"].as_str().context("no asset for this platform")?;
    let sha_url = info["sha256_url"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{url}.sha256"));
    let exe = exe_path()?;
    let parent = exe.parent().context("executable has no directory")?;
    // Same directory, so the final rename is atomic.
    let tmp = parent.join(format!(".justrust-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)
        .with_context(|| format!("{} is not writable", parent.display()))?;
    let r = (|| -> Result<PathBuf> {
        let tarball = tmp.join("justrust.tar.gz");
        crate::toolchain::curl(url, &tarball)?;
        let want = curl_text(&sha_url)?
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let got = crate::toolchain::sha256(&tarball)?;
        if want.len() != 64 || got != want {
            bail!("checksum mismatch (published {want}, downloaded {got})");
        }
        let st = Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(&tmp)
            .status()
            .context("running tar")?;
        if !st.success() {
            bail!("tar failed");
        }
        let name = crate::sys::exe_name("justrust");
        let new = tmp
            .join(format!("justrust-{}", target().unwrap_or("")))
            .join(&name);
        if !new.is_file() {
            bail!("archive has no {name}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
        }
        let out = Command::new(&new)
            .arg("--version")
            .output()
            .context("running the new binary")?;
        let said = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || said.split_whitespace().nth(1) != Some(version) {
            bail!("new binary reports `{}`, expected {version}", said.trim());
        }
        // Windows cannot replace a running executable, but can rename it.
        if cfg!(windows) {
            let old = exe.with_extension("old.exe");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&exe, &old)?;
        }
        std::fs::rename(&new, &exe).with_context(|| format!("replacing {}", exe.display()))?;
        Ok(exe.clone())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn note_applied(from: &str, to: &str) {
    if let Some(d) = dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(
            d.join("applied.json"),
            json!({ "from": from, "to": to, "at": paths::now() }).to_string(),
        );
    }
}

/// The background check (`justrust __bg`).
pub fn background() {
    schedule(interval());
    let info = match latest() {
        Ok(v) => v,
        Err(e) => {
            schedule(RETRY_SECS);
            crate::telemetry::record(
                "update",
                json!({ "outcome": "check_failed", "stage": "check", "error": short(&e) }),
            );
            return;
        }
    };
    let version = info["version"].as_str().unwrap_or("").to_string();
    let wanted = info["update"].as_bool().unwrap_or(false) && newer(&version, VERSION);
    if let Some(d) = dir() {
        let _ = std::fs::write(d.join("latest"), &version);
    }
    if !wanted {
        return;
    }
    if !release_build() {
        // Source build: the next command prints a notice instead.
        return;
    }
    if builds_running() {
        // Try again after the build.
        schedule(60.0);
        return;
    }
    match install(&info) {
        Ok(_) => {
            note_applied(VERSION, &version);
            crate::telemetry::record(
                "update",
                json!({ "outcome": "applied", "from": VERSION, "to": version }),
            );
        }
        Err(e) => {
            schedule(RETRY_SECS);
            crate::telemetry::record(
                "update",
                json!({ "outcome": "failed", "stage": "install", "to": version, "error": short(&e) }),
            );
        }
    }
}

/// An error as a short label without paths (telemetry).
fn short(e: &anyhow::Error) -> String {
    let s = format!("{e}");
    let s = s.split(':').next().unwrap_or("").trim();
    s.split_whitespace()
        .filter(|w| !w.contains('/') && !w.contains('\\'))
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Once per update, on the next command: what changed. Also, for source
/// builds, a one-line notice per newer release.
pub fn notices() {
    if cfg!(test) {
        return;
    }
    let Some(d) = dir() else { return };
    let applied = d.join("applied.json");
    if let Some(v) = std::fs::read(&applied)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        let _ = std::fs::remove_file(&applied);
        if v["to"].as_str() == Some(VERSION) {
            eprintln!(
                "justrust: updated {} -> {VERSION} (https://github.com/1jehuang/justrust/releases/tag/v{VERSION}). \
                 `justrust update off` stops automatic updates.",
                v["from"].as_str().unwrap_or("?")
            );
        }
        return;
    }
    if release_build() || disabled_reason().is_some() {
        return;
    }
    let Ok(latest) = std::fs::read_to_string(d.join("latest")) else {
        return;
    };
    let latest = latest.trim();
    if !newer(latest, VERSION) {
        return;
    }
    let shown = d.join("notified");
    if std::fs::read_to_string(&shown).is_ok_and(|s| s.trim() == latest) {
        return;
    }
    let _ = std::fs::write(&shown, latest);
    eprintln!(
        "justrust: {latest} is out (this is {VERSION}, built from source). \
         `cargo install justrust --locked` or `curl -fsSL https://jcode.sh/rust.sh | sh`."
    );
}

/// `justrust update [now|status|on|off] [--force]`.
pub fn command(action: &str, force: bool) -> Result<()> {
    match action {
        "off" | "disable" => {
            crate::telemetry::set_setting("auto_update", false)?;
            println!("automatic updates off. `justrust update` still updates on demand.");
        }
        "on" | "enable" => {
            crate::telemetry::set_setting("auto_update", true)?;
            match disabled_reason() {
                None => println!("automatic updates on (checked at most daily)."),
                Some(why) => println!("automatic updates setting on, but still off here: {why}"),
            }
        }
        "status" => {
            println!(
                "justrust {VERSION} ({}), {}",
                if release_build() {
                    "release build"
                } else {
                    "built from source"
                },
                exe_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            match disabled_reason() {
                None if release_build() => {
                    println!("automatic updates: on (daily, `justrust update off` to stop)")
                }
                None => {
                    println!("automatic updates: notify only (source builds are never replaced)")
                }
                Some(why) => println!("automatic updates: off ({why})"),
            }
            if let Some(t) = dir().and_then(|d| mtime(&d.join("last-check"))) {
                println!("last check: {:.1} h ago", (paths::now() - t) / 3600.0);
            }
            if let Some(l) = dir().and_then(|d| std::fs::read_to_string(d.join("latest")).ok()) {
                println!("latest release seen: {}", l.trim());
            }
        }
        "now" | "" => {
            if !release_build() && !force {
                let info = latest()?;
                let v = info["version"].as_str().unwrap_or("?");
                if newer(v, VERSION) {
                    println!(
                        "{v} is out; this justrust {VERSION} was built from source, so it is not replaced. \
                         Run `cargo install justrust --locked`, or `justrust update --force` to install the release binary over it."
                    );
                } else {
                    println!("justrust {VERSION} is up to date (latest release {v}).");
                }
                return Ok(());
            }
            schedule(interval());
            let info = latest()?;
            let v = info["version"].as_str().unwrap_or("?").to_string();
            if !newer(&v, VERSION) {
                println!("justrust {VERSION} is up to date (latest release {v}).");
                return Ok(());
            }
            if builds_running() && !force {
                bail!("a build is running; rerun when it finishes (or pass --force)");
            }
            println!("updating justrust {VERSION} -> {v} ...");
            let path = install(&info).inspect_err(|e| {
                crate::telemetry::record(
                    "update",
                    json!({ "outcome": "failed", "stage": "manual", "to": v, "error": short(e) }),
                );
            })?;
            crate::telemetry::record(
                "update",
                json!({ "outcome": "applied", "manual": true, "from": VERSION, "to": v }),
            );
            println!("installed {v} at {}", path.display());
        }
        other => bail!("unknown action `{other}`: use now, status, on, or off"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_order() {
        let order = [
            "0.1.0-alpha.2",
            "0.1.0-alpha.10",
            "0.1.0-alpha.11",
            "0.1.0-beta.1",
            "0.1.0",
            "0.1.1",
            "0.2.0",
            "1.0.0",
        ];
        for (i, a) in order.iter().enumerate() {
            assert!(!newer(a, a), "{a}");
            for b in &order[i + 1..] {
                assert!(newer(b, a), "{b} > {a}");
                assert!(!newer(a, b), "{a} < {b}");
            }
        }
        assert!(newer("v0.1.1", "0.1.0"));
    }

    #[test]
    fn short_errors_drop_paths() {
        let e = anyhow::anyhow!("/home/alice/.local/bin is not writable: denied");
        assert_eq!(short(&e), "is not writable");
    }

    #[test]
    fn this_platform_has_a_release_target() {
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(target().is_some());
        }
    }
}
