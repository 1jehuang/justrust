//! Checksum-pinned, account-local cross tools. No SDK license acceptance and no root.
//! Windows GNU is an additional opt-in target, never a replacement for MSVC.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const WINDOWS_GNU: &str = "x86_64-pc-windows-gnu";
const REVISION: &str = "mingw-13.2.0-6ubuntu1_26.1-v1-linux-x86_64";
const PACKAGES: &[(&str, &str)] = &[
    (
        "b/binutils-mingw-w64/binutils-mingw-w64-x86-64_2.41.90.20240122-1ubuntu1+11.4_amd64.deb",
        "28bfabbf64a68813a56360bca056072af9ff71ef4090a4b8a8a085afcda3e4dd",
    ),
    (
        "g/gcc-mingw-w64/gcc-mingw-w64-base_13.2.0-6ubuntu1+26.1_amd64.deb",
        "a30c07e807e9874ea93909448c5d6d37f9569ebe60ad32de8e490aad198538a3",
    ),
    (
        "g/gcc-mingw-w64/gcc-mingw-w64-x86-64-posix_13.2.0-6ubuntu1+26.1_amd64.deb",
        "0855fa1d4a618550faac3a3e4352aa3226833b28184ad3c3b496a68a22c4eda2",
    ),
    (
        "g/gcc-mingw-w64/gcc-mingw-w64-x86-64-posix-runtime_13.2.0-6ubuntu1+26.1_amd64.deb",
        "aca6b438e0b55b11cedcf1170a1ee26e7979e34192e34f6e6389e560908a5bfc",
    ),
    (
        "g/gcc-mingw-w64/g++-mingw-w64-x86-64-posix_13.2.0-6ubuntu1+26.1_amd64.deb",
        "52c3613f386e462ae71105d1a0b2a48d866ccb048ad253acff629c5a2291252d",
    ),
    (
        "m/mingw-w64/mingw-w64-common_11.0.1-3build1_all.deb",
        "4a42fd30fbc3838c58c929d8a4882682fc12408bb7a0e6cafa913c0f0a57c187",
    ),
    (
        "m/mingw-w64/mingw-w64-x86-64-dev_11.0.1-3build1_all.deb",
        "018480a62907fcca19c6bf2975ae01f5c92c23dee8c3cf7a8355c10bf7d42c6f",
    ),
];

pub fn supported(target: &str) -> bool {
    target == WINDOWS_GNU
}

fn ready(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".complete")).is_ok_and(|value| value == REVISION)
        && [
            "usr/bin/x86_64-w64-mingw32-gcc-posix",
            "usr/bin/x86_64-w64-mingw32-g++-posix",
            "usr/bin/x86_64-w64-mingw32-dlltool",
            "usr/x86_64-w64-mingw32/lib/libmingw32.a",
            "usr/x86_64-w64-mingw32/include/windows.h",
        ]
        .iter()
        .all(|path| root.join(path).is_file())
}

/// Extract official distribution packages into the account's versioned tool
/// store. Checksums are pinned from Ubuntu's repository metadata; package
/// maintainer scripts are never executed. Existing partial installs fail closed.
pub fn ensure() -> Result<PathBuf> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        bail!("managed Windows GNU cross tools require an x86_64 Linux build host");
    }
    let parent = crate::toolchain::root()?.join("cross-tools");
    std::fs::create_dir_all(&parent)?;
    let root = parent.join(REVISION);
    if ready(&root) {
        return Ok(root);
    }
    if matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_AUTO_INSTALL").as_deref(),
        Ok("0" | "off" | "false")
    ) {
        bail!(
            "pinned Windows GNU tools are missing and JUSTRUST_TOOLCHAIN_AUTO_INSTALL is disabled"
        );
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join(".install.lock"))?;
    lock.lock()
        .context("locking Windows GNU tool installation")?;
    if ready(&root) {
        return Ok(root);
    }
    if root.exists() {
        bail!("incomplete Windows GNU toolchain at {}", root.display());
    }
    let stage = parent.join(format!(".staging-{}", std::process::id()));
    std::fs::create_dir(&stage).context("creating Windows GNU staging directory")?;
    let result = (|| {
        eprintln!("justrust: installing checksum-pinned Windows GNU cross tools ({REVISION})");
        for (index, (package, hash)) in PACKAGES.iter().enumerate() {
            let archive = stage.join(format!("package-{index}.deb"));
            crate::toolchain::curl(
                &format!("https://archive.ubuntu.com/ubuntu/pool/universe/{package}"),
                &archive,
            )?;
            if crate::toolchain::sha256(&archive)? != *hash {
                bail!("Windows GNU package checksum mismatch: {package}");
            }
            let status = Command::new("dpkg-deb")
                .arg("--extract")
                .arg(&archive)
                .arg(&stage)
                .stdin(Stdio::null())
                .status()
                .context("extracting Windows GNU tools (dpkg-deb is required)")?;
            if !status.success() {
                bail!("could not extract Windows GNU package {package}");
            }
            std::fs::remove_file(archive)?;
        }
        // Running a Linux compiler to inspect its version does not execute any
        // target program. This catches missing host libraries before a build.
        let output = Command::new(stage.join("usr/bin/x86_64-w64-mingw32-gcc-posix"))
            .arg("-dumpfullversion")
            .output()
            .context("validating Windows GNU compiler")?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "13-posix"
        {
            bail!(
                "pinned Windows GNU compiler is not runnable: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::write(stage.join(".complete"), REVISION)?;
        if !ready(&stage) {
            bail!("Windows GNU toolchain is missing required files");
        }
        std::fs::rename(&stage, &root).context("publishing Windows GNU tools")?;
        Ok(root.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

pub fn apply(cmd: &mut Command, root: &Path) -> Result<()> {
    let bin = root.join("usr/bin");
    let cc = bin.join("x86_64-w64-mingw32-gcc-posix");
    let cxx = bin.join("x86_64-w64-mingw32-g++-posix");
    let ar = bin.join("x86_64-w64-mingw32-ar");
    cmd.env("CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER", &cc)
        .env("CC_x86_64_pc_windows_gnu", &cc)
        .env("CXX_x86_64_pc_windows_gnu", &cxx)
        .env("AR_x86_64_pc_windows_gnu", &ar)
        .env("JUSTRUST_LINKER", "off");
    let existing = cmd
        .get_envs()
        .find(|(key, _)| *key == "PATH")
        .and_then(|(_, value)| value.map(std::ffi::OsStr::to_os_string))
        .or_else(|| std::env::var_os("PATH"));
    let mut paths = vec![bin];
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
    fn readiness_requires_revision_and_complete_toolchain() {
        let root = std::env::temp_dir().join(format!(
            "justrust-cross-ready-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        assert!(!ready(&root));
        for name in [
            "usr/bin/x86_64-w64-mingw32-gcc-posix",
            "usr/bin/x86_64-w64-mingw32-g++-posix",
            "usr/bin/x86_64-w64-mingw32-dlltool",
            "usr/x86_64-w64-mingw32/lib/libmingw32.a",
            "usr/x86_64-w64-mingw32/include/windows.h",
        ] {
            let file = root.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "fixture").unwrap();
        }
        std::fs::write(root.join(".complete"), "different revision").unwrap();
        assert!(!ready(&root));
        std::fs::write(root.join(".complete"), REVISION).unwrap();
        assert!(ready(&root));
        std::fs::remove_file(root.join("usr/bin/x86_64-w64-mingw32-dlltool")).unwrap();
        assert!(!ready(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn only_explicit_gnu_target_is_supported() {
        assert!(supported(WINDOWS_GNU));
        for target in [
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
            "x86_64-apple-darwin",
            "../../custom.json",
        ] {
            assert!(!supported(target));
        }
        for (package, hash) in PACKAGES {
            assert!(!package.contains(".."));
            assert_eq!(hash.len(), 64);
            assert!(hash.bytes().all(|c| c.is_ascii_hexdigit()));
        }
    }
    #[test]
    #[cfg(unix)]
    fn cross_env_does_not_override_host_compiler() {
        let mut cmd = Command::new("cargo");
        cmd.env("PATH", "/pinned/rust/bin:/usr/bin");
        cmd.env("CC", "/native/cc");
        apply(&mut cmd, Path::new("/cross/pin")).unwrap();
        let vars: std::collections::BTreeMap<_, _> = cmd
            .get_envs()
            .map(|(key, value)| (key.to_os_string(), value.map(|v| v.to_os_string())))
            .collect();
        assert_eq!(
            vars.get(std::ffi::OsStr::new("CC")).unwrap().as_deref(),
            Some(std::ffi::OsStr::new("/native/cc"))
        );
        assert_eq!(
            vars.get(std::ffi::OsStr::new(
                "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER"
            ))
            .unwrap()
            .as_deref(),
            Some(std::ffi::OsStr::new(
                "/cross/pin/usr/bin/x86_64-w64-mingw32-gcc-posix"
            ))
        );
    }
}
