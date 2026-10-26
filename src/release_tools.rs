//! Pinned, checksum-verified tools for the hosted static Linux release path.
//! Installed in the account's toolchain store, never system-wide or as root.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const ZIG_VERSION: &str = "0.13.0";
pub const CARGO_ZIGBUILD_VERSION: &str = "0.20.1";

fn artifacts(arch: &str) -> Result<(&'static str, &'static str)> {
    match arch {
        "x86_64" => Ok((
            "d45312e61ebcc48032b77bc4cf7fd6915c11fa16e4aad116b66c9468211230ea",
            "742ed281f15fef6eaf49535ac10ffe98fb57913d0c4c88d6888d794043c05618",
        )),
        "aarch64" => Ok((
            "041ac42323837eb5624068acd8b00cd5777dac4cf91179e8dad7a7e90dd0c556",
            "e9631045cc5f33ef0d6d9186196192d70b8018bf7633626c3a7c1384e81b7f67",
        )),
        _ => bail!("pinned release tools do not support Linux {arch}"),
    }
}

fn unpack(url: &str, hash: &str, archive: &Path, destination: &Path, xz: bool) -> Result<()> {
    crate::toolchain::curl(url, archive)?;
    if crate::toolchain::sha256(archive)? != hash {
        bail!("release tool archive checksum mismatch");
    }
    std::fs::create_dir_all(destination)?;
    let status = Command::new("tar")
        .arg(if xz { "-xJf" } else { "-xzf" })
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("could not unpack verified release tool archive");
    }
    Ok(())
}

fn version(binary: &Path, arg: &str, expected: &str) -> bool {
    Command::new(binary)
        .arg(arg)
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == expected)
}

fn ready(dir: &Path) -> bool {
    version(
        &dir.join("bin/cargo-zigbuild"),
        "--version",
        &format!("cargo-zigbuild {CARGO_ZIGBUILD_VERSION}"),
    ) && version(
        &dir.join(format!(
            "zig-linux-{}-{ZIG_VERSION}/zig",
            std::env::consts::ARCH
        )),
        "version",
        ZIG_VERSION,
    )
}

/// The versioned tool directories, in PATH order. No arbitrary host install is
/// silently accepted. Explicit auto-install=0 also disables downloading these.
pub fn ensure() -> Result<Vec<PathBuf>> {
    if !cfg!(target_os = "linux") {
        bail!("pinned release tools are supported only on Linux");
    }
    let arch = std::env::consts::ARCH;
    let (zig_hash, cargo_hash) = artifacts(arch)?;
    let root = crate::toolchain::root()?.join("release-tools");
    std::fs::create_dir_all(&root)?;
    let dir = root.join(format!(
        "zig-{ZIG_VERSION}-cargo-zigbuild-{CARGO_ZIGBUILD_VERSION}-{arch}"
    ));
    let paths = || {
        vec![
            dir.join("bin"),
            dir.join(format!("zig-linux-{arch}-{ZIG_VERSION}")),
        ]
    };
    if ready(&dir) {
        return Ok(paths());
    }
    if matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_AUTO_INSTALL").as_deref(),
        Ok("0" | "off" | "false")
    ) {
        bail!("pinned release tools are missing and JUSTRUST_TOOLCHAIN_AUTO_INSTALL is disabled");
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".install.lock"))?;
    lock.lock().context("locking release tool installation")?;
    if ready(&dir) {
        return Ok(paths());
    }
    if dir.exists() {
        bail!(
            "incomplete release tools at {}; repair before retrying",
            dir.display()
        );
    }
    let stage = root.join(format!(".staging-{}", std::process::id()));
    std::fs::create_dir(&stage).context("creating release tool staging directory")?;
    let result = (|| {
        eprintln!(
            "justrust: installing pinned Zig {ZIG_VERSION} and cargo-zigbuild {CARGO_ZIGBUILD_VERSION}"
        );
        unpack(
            &format!(
                "https://ziglang.org/download/{ZIG_VERSION}/zig-linux-{arch}-{ZIG_VERSION}.tar.xz"
            ),
            zig_hash,
            &stage.join("zig.tar.xz"),
            &stage,
            true,
        )?;
        unpack(
            &format!(
                "https://github.com/rust-cross/cargo-zigbuild/releases/download/v{CARGO_ZIGBUILD_VERSION}/cargo-zigbuild-v{CARGO_ZIGBUILD_VERSION}.{arch}-unknown-linux-musl.tar.gz"
            ),
            cargo_hash,
            &stage.join("cargo.tar.gz"),
            &stage.join("bin"),
            false,
        )?;
        if !ready(&stage) {
            bail!("verified release tools do not report the pinned versions");
        }
        std::fs::remove_file(stage.join("zig.tar.xz"))?;
        std::fs::remove_file(stage.join("cargo.tar.gz"))?;
        std::fs::rename(&stage, &dir).context("publishing release tools")?;
        Ok(paths())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

pub fn apply(cmd: &mut Command, paths: Vec<PathBuf>) -> Result<()> {
    let zig = paths
        .get(1)
        .context("missing pinned Zig directory")?
        .join("zig");
    // cargo-zigbuild tries Python's ziglang package before PATH. Disable that
    // discovery and override its binary selector so the verified pin wins.
    cmd.env("CARGO_ZIGBUILD_PYTHON_PATH", "/dev/null")
        .env("CARGO_ZIGBUILD_ZIG_PATH", zig)
        .env_remove("CARGO_ZIGBUILD_RUSTC_VERSION");
    let existing = cmd
        .get_envs()
        .find(|(key, _)| *key == "PATH")
        .and_then(|(_, value)| value.map(std::ffi::OsStr::to_os_string))
        .or_else(|| std::env::var_os("PATH"));
    let mut all = paths;
    if let Some(path) = existing {
        all.extend(std::env::split_paths(&path));
    }
    cmd.env("PATH", std::env::join_paths(all)?);
    // Native development linker interception must not replace Zig's target
    // C runtime and linker in this release path.
    cmd.env("JUSTRUST_LINKER", "off");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_reviewed_archives_are_allowed() {
        for arch in ["x86_64", "aarch64"] {
            let (zig, cargo) = artifacts(arch).unwrap();
            for hash in [zig, cargo] {
                assert_eq!(hash.len(), 64);
                assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
            }
        }
        assert!(artifacts("../x86_64").is_err());
        assert!(artifacts("riscv64").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn tools_preserve_the_selected_compiler_path() {
        let mut cmd = Command::new("cargo");
        cmd.env("PATH", "/pinned/compiler/bin:/usr/bin");
        apply(
            &mut cmd,
            vec!["/pinned/tools/bin".into(), "/pinned/zig".into()],
        )
        .unwrap();
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            path,
            "/pinned/tools/bin:/pinned/zig:/pinned/compiler/bin:/usr/bin"
        );
        assert!(
            cmd.get_envs()
                .any(|(key, value)| key == "CARGO_ZIGBUILD_ZIG_PATH"
                    && value == Some(std::ffi::OsStr::new("/pinned/zig/zig")))
        );
        assert!(
            cmd.get_envs()
                .any(|(key, value)| key == "CARGO_ZIGBUILD_PYTHON_PATH"
                    && value == Some(std::ffi::OsStr::new("/dev/null")))
        );
        assert!(
            cmd.get_envs().any(|(key, value)| key == "JUSTRUST_LINKER"
                && value == Some(std::ffi::OsStr::new("off")))
        );
    }
}
