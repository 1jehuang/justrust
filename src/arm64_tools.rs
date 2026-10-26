//! Managed GNU/Linux ARM64 cross tools and the official Jcode TLS contract.
use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const TARGET: &str = "aarch64-unknown-linux-gnu";
pub const TARGET_RUSTFLAGS: &str = "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS";

pub fn supported(target: &str) -> bool {
    target == TARGET
}

fn environment(cmd: &Command, name: &str) -> Option<OsString> {
    match cmd.get_envs().find(|(key, _)| *key == name) {
        Some((_, value)) => value.map(ToOwned::to_owned),
        None => std::env::var_os(name),
    }
}

fn emulated(flags: &str) -> bool {
    let normalized = flags.replace('\u{1f}', " ");
    let words: Vec<_> = normalized.split_whitespace().collect();
    words.contains(&"-Ztls-model=emulated")
        || words
            .windows(2)
            .any(|pair| pair == ["-Z", "tls-model=emulated"])
}

/// Enforce the dedicated official ARM64 mode: rebuild std and use target-only
/// emulated TLS. This pure check also accepts resolved configuration-file flags.
pub fn validate(args: &[String], target_flags: &str, global_flags: &str) -> Result<()> {
    if !global_flags.trim().is_empty() {
        bail!(
            "official ARM64 builds require target-only rustflags, not global RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS (host build scripts must retain native TLS)"
        );
    }
    let normalized = target_flags.replace('\u{1f}', " ");
    let words: Vec<_> = normalized.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let setting = word
            .strip_prefix("-Z")
            .filter(|value| !value.is_empty())
            .or_else(|| (*word == "-Z").then(|| words.get(i + 1).copied()).flatten());
        if let Some(model) = setting.and_then(|value| value.strip_prefix("tls-model="))
            && model != "emulated"
        {
            bail!("official ARM64 builds require tls-model=emulated, not {model}");
        }
    }
    let mut std = false;
    let mut panic_abort = false;
    let mut args = args
        .iter()
        .map(String::as_str)
        .take_while(|arg| *arg != "--");
    while let Some(arg) = args.next() {
        let value = if arg == "-Z" {
            args.next().and_then(|arg| arg.strip_prefix("build-std="))
        } else {
            arg.strip_prefix("-Zbuild-std=")
        };
        if let Some(value) = value {
            // The last repeated Cargo option wins. Do not union separate lists.
            std = value.split(',').any(|item| item == "std");
            panic_abort = value.split(',').any(|item| item == "panic_abort");
        }
    }
    if !std || !panic_abort {
        bail!(
            "ARM64 emulated TLS requires nightly Cargo with -Zbuild-std=std,panic_abort and matching rust-src; stock std does not preserve the official Jcode TLS contract"
        );
    }
    Ok(())
}

/// Validate the target command, then install/apply checksum-pinned GCC and
/// binutils. The caller dispatches native `cargo build` and selects the nightly
/// compiler. This helper never changes the requested Rust target.
pub fn configure(cmd: &mut Command, args: &[String]) -> Result<()> {
    configure_flags(cmd, args)?;
    apply(cmd, &ensure()?)
}

fn configure_flags(cmd: &mut Command, args: &[String]) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("managed ARM64 cross tools require a Linux build host");
    }
    let target = args
        .iter()
        .enumerate()
        .take_while(|(_, a)| a.as_str() != "--")
        .filter_map(|(i, a)| {
            a.strip_prefix("--target=").or_else(|| {
                (a == "--target")
                    .then(|| args.get(i + 1).map(String::as_str))
                    .flatten()
            })
        })
        .last();
    if !target.is_some_and(supported) {
        bail!("managed ARM64 tools require an explicit --target {TARGET} on Linux");
    }
    let target_flags = environment(cmd, TARGET_RUSTFLAGS).unwrap_or_default();
    let global_flags = ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"]
        .map(|name| {
            environment(cmd, name)
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .join(" ");
    validate(args, &target_flags.to_string_lossy(), &global_flags)?;
    let mut target_flags = target_flags.to_string_lossy().into_owned();
    if !emulated(&target_flags) {
        if !target_flags.is_empty() {
            target_flags.push(' ');
        }
        target_flags.push_str("-Ztls-model=emulated");
    }
    cmd.env(TARGET_RUSTFLAGS, target_flags)
        // Even an empty encoded/global value takes precedence over target flags.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    Ok(())
}

/// Structural target validation only. TLS provenance and runtime acceptance
/// require the explicit build contract and separate acceptance checks.
pub fn validate_artifact(data: &[u8]) -> Result<()> {
    use anyhow::Context;
    use object::read::elf::{ElfFile64, FileHeader, ProgramHeader};
    let elf =
        ElfFile64::<object::Endianness>::parse(data).context("expected ARM64 ELF64 artifact")?;
    let endian = elf.endian();
    if endian != object::Endianness::Little
        || elf.elf_header().e_machine(endian) != object::elf::EM_AARCH64
        || !matches!(
            elf.elf_header().e_type(endian),
            object::elf::ET_EXEC | object::elf::ET_DYN
        )
    {
        bail!("artifact is not an AArch64 Linux executable ELF");
    }
    let mut executable = false;
    for header in elf.elf_program_headers() {
        header
            .data(endian, data)
            .map_err(|()| anyhow::anyhow!("invalid ARM64 ELF segment extent"))?;
        executable |= header.p_type(endian) == object::elf::PT_LOAD
            && header.p_flags(endian) & object::elf::PF_X != 0;
    }
    if !executable {
        bail!("ARM64 artifact has no executable load segment");
    }
    Ok(())
}

const REVISION: &str = "arm64-gcc13-ubuntu24.04-v1-linux-x86_64";
// Official Ubuntu Noble release Packages indexes, SHA256 fields. Extraction
// deliberately does not execute maintainer scripts or install system packages.
const PACKAGES: &[(&str, &str)] = &[
    (
        "pool/main/g/gcc-13-cross/gcc-13-aarch64-linux-gnu_13.2.0-23ubuntu4cross1_amd64.deb",
        "4b51c73c765a663d24302f9548180c28c8ba129b43c139cbdd2d42fef3025545",
    ),
    (
        "pool/main/g/gcc-13-cross/g++-13-aarch64-linux-gnu_13.2.0-23ubuntu4cross1_amd64.deb",
        "aaddcb271fe50a6bd300425633dbb003f2251f9cf3ba65cbfe0ed3a48a3f4095",
    ),
    (
        "pool/main/g/gcc-13-cross/cpp-13-aarch64-linux-gnu_13.2.0-23ubuntu4cross1_amd64.deb",
        "c5712899bcb4b81e26532f2717c1148aa89fbbd2862dae8f022306dec885c662",
    ),
    (
        "pool/main/g/gcc-13-cross/gcc-13-aarch64-linux-gnu-base_13.2.0-23ubuntu4cross1_amd64.deb",
        "cfb0290f592c4612c79ee4b61772c5bcdbc2970023151b3ec699f20a7e81a943",
    ),
    (
        "pool/main/g/gcc-13-cross/gcc-13-cross-base_13.2.0-23ubuntu4cross1_all.deb",
        "1a9adce5c298fa1ded7a9f3448d9cdbcd68c2b14e5d7a7f8b356a1ff54f37e91",
    ),
    (
        "pool/main/b/binutils/binutils-aarch64-linux-gnu_2.42-4ubuntu2_amd64.deb",
        "2628d4f91d41da925add56392e1dd5289929db70413d29115b81c74397b5f16f",
    ),
    (
        "pool/main/b/binutils/binutils-common_2.42-4ubuntu2_amd64.deb",
        "0f5be03f1a6237bb7f156f0e8a25223cf2f209b75c591e4e741214f2e203aa48",
    ),
    (
        "pool/main/c/cross-toolchain-base/libc6-arm64-cross_2.39-0ubuntu8cross1_all.deb",
        "76547cb8a4d7f057e23dcade45a17fc0f3d5c2fa0d3ff125bca56f7c4ecd7a07",
    ),
    (
        "pool/main/c/cross-toolchain-base/libc6-dev-arm64-cross_2.39-0ubuntu8cross1_all.deb",
        "04c6956de42669a53e620ad04a32475ea0bd4325808c227bf5ba52498f0d3f90",
    ),
    (
        "pool/main/c/cross-toolchain-base/linux-libc-dev-arm64-cross_6.8.0-25.25cross1_all.deb",
        "6a5a00b8ba8de66862e05493def87a5cbbb23b949601e45a5e3cbde56505cb6b",
    ),
    (
        "pool/main/g/gcc-13-cross/libgcc-13-dev-arm64-cross_13.2.0-23ubuntu4cross1_all.deb",
        "281d290622cc9cb33ef5e2eeb5f80a8c372abcbbd68d42c2455e39e9606741a2",
    ),
    (
        "pool/main/g/gcc-13-cross/libstdc++-13-dev-arm64-cross_13.2.0-23ubuntu4cross1_all.deb",
        "b84702770d493c63728ac151e45dcd6a82e5d065b1e3935a8406c4891a01d327",
    ),
    (
        "pool/main/g/gcc-14-cross/libgcc-s1-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "8429020abee5e84eadb5ff40e3b4c0b122134998b6c4c9bb392abf7b43daa058",
    ),
    (
        "pool/main/g/gcc-14-cross/libstdc++6-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "b057ba7b09c39dec1bfdf232cd6e93601928fea7aa21accea35621a28f8b06e3",
    ),
    (
        "pool/main/g/gcc-14-cross/libgomp1-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "a790d8f02b3f353807cf9aea9575b7e055989105fc9743e8f53c2fa5f6ce7b85",
    ),
    (
        "pool/main/g/gcc-14-cross/libatomic1-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "23e8901e410e640cbbd94fd15ab709242fbc836d0d384702582288aa2ce153b2",
    ),
    (
        "pool/main/g/gcc-14-cross/libitm1-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "6ab9d595eb3efacaaf58d87366f54e2f7348fba2887c8ea351f9f32714a80f2b",
    ),
    (
        "pool/main/g/gcc-14-cross/libasan8-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "e20f0b3f9c1024ae4fb21a4f3a659b971d51eca7d4d8be6ac0d101c99596df35",
    ),
    (
        "pool/main/g/gcc-14-cross/liblsan0-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "e03e64ed2fce49cb57600663a357757bff3532cd28f3b0a23022c4faf457f6ca",
    ),
    (
        "pool/main/g/gcc-14-cross/libtsan2-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "0e90e2734207a91c91822f374cf0356c2b190ca3d2529e2fa12fcfb31442cfd2",
    ),
    (
        "pool/main/g/gcc-14-cross/libubsan1-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "2397491389a02d926ab3361b435172fd77a9c6150bd319eb350dee00da2d3c5e",
    ),
    (
        "pool/main/g/gcc-14-cross/libhwasan0-arm64-cross_14-20240412-0ubuntu1cross1_all.deb",
        "2cb6b8016f66859bbd4f9a2a9f8dc28a10b52d8d999155017ef5702ef1a4d36e",
    ),
    (
        "pool/main/i/isl/libisl23_0.26-3build1_amd64.deb",
        "24b1aa0e1772080fd23df7bed10aadcbbe16b3181b77d3a013c14dfaf9848541",
    ),
    (
        "pool/main/m/mpclib3/libmpc3_1.3.1-1build1_amd64.deb",
        "34af1193734f40723c7fc03d855d98c107d8b62bd2fa17dae8b09d2369cdd45e",
    ),
    (
        "pool/main/m/mpfr4/libmpfr6_4.2.1-1build1_amd64.deb",
        "a25c2f5f123829ab144f31ddb22f4363b7206eb21bf821bfbecca5753ccfc6be",
    ),
    (
        "pool/main/g/gmp/libgmp10_6.3.0+dfsg-2ubuntu6_amd64.deb",
        "649a5ee5f74f0aa1e1b9c4f169eabbae56a79aa5aac51ef733eb9e18700bd8fc",
    ),
    (
        "pool/main/j/jansson/libjansson4_2.14-2build2_amd64.deb",
        "0cf79113f5d193ce9af2be2ff4b2c3b30dd4e55a0b6c47f7d28f6c849ff3aa60",
    ),
    (
        "pool/main/b/binutils/libsframe1_2.42-4ubuntu2_amd64.deb",
        "0b8d3e9338b2d47a523532463e341962edd1e669fb1760f12d82d7a68b33096a",
    ),
    (
        "pool/main/libz/libzstd/libzstd1_1.5.5+dfsg2-2build1_amd64.deb",
        "4dcc3ba9606a5837a55248d08162a5b426dd621f75c0eb24d86409e0d0e71d52",
    ),
    (
        "pool/main/z/zlib/zlib1g_1.3.dfsg-3.1ubuntu2_amd64.deb",
        "0b93d16d7498f092fa3070fbbad28cdbc6b3d640f1a7681b96fc37f20d1219f1",
    ),
];

fn extract_deb(archive: &Path, destination: &Path) -> Result<()> {
    let listing = Command::new("ar")
        .arg("t")
        .arg(archive)
        .output()
        .context("listing verified ARM64 Debian archive (ar is required)")?;
    if !listing.status.success() {
        bail!("could not list verified ARM64 Debian archive");
    }
    let listing = String::from_utf8(listing.stdout)?;
    let member = listing
        .lines()
        .find(|line| {
            matches!(
                *line,
                "data.tar.xz" | "data.tar.zst" | "data.tar.gz" | "data.tar"
            )
        })
        .context("verified Debian archive has no supported data member")?;
    let data = destination.join(".package-data.tar");
    let output = std::fs::File::create(&data)?;
    let status = Command::new("ar")
        .arg("p")
        .arg(archive)
        .arg(member)
        .stdout(output)
        .status()?;
    if !status.success() {
        bail!("could not extract verified Debian data member");
    }
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&data)
        .arg("-C")
        .arg(destination)
        .arg("--no-same-owner")
        .stdin(Stdio::null())
        .status()
        .context("unpacking ARM64 GCC package (tar is required)")?;
    std::fs::remove_file(data)?;
    if !status.success() {
        bail!("could not unpack verified ARM64 GCC package");
    }
    Ok(())
}

fn ready(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".complete")).is_ok_and(|value| value == REVISION)
        && [
            "usr/bin/aarch64-linux-gnu-gcc-13",
            "usr/bin/aarch64-linux-gnu-g++-13",
            "usr/bin/aarch64-linux-gnu-ld.bfd",
            "usr/aarch64-linux-gnu/include/stdlib.h",
            "usr/aarch64-linux-gnu/lib/libc.so",
            "usr/lib/gcc-cross/aarch64-linux-gnu/13/libgcc.a",
        ]
        .iter()
        .all(|path| root.join(path).is_file())
}

/// Pinned official distribution binaries and sysroot, extracted without root or
/// package scripts. Host glibc >= 2.38 plus `ar` and `tar` are required.
pub fn ensure() -> Result<PathBuf> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        bail!("managed ARM64 GCC requires an x86_64 Linux build host");
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
        bail!("pinned ARM64 GCC is missing and JUSTRUST_TOOLCHAIN_AUTO_INSTALL is disabled");
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join(".install.lock"))?;
    lock.lock().context("locking ARM64 GCC installation")?;
    if ready(&root) {
        return Ok(root);
    }
    if root.exists() {
        bail!("incomplete ARM64 GCC toolchain at {}", root.display());
    }
    let stage = parent.join(format!(".arm64-staging-{}", std::process::id()));
    std::fs::create_dir(&stage).context("creating ARM64 GCC staging directory")?;
    let result = (|| {
        eprintln!("justrust: installing checksum-pinned ARM64 GCC ({REVISION})");
        for (index, (package, hash)) in PACKAGES.iter().enumerate() {
            let archive = stage.join(format!("package-{index}.deb"));
            crate::toolchain::curl(
                &format!("https://archive.ubuntu.com/ubuntu/{package}"),
                &archive,
            )?;
            if crate::toolchain::sha256(&archive)? != *hash {
                bail!("ARM64 GCC package checksum mismatch: {package}");
            }
            extract_deb(&archive, &stage)?;
            std::fs::remove_file(archive)?;
        }
        let mut command = Command::new(stage.join("usr/bin/aarch64-linux-gnu-gcc-13"));
        apply(&mut command, &stage)?;
        let output = command
            .arg("-dumpfullversion")
            .output()
            .context("validating ARM64 GCC")?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "13.2.0" {
            bail!(
                "pinned ARM64 GCC is not runnable: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::write(stage.join(".complete"), REVISION)?;
        if !ready(&stage) {
            bail!("ARM64 GCC toolchain is missing required files");
        }
        std::fs::rename(&stage, &root).context("publishing ARM64 GCC tools")?;
        Ok(root.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

/// Apply target-only compiler/linker selection. Host CC/CXX/AR are untouched.
pub fn apply(cmd: &mut Command, root: &Path) -> Result<()> {
    let bin = root.join("usr/bin");
    cmd.env(
        "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER",
        bin.join("aarch64-linux-gnu-gcc-13"),
    )
    .env(
        "CC_aarch64_unknown_linux_gnu",
        bin.join("aarch64-linux-gnu-gcc-13"),
    )
    .env(
        "CXX_aarch64_unknown_linux_gnu",
        bin.join("aarch64-linux-gnu-g++-13"),
    )
    .env(
        "AR_aarch64_unknown_linux_gnu",
        bin.join("aarch64-linux-gnu-ar"),
    )
    .env("JUSTRUST_LINKER", "off");
    for (name, prefix) in [
        ("PATH", bin),
        ("LD_LIBRARY_PATH", root.join("usr/lib/x86_64-linux-gnu")),
    ] {
        let mut paths = vec![prefix];
        if let Some(existing) = environment(cmd, name) {
            paths.extend(std::env::split_paths(&existing));
        }
        cmd.env(name, std::env::join_paths(paths)?);
    }
    let sysroot = format!("--sysroot={}", root.display());
    for name in [
        "CFLAGS_aarch64_unknown_linux_gnu",
        "CXXFLAGS_aarch64_unknown_linux_gnu",
    ] {
        let mut flags = environment(cmd, name).unwrap_or_default();
        if !flags.is_empty() {
            flags.push(" ");
        }
        flags.push(&sysroot);
        cmd.env(name, flags);
    }
    let mut flags = environment(cmd, TARGET_RUSTFLAGS).unwrap_or_default();
    flags.push(format!(
        " -Clinker-flavor=gcc -Clink-arg=-fuse-ld=bfd -Clink-arg={sysroot}"
    ));
    cmd.env(TARGET_RUSTFLAGS, flags);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn gcc_environment_is_target_scoped() {
        let mut cmd = Command::new("cargo");
        cmd.env("CC", "/native/cc")
            .env("PATH", "/rust/bin:/usr/bin")
            .env(TARGET_RUSTFLAGS, "-Ztls-model=emulated");
        apply(&mut cmd, Path::new("/managed/arm")).unwrap();
        assert_eq!(environment(&cmd, "CC").unwrap(), "/native/cc");
        assert_eq!(
            environment(&cmd, "CC_aarch64_unknown_linux_gnu").unwrap(),
            "/managed/arm/usr/bin/aarch64-linux-gnu-gcc-13"
        );
        let flags = environment(&cmd, TARGET_RUSTFLAGS).unwrap();
        assert!(flags.to_string_lossy().contains("-Ztls-model=emulated"));
        assert!(flags.to_string_lossy().contains("-Clinker-flavor=gcc"));
        assert!(flags.to_string_lossy().contains("-Clink-arg=-fuse-ld=bfd"));
        assert!(flags.to_string_lossy().contains("--sysroot=/managed/arm"));
        for (path, hash) in PACKAGES {
            assert!(path.starts_with("pool/main/") && !path.contains(".."));
            assert!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    #[ignore = "downloads pinned Ubuntu cross packages, requires network, ar and tar"]
    fn managed_gcc_download_smoke() {
        let root = ensure().unwrap();
        let output = root.join("arm64-link-smoke");
        let mut cmd = Command::new(root.join("usr/bin/aarch64-linux-gnu-gcc-13"));
        apply(&mut cmd, &root).unwrap();
        let result = cmd
            .args(["-x", "c", "-", "-Wl,--fix-cortex-a53-843419"])
            .arg(format!("--sysroot={}", root.display()))
            .arg("-o")
            .arg(&output)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        let mut child = result;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"#include <stdio.h>\nint main(void) { puts(\"ARM64\"); return 0; }\n")
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        validate_artifact(&std::fs::read(&output).unwrap()).unwrap();
        std::fs::remove_file(output).unwrap();
        eprintln!("managed ARM64 GCC ready at {}", root.display());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn default_mode_injects_target_tls_and_preserves_other_flags() {
        let mut cmd = Command::new("cargo");
        cmd.env_remove("RUSTFLAGS")
            .env("CARGO_ENCODED_RUSTFLAGS", "")
            .env(TARGET_RUSTFLAGS, "-C opt-level=2");
        let args = args(&["--target", TARGET, "-Zbuild-std=std,panic_abort"]);
        configure_flags(&mut cmd, &args).unwrap();
        assert_eq!(
            environment(&cmd, TARGET_RUSTFLAGS).unwrap(),
            "-C opt-level=2 -Ztls-model=emulated"
        );
        assert!(environment(&cmd, "RUSTFLAGS").is_none());
        assert!(environment(&cmd, "CARGO_ENCODED_RUSTFLAGS").is_none());
        configure_flags(&mut cmd, &args).unwrap();
        assert_eq!(
            environment(&cmd, TARGET_RUSTFLAGS).unwrap(),
            "-C opt-level=2 -Ztls-model=emulated"
        );
        assert!(configure_flags(&mut cmd, &args[..2]).is_err());
    }

    #[test]
    fn default_mode_rejects_missing_std_and_conflicting_flags() {
        let full = args(&["-Zbuild-std=std,panic_abort"]);
        assert!(validate(&[], "", "").is_err());
        assert!(validate(&full, "", "").is_ok());
        for flags in [
            "-Ztls-model=local-exec",
            "-Z tls-model=global-dynamic",
            "-Ztls-model=emulated -Ztls-model=local-exec",
        ] {
            assert!(validate(&full, flags, "").is_err());
        }
        assert!(validate(&full, "", "-Copt-level=2").is_err());
    }

    #[test]
    fn artifact_requires_aarch64_executable_segments() {
        let mut elf = vec![0u8; 120];
        elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        elf[16..18].copy_from_slice(&object::elf::ET_DYN.to_le_bytes());
        elf[18..20].copy_from_slice(&object::elf::EM_AARCH64.to_le_bytes());
        elf[20..24].copy_from_slice(&1u32.to_le_bytes());
        elf[32..40].copy_from_slice(&64u64.to_le_bytes());
        elf[52..54].copy_from_slice(&64u16.to_le_bytes());
        elf[54..56].copy_from_slice(&56u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());
        elf[64..68].copy_from_slice(&object::elf::PT_LOAD.to_le_bytes());
        elf[68..72].copy_from_slice(&object::elf::PF_X.to_le_bytes());
        assert!(validate_artifact(&elf).is_ok());
        let mut bad = elf.clone();
        bad[18..20].copy_from_slice(&object::elf::EM_X86_64.to_le_bytes());
        assert!(validate_artifact(&bad).is_err());
        let mut bad = elf.clone();
        bad[68..72].fill(0);
        assert!(validate_artifact(&bad).is_err());
        let mut bad = elf;
        bad[96..104].copy_from_slice(&1024u64.to_le_bytes());
        assert!(validate_artifact(&bad).is_err());
        assert!(validate_artifact(b"not an ELF").is_err());
    }

    #[test]
    fn official_tls_requires_both_rebuilt_libraries() {
        for values in [
            vec![],
            vec!["-Zbuild-std=std"],
            vec!["-Zbuild-std=panic_abort"],
            vec!["--", "-Zbuild-std=std,panic_abort"],
        ] {
            assert!(validate(&args(&values), "-Ztls-model=emulated", "").is_err());
        }
        for values in [
            vec!["-Zbuild-std=std,panic_abort"],
            vec!["-Z", "build-std=std,panic_abort,alloc"],
        ] {
            assert!(validate(&args(&values), "-Z tls-model=emulated", "").is_ok());
        }
        assert!(
            validate(
                &args(&["-Zbuild-std=std,panic_abort", "-Zbuild-std=std"]),
                "-Ztls-model=emulated",
                ""
            )
            .is_err()
        );
    }
    #[test]
    fn global_emulated_tls_is_rejected() {
        assert!(
            validate(
                &args(&["-Zbuild-std=std,panic_abort"]),
                "",
                "-Z\u{1f}tls-model=emulated"
            )
            .is_err()
        );
        assert!(validate(&[], "-C opt-level=2", "").is_err());
        assert!(supported(TARGET));
        assert!(!supported("x86_64-unknown-linux-gnu"));
        assert!(!supported("aarch64-unknown-linux-musl"));
    }
}
