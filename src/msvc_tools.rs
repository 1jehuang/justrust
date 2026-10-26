//! Account-local, explicitly licensed Windows MSVC cross compilation.
//! This prepares cargo-xwin, not signing services or a Windows execution host.
use anyhow::{Context, Result, bail};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const CARGO_XWIN_VERSION: &str = "0.20.2";
pub const SDK_VERSION: &str = "10.0.26100";
pub const CRT_VERSION: &str = "14.44.17.14";
pub const VS_VERSION: &str = "17.14";
pub const ACCEPT_LICENSE_ENV: &str = "JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE";
const REVISION: &str = "cargo-xwin-0.20.2-vs17.14-sdk10.0.26100-crt14.44.17.14-v1";

pub fn supported(target: &str) -> bool {
    matches!(target, "x86_64-pc-windows-msvc" | "aarch64-pc-windows-msvc")
}

fn validate_target(target: &str) -> Result<()> {
    if !supported(target) {
        bail!("unsupported managed Windows MSVC target: {target}");
    }
    Ok(())
}

fn require_license(value: Option<&OsStr>) -> Result<()> {
    if value != Some(OsStr::new("1")) {
        bail!(
            "Windows MSVC requires explicit Microsoft SDK license acceptance: review https://go.microsoft.com/fwlink/?LinkId=2086102, then set {ACCEPT_LICENSE_ENV}=1 if authorized. No SDK was downloaded or used"
        );
    }
    Ok(())
}

fn archive_hash(arch: &str) -> Result<&'static str> {
    match arch {
        "x86_64" => Ok("81eb698b63fc196ad0601167dbc4bd191fdf7543b48ff1ab6cded6eb354f92bd"),
        "aarch64" => Ok("71420b6949774eb75fc89704191b5e858146645083ef7ffba11cd008030a9a0f"),
        _ => bail!("managed Windows MSVC tools do not support Linux {arch}"),
    }
}

fn ready(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".complete")).is_ok_and(|s| s == REVISION)
        && root.join("bin/cargo-xwin").is_file()
}

/// Install the reviewed cargo-xwin archive, never SDK payloads. License consent
/// is required even for warm installations because cargo-xwin itself assumes it.
/// The caller must separately provision clang/LLVM and the Rust target stdlib.
pub fn prepare(target: &str) -> Result<PathBuf> {
    validate_target(target)?;
    require_license(std::env::var_os(ACCEPT_LICENSE_ENV).as_deref())?;
    if !cfg!(target_os = "linux") {
        bail!("managed Windows MSVC cross tools currently require a Linux host");
    }
    let arch = std::env::consts::ARCH;
    let hash = archive_hash(arch)?;
    let parent = crate::toolchain::root()?.join("msvc-tools");
    let root = parent.join(format!("{REVISION}-{arch}"));
    if ready(&root) {
        return Ok(root);
    }
    if matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_AUTO_INSTALL").as_deref(),
        Ok("0" | "off" | "false")
    ) {
        bail!("pinned cargo-xwin is missing and JUSTRUST_TOOLCHAIN_AUTO_INSTALL is disabled");
    }
    std::fs::create_dir_all(&parent)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join(".install.lock"))?;
    lock.lock()
        .context("locking Windows MSVC tool installation")?;
    if ready(&root) {
        return Ok(root);
    }
    if root.exists() {
        bail!(
            "incomplete Windows MSVC tools at {}; repair before retrying",
            root.display()
        );
    }
    let stage = parent.join(format!(".staging-{}", std::process::id()));
    std::fs::create_dir(&stage).context("creating Windows MSVC tool staging directory")?;
    let result = (|| {
        let archive = stage.join("cargo-xwin.tar.gz");
        crate::toolchain::curl(
            &format!(
                "https://github.com/rust-cross/cargo-xwin/releases/download/v{CARGO_XWIN_VERSION}/cargo-xwin-v{CARGO_XWIN_VERSION}.{arch}-unknown-linux-musl.tar.gz"
            ),
            &archive,
        )?;
        if crate::toolchain::sha256(&archive)? != hash {
            bail!("cargo-xwin archive checksum mismatch");
        }
        let bin = stage.join("bin");
        std::fs::create_dir(&bin)?;
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&bin)
            .stdin(Stdio::null())
            .status()
            .context("extracting verified cargo-xwin archive")?;
        if !status.success() {
            bail!("could not extract verified cargo-xwin archive");
        }
        let output = Command::new(bin.join("cargo-xwin"))
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .context("checking pinned cargo-xwin version")?;
        if !output.status.success()
            || String::from_utf8_lossy(&output.stdout).trim()
                != format!("cargo-xwin {CARGO_XWIN_VERSION}")
        {
            bail!("verified cargo-xwin does not report the pinned version");
        }
        std::fs::remove_file(archive)?;
        std::fs::write(stage.join(".complete"), REVISION)?;
        std::fs::rename(&stage, &root).context("publishing Windows MSVC tools")?;
        Ok(root.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

/// Configure a `cargo xwin ... --target TARGET` command. This does not invoke it.
/// SDK/CRT data is account-local and separated by immutable version and target.
/// No generic CC/CXX/AR or signing configuration is read or changed.
pub fn apply(cmd: &mut Command, root: &Path, target: &str) -> Result<()> {
    validate_target(target)?;
    let inherited = std::env::var_os(ACCEPT_LICENSE_ENV);
    let acceptance = cmd
        .get_envs()
        .find(|(key, _)| *key == ACCEPT_LICENSE_ENV)
        .map(|(_, value)| value)
        .unwrap_or(inherited.as_deref());
    require_license(acceptance)?;
    configure(cmd, root, target)
}

fn configure(cmd: &mut Command, root: &Path, target: &str) -> Result<()> {
    let arch = match target {
        "x86_64-pc-windows-msvc" => "x86_64",
        "aarch64-pc-windows-msvc" => "aarch64",
        _ => bail!("unsupported managed Windows MSVC target: {target}"),
    };
    cmd.env("XWIN_CROSS_COMPILER", "clang-cl")
        .env("XWIN_ARCH", arch)
        .env("XWIN_VARIANT", "desktop")
        .env("XWIN_VERSION", VS_VERSION)
        .env("XWIN_SDK_VERSION", SDK_VERSION)
        .env("XWIN_CRT_VERSION", CRT_VERSION)
        .env("XWIN_CACHE_DIR", root.join("sdk-cache").join(target))
        .env("XWIN_INCLUDE_ATL", "false")
        .env("XWIN_INCLUDE_DEBUG_LIBS", "false")
        .env("XWIN_INCLUDE_DEBUG_SYMBOLS", "false")
        .env("JUSTRUST_LINKER", "off");
    let existing = cmd
        .get_envs()
        .find(|(key, _)| *key == "PATH")
        .map(|(_, value)| value.map(OsStr::to_os_string))
        .unwrap_or_else(|| std::env::var_os("PATH"));
    let mut paths = vec![root.join("bin")];
    if let Some(existing) = existing {
        paths.extend(std::env::split_paths(&existing));
    }
    cmd.env("PATH", std::env::join_paths(paths)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reviewed_targets_and_archives() {
        for target in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
            assert!(supported(target));
        }
        for target in [
            "i686-pc-windows-msvc",
            "x86_64-pc-windows-gnu",
            "../../custom.json",
        ] {
            assert!(!supported(target));
        }
        for arch in ["x86_64", "aarch64"] {
            let hash = archive_hash(arch).unwrap();
            assert_eq!(hash.len(), 64);
            assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert!(archive_hash("../x86_64").is_err());
    }

    #[test]
    fn readiness_requires_matching_revision_and_binary() {
        let root = std::env::temp_dir().join(format!(
            "justrust-msvc-ready-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        assert!(!ready(&root));
        std::fs::write(root.join(".complete"), REVISION).unwrap();
        assert!(!ready(&root));
        std::fs::write(root.join("bin/cargo-xwin"), "fixture").unwrap();
        assert!(ready(&root));
        std::fs::write(root.join(".complete"), "old-sdk-revision").unwrap();
        assert!(!ready(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn license_requires_exact_opt_in() {
        assert!(require_license(Some(OsStr::new("1"))).is_ok());
        for value in [
            None,
            Some(""),
            Some("true"),
            Some("yes"),
            Some("0"),
            Some(" 1"),
        ] {
            assert!(require_license(value.map(OsStr::new)).is_err());
        }
        let mut cmd = Command::new("cargo");
        cmd.env_remove(ACCEPT_LICENSE_ENV);
        assert!(apply(&mut cmd, Path::new("/unused"), "x86_64-pc-windows-msvc").is_err());
        assert!(!cmd.get_envs().any(|(key, _)| key == "XWIN_CACHE_DIR"));
    }

    #[test]
    #[cfg(unix)]
    fn fixed_pins_preserve_host_tools_and_separate_target_caches() {
        for target in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
            let mut cmd = Command::new("cargo");
            cmd.env(ACCEPT_LICENSE_ENV, "1")
                .env("CC", "/native/cc")
                .env("CXX", "/native/c++")
                .env("AR", "/native/ar")
                .env("PATH", "/pinned/rust/bin:/usr/bin")
                .env("XWIN_SDK_VERSION", "latest")
                .env("XWIN_CACHE_DIR", "/shared");
            apply(&mut cmd, Path::new("/account/tools"), target).unwrap();
            let env: std::collections::BTreeMap<_, _> = cmd.get_envs().collect();
            for (key, value) in [
                ("CC", "/native/cc"),
                ("CXX", "/native/c++"),
                ("AR", "/native/ar"),
                ("PATH", "/account/tools/bin:/pinned/rust/bin:/usr/bin"),
                ("XWIN_SDK_VERSION", SDK_VERSION),
                ("XWIN_CRT_VERSION", CRT_VERSION),
            ] {
                assert_eq!(env[OsStr::new(key)], Some(OsStr::new(value)));
            }
            let cache = Path::new("/account/tools/sdk-cache").join(target);
            assert_eq!(env[OsStr::new("XWIN_CACHE_DIR")], Some(cache.as_os_str()));
        }
    }
}
