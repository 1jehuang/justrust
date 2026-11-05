//! Linux-hosted iOS device builds (`aarch64-apple-ios`) against a managed,
//! administrator-provisioned iPhoneOS SDK. Never downloads Apple material.
//!
//! The SDK is imported once per build host with
//! `justrust toolchain import-darwin-sdk --licensed <iPhoneOS.sdk>` and selected
//! by its content ID in `JUSTRUST_DARWIN_SDK`, exactly like the macOS path.
//! Compilation uses genuine LLVM clang, llvm-ar and ld64.lld from the host.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const TARGET: &str = "aarch64-apple-ios";
/// Oldest iOS the linker wrapper targets. Matches rustc's own default floor
/// closely enough for UIKit apps and keeps `-platform_version` well formed.
const MIN_IOS: &str = "15.0";

pub fn supported(target: &str) -> bool {
    target == TARGET
}

fn llvm_names(base: &str) -> Vec<String> {
    std::iter::once(base.to_owned())
        .chain((18..=22).rev().map(|v| format!("{base}-{v}")))
        .collect()
}

fn find(base: &str, marker: &str) -> Result<PathBuf> {
    let names = llvm_names(base);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    crate::msvc_tools::find_llvm(&refs, marker)
}

/// The wrapper script body for one SDK and tool set. Pure, for tests.
fn wrapper(clang: &Path, ld64: &Path, sdk: &Path) -> String {
    let q = |p: &Path| crate::remote_build::shell_quote(&p.display().to_string());
    format!(
        "#!/bin/sh\n# justrust managed iOS compiler/linker driver. Generated, do not edit.\nexec {} --target=arm64-apple-ios{MIN_IOS} -isysroot {} -fuse-ld=lld --ld-path={} -mlinker-version=711 \"$@\"\n",
        q(clang),
        q(sdk),
        q(ld64),
    )
}

/// Publish (once) a content-addressed driver for this SDK and these tools.
fn ensure_wrapper(clang: &Path, ld64: &Path, sdk: &Path) -> Result<PathBuf> {
    let body = wrapper(clang, ld64, sdk);
    let dir = crate::toolchain::root()?.join("ios-tools");
    std::fs::create_dir_all(&dir)?;
    let stage = dir.join(format!(".stage-{}", std::process::id()));
    std::fs::write(&stage, &body)?;
    let id = crate::toolchain::sha256(&stage)?;
    let path = dir.join(format!("clang-ios-{}", &id[..16]));
    if std::fs::read_to_string(&path).is_ok_and(|s| s == body) {
        let _ = std::fs::remove_file(&stage);
        return Ok(path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&stage, &path).context("publishing iOS driver")?;
    Ok(path)
}

/// lld picks its flavor from argv[0]. Distribution `ld64.lld` is a symlink to
/// `lld`, and tool discovery canonicalizes it, so expose it under its name.
fn ld64_alias(lld: &Path) -> Result<PathBuf> {
    let id = crate::toolchain::sha256(lld)?;
    let dir = crate::toolchain::root()?
        .join("ios-tools")
        .join(format!("lld-{}", &id[..16]));
    let alias = dir.join("ld64.lld");
    if std::fs::read_link(&alias).is_ok_and(|t| t == lld) {
        return Ok(alias);
    }
    std::fs::create_dir_all(&dir)?;
    let stage = dir.join(format!(".ld64-{}", std::process::id()));
    let _ = std::fs::remove_file(&stage);
    #[cfg(unix)]
    std::os::unix::fs::symlink(lld, &stage).context("creating ld64.lld alias")?;
    std::fs::rename(&stage, &alias)?;
    Ok(alias)
}

/// Remove target-triple tokens that distribution lld (18-21) rejects as
/// "unknown architecture" from one TAPI stub. Only the `targets:` list and
/// per-section `targets:` entries name architectures, and the remaining
/// arm64e/arm64 entries keep every symbol available to the linker.
fn normalize_tbd(text: &str) -> String {
    text.replace(", arm64e.x1-ios", "")
        .replace("arm64e.x1-ios, ", "")
}

/// A linker view of the managed SDK with normalized stubs. The managed SDK
/// itself is never modified. Keyed by the SDK ID and normalization version.
fn linker_sdk(sdk: &Path, sdk_id: &str) -> Result<PathBuf> {
    let dir = crate::toolchain::root()?
        .join("ios-tools")
        .join(format!("sdk-v1-{sdk_id}"));
    if dir.join(".complete").is_file() {
        return Ok(dir.join("sdk"));
    }
    let parent = dir.parent().context("ios tools dir")?;
    std::fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".sdk-stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    fn copy(src: &Path, dst: &Path) -> Result<()> {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            let (s, d) = (e.path(), dst.join(e.file_name()));
            if e.file_type()?.is_dir() {
                copy(&s, &d)?;
            } else if s.extension().is_some_and(|x| x == "tbd") {
                std::fs::write(&d, normalize_tbd(&std::fs::read_to_string(&s)?))?;
            } else if std::fs::hard_link(&s, &d).is_err() {
                std::fs::copy(&s, &d)?;
            }
        }
        Ok(())
    }
    copy(sdk, &stage.join("sdk"))?;
    std::fs::write(stage.join(".complete"), sdk_id)?;
    if let Err(e) = std::fs::rename(&stage, &dir) {
        let _ = std::fs::remove_dir_all(&stage);
        if !dir.join(".complete").is_file() {
            return Err(e).context("publishing iOS linker SDK");
        }
    }
    Ok(dir.join("sdk"))
}

/// Configure only the owned cargo command: linker, C compiler, archiver and
/// SDKROOT for `aarch64-apple-ios`. Overrides inherited values so a laptop's
/// wrapper paths never leak into a hosted build.
pub fn apply(cmd: &mut Command, sdk_id: &str) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("managed iOS cross builds require a Linux build host");
    }
    let sdk = crate::darwin_tools::validated(sdk_id, crate::darwin_tools::Platform::Ios)?;
    let sdk = linker_sdk(&sdk, sdk_id)?;
    let clang = find("clang", "clang version")?;
    let ld64 = find("ld64.lld", "LLD")?;
    let ar = match find("llvm-ar", "LLVM") {
        Ok(p) => p,
        Err(_) => crate::msvc_tools::install_llvm_ar(
            std::env::var("JUSTRUST_INSTALL_LLVM_TOOLS").as_deref() == Ok("1"),
        )?,
    };
    let ld64 = ld64_alias(&ld64)?;
    let driver = ensure_wrapper(&clang, &ld64, &sdk)?;
    cmd.env("SDKROOT", &sdk)
        .env("CARGO_TARGET_AARCH64_APPLE_IOS_LINKER", &driver)
        .env("CC_aarch64_apple_ios", &driver)
        .env("CXX_aarch64_apple_ios", &driver)
        .env("AR_aarch64_apple_ios", &ar)
        .env("IPHONEOS_DEPLOYMENT_TARGET", MIN_IOS)
        // Native linker interception targets Linux ELF links only.
        .env("JUSTRUST_LINKER", "off");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tbd_normalization_drops_only_unknown_arch() {
        assert_eq!(
            normalize_tbd("targets:         [ arm64e-ios, arm64e.x1-ios ]\n"),
            "targets:         [ arm64e-ios ]\n"
        );
        assert_eq!(
            normalize_tbd("[ arm64e.x1-ios, arm64-ios ]"),
            "[ arm64-ios ]"
        );
        assert_eq!(normalize_tbd("[ arm64-ios ]"), "[ arm64-ios ]");
    }

    #[test]
    fn only_device_target() {
        assert!(supported("aarch64-apple-ios"));
        assert!(!supported("aarch64-apple-ios-sim"));
        assert!(!supported("x86_64-apple-ios"));
        assert!(!supported("aarch64-apple-darwin"));
    }

    #[test]
    fn wrapper_quotes_paths_and_pins_target() {
        let w = wrapper(
            Path::new("/usr/bin/clang"),
            Path::new("/usr/lib/llvm-18/bin/ld64.lld"),
            Path::new("/home/a b/sdk"),
        );
        assert!(w.starts_with("#!/bin/sh\n"));
        assert!(w.contains("--target=arm64-apple-ios15.0"));
        assert!(w.contains("-isysroot '/home/a b/sdk'"));
        assert!(
            w.contains("--ld-path=/usr/lib/llvm-18/bin/ld64.lld")
                || w.contains("--ld-path='/usr/lib/llvm-18/bin/ld64.lld'")
        );
        assert!(w.trim_end().ends_with("\"$@\""));
    }
}
