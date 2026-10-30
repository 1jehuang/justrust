//! Experimental justrust entry for the imported Blaze native engine.
//! Blaze origin and licenses remain in the resident source tree. This is not an
//! independent compiler implementation and does not launch the upstream wrapper.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, bail};

pub(crate) fn is_engine() -> bool {
    // Not argv[0] or current_exe: internal invocations can be named `rustc`.
    env!("CARGO_BIN_NAME") == "jr-engine"
}

pub(crate) fn version() -> String {
    format!(
        "jr-engine experimental {} (Blaze-derived origin {}; build {})",
        env!("CARGO_PKG_VERSION"),
        option_env!("JR_ENGINE_ORIGIN").unwrap_or("5d4f78f7577cf7170993b74f735099dd03f5c6ca"),
        option_env!("JR_ENGINE_BUILD_ID").unwrap_or("unidentified-local-build"),
    )
}

/// Reader-only entry points do not open stores or require a private home.
pub(crate) fn reader_command() -> bool {
    if crate::as_rustc::is_me()
        || std::env::var_os("RA_JIT_WRAPS_RUSTC").is_some()
        || std::env::var_os("BLAZE_WRAPS_RUSTC").is_some()
    {
        return false;
    }
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => println!("{}", version()),
        None | Some("--help" | "-h" | "help") => println!(
            "jr-engine: experimental Blaze-derived native engine\n\
             usage: jr-engine build|test|check|run [cargo flags]\n\
             Requires JR_ENGINE_HOME: an existing absolute, canonical, owner-only directory.\n\
             Internal worker commands are preserved. Not a shipping justrust default.\n\
             See docs/resident-engine-entry.md for namespace and compatibility limits."
        ),
        _ => return false,
    }
    true
}

fn identified_build(build: &str) -> bool {
    build.strip_prefix("resident-v1-").is_some_and(|digest| {
        digest.len() == 64
            && digest.bytes().all(|it| it.is_ascii_digit() || (b'a'..=b'f').contains(&it))
    })
}

fn absolute_home(path: &Path) -> anyhow::Result<()> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|it| matches!(it, Component::ParentDir | Component::CurDir))
    {
        bail!("JR_ENGINE_HOME must be an absolute canonical private directory, not a root");
    }
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path, create: bool) -> anyhow::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
    if create {
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    let metadata = std::fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no arguments or memory preconditions.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o077 != 0
        || std::fs::canonicalize(path)? != path
    {
        bail!(
            "{} must be canonical, owned by this user and inaccessible to group/other",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn private_directory(_path: &Path, _create: bool) -> anyhow::Result<()> {
    bail!("jr-engine private namespace validation is currently supported only on Unix")
}

/// Must run before any cache, worker, sandbox, or thread initialization.
pub(crate) fn initialize() -> anyhow::Result<()> {
    let build = option_env!("JR_ENGINE_BUILD_ID").unwrap_or("");
    if !identified_build(build) {
        bail!(
            "jr-engine refuses engine operations without a resident-v1-<64 lowercase hex> build identity"
        );
    }
    let home = PathBuf::from(std::env::var_os("JR_ENGINE_HOME").context(
        "jr-engine requires explicit JR_ENGINE_HOME, no upstream or user-home fallback is allowed",
    )?);
    absolute_home(&home)?;
    private_directory(&home, false).context("unsafe JR_ENGINE_HOME")?;
    let namespace = home.join("resident-v1");
    private_directory(&namespace, true)?;
    for name in ["runtime", "store", "functions"] {
        private_directory(&namespace.join(name), true)?;
    }
    let store = namespace.join("store").join(build);
    private_directory(&store, true)?;
    // Do not expose upstream mutation/remote management as an experimental public API.
    let command = std::env::args().nth(1);
    let cargo_command =
        (command.as_deref() == Some("cargo")).then(|| std::env::args().nth(2)).flatten();
    if [command.as_deref(), cargo_command.as_deref()]
        .into_iter()
        .any(|it| matches!(it, Some("enable" | "disable" | "cache" | "closure" | "std")))
    {
        bail!(
            "jr-engine does not support upstream install, shared cache, or closure management commands"
        );
    }
    // SAFETY: called first in main, before sandbox forks or any threads. Set both
    // spellings because take_blaze_names translates BLAZE_* over RA_JIT_* later.
    unsafe {
        for (key, path) in [
            ("HOME", namespace.join("runtime")),
            ("CACHE", store),
            ("FN_CACHE_DIR", namespace.join("functions")),
        ] {
            std::env::set_var(format!("RA_JIT_{key}"), &path);
            std::env::set_var(format!("BLAZE_{key}"), &path);
        }
        // BLAZE_HOME owns toolchains as well as daemon state. Keep both in
        // runtime: take_blaze_names requires BLAZE_HOME and RA_JIT_HOME agree.
        std::env::set_var("BLAZE_REMOTE", "off");
        std::env::set_var("RA_JIT_REMOTE", "off");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn homes_must_be_explicit_absolute_non_root_paths() {
        use std::path::Path;
        for path in ["", "relative", "/", "/private/../shared"] {
            assert!(super::absolute_home(Path::new(path)).is_err(), "{path}");
        }
        assert!(super::absolute_home(Path::new("/private/engine")).is_ok());
    }

    #[test]
    fn build_identity_requires_a_full_lowercase_digest() {
        assert!(super::identified_build(&format!("resident-v1-{}", "a".repeat(64))));
        for invalid in ["resident-v1-unidentified", "resident-v1-abc", "other-resident-v1-abc"] {
            assert!(!super::identified_build(invalid));
        }
        assert!(!super::identified_build(&format!("resident-v1-{}", "G".repeat(64))));
    }

    #[test]
    fn identity_never_claims_to_be_upstream() {
        let version = super::version();
        assert!(version.starts_with("jr-engine experimental "));
        assert!(version.contains("Blaze-derived origin "));
    }
}
