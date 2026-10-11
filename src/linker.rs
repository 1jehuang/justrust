//! wild as the linker for local crates, fail open.
//!
//! The rustc shim adds `-B ~/.justrust/bin/ld` to the link of local units
//! (the test binaries and bins relinked after every edit). gcc and clang
//! look up `ld`, `ld.mold`, `ld.lld`, ... in `-B` directories before their
//! own, whatever `-fuse-ld=` asks for, so every driver setup (default `cc`,
//! `clang -fuse-ld=mold` wrappers, rustc's self-contained lld) ends up here.
//! Those names are symlinks to justrust, which runs wild. If wild fails,
//! the linker that was asked for runs instead with the same arguments, so a
//! wild bug costs one extra link, never a failed build.
//!
//! Untouched: dependency units (they go through the depcache, and their
//! cache keys and stored artifacts stay linker-independent), and link args
//! are untracked by rustc's incremental cache and invisible to cargo's
//! fingerprints, so switching wild on or off never rebuilds anything.
//!
//! `JUSTRUST_LINKER=off` (or `0`) disables it. Install wild with
//! `justrust toolchain wild`.

use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const WILD_VERSION: &str = "0.10.0";
const WILD_SHA256: &str = "641265506a7c06cfb03181b8916ab663ec8407855db6d4db7f8450667d105283";
/// Linker names gcc and clang look up in `-B` directories.
const LD_NAMES: &[&str] = &["ld", "ld.bfd", "ld.gold", "ld.lld", "ld.mold", "ld.wild"];

fn enabled() -> bool {
    !matches!(
        std::env::var("JUSTRUST_LINKER").as_deref(),
        Ok("off") | Ok("0") | Ok("false") | Ok("no") | Ok("default")
    )
}

/// `~/.justrust/toolchains/linkers/wild-<version>/wild`.
pub fn wild_path() -> Result<PathBuf> {
    Ok(crate::paths::home()?
        .join("toolchains/linkers")
        .join(format!("wild-{WILD_VERSION}"))
        .join("wild"))
}

fn ld_dir() -> Result<PathBuf> {
    Ok(crate::paths::home()?.join("bin/ld"))
}

fn installed() -> Option<PathBuf> {
    let p = wild_path().ok()?;
    is_executable(&p).then_some(p)
}

/// The pinned wild is installed here.
pub fn is_installed() -> bool {
    installed().is_some()
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Extra rustc args for a unit: `-B <ld dir>` when wild should link it.
/// Empty (link as configured) when disabled, not installed, the unit does
/// not link, or the `-B` dir cannot be set up.
pub fn rustc_args(args: &[OsString], local: bool, nightly: bool) -> Vec<OsString> {
    if !local || !enabled() || !links(args) || !host_like_target(args) || installed().is_none() {
        return Vec::new();
    }
    let Ok(dir) = ensure_ld_dir() else {
        return Vec::new();
    };
    let b = format!("-B{}", dir.display());
    // rustc puts `-C link-arg`s after its own. When it links with its
    // self-contained lld (no `-C linker`, nightly), its `-B .../gcc-ld`
    // comes first and wins; pre-link args go before it. Both are untracked.
    let explicit_linker = args.iter().any(|a| {
        let a = a.to_string_lossy();
        a.starts_with("linker=") || a.starts_with("-Clinker=")
    });
    if nightly && !explicit_linker {
        vec![OsString::from(format!("-Zpre-link-arg={b}"))]
    } else {
        vec![
            OsString::from("-C"),
            OsString::from(format!("link-arg={b}")),
        ]
    }
}

/// wild links Linux ELF only, and `-B` is a gcc/clang driver flag: rust-lld
/// in wasm or other flavors rejects it outright (`unknown argument: -B`).
/// No `--target` means the host, which is Linux here.
fn host_like_target(args: &[OsString]) -> bool {
    let s: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let target = s.iter().enumerate().find_map(|(i, a)| {
        if a == "--target" {
            s.get(i + 1).cloned()
        } else {
            a.strip_prefix("--target=").map(str::to_owned)
        }
    });
    target.is_none_or(|t| t.contains("-linux"))
}

/// Does this rustc invocation run a linker? (`--emit` with `link` and a
/// crate type that produces an executable or shared object.)
fn links(args: &[OsString]) -> bool {
    let s: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let emit_link = s.iter().any(|a| {
        a.strip_prefix("--emit=")
            .is_some_and(|v| v.split(',').any(|e| e == "link" || e.starts_with("link=")))
    });
    if !emit_link {
        return false;
    }
    if s.iter().any(|a| a == "--test") {
        return true;
    }
    let mut types = Vec::new();
    for (i, a) in s.iter().enumerate() {
        if a == "--crate-type" {
            if let Some(t) = s.get(i + 1) {
                types.extend(t.split(',').map(str::to_owned));
            }
        } else if let Some(t) = a.strip_prefix("--crate-type=") {
            types.extend(t.split(',').map(str::to_owned));
        }
    }
    // No --crate-type: rustc infers bin for a main.rs; cargo always passes it.
    types.is_empty()
        || types
            .iter()
            .any(|t| matches!(t.as_str(), "bin" | "dylib" | "cdylib" | "proc-macro"))
}

fn ensure_ld_dir() -> Result<PathBuf> {
    let dir = ld_dir()?;
    let exe = std::env::current_exe()?.canonicalize()?;
    std::fs::create_dir_all(&dir)?;
    for name in LD_NAMES {
        crate::paths::replace_symlink(&exe, &dir.join(name))?;
    }
    Ok(dir)
}

/// Started as one of the `ld*` names? justrust is never installed under
/// those names other than in the `-B` dir (a driver may pass a bare name).
pub fn invoked_as_ld(argv0: &OsStr) -> bool {
    Path::new(argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| LD_NAMES.contains(&n))
}

/// The `ld` entry point: link with wild, fall back to the requested linker.
pub fn ld_main() -> ! {
    let mut argv = std::env::args_os();
    let argv0 = argv.next().unwrap_or_default();
    let args: Vec<OsString> = argv.collect();
    let name = Path::new(&argv0)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ld".into());
    if let Some(wild) = installed().filter(|_| enabled()) {
        match Command::new(&wild).args(&args).output() {
            Ok(out) if out.status.success() => {
                use std::io::Write;
                let _ = std::io::stdout().write_all(&out.stdout);
                let _ = std::io::stderr().write_all(&out.stderr);
                std::process::exit(0);
            }
            Ok(out) => log_fallback(&name, &String::from_utf8_lossy(&out.stderr)),
            Err(e) => log_fallback(&name, &e.to_string()),
        }
    }
    let real = fallback(&name);
    let err = Command::new(&real).args(&args).exec();
    eprintln!("justrust: could not run linker {}: {err}", real.display());
    std::process::exit(127);
}

/// The linker the compiler driver asked for, outside the `-B` dir.
fn fallback(name: &str) -> PathBuf {
    let dir = ld_dir().ok().and_then(|d| d.canonicalize().ok());
    let skip = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_default();
    let candidates: &[&str] = match name {
        "ld.wild" | "ld" => &["ld"],
        "ld.lld" => &["ld.lld", "rust-lld", "ld"],
        other => &[other, "ld"],
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    for c in candidates {
        for d in std::env::split_paths(&path) {
            if dir.is_some() && d.canonicalize().ok() == dir {
                continue;
            }
            let p = d.join(c);
            if is_executable(&p)
                && !crate::paths::is_justrust(&p)
                && p.canonicalize().ok() != Some(skip.clone())
            {
                return p;
            }
        }
    }
    PathBuf::from("/usr/bin/ld")
}

fn log_fallback(name: &str, stderr: &str) {
    let line = serde_json::json!({
        "time": crate::paths::now(),
        "wild": WILD_VERSION,
        "requested": name,
        "cwd": std::env::current_dir().unwrap_or_default(),
        "stderr": stderr.chars().take(2000).collect::<String>(),
    });
    if let Ok(home) = crate::paths::home() {
        let _ = crate::paths::append_line(&home.join("linker-fallbacks.jsonl"), &line.to_string());
    }
}

/// `justrust toolchain wild`: download the pinned wild release.
pub fn install_command() -> Result<()> {
    let dest = wild_path()?;
    if is_executable(&dest) {
        println!(
            "wild {WILD_VERSION} already installed at {}",
            dest.display()
        );
        return Ok(());
    }
    if std::env::consts::ARCH != "x86_64" || std::env::consts::OS != "linux" {
        bail!("justrust ships wild for x86_64 Linux only");
    }
    let dir = dest.parent().context("wild path has no parent")?;
    let work = dir.with_extension("tmp");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let tarball = work.join("wild.tar.gz");
    let url = format!(
        "https://github.com/wild-linker/wild/releases/download/{WILD_VERSION}/wild-linker-{WILD_VERSION}-x86_64-unknown-linux-gnu.tar.gz"
    );
    crate::toolchain::curl(&url, &tarball)?;
    let got = crate::toolchain::sha256(&tarball)?;
    if got != WILD_SHA256 {
        bail!("checksum mismatch for {url}: {got} != {WILD_SHA256}");
    }
    let st = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&work)
        .arg("--strip-components=1")
        .status()
        .context("running tar")?;
    if !st.success() || !is_executable(&work.join("wild")) {
        bail!("unpacking {url} failed");
    }
    let _ = std::fs::remove_file(&tarball);
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&work, dir)?;
    ensure_ld_dir()?;
    println!(
        "installed wild {WILD_VERSION} at {}: local crates now link with it (JUSTRUST_LINKER=off to disable)",
        dest.display()
    );
    Ok(())
}

/// One line for `justrust toolchain status`.
pub fn status_line() -> String {
    match installed() {
        Some(p) if enabled() => format!(
            "linker  wild {WILD_VERSION} for local crates ({})",
            p.display()
        ),
        Some(_) => "linker  wild installed, off (JUSTRUST_LINKER)".into(),
        None => "linker  as configured (`justrust toolchain wild` installs wild for local crates)"
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn detects_linking_units() {
        assert!(links(&a(&["--crate-type", "bin", "--emit=dep-info,link"])));
        assert!(links(&a(&["--test", "--emit=dep-info,link"])));
        assert!(links(&a(&[
            "--crate-type",
            "lib",
            "--crate-type",
            "cdylib",
            "--emit=dep-info,metadata,link"
        ])));
        assert!(links(&a(&[
            "--crate-type",
            "proc-macro",
            "--emit=dep-info,link"
        ])));
        assert!(!links(&a(&[
            "--crate-type",
            "lib",
            "--emit=dep-info,metadata,link"
        ])));
        assert!(!links(&a(&[
            "--crate-type",
            "bin",
            "--emit=dep-info,metadata"
        ])));
        assert!(!links(&a(&["--test", "--emit=metadata"])));
    }

    #[test]
    fn only_local_linking_units_get_args() {
        let bin = a(&["--crate-type", "bin", "--emit=dep-info,link"]);
        assert!(rustc_args(&bin, false, false).is_empty());
        let lib = a(&["--crate-type", "lib", "--emit=dep-info,metadata,link"]);
        assert!(rustc_args(&lib, true, false).is_empty());
        let wasm = a(&[
            "--crate-type",
            "bin",
            "--emit=dep-info,link",
            "--target",
            "wasm32-unknown-unknown",
        ]);
        assert!(rustc_args(&wasm, true, false).is_empty());
    }

    #[test]
    fn only_linux_targets_use_wild() {
        assert!(host_like_target(&a(&["--crate-type", "bin"])));
        assert!(host_like_target(&a(&[
            "--target",
            "x86_64-unknown-linux-gnu"
        ])));
        assert!(host_like_target(&a(&[
            "--target=aarch64-unknown-linux-musl"
        ])));
        assert!(!host_like_target(&a(&[
            "--target",
            "wasm32-unknown-unknown"
        ])));
        assert!(!host_like_target(&a(&["--target=x86_64-pc-windows-msvc"])));
        assert!(!host_like_target(&a(&["--target", "aarch64-apple-darwin"])));
    }
}
