//! Pinned Rust toolchains, owned by justrust (phase 1 of `docs/toolchain.md`).
//!
//! ```text
//! ~/.justrust/toolchains/
//!   nightly-2026-10-08/            one complete toolchain prefix
//!     bin/{cargo,rustc,rustdoc,cargo-clippy,clippy-driver,rustfmt}
//!     lib/rustlib/<target>/codegen-backends/librustc_codegen_cranelift-*.so
//!     lib/rustlib/src/rust/library   (rust-src)
//!     justrust-toolchain.json        what was installed, from which manifest
//!   .install.lock                  flock: one installer at a time
//!   .staging-<pid>/                in-progress install, renamed into place
//! ```
//!
//! A project opts in with `justrust.toml` in the workspace (or any parent):
//!
//! ```toml
//! [toolchain]
//! channel = "nightly-2026-10-08"
//! ```
//!
//! `JUSTRUST_TOOLCHAIN=<spec>` overrides it for one run, and
//! `JUSTRUST_TOOLCHAIN=system` turns it off. Recorded runs then use the pinned
//! cargo and rustc. Because every machine resolves the same spec to the same
//! bytes, rustc `-vV` (and with it every depcache key) stops churning when the
//! distro updates its Rust package.
//!
//! The toolchain comes straight from the official dist server
//! (`static.rust-lang.org`, override with `JUSTRUST_DIST_SERVER`): the channel
//! manifest is checked against its published sha256, each component tarball
//! against the hash in the manifest. No rustup involved.
//!
//! Fail open: when a pinned toolchain cannot be installed, the build runs
//! with the system toolchain and says so.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const TARGET: &str = "x86_64-unknown-linux-gnu";
pub const CONFIG_FILE: &str = "justrust.toml";
const MARKER: &str = "justrust-toolchain.json";
/// After a failed automatic install, builds use the system toolchain without
/// retrying for this long, so a bad pin or no network costs one attempt.
const RETRY_SECS: u64 = 600;

/// Installed by default. Manifest package names (`-preview` where the
/// manifest renames them). `rustc-dev` and cranelift are what phase 2 and
/// the jr-rustc fork build on.
pub const DEFAULT_COMPONENTS: &[&str] = &[
    "rustc",
    "cargo",
    "rust-std",
    "rust-src",
    "clippy-preview",
    "rustfmt-preview",
    "rustc-codegen-cranelift-preview",
    "rustc-dev",
    "llvm-tools-preview",
];

/// A toolchain name: `nightly-YYYY-MM-DD`, `beta-YYYY-MM-DD`, or `X.Y.Z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub channel: String,
    pub date: Option<String>,
}

impl Spec {
    pub fn parse(s: &str) -> Result<Spec> {
        let s = s.trim();
        for ch in ["nightly", "beta"] {
            if let Some(date) = s.strip_prefix(ch).and_then(|r| r.strip_prefix('-')) {
                if !is_date(date) {
                    bail!("`{s}`: expected {ch}-YYYY-MM-DD");
                }
                return Ok(Spec {
                    channel: ch.into(),
                    date: Some(date.into()),
                });
            }
        }
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            return Ok(Spec {
                channel: s.into(),
                date: None,
            });
        }
        bail!(
            "`{s}` is not a pinned toolchain: use nightly-YYYY-MM-DD, beta-YYYY-MM-DD, or X.Y.Z \
             (`justrust toolchain pin nightly` picks the latest complete nightly)"
        )
    }

    pub fn id(&self) -> String {
        match &self.date {
            Some(d) => format!("{}-{d}", self.channel),
            None => self.channel.clone(),
        }
    }

    pub fn is_nightly(&self) -> bool {
        self.channel == "nightly"
    }

    /// The channel manifest URL, relative to the dist server.
    fn manifest_path(&self) -> String {
        match &self.date {
            Some(d) => format!("dist/{d}/channel-rust-{}.toml", self.channel),
            None => format!("dist/channel-rust-{}.toml", self.channel),
        }
    }
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

pub fn root() -> Result<PathBuf> {
    Ok(crate::paths::home()?.join("toolchains"))
}

fn dist_server() -> String {
    std::env::var("JUSTRUST_DIST_SERVER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://static.rust-lang.org".into())
        .trim_end_matches('/')
        .to_string()
}

/// Where the pin for this run came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Env,
    File(PathBuf),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Env => write!(f, "JUSTRUST_TOOLCHAIN"),
            Source::File(p) => write!(f, "{}", p.display()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pin {
    pub spec: Spec,
    pub source: Source,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    toolchain: Option<ToolchainSection>,
}

#[derive(Debug, Default, Deserialize)]
struct ToolchainSection {
    channel: Option<String>,
}

/// The nearest `justrust.toml` at or above `dir`.
pub fn find_config(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .map(|d| d.join(CONFIG_FILE))
        .find(|p| p.is_file())
}

fn read_config(path: &Path) -> Result<Option<String>> {
    let text = std::fs::read_to_string(path)?;
    let cfg: ConfigFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(cfg.toolchain.and_then(|t| t.channel))
}

fn is_off(v: &str) -> bool {
    matches!(v, "" | "system" | "off" | "0" | "none")
}

/// The toolchain a build in `cwd` should use, or `None` for the system one.
pub fn resolve(cwd: &Path) -> Result<Option<Pin>> {
    if let Ok(v) = std::env::var("JUSTRUST_TOOLCHAIN") {
        if is_off(v.trim()) {
            return Ok(None);
        }
        return Ok(Some(Pin {
            spec: Spec::parse(&v)?,
            source: Source::Env,
        }));
    }
    let Some(path) = find_config(cwd) else {
        return Ok(None);
    };
    match read_config(&path)? {
        Some(ch) if !is_off(ch.trim()) => Ok(Some(Pin {
            spec: Spec::parse(&ch).with_context(|| format!("in {}", path.display()))?,
            source: Source::File(path),
        })),
        _ => Ok(None),
    }
}

/// What `justrust-toolchain.json` records about an installed toolchain.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Installed {
    pub id: String,
    pub manifest_date: String,
    pub manifest_sha256: String,
    pub rustc_version: String,
    pub components: Vec<String>,
    pub installed_at: f64,
    pub install_secs: f64,
}

/// A ready toolchain prefix.
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub id: String,
    pub dir: PathBuf,
    pub nightly: bool,
}

impl Toolchain {
    pub fn bin(&self, name: &str) -> PathBuf {
        self.dir.join("bin").join(name)
    }
    pub fn info(&self) -> Option<Installed> {
        serde_json::from_slice(&std::fs::read(self.dir.join(MARKER)).ok()?).ok()
    }
}

pub fn installed(spec: &Spec) -> Result<Option<Toolchain>> {
    let dir = root()?.join(spec.id());
    Ok(dir.join(MARKER).is_file().then(|| Toolchain {
        id: spec.id(),
        dir,
        nightly: spec.is_nightly(),
    }))
}

/// For a recorded run: the pinned toolchain, installed if needed. `None`
/// means use the system toolchain (no pin, or it failed and was reported).
pub fn for_build() -> Option<Toolchain> {
    let cwd = std::env::current_dir().ok()?;
    let pin = match resolve(&cwd) {
        Ok(p) => p?,
        Err(e) => {
            eprintln!("justrust: ignoring the toolchain pin: {e:#}");
            return None;
        }
    };
    if let Ok(Some(tc)) = installed(&pin.spec) {
        return Some(tc);
    }
    if matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_AUTO_INSTALL").as_deref(),
        Ok("0") | Ok("off") | Ok("no")
    ) {
        eprintln!(
            "justrust: toolchain {} (from {}) is not installed, using the system toolchain. \
             Run `justrust toolchain install`",
            pin.spec.id(),
            pin.source
        );
        return None;
    }
    let failed = root().ok()?.join(format!(".failed-{}", pin.spec.id()));
    if let Ok(age) = std::fs::metadata(&failed)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default().as_secs())
        && age < RETRY_SECS
    {
        eprintln!(
            "justrust: toolchain {} failed to install {age}s ago ({}), using the system \
             toolchain. Retry now: `justrust toolchain install`",
            pin.spec.id(),
            std::fs::read_to_string(&failed).unwrap_or_default().trim()
        );
        return None;
    }
    eprintln!(
        "justrust: installing pinned toolchain {} (from {}), one time",
        pin.spec.id(),
        pin.source
    );
    match install(&pin.spec, DEFAULT_COMPONENTS, &mut std::io::stderr()) {
        Ok(tc) => {
            let _ = std::fs::remove_file(&failed);
            Some(tc)
        }
        Err(e) => {
            let _ = std::fs::write(&failed, format!("{e:#}"));
            eprintln!(
                "justrust: could not install toolchain {}: {e:#}. Using the system toolchain",
                pin.spec.id()
            );
            None
        }
    }
}

// ------------------------------------------------------------------ install

fn curl(url: &str, dest: &Path) -> Result<()> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "--connect-timeout", "20", "-o"])
        .arg(dest)
        .arg(url)
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!(
            "downloading {url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String> {
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .context("running sha256sum")?;
    if !out.status.success() {
        bail!("sha256sum {} failed", path.display());
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .context("sha256sum printed nothing")
}

/// One component to download, from the channel manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub name: String,
    pub url: String,
    pub hash: String,
}

pub struct Manifest {
    pub date: String,
    pub sha256: String,
    table: toml::Table,
}

impl Manifest {
    pub fn parse(text: &str, sha256: String) -> Result<Manifest> {
        let table: toml::Table = text.parse().context("parsing the channel manifest")?;
        let date = table
            .get("date")
            .and_then(|d| d.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(Manifest {
            date,
            sha256,
            table,
        })
    }

    /// `rustc 1.101.0-nightly (1d81eb4ad 2026-10-07)` style version of `pkg`.
    pub fn version(&self, pkg: &str) -> Option<String> {
        self.table
            .get("pkg")?
            .get(pkg)?
            .get("version")?
            .as_str()
            .map(str::to_owned)
    }

    /// The tarball for `name` on this target, following `renames`.
    pub fn package(&self, name: &str) -> Result<Package> {
        let renamed = self
            .table
            .get("renames")
            .and_then(|r| r.get(name))
            .and_then(|r| r.get("to"))
            .and_then(|t| t.as_str())
            .unwrap_or(name);
        let pkg = self
            .table
            .get("pkg")
            .and_then(|p| p.get(renamed))
            .with_context(|| format!("component {name} is not in the {} manifest", self.date))?;
        let targets = pkg.get("target").context("no targets")?;
        let t = targets
            .get(TARGET)
            .or_else(|| targets.get("*"))
            .with_context(|| format!("component {name} has no {TARGET} build"))?;
        if !t
            .get("available")
            .and_then(|a| a.as_bool())
            .unwrap_or(false)
        {
            bail!(
                "component {name} is not available in the {} build",
                self.date
            );
        }
        let (url, hash) = match (t.get("xz_url"), t.get("xz_hash")) {
            (Some(u), Some(h)) => (u, h),
            _ => (
                t.get("url").context("no url")?,
                t.get("hash").context("no hash")?,
            ),
        };
        Ok(Package {
            name: renamed.to_string(),
            url: url.as_str().context("url")?.to_string(),
            hash: hash.as_str().context("hash")?.to_string(),
        })
    }
}

/// Download and verify the channel manifest for `spec`.
pub fn fetch_manifest(spec: &Spec, scratch: &Path) -> Result<Manifest> {
    let url = format!("{}/{}", dist_server(), spec.manifest_path());
    let file = scratch.join("channel.toml");
    let sum = scratch.join("channel.toml.sha256");
    curl(&url, &file).with_context(|| format!("no published toolchain {}", spec.id()))?;
    curl(&format!("{url}.sha256"), &sum)?;
    let want = std::fs::read_to_string(&sum)?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let got = sha256(&file)?;
    if want != got {
        bail!(
            "channel manifest checksum mismatch for {}: {got} != {want}",
            spec.id()
        );
    }
    Manifest::parse(&std::fs::read_to_string(&file)?, got)
}

fn lock_exclusive(path: &Path) -> Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: flock on a valid fd; blocks until the other installer finishes.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    if r != 0 {
        bail!(
            "locking {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    Ok(f)
}

/// Install `spec` with `components` under `root()`. Idempotent and safe to
/// run from several processes at once: the second waits, then finds it.
pub fn install(
    spec: &Spec,
    components: &[&str],
    log: &mut dyn std::io::Write,
) -> Result<Toolchain> {
    let root = root()?;
    std::fs::create_dir_all(&root)?;
    let _lock = lock_exclusive(&root.join(".install.lock"))?;
    if let Some(tc) = installed(spec)? {
        return Ok(tc);
    }
    let start = crate::paths::now();
    let staging = root.join(format!(".staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let result = install_into(spec, components, &staging, log, start);
    match result {
        Ok(info) => {
            let dest = root.join(spec.id());
            let prefix = staging.join("prefix");
            std::fs::write(prefix.join(MARKER), serde_json::to_vec_pretty(&info)?)?;
            if dest.exists() {
                // Left over from an interrupted install (no marker): replace.
                let trash = root.join(format!(".trash-{}", std::process::id()));
                std::fs::rename(&dest, &trash)?;
                let _ = std::fs::remove_dir_all(&trash);
            }
            std::fs::rename(&prefix, &dest)?;
            let _ = std::fs::remove_dir_all(&staging);
            let _ = writeln!(
                log,
                "justrust: installed {} ({}) in {:.1}s at {}",
                spec.id(),
                info.rustc_version,
                info.install_secs,
                dest.display()
            );
            Ok(Toolchain {
                id: spec.id(),
                dir: dest,
                nightly: spec.is_nightly(),
            })
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

fn install_into(
    spec: &Spec,
    components: &[&str],
    staging: &Path,
    log: &mut dyn std::io::Write,
    start: f64,
) -> Result<Installed> {
    let dl = staging.join("dl");
    let prefix = staging.join("prefix");
    std::fs::create_dir_all(&dl)?;
    std::fs::create_dir_all(&prefix)?;
    let manifest = fetch_manifest(spec, &dl)?;
    if let Some(d) = &spec.date
        && !manifest.date.is_empty()
        && &manifest.date != d
    {
        bail!("manifest for {} is dated {}", spec.id(), manifest.date);
    }
    let pkgs: Vec<Package> = components
        .iter()
        .map(|c| manifest.package(c))
        .collect::<Result<_>>()?;
    let _ = writeln!(
        log,
        "justrust: downloading {} components of {} ({})",
        pkgs.len(),
        spec.id(),
        manifest.version("rustc").unwrap_or_default()
    );
    // Download, verify and unpack every component in parallel: the large
    // ones (rustc, rustc-dev) dominate, and xz decoding is single-threaded.
    let results: Vec<Result<()>> = std::thread::scope(|s| {
        let handles: Vec<_> = pkgs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let dl = &dl;
                let prefix = &prefix;
                s.spawn(move || fetch_component(p, &dl.join(i.to_string()), prefix))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("panicked")))
            })
            .collect()
    });
    for (p, r) in pkgs.iter().zip(results) {
        r.with_context(|| format!("component {}", p.name))?;
    }
    let rustc = prefix.join("bin/rustc");
    let out = Command::new(&rustc)
        .arg("-vV")
        .output()
        .with_context(|| format!("running {}", rustc.display()))?;
    if !out.status.success() {
        bail!(
            "the installed rustc does not run: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let rustc_version = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let _ = std::fs::remove_dir_all(&dl);
    Ok(Installed {
        id: spec.id(),
        manifest_date: manifest.date.clone(),
        manifest_sha256: manifest.sha256.clone(),
        rustc_version,
        components: pkgs.iter().map(|p| p.name.clone()).collect(),
        installed_at: crate::paths::now(),
        install_secs: crate::paths::now() - start,
    })
}

fn fetch_component(p: &Package, work: &Path, prefix: &Path) -> Result<()> {
    std::fs::create_dir_all(work)?;
    let tarball = work.join("pkg.tar");
    curl(&p.url, &tarball)?;
    let got = sha256(&tarball)?;
    if got != p.hash {
        bail!("checksum mismatch for {}: {got} != {}", p.url, p.hash);
    }
    let unpack = work.join("x");
    std::fs::create_dir_all(&unpack)?;
    let flag = if p.url.ends_with(".xz") {
        "-xJf"
    } else {
        "-xzf"
    };
    let st = Command::new("tar")
        .arg(flag)
        .arg(&tarball)
        .arg("-C")
        .arg(&unpack)
        .status()
        .context("running tar")?;
    if !st.success() {
        bail!("unpacking {} failed", p.url);
    }
    let _ = std::fs::remove_file(&tarball);
    // rust-installer layout: <top>/components lists component dirs, each with
    // a manifest.in of `file:<path>` and `dir:<path>` entries.
    let top = std::fs::read_dir(&unpack)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .context("empty tarball")?;
    let comps = std::fs::read_to_string(top.join("components")).context("no components file")?;
    for comp in comps.lines().map(str::trim).filter(|c| !c.is_empty()) {
        let cdir = top.join(comp);
        let list = std::fs::read_to_string(cdir.join("manifest.in"))
            .with_context(|| format!("{comp}: no manifest.in"))?;
        for line in list.lines() {
            let rel = match line.split_once(':') {
                Some(("file" | "dir", rel)) => rel,
                _ => continue,
            };
            if rel.split('/').any(|c| c == ".." || c.is_empty()) {
                bail!("{comp}: bad path {rel}");
            }
            move_merge(&cdir.join(rel), &prefix.join(rel))?;
        }
    }
    let _ = std::fs::remove_dir_all(work);
    Ok(())
}

/// Move `src` to `dst`, merging into an existing directory. Components can
/// share directories (`lib/rustlib/<target>/lib`) but never files.
fn move_merge(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let src_meta = std::fs::symlink_metadata(src)?;
    match std::fs::symlink_metadata(dst) {
        Err(_) => std::fs::rename(src, dst)
            .with_context(|| format!("{} -> {}", src.display(), dst.display())),
        Ok(m) if m.is_dir() && src_meta.is_dir() => {
            for e in std::fs::read_dir(src)? {
                let e = e?;
                move_merge(&e.path(), &dst.join(e.file_name()))?;
            }
            Ok(())
        }
        Ok(_) => {
            // Same file from two components (license texts): last wins.
            std::fs::rename(src, dst)?;
            Ok(())
        }
    }
}

/// Resolve `nightly` / `beta` to the newest dated build that has every
/// default component, walking back up to two weeks.
pub fn latest(channel: &str) -> Result<Spec> {
    let scratch = root()?.join(format!(".probe-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;
    let result = (|| {
        let head = Spec {
            channel: channel.into(),
            date: None,
        };
        let m = fetch_manifest(&head, &scratch)?;
        let newest = chrono::NaiveDate::parse_from_str(&m.date, "%Y-%m-%d")
            .context("the channel manifest has no date")?;
        let complete = |m: &Manifest| DEFAULT_COMPONENTS.iter().all(|c| m.package(c).is_ok());
        if complete(&m) {
            return Spec::parse(&format!("{channel}-{}", m.date));
        }
        for back in 1..=14 {
            let d = newest - chrono::Duration::days(back);
            let spec = Spec::parse(&format!("{channel}-{}", d.format("%Y-%m-%d")))?;
            if let Ok(m) = fetch_manifest(&spec, &scratch)
                && complete(&m)
            {
                return Ok(spec);
            }
        }
        bail!("no {channel} build in the last two weeks has every component")
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

// ---------------------------------------------------------------------- CLI

pub fn list_installed() -> Vec<Toolchain> {
    let Ok(root) = root() else { return Vec::new() };
    let mut v: Vec<Toolchain> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(MARKER).is_file())
        .map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            Toolchain {
                nightly: id.starts_with("nightly-"),
                id,
                dir: e.path(),
            }
        })
        .collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

fn dir_bytes(dir: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((e.path(), std::fs::symlink_metadata(e.path()).ok()?)))
        .map(|(p, m)| {
            if m.is_dir() {
                dir_bytes(&p)
            } else {
                m.blocks() * 512
            }
        })
        .sum()
}

pub fn status() -> Result<()> {
    let cwd = std::env::current_dir()?;
    match resolve(&cwd)? {
        Some(pin) => {
            let state = match installed(&pin.spec)? {
                Some(tc) => tc
                    .info()
                    .map(|i| format!("installed, {}", i.rustc_version))
                    .unwrap_or_else(|| "installed".into()),
                None => "not installed yet (the next build installs it)".into(),
            };
            println!("pinned  {} from {} ({state})", pin.spec.id(), pin.source);
        }
        None => {
            let rustc = crate::paths::find_on_path("rustc", Path::new(""))
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "rustc not found".into());
            println!("system  {rustc} (no {CONFIG_FILE} [toolchain] pin here)");
        }
    }
    let all = list_installed();
    if all.is_empty() {
        println!("no toolchains installed in {}", root()?.display());
    }
    for tc in all {
        let ver = tc.info().map(|i| i.rustc_version).unwrap_or_default();
        println!(
            "  {:<22} {:>6.2} GB  {ver}",
            tc.id,
            dir_bytes(&tc.dir) as f64 / 1e9
        );
    }
    Ok(())
}

/// Turn `nightly`/`beta` into the latest dated build, otherwise parse.
fn concrete(spec: &str) -> Result<Spec> {
    match spec.trim() {
        "nightly" | "beta" => latest(spec.trim()),
        s => Spec::parse(s),
    }
}

pub fn install_command(spec: Option<String>) -> Result<()> {
    let spec = match spec {
        Some(s) => concrete(&s)?,
        None => resolve(&std::env::current_dir()?)?
            .map(|p| p.spec)
            .context("nothing pinned here: pass a toolchain (for example `nightly`)")?,
    };
    let tc = install(&spec, DEFAULT_COMPONENTS, &mut std::io::stderr())?;
    println!("{}", tc.dir.display());
    Ok(())
}

/// Write `[toolchain] channel` into `justrust.toml` (the nearest one, or a
/// new one in the git root / current directory), keeping other content.
pub fn pin_command(spec: &str, no_install: bool) -> Result<()> {
    let spec = concrete(spec)?;
    let cwd = std::env::current_dir()?;
    let path = find_config(&cwd).unwrap_or_else(|| project_root(&cwd).join(CONFIG_FILE));
    write_pin(&path, Some(&spec.id()))?;
    println!("pinned {} in {}", spec.id(), path.display());
    if !no_install {
        install(&spec, DEFAULT_COMPONENTS, &mut std::io::stderr())?;
    }
    Ok(())
}

pub fn unpin_command() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let Some(path) = find_config(&cwd) else {
        println!("nothing pinned");
        return Ok(());
    };
    write_pin(&path, None)?;
    println!("removed the toolchain pin from {}", path.display());
    Ok(())
}

pub fn remove_command(spec: &str) -> Result<()> {
    let spec = Spec::parse(spec)?;
    let root = root()?;
    let _lock = lock_exclusive(&root.join(".install.lock"))?;
    let dir = root.join(spec.id());
    if !dir.exists() {
        bail!("{} is not installed", spec.id());
    }
    let trash = root.join(format!(".trash-{}", std::process::id()));
    std::fs::rename(&dir, &trash)?;
    std::fs::remove_dir_all(&trash)?;
    println!("removed {}", spec.id());
    Ok(())
}

fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

pub fn write_pin(path: &Path, channel: Option<&str>) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    match channel {
        Some(ch) => {
            if !doc.contains_table("toolchain") {
                doc["toolchain"] = toml_edit::table();
            }
            doc["toolchain"]["channel"] = toml_edit::value(ch);
        }
        None => {
            if let Some(t) = doc.get_mut("toolchain").and_then(|t| t.as_table_mut()) {
                t.remove("channel");
                if t.is_empty() {
                    doc.remove("toolchain");
                }
            }
        }
    }
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_specs() {
        let s = Spec::parse("nightly-2026-10-08").unwrap();
        assert_eq!(s.channel, "nightly");
        assert_eq!(s.id(), "nightly-2026-10-08");
        assert!(s.is_nightly());
        assert_eq!(
            s.manifest_path(),
            "dist/2026-10-08/channel-rust-nightly.toml"
        );
        let s = Spec::parse("1.91.0").unwrap();
        assert_eq!(s.manifest_path(), "dist/channel-rust-1.91.0.toml");
        assert!(!s.is_nightly());
        assert!(Spec::parse("beta-2026-10-01").is_ok());
        for bad in [
            "nightly",
            "stable",
            "nightly-2026-1-08",
            "1.91",
            "../x",
            "nightly-2026-10-0x",
        ] {
            assert!(Spec::parse(bad).is_err(), "{bad}");
        }
    }

    const MANIFEST: &str = r#"
manifest-version = "2"
date = "2026-10-08"

[pkg.rustc]
version = "1.101.0-nightly (1d81eb4ad 2026-10-07)"

[pkg.rustc.target.x86_64-unknown-linux-gnu]
available = true
url = "https://x/rustc.tar.gz"
hash = "aa"
xz_url = "https://x/rustc.tar.xz"
xz_hash = "bb"

[pkg.rust-src.target."*"]
available = true
url = "https://x/src.tar.gz"
hash = "cc"

[pkg.clippy-preview.target.x86_64-unknown-linux-gnu]
available = false

[renames.clippy]
to = "clippy-preview"
"#;

    #[test]
    fn reads_manifest_packages() {
        let m = Manifest::parse(MANIFEST, "h".into()).unwrap();
        assert_eq!(m.date, "2026-10-08");
        assert_eq!(
            m.version("rustc").unwrap(),
            "1.101.0-nightly (1d81eb4ad 2026-10-07)"
        );
        let p = m.package("rustc").unwrap();
        assert_eq!(
            (p.url.as_str(), p.hash.as_str()),
            ("https://x/rustc.tar.xz", "bb")
        );
        let p = m.package("rust-src").unwrap();
        assert_eq!(p.url, "https://x/src.tar.gz");
        let e = m.package("clippy").unwrap_err().to_string();
        assert!(e.contains("not available"), "{e}");
        assert!(m.package("miri").is_err());
    }

    #[test]
    fn pin_round_trips_and_keeps_other_content() {
        let dir = std::env::temp_dir().join(format!("jr-tc-pin-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let path = dir.join(CONFIG_FILE);
        std::fs::write(&path, "# keep me\n[other]\nx = 1\n").unwrap();
        write_pin(&path, Some("nightly-2026-10-08")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# keep me") && text.contains("x = 1"),
            "{text}"
        );
        assert_eq!(find_config(&dir.join("sub")).unwrap(), path);
        assert_eq!(
            read_config(&path).unwrap().as_deref(),
            Some("nightly-2026-10-08")
        );
        write_pin(&path, None).unwrap();
        assert_eq!(read_config(&path).unwrap(), None);
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("toolchain")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_merge_merges_directories() {
        let dir = std::env::temp_dir().join(format!("jr-tc-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (p, body) in [("a/lib/x/one", "1"), ("b/lib/x/two", "2")] {
            let f = dir.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, body).unwrap();
        }
        let dst = dir.join("prefix/lib");
        move_merge(&dir.join("a/lib"), &dst).unwrap();
        move_merge(&dir.join("b/lib"), &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("x/one")).unwrap(), "1");
        assert_eq!(std::fs::read_to_string(dst.join("x/two")).unwrap(), "2");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
