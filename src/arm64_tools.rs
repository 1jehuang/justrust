//! Managed GNU/Linux ARM64 cross tools and the official Jcode TLS contract.
use anyhow::{Result, bail};
use std::ffi::OsString;
use std::process::Command;

pub const TARGET: &str = "aarch64-unknown-linux-gnu";
pub const TARGET_RUSTFLAGS: &str = "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS";

pub fn supported(target: &str) -> bool {
    cfg!(target_os = "linux") && target == TARGET
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

/// Reject an emulated-TLS build which would silently reuse stock std. Global
/// flags also affect host build scripts/proc macros and are never appropriate.
/// This pure check is exposed for callers with configuration-file rustflags.
pub fn validate(args: &[String], target_flags: &str, global_flags: &str) -> Result<()> {
    if emulated(global_flags) {
        bail!(
            "ARM64 emulated TLS must use {TARGET_RUSTFLAGS}, not global RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS (host build scripts must retain native TLS)"
        );
    }
    if !emulated(target_flags) {
        return Ok(());
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

/// Validate the target command, then install/apply the checksum-pinned Zig and
/// cargo-zigbuild tools. The caller dispatches `cargo zigbuild` and selects the
/// nightly compiler. This helper never changes the requested Rust target.
pub fn configure(cmd: &mut Command, args: &[String]) -> Result<()> {
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
    if emulated(&target_flags.to_string_lossy()) && !global_flags.trim().is_empty() {
        bail!(
            "global RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS override target rustflags; remove them for the official ARM64 emulated TLS build"
        );
    }
    crate::release_tools::apply(cmd, crate::release_tools::ensure()?)
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

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
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
        assert!(validate(&[], "-C opt-level=2", "").is_ok());
        assert!(!supported("x86_64-unknown-linux-gnu"));
        assert!(!supported("aarch64-unknown-linux-musl"));
    }
}
