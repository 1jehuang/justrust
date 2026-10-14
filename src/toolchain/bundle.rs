//! Trusted maintainer-published, immutable toolchain prefixes. Never a tenant upload.
use super::*;
use std::io::{Read, Write};

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;

const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_ARCHIVE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_UNPACKED: u64 = 32 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 500_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BundleManifest {
    pub schema_version: u32,
    pub id: String,
    pub target: String,
    pub rustc_version: String,
    pub rustc_commit_hash: String,
    pub cargo_upstream_commit: String,
    #[serde(default)]
    pub jr_cargo: Option<String>,
    pub archive_sha256: String,
    pub archive_size: u64,
    /// Sum of regular file payload bytes, not tar headers/padding.
    pub unpacked_size: u64,
    pub components: Vec<String>,
    #[serde(default)]
    pub manifest_date: String,
    #[serde(default)]
    pub official_manifest_sha256: Option<String>,
}

fn hex(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl BundleManifest {
    fn validate(&self, spec: &Spec) -> Result<()> {
        if self.schema_version != 1 || self.id != spec.id() || self.target != TARGET {
            bail!("toolchain bundle schema, id, or target mismatch");
        }
        if !hex(&self.archive_sha256, 64, 64)
            || !hex(&self.rustc_commit_hash, 40, 40)
            || !hex(&self.cargo_upstream_commit, 7, 40)
            || self.jr_cargo.as_deref().is_some_and(|s| !hex(s, 7, 40))
            || self
                .official_manifest_sha256
                .as_deref()
                .is_some_and(|s| !hex(s, 64, 64))
            || self.archive_size == 0
            || self.archive_size > MAX_ARCHIVE
            || self.unpacked_size == 0
            || self.unpacked_size > MAX_UNPACKED
            || self.rustc_version.len() > 256
            || !self.rustc_version.starts_with("rustc ")
            || self.rustc_version.contains(['\n', '\r', '\0'])
            || self.components.len() > 64
            || self.components.iter().any(|s| {
                s.is_empty()
                    || s.len() > 80
                    || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            bail!("invalid or oversized toolchain bundle manifest");
        }
        for required in ["rustc", "cargo", "rust-std"] {
            if !self.components.iter().any(|s| s == required) {
                bail!("toolchain bundle lacks required component {required}");
            }
        }
        if spec.is_jr() {
            if !self
                .rustc_commit_hash
                .starts_with(spec.date.as_deref().unwrap_or_default())
            {
                bail!("JR bundle commit does not match requested id");
            }
        } else if let Some(date) = &spec.date
            && &self.manifest_date != date
        {
            bail!("official bundle manifest date does not match requested id");
        }
        Ok(())
    }
}

/// No default CDN probing until the publisher has opted this client in.
/// The configured store is a code execution trust root, not a tenant cache URL.
pub(super) fn base() -> Result<Option<String>> {
    std::env::var("JUSTRUST_TOOLCHAIN_DIST")
        .ok()
        .map(|s| parse_base(&s))
        .transpose()
        .map(Option::flatten)
}

fn parse_base(s: &str) -> Result<Option<String>> {
    let s = s.trim().trim_end_matches('/');
    if s.is_empty() || matches!(s, "0" | "off" | "false" | "no") {
        return Ok(None);
    }
    if s.starts_with("https://")
        && !s[8..].is_empty()
        && !s.contains(['?', '#', '@', '\\', '\n', '\r'])
    {
        return Ok(Some(s.to_owned()));
    }
    #[cfg(test)]
    if s.starts_with("file:///") {
        return Ok(Some(s.to_owned()));
    }
    bail!("JUSTRUST_TOOLCHAIN_DIST must be a trusted HTTPS public store");
}

fn download(base: &str, rel: &str, dest: &Path, max: u64) -> Result<()> {
    #[cfg(test)]
    if let Some(dir) = base.strip_prefix("file://") {
        let src = Path::new(dir).join(rel);
        if std::fs::metadata(&src)?.len() > max {
            bail!("toolchain download exceeds size bound");
        }
        std::fs::copy(src, dest)?;
        return Ok(());
    }
    let out = Command::new("curl")
        .args([
            "-fsS",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "20",
            "--max-time",
            "900",
            "--max-filesize",
        ])
        .arg(max.to_string())
        .arg("-o")
        .arg(dest)
        .arg(format!("{base}/{rel}"))
        .output()
        .context("running toolchain bundle download")?;
    // No redirects: only the explicitly trusted origin provides executable bytes.
    if !out.status.success() || std::fs::metadata(dest)?.len() > max {
        bail!(
            "toolchain bundle download failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn safe_path(path: &Path) -> bool {
    let Some(s) = path.to_str() else { return false };
    !s.is_empty()
        && s.len() <= 4096
        && !s.contains(['\\', ':', '\0'])
        && !path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        && matches!(path.components().next(), Some(std::path::Component::Normal(p)) if p == "bin" || p == "lib" || p == "libexec" || p == "share" || p == "etc")
}

/// V1 deliberately accepts no links, devices, or extension-defined entry types.
/// Every regular file is created exclusively under a fresh, private prefix.
fn extract(archive: &Path, prefix: &Path, want_bytes: u64) -> Result<()> {
    std::fs::create_dir(prefix)?;
    let file = std::fs::File::open(archive)?;
    let max_stream = want_bytes
        .checked_add((MAX_ENTRIES as u64 + 4) * 1024)
        .context("tar size overflow")?;
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut tar = tar::Archive::new(gz.take(max_stream + 1));
    let mut paths = std::collections::HashSet::new();
    let mut bytes = 0u64;
    for (n, entry) in tar.entries()?.raw(true).enumerate() {
        if n >= MAX_ENTRIES {
            bail!("too many toolchain archive entries");
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if !safe_path(&path) || !paths.insert(path.clone()) {
            bail!(
                "unsafe or duplicate toolchain archive path: {}",
                path.display()
            );
        }
        let kind = entry.header().entry_type();
        let out = prefix.join(&path);
        if kind.is_dir() {
            if entry.size() != 0 {
                bail!("directory with payload");
            }
            std::fs::create_dir_all(out)?;
        } else if kind.is_file() {
            bytes = bytes
                .checked_add(entry.size())
                .context("archive size overflow")?;
            if bytes > want_bytes {
                bail!("toolchain archive exceeds declared unpacked size");
            }
            std::fs::create_dir_all(out.parent().context("missing archive parent")?)?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)?;
            if std::io::copy(&mut entry, &mut file)? != entry.size() {
                bail!("truncated toolchain archive");
            }
            file.flush()?;
            // Strip special bits and never preserve ownership from the publisher.
            crate::sys::set_mode(&out, (entry.header().mode()? & 0o755) | 0o600)?;
        } else {
            bail!("toolchain archives cannot contain links or special files");
        }
    }
    // Drain through EOF to validate the gzip footer and bounded trailing padding.
    let mut stream = tar.into_inner();
    let mut tail = [0u8; 8192];
    loop {
        let n = stream.read(&mut tail)?;
        if n == 0 {
            break;
        }
        if tail[..n].iter().any(|b| *b != 0) {
            bail!("nonzero trailing tar data");
        }
    }
    if stream.limit() == 0 || bytes != want_bytes {
        bail!("toolchain archive size mismatch");
    }
    Ok(())
}

fn version(prefix: &Path, bin: &str, flag: &str) -> Result<String> {
    let path = prefix.join("bin").join(crate::sys::exe_name(bin));
    if !is_executable(&path) {
        bail!("toolchain bundle missing executable {bin}");
    }
    let out = Command::new(path).arg(flag).output()?;
    if !out.status.success() {
        bail!(
            "toolchain bundle {bin} does not run: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}

/// Validate the shipped runtime closure without ambient loader overrides.
/// Null output avoids unbounded buffering and pipe-blocked version processes.
fn helper_version(prefix: &Path, bin: &str, timeout: std::time::Duration) -> Result<()> {
    let path = prefix.join("bin").join(crate::sys::exe_name(bin));
    if !is_executable(&path) {
        bail!("toolchain bundle missing executable {bin}");
    }
    let mut child = Command::new(path)
        .arg("--version")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("starting toolchain bundle {bin} --version"))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => bail!("toolchain bundle {bin} --version does not run: {status}"),
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                if let Err(error) = result {
                    return Err(error).context(format!("waiting for toolchain bundle {bin}"));
                }
                bail!("toolchain bundle {bin} --version timed out after {timeout:?}");
            }
        }
    }
}

/// Non-mutating validation also available to repeat-install callers.
pub(super) fn verify_helpers(prefix: &Path, components: &[String]) -> Result<()> {
    helper_version(prefix, "rustdoc", std::time::Duration::from_secs(10))?;
    for (component, bins) in [
        ("clippy-preview", &["clippy-driver", "cargo-clippy"][..]),
        ("rustfmt-preview", &["rustfmt", "cargo-fmt"][..]),
        ("rust-analyzer-preview", &["rust-analyzer"][..]),
    ] {
        if components.iter().any(|c| c == component) {
            for bin in bins {
                helper_version(prefix, bin, std::time::Duration::from_secs(10))?;
            }
        }
    }
    Ok(())
}

fn verify_prefix(prefix: &Path, m: &BundleManifest) -> Result<()> {
    for bin in ["rustc", "rustdoc", "cargo"] {
        if !is_executable(&prefix.join("bin").join(crate::sys::exe_name(bin))) {
            bail!("toolchain bundle missing executable {bin}");
        }
    }
    let stdlib = prefix.join("lib/rustlib").join(TARGET).join("lib");
    if !std::fs::read_dir(stdlib)?.filter_map(|e| e.ok()).any(|e| {
        e.file_type().is_ok_and(|t| t.is_file())
            && e.file_name().to_string_lossy().starts_with("libstd-")
            && e.path().extension().is_some_and(|e| e == "rlib")
    }) {
        bail!("toolchain bundle missing target standard library");
    }
    verify_helpers(prefix, &m.components)?;
    if m.components.iter().any(|c| c == "rust-src")
        && !prefix
            .join("lib/rustlib/src/rust/library/core/src/lib.rs")
            .is_file()
    {
        bail!("toolchain bundle missing rust-src");
    }
    for (component, dir, needle) in [
        (
            "rustc-dev",
            prefix.join("lib/rustlib").join(TARGET).join("lib"),
            "librustc_driver",
        ),
        (
            "rustc-codegen-cranelift-preview",
            prefix
                .join("lib/rustlib")
                .join(TARGET)
                .join("codegen-backends"),
            "cranelift",
        ),
        (
            "llvm-tools-preview",
            prefix.join("lib/rustlib").join(TARGET).join("bin"),
            "llvm-",
        ),
    ] {
        if m.components.iter().any(|c| c == component)
            && !std::fs::read_dir(dir)?.filter_map(|e| e.ok()).any(|e| {
                e.file_type().is_ok_and(|t| t.is_file())
                    && e.file_name().to_string_lossy().contains(needle)
            })
        {
            bail!("toolchain bundle incomplete {component}");
        }
    }
    let vv = version(prefix, "rustc", "-vV")?;
    if vv.lines().next() != Some(m.rustc_version.as_str())
        || vv.lines().find_map(|s| s.strip_prefix("commit-hash: "))
            != Some(m.rustc_commit_hash.as_str())
        || vv.lines().find_map(|s| s.strip_prefix("host: ")) != Some(m.target.as_str())
    {
        bail!("toolchain bundle rustc identity mismatch");
    }
    let cargo = version(prefix, "cargo", "-V")?;
    if cargo_commit(&cargo) != Some(m.cargo_upstream_commit.as_str()) {
        bail!("toolchain bundle cargo upstream commit mismatch");
    }
    let fork = prefix.join("bin").join(crate::sys::exe_name(JR_CARGO));
    match &m.jr_cargo {
        Some(commit) => {
            let v = version(prefix, JR_CARGO, "-V")?;
            if cargo_commit(&v) != Some(m.cargo_upstream_commit.as_str())
                || !v.ends_with(&format!("(jr-cargo {commit})"))
            {
                bail!("toolchain bundle jr-cargo identity mismatch");
            }
        }
        None if fork.exists() => bail!("undeclared jr-cargo in toolchain bundle"),
        None => {}
    }
    Ok(())
}

pub(super) fn install_into(
    spec: &Spec,
    components: &[&str],
    staging: &Path,
    base: &str,
    start: f64,
) -> Result<Installed> {
    std::fs::create_dir_all(staging)?;
    let manifest_path = staging.join("bundle.json");
    download(
        base,
        &format!("v1/toolchains/{TARGET}/{}/manifest.json", spec.id()),
        &manifest_path,
        MAX_MANIFEST,
    )?;
    let bytes = std::fs::read(&manifest_path)?;
    if bytes.len() as u64 > MAX_MANIFEST {
        bail!("toolchain manifest too large");
    }
    let m: BundleManifest =
        serde_json::from_slice(&bytes).context("parsing toolchain bundle manifest")?;
    m.validate(spec)?;
    // Shared archives only substitute a complete requested prefix.
    for c in components.iter().filter(|_| !spec.is_jr()) {
        if !m.components.iter().any(|s| s == c) {
            bail!("shared bundle lacks requested component {c}");
        }
    }
    let archive = staging.join("bundle.tar.gz");
    download(
        base,
        &format!("v1/toolchains/o/{}.tar.gz", m.archive_sha256),
        &archive,
        m.archive_size,
    )?;
    if std::fs::metadata(&archive)?.len() != m.archive_size || sha256(&archive)? != m.archive_sha256
    {
        bail!("toolchain archive checksum or size mismatch");
    }
    let prefix = staging.join("prefix");
    extract(&archive, &prefix, m.unpacked_size)?;
    verify_prefix(&prefix, &m)?;
    Ok(Installed {
        id: spec.id(),
        manifest_date: m.manifest_date,
        manifest_sha256: m
            .official_manifest_sha256
            .unwrap_or(sha256(&manifest_path)?),
        rustc_version: m.rustc_version,
        components: m.components,
        installed_at: if spec.is_jr() {
            0.0
        } else {
            crate::paths::now()
        },
        install_secs: crate::paths::now() - start,
    })
}
