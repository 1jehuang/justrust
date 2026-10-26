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
/// LLVM aliases are provisioned from installed tools and validated without SDK
/// access. The caller must separately provision the Rust target stdlib.
pub fn prepare(target: &str) -> Result<PathBuf> {
    validate_target(target)?;
    require_license(std::env::var_os(ACCEPT_LICENSE_ENV).as_deref())?;
    if !cfg!(target_os = "linux") {
        bail!("managed Windows MSVC cross tools currently require a Linux host");
    }
    let llvm = prepare_llvm()?;
    let arch = std::env::consts::ARCH;
    let hash = archive_hash(arch)?;
    let parent = crate::toolchain::root()?.join("msvc-tools");
    let root = parent.join(format!(
        "{REVISION}-{arch}-{}",
        llvm.file_name()
            .context("missing LLVM identity")?
            .to_string_lossy()
    ));
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
        link_tool(&llvm, &stage.join("llvm"))?;
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
    let mut paths = vec![root.join("bin"), root.join("llvm/bin")];
    if let Some(existing) = existing {
        paths.extend(std::env::split_paths(&existing));
    }
    cmd.env("PATH", std::env::join_paths(paths)?);
    Ok(())
}

#[cfg(unix)]
fn link_tool(source: &Path, dest: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, dest).context("creating LLVM compatibility alias")
}

#[cfg(not(unix))]
fn link_tool(_source: &Path, _dest: &Path) -> Result<()> {
    bail!("LLVM compatibility aliases require a Unix build host")
}

fn llvm_output(cmd: &mut Command) -> Result<std::process::Output> {
    let output = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running LLVM prerequisite probe: {cmd:?}"))?;
    if !output.status.success() {
        bail!(
            "LLVM prerequisite probe {cmd:?} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output)
}

fn find_llvm(names: &[&str], marker: &str) -> Result<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    // Distribution LLVM packages keep unsuffixed binaries here even when PATH
    // exposes only clang-N. Never substitute GNU ar or Microsoft's tools.
    for version in (18..=22).rev() {
        dirs.push(PathBuf::from(format!("/usr/lib/llvm-{version}/bin")));
    }
    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if !candidate.is_file() {
                continue;
            }
            let Ok(output) = Command::new(&candidate)
                .arg("--version")
                .stdin(Stdio::null())
                .output()
            else {
                continue;
            };
            let version = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if output.status.success() && version.contains(marker) {
                return candidate
                    .canonicalize()
                    .context("resolving LLVM executable");
            }
        }
    }
    bail!(
        "missing genuine LLVM prerequisite {}: provision LLVM clang, llvm-ar and lld-link; GNU ar and unrelated linker drivers are not substitutes",
        names[0]
    )
}

/// Prepare LLVM tools without SDK access, license acceptance or target
/// execution. Missing archive tools require explicit LLVM-only download opt-in.
/// Aliases are content-addressed and tested for
/// both MSVC architectures before publication. The caller may run this preflight
/// without consenting to Microsoft's separate SDK terms.
pub fn prepare_llvm() -> Result<PathBuf> {
    prepare_llvm_with_install(std::env::var("JUSTRUST_INSTALL_LLVM_TOOLS").as_deref() == Ok("1"))
}

/// Explicitly authorize only the pinned LLVM package download. This flag is
/// unrelated to Microsoft's SDK license and cannot authorize any SDK operation.
pub fn prepare_llvm_with_install(install: bool) -> Result<PathBuf> {
    if !cfg!(target_os = "linux") {
        bail!("managed LLVM prerequisite aliases currently require Linux");
    }
    let clang = find_llvm(
        &[
            "clang", "clang-22", "clang-21", "clang-20", "clang-19", "clang-18",
        ],
        "clang version",
    )?;
    let ar = match find_llvm(
        &[
            "llvm-ar",
            "llvm-ar-22",
            "llvm-ar-21",
            "llvm-ar-20",
            "llvm-ar-19",
            "llvm-ar-18",
        ],
        "LLVM",
    ) {
        Ok(path) => path,
        Err(_) => install_llvm_ar(install)?,
    };
    let lld = find_llvm(
        &[
            "lld-link",
            "lld-link-22",
            "lld-link-21",
            "lld-link-20",
            "lld-link-19",
            "lld-link-18",
        ],
        "LLD",
    )?;
    let parent = crate::toolchain::root()?.join("llvm-tools");
    std::fs::create_dir_all(&parent)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join(".install.lock"))?;
    lock.lock().context("locking LLVM alias preparation")?;
    let stage = parent.join(format!(".staging-{}", std::process::id()));
    std::fs::create_dir(&stage).context("creating LLVM alias staging directory")?;
    let result = (|| {
        let mut manifest = String::from("LLVM-aliases-v1\n");
        for path in [&clang, &ar, &lld] {
            manifest.push_str(&format!(
                "{} {}\n",
                crate::toolchain::sha256(path)?,
                path.display()
            ));
        }
        let manifest_path = stage.join(".identity");
        std::fs::write(&manifest_path, &manifest)?;
        let identity = crate::toolchain::sha256(&manifest_path)?;
        let root = parent.join(format!("llvm-v1-{identity}"));
        if std::fs::read_to_string(root.join(".complete")).is_ok_and(|s| s == manifest)
            && [
                "clang",
                "clang-cl",
                "llvm-ar",
                "llvm-lib",
                "llvm-dlltool",
                "lld-link",
            ]
            .iter()
            .all(|name| root.join("bin").join(name).is_file())
        {
            return Ok(root);
        }
        if root.exists() {
            bail!("incomplete LLVM alias installation: {}", root.display());
        }
        std::fs::create_dir(stage.join("bin"))?;
        for (name, source) in [
            ("clang", &clang),
            ("clang-cl", &clang),
            ("llvm-ar", &ar),
            ("llvm-lib", &ar),
            ("llvm-dlltool", &ar),
            ("lld-link", &lld),
        ] {
            link_tool(source, &stage.join("bin").join(name))?;
        }
        probe_llvm(&stage)?;
        std::fs::write(stage.join(".complete"), manifest)?;
        std::fs::rename(&stage, &root).context("publishing probed LLVM aliases")?;
        Ok(root)
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

fn llvm_package(arch: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match arch {
        "x86_64" => Ok((
            "https://archive.ubuntu.com/ubuntu",
            "amd64",
            "24c6a9d7b763c2b9ce3199012eca7366cada3e41978ecee022e9d44a110a9c6d",
        )),
        "aarch64" => Ok((
            "https://ports.ubuntu.com/ubuntu-ports",
            "arm64",
            "e9dd1b2ae8187de6b138bcd8a8ab3bf04fb7639c68e90e30a1b86b266510e7e6",
        )),
        _ => bail!("no pinned LLVM package for host {arch}"),
    }
}

fn install_llvm_ar(install: bool) -> Result<PathBuf> {
    let (origin, arch, hash) = llvm_package(std::env::consts::ARCH)?;
    let parent = crate::toolchain::root()?.join("llvm-packages");
    let root = parent.join(format!("llvm-18.1.3-1-{arch}-{hash}"));
    let binary = root.join("usr/lib/llvm-18/bin/llvm-ar");
    let ready = || {
        std::fs::read_to_string(root.join(".complete")).is_ok_and(|s| s == hash) && binary.is_file()
    };
    if ready() {
        return Ok(binary);
    }
    if !install {
        bail!(
            "genuine llvm-ar is missing; explicitly set JUSTRUST_INSTALL_LLVM_TOOLS=1 to extract the checksum-pinned Ubuntu LLVM18 package without root. This does not accept Microsoft's SDK license"
        );
    }
    if matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_AUTO_INSTALL").as_deref(),
        Ok("0" | "off" | "false")
    ) {
        bail!("LLVM tools are missing and JUSTRUST_TOOLCHAIN_AUTO_INSTALL is disabled");
    }
    std::fs::create_dir_all(&parent)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join(".install.lock"))?;
    lock.lock().context("locking LLVM package installation")?;
    if ready() {
        return Ok(binary);
    }
    if root.exists() {
        bail!("incomplete LLVM package at {}", root.display());
    }
    let stage = parent.join(format!(".staging-{}", std::process::id()));
    std::fs::create_dir(&stage)?;
    let result = (|| {
        let archive = stage.join("llvm.deb");
        crate::toolchain::curl(
            &format!("{origin}/pool/universe/l/llvm-toolchain-18/llvm-18_18.1.3-1_{arch}.deb"),
            &archive,
        )?;
        if crate::toolchain::sha256(&archive)? != hash {
            bail!("LLVM package checksum mismatch");
        }
        llvm_output(
            Command::new("dpkg-deb")
                .arg("--extract")
                .arg(&archive)
                .arg(&stage),
        )?;
        let ar = stage.join("usr/lib/llvm-18/bin/llvm-ar");
        // The host supplies libLLVM18 and its dependencies. Run the verified
        // binary before publishing, so loader/version errors fail closed.
        let output = llvm_output(Command::new(&ar).arg("--version"))
            .context("pinned llvm-ar requires compatible host libLLVM18 and glibc >= 2.38")?;
        if !String::from_utf8_lossy(&output.stdout).contains("LLVM version 18.1.3") {
            bail!("pinned llvm-ar reports an unexpected version");
        }
        std::fs::remove_file(archive)?;
        std::fs::write(stage.join(".complete"), hash)?;
        std::fs::rename(&stage, &root)?;
        Ok(binary)
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(stage);
    }
    result
}

fn probe_llvm(root: &Path) -> Result<()> {
    let work = root.join("probe");
    std::fs::create_dir(&work)?;
    let result = (|| {
        let bin = root.join("bin");
        let answer = work.join("answer.c");
        let entry = work.join("entry.c");
        std::fs::write(&answer, "int answer(void) { return 42; }\n")?;
        std::fs::write(
            &entry,
            "extern int answer(void); int entry(void) { return answer(); }\n",
        )?;
        for (target, machine, pe_machine) in [
            ("x86_64-pc-windows-msvc", "x64", 0x8664_u16),
            ("aarch64-pc-windows-msvc", "arm64", 0xaa64_u16),
        ] {
            let object = work.join(format!("{machine}-answer.obj"));
            let main = work.join(format!("{machine}-entry.obj"));
            let library = work.join(format!("{machine}.lib"));
            let executable = work.join(format!("{machine}.exe"));
            for (source, dest) in [(&answer, &object), (&entry, &main)] {
                llvm_output(
                    Command::new(bin.join("clang-cl"))
                        .args(["/nologo", "/c", "/GS-", "/Zl"])
                        .arg(format!("--target={target}"))
                        .arg(source)
                        .arg(format!("/Fo{}", dest.display())),
                )?;
                let bytes = std::fs::read(dest)?;
                if bytes.get(..2) != Some(pe_machine.to_le_bytes().as_slice()) {
                    bail!("LLVM probe generated incorrect COFF machine for {target}");
                }
            }
            llvm_output(
                Command::new(bin.join("llvm-lib"))
                    .arg(format!("/out:{}", library.display()))
                    .arg(&object),
            )?;
            let table = llvm_output(Command::new(bin.join("llvm-ar")).arg("t").arg(&library))?;
            if !String::from_utf8_lossy(&table.stdout).contains("answer.obj") {
                bail!("LLVM librarian probe produced no expected archive member");
            }
            let definition = work.join(format!("{machine}.def"));
            let imports = work.join(format!("{machine}-imports.lib"));
            std::fs::write(&definition, "LIBRARY probe.dll\nEXPORTS\nanswer\n")?;
            llvm_output(
                Command::new(bin.join("llvm-dlltool"))
                    .arg("-m")
                    .arg(if machine == "x64" {
                        "i386:x86-64"
                    } else {
                        "arm64"
                    })
                    .arg("-d")
                    .arg(&definition)
                    .arg("-l")
                    .arg(&imports),
            )?;
            llvm_output(Command::new(bin.join("llvm-ar")).arg("t").arg(&imports))?;
            llvm_output(
                Command::new(bin.join("lld-link"))
                    .args(["/entry:entry", "/subsystem:console", "/nodefaultlib"])
                    .arg(format!("/machine:{machine}"))
                    .arg(format!("/out:{}", executable.display()))
                    .arg(&main)
                    .arg(&library),
            )?;
            let bytes = std::fs::read(executable)?;
            let offset = bytes
                .get(0x3c..0x40)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
                .context("LLVM linker probe emitted truncated PE header")?;
            if bytes.get(..2) != Some(b"MZ")
                || bytes.get(offset..offset + 4) != Some(b"PE\0\0")
                || bytes.get(offset + 4..offset + 6) != Some(pe_machine.to_le_bytes().as_slice())
            {
                bail!("LLVM linker probe emitted incorrect PE machine for {target}");
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(work);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "explicitly downloads pinned LLVM tools if missing; no Microsoft SDK"]
    fn installed_llvm_compiles_archives_and_links_both_targets_without_sdk() {
        let root = prepare_llvm_with_install(true).unwrap();
        assert!(root.join("bin/clang-cl").is_file());
        assert!(root.join("bin/llvm-lib").is_file());
        assert_eq!(prepare_llvm().unwrap(), root);
    }

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
        for arch in ["x86_64", "aarch64"] {
            let (origin, package_arch, hash) = llvm_package(arch).unwrap();
            assert!(origin.starts_with("https://"));
            assert!(matches!(package_arch, "amd64" | "arm64"));
            assert_eq!(hash.len(), 64);
            assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert!(llvm_package("../x86_64").is_err());
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
                (
                    "PATH",
                    "/account/tools/bin:/account/tools/llvm/bin:/pinned/rust/bin:/usr/bin",
                ),
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
