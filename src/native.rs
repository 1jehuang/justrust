//! Explicit, transitional entry into the owned resident compiler.
//!
//! Cargo remains the default. Do not retry a failed native build with Cargo:
//! doing so would hide native diagnostics and can run a test/program twice.
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

const HELP: &str = "justrust native <build|test|check|run|status|stop|toolchain> [arguments...]

Experimental local Blaze-derived resident compiler, not the default Cargo path.
Arguments, including the test runner's -- separator, reach the engine unchanged.
Native compilation is not yet a general replacement for rustc. Unsupported code
fails visibly. There is no automatic Cargo retry after native execution.

Install the owned engine with toolchain/resident/build-jr.sh TOOLCHAIN_DIRECTORY,
or set JUSTRUST_NATIVE_ENGINE to an absolute path to a trusted engine binary.
The default engine is jr-native beside the justrust executable.
Native state is private under JUSTRUST_HOME/native (default ~/.justrust/native).
Set JR_ENGINE_HOME to an existing short owner-only directory to override it.
See docs/stable-native-engine.md for architecture, compatibility and measurements.
Use plain justrust test/build/check/run for the existing Cargo/hosted path.
";

fn engine_path(override_path: Option<OsString>, executable: &Path) -> anyhow::Result<PathBuf> {
    let path = match override_path {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                bail!("JUSTRUST_NATIVE_ENGINE must be an absolute path to a trusted native engine");
            }
            path
        }
        None => executable
            .parent()
            .context("justrust executable has no parent directory")?
            .join("jr-native"),
    };
    let resolved = path.canonicalize().with_context(|| {
        format!(
            "native engine not available at {}. Build the owned engine with toolchain/resident/build-jr.sh TOOLCHAIN_DIRECTORY, or set JUSTRUST_NATIVE_ENGINE to its absolute path. The default justrust commands still use Cargo",
            path.display()
        )
    })?;
    if resolved
        == executable
            .canonicalize()
            .context("resolve justrust executable")?
    {
        bail!("native engine resolves to justrust itself, refusing recursive execution");
    }
    if !resolved.is_file() {
        bail!(
            "native engine is not a regular file: {}",
            resolved.display()
        );
    }
    Ok(resolved)
}

fn engine_args(args: &[OsString]) -> anyhow::Result<Vec<OsString>> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        bail!("native requires a command, see justrust native --help");
    };
    if !matches!(
        command,
        "build" | "test" | "check" | "run" | "status" | "stop" | "toolchain"
    ) {
        bail!(
            "unsupported native command {command:?}. Use justrust {command} for the Cargo path, or justrust native --help"
        );
    }
    // The owned engine currently parses UTF-8 argv. Refuse explicitly rather
    // than allowing std::env::args in that process to panic or using lossy text.
    if args.iter().any(|arg| arg.to_str().is_none()) {
        bail!(
            "the experimental native engine requires UTF-8 arguments; use the Cargo path for non-UTF-8 paths"
        );
    }
    let mut forwarded = vec![OsString::from("cargo")];
    forwarded.extend_from_slice(args);
    Ok(forwarded)
}

pub(crate) fn main(args: Vec<OsString>) -> anyhow::Result<()> {
    if args.is_empty() || (args.len() == 1 && matches!(args[0].to_str(), Some("--help" | "-h"))) {
        print!("{HELP}");
        return Ok(());
    }
    let forwarded = engine_args(&args)?;
    if !cfg!(unix) {
        bail!(
            "the experimental stable native integration currently requires Unix; the normal Cargo path is unchanged"
        );
    }
    let engine = engine_path(
        std::env::var_os("JUSTRUST_NATIVE_ENGINE"),
        &std::env::current_exe().context("locate justrust executable")?,
    )?;
    let home = match std::env::var_os("JR_ENGINE_HOME") {
        Some(home) => PathBuf::from(home),
        None => default_home()?,
    };
    // Inherit stdin/stdout/stderr and replace the client: statuses and signals
    // are the engine's, without a wrapper process or post-execution fallback.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let error = std::process::Command::new(&engine)
            .args(forwarded)
            .env("JR_ENGINE_HOME", home)
            .exec();
        Err(error).with_context(|| format!("execute native engine {}", engine.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (forwarded, engine, home);
        unreachable!()
    }
}

/// Only the default home is created here. An explicit override is left to the
/// engine's strict canonical-path, ownership and mode validation. Never chmod
/// or repurpose an existing directory on the user's behalf.
fn default_home() -> anyhow::Result<PathBuf> {
    let parent = crate::paths::home()?;
    std::fs::create_dir_all(&parent).context("create justrust state directory")?;
    let home = parent.canonicalize()?.join("native");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&home) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("create private native state directory"),
        }
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_each_supported_command_and_exact_test_separator() {
        for command in [
            "build",
            "test",
            "check",
            "run",
            "status",
            "stop",
            "toolchain",
        ] {
            let args: Vec<OsString> = [command, "--lib", "--", "--skip", "a b"]
                .map(Into::into)
                .into();
            let forwarded = engine_args(&args).unwrap();
            assert_eq!(forwarded[0], std::ffi::OsStr::new("cargo"));
            assert_eq!(&forwarded[1..], args);
        }
    }

    #[test]
    fn rejects_unsupported_commands_without_falling_back() {
        for command in ["clippy", "cargo", "enable", "--remote", "install"] {
            assert!(engine_args(&[command.into()]).is_err());
        }
        assert!(engine_args(&[]).is_err());
    }

    #[test]
    fn rejects_relative_engine_override_and_self_recursion() {
        let executable = std::env::current_exe().unwrap();
        assert!(engine_path(Some("jr-native".into()), &executable).is_err());
        assert!(engine_path(Some(executable.clone().into_os_string()), &executable).is_err());
    }

    #[test]
    fn missing_engine_explains_install_and_unchanged_cargo_path() {
        let executable = std::env::current_exe().unwrap();
        let missing = executable.join("nonexistent-native-engine");
        let error = engine_path(Some(missing.into_os_string()), &executable).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("build-jr.sh"));
        assert!(message.contains("still use Cargo"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_non_utf8_instead_of_lossy_arguments() {
        use std::os::unix::ffi::OsStringExt as _;
        assert!(engine_args(&["test".into(), OsString::from_vec(vec![255])]).is_err());
    }
}
