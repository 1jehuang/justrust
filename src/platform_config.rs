//! User-only platform defaults. Signing references are metadata, not credentials.
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::{fs::File, io::Write};

#[derive(Subcommand)]
pub enum Command {
    /// Persist a validated default. Consent requires --accept-license when set to 1.
    Set {
        key: String,
        value: String,
        #[arg(long)]
        accept_license: bool,
    },
    /// Show stored defaults, not environment overrides or credentials.
    Show,
    /// Remove a stored default (does not change this invocation's environment).
    Unset { key: String },
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    values: BTreeMap<String, String>,
}

fn env_name(key: &str) -> Result<Option<&'static str>> {
    Ok(match key {
        "microsoft-sdk-license" => Some("JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE"),
        "install-llvm-tools" => Some("JUSTRUST_INSTALL_LLVM_TOOLS"),
        "darwin-sdk" => Some("JUSTRUST_DARWIN_SDK"),
        "darwin-signing-identity" | "windows-signing-thumbprint" => None,
        _ => bail!("unknown platform setup key: {key}"),
    })
}

fn validate(key: &str, value: &str) -> Result<()> {
    env_name(key)?;
    match key {
        "microsoft-sdk-license" | "install-llvm-tools" => {
            ensure!(matches!(value, "0" | "1"), "{key} must be 0 or 1")
        }
        "darwin-sdk" => ensure!(
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "darwin-sdk must be a lowercase SHA-256 managed SDK ID"
        ),
        // Fingerprints only: never accept arbitrary strings, paths, PEM, passwords or key data.
        "darwin-signing-identity" | "windows-signing-thumbprint" => ensure!(
            value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()),
            "{key} must be a public certificate SHA-1 40-hex fingerprint, not a key, password, path or certificate"
        ),
        _ => unreachable!(),
    }
    Ok(())
}

fn config_path(home: &Path) -> PathBuf {
    home.join("platform").join("config.json")
}

fn load(home: &Path) -> Result<Config> {
    let path = config_path(home);
    match fs::symlink_metadata(home.join("platform")) {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "platform directory must not be a symlink"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).context("opening platform configuration"),
    };
    ensure!(
        file.metadata()?.is_file(),
        "platform configuration must be a regular file"
    );
    ensure!(
        file.metadata()?.len() <= 16384,
        "platform configuration is too large"
    );
    let config: Config = serde_json::from_reader(file).context("invalid platform configuration")?;
    for (key, value) in &config.values {
        validate(key, value)?;
    }
    Ok(config)
}

#[cfg(unix)]
fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options
}

#[cfg(not(unix))]
fn save(_home: &Path, _command: Command) -> Result<()> {
    bail!("persisting platform setup currently requires Unix private-file permissions")
}

#[cfg(unix)]
fn save(home: &Path, command: Command) -> Result<()> {
    let dir = home.join("platform");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::create_dir_all(home)?;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let meta = fs::symlink_metadata(&dir)?;
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "platform directory must not be a symlink"
        );
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let lock = private_options()
        .create(true)
        .open(dir.join("config.lock"))?;
    ensure!(
        lock.metadata()?.is_file(),
        "platform lock must be a regular file"
    );
    lock.lock()?;
    let mut config = load(home)?;
    match command {
        Command::Set {
            key,
            value,
            accept_license,
        } => {
            validate(&key, &value)?;
            ensure!(
                key != "microsoft-sdk-license" || value != "1" || accept_license,
                "Microsoft SDK consent requires --accept-license: review https://go.microsoft.com/fwlink/?LinkId=2086102 and confirm you are authorized"
            );
            config.values.insert(key, value);
        }
        Command::Unset { key } => {
            env_name(&key)?;
            config.values.remove(&key);
        }
        Command::Show => unreachable!(),
    }
    let temporary = dir.join(format!("config.{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = private_options().create_new(true).open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, &config)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, config_path(home))?;
        File::open(&dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn command(command: Command) -> Result<()> {
    let home = crate::paths::home()?;
    match command {
        Command::Show => println!("{}", serde_json::to_string_pretty(&load(&home)?)?),
        command => save(&home, command)?,
    }
    Ok(())
}

fn signing_args(config: Config, mut args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    // Match argparse's last-value-wins behavior. Abbreviated options are disabled
    // in the embedded helper so an alias cannot accidentally override a default.
    let option = |name: &str| -> Option<std::ffi::OsString> {
        let mut value = None;
        for (index, arg) in args.iter().enumerate() {
            if arg == name {
                value = Some(args.get(index + 1).cloned().unwrap_or_default());
            } else if let Some(text) = arg
                .to_str()
                .and_then(|s| s.strip_prefix(&format!("{name}=")))
            {
                value = Some(text.into());
            }
        }
        value
    };
    let target = option("--target");
    let setting = match target.as_deref().and_then(|s| s.to_str()) {
        Some("aarch64-apple-darwin" | "x86_64-apple-darwin") if option("--identity").is_none() => {
            Some(("darwin-signing-identity", "--identity"))
        }
        Some("aarch64-pc-windows-msvc" | "x86_64-pc-windows-msvc")
            if option("--thumbprint").is_none() =>
        {
            Some(("windows-signing-thumbprint", "--thumbprint"))
        }
        _ => None,
    };
    if let Some((key, flag)) = setting
        && let Some(value) = config.values.get(key)
    {
        args.push(flag.into());
        args.push(value.into());
    }
    args
}

/// The embedded helper remains plan-only unless --execute is explicitly passed.
/// It owns header/hash checks, host restrictions and native chain verification.
pub fn sign_release(args: Vec<std::ffi::OsString>) -> Result<()> {
    let args = signing_args(load(&crate::paths::home()?)?, args);
    for python in ["python3", "python"] {
        let result = std::process::Command::new(python)
            // Ignore PYTHONPATH/user-site and the caller's working directory:
            // project-local modules must not intercept the signing helper.
            .arg("-I")
            .arg("-c")
            .arg(include_str!("../scripts/sign-release.py"))
            .args(&args)
            .status();
        match result {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).context("launching local signing helper"),
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => std::process::exit(crate::sys::exit_code(status)),
        }
    }
    bail!("local release signing requires Python 3 (python3 or python on PATH)")
}

fn defaults(config: Config, mut present: impl FnMut(&str) -> bool) -> Vec<(&'static str, String)> {
    config
        .values
        .into_iter()
        .filter_map(|(key, value)| {
            let name = env_name(&key).ok().flatten()?;
            (!present(name)).then_some((name, value))
        })
        .collect()
}

/// Called once before normal build routing, before justrust starts threads.
pub fn apply_defaults() -> Result<()> {
    let config = load(&crate::paths::home()?)?;
    for (name, value) in defaults(config, |name| std::env::var_os(name).is_some()) {
        // SAFETY: normal CLI startup is single-threaded here. Internal subprocess
        // entry points and rustc/linker shims return before this function.
        unsafe {
            std::env::set_var(name, value);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_syntax_and_missing_consent_flag() {
        use clap::Parser;
        assert!(crate::Cli::try_parse_from(["justrust", "setup", "show"]).is_ok());
        assert!(
            crate::Cli::try_parse_from([
                "justrust",
                "setup",
                "set",
                "microsoft-sdk-license",
                "1",
                "--accept-license"
            ])
            .is_ok()
        );
        assert!(crate::Cli::try_parse_from(["justrust", "setup", "set", "darwin-sdk"]).is_err());
        assert!(crate::Cli::try_parse_from(["justrust", "setup", "unset", "darwin-sdk"]).is_ok());
        assert!(defaults(Config::default(), |_| false).is_empty());
        let parsed = crate::Cli::try_parse_from([
            "justrust",
            "sign-release",
            "--target",
            "aarch64-apple-darwin",
            "--identity",
            "Developer ID Application: Example (TEAM)",
            "--execute",
        ])
        .unwrap();
        match parsed.command {
            crate::Command::SignRelease { args } => {
                assert_eq!(args.len(), 5);
                assert_eq!(args[3], "Developer ID Application: Example (TEAM)");
                assert_eq!(args[4], "--execute");
            }
            _ => panic!("expected sign-release"),
        }
    }
    #[test]
    fn signing_defaults_are_targeted_and_explicit_flags_win() {
        use std::ffi::OsString;
        let config = || Config {
            values: BTreeMap::from([
                ("darwin-signing-identity".into(), "a".repeat(40)),
                ("windows-signing-thumbprint".into(), "b".repeat(40)),
            ]),
        };
        let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        let actual = signing_args(config(), args(&["--target=aarch64-apple-darwin"]));
        assert_eq!(
            actual,
            args(&[
                "--target=aarch64-apple-darwin",
                "--identity",
                &"a".repeat(40)
            ])
        );
        assert!(!actual.contains(&OsString::from("--execute")));
        for flag in ["--identity=explicit", "--identity="] {
            let explicit = args(&["--target", "x86_64-apple-darwin", flag]);
            assert_eq!(signing_args(config(), explicit.clone()), explicit);
        }
        let explicit = args(&[
            "--target",
            "x86_64-pc-windows-msvc",
            "--thumbprint",
            "explicit",
            "--signtool",
            "/trusted/tool",
        ]);
        assert_eq!(signing_args(config(), explicit.clone()), explicit);
        let windows = signing_args(
            config(),
            args(&[
                "--target",
                "aarch64-apple-darwin",
                "--target=x86_64-pc-windows-msvc",
            ]),
        );
        assert_eq!(
            &windows[windows.len() - 2..],
            &args(&["--thumbprint", &"b".repeat(40)])
        );
        assert!(!windows.contains(&OsString::from("--identity")));
        assert!(!windows.contains(&OsString::from("--signtool")));
        assert!(signing_args(config(), Vec::new()).is_empty());
        assert!(validate("windows-signing-thumbprint", &"a".repeat(64)).is_err());
        assert!(validate("darwin-signing-identity", &"a".repeat(64)).is_err());
    }
    #[test]
    fn validates_allowlist_and_rejects_secrets() {
        for key in ["password", "private-key", "SDKROOT"] {
            assert!(validate(key, "1").is_err());
        }
        for value in ["yes", "true", "", "2"] {
            assert!(validate("microsoft-sdk-license", value).is_err());
        }
        assert!(validate("darwin-sdk", &"a".repeat(64)).is_ok());
        assert!(validate("darwin-sdk", &"A".repeat(64)).is_err());
        for value in ["secret", "-----BEGIN PRIVATE KEY-----", "/tmp/key.p12"] {
            assert!(validate("darwin-signing-identity", value).is_err());
        }
        assert!(validate("windows-signing-thumbprint", &"A".repeat(40)).is_ok());
    }
    #[test]
    fn environment_wins_and_signing_is_not_applied() {
        let mut config = Config::default();
        config
            .values
            .insert("install-llvm-tools".into(), "1".into());
        config
            .values
            .insert("microsoft-sdk-license".into(), "1".into());
        config
            .values
            .insert("darwin-signing-identity".into(), "a".repeat(40));
        assert_eq!(
            defaults(config, |key| key == "JUSTRUST_INSTALL_LLVM_TOOLS"),
            vec![("JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE", "1".into())]
        );
    }
    #[test]
    #[cfg(unix)]
    fn rejects_symlink_and_invalid_stored_config() -> Result<()> {
        use std::os::unix::fs::symlink;
        let home =
            std::env::temp_dir().join(format!("justrust-platform-invalid-{}", std::process::id()));
        fs::create_dir_all(home.join("platform"))?;
        fs::write(
            home.join("invalid.json"),
            r#"{"values":{"password":"secret"}}"#,
        )?;
        symlink(home.join("invalid.json"), config_path(&home))?;
        assert!(load(&home).is_err());
        fs::remove_file(config_path(&home))?;
        fs::write(config_path(&home), r#"{"values":{"password":"secret"}}"#)?;
        assert!(load(&home).is_err());
        fs::write(config_path(&home), "not json")?;
        assert!(load(&home).is_err());
        fs::remove_dir_all(home)?;
        Ok(())
    }
    #[test]
    #[cfg(unix)]
    fn persistence_consent_permissions_and_unset() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let home =
            std::env::temp_dir().join(format!("justrust-platform-test-{}", std::process::id()));
        fs::create_dir_all(&home)?;
        assert!(load(&home)?.values.is_empty());
        assert!(
            save(
                &home,
                Command::Set {
                    key: "microsoft-sdk-license".into(),
                    value: "1".into(),
                    accept_license: false
                }
            )
            .is_err()
        );
        save(
            &home,
            Command::Set {
                key: "microsoft-sdk-license".into(),
                value: "1".into(),
                accept_license: true,
            },
        )?;
        assert_eq!(load(&home)?.values["microsoft-sdk-license"], "1");
        assert_eq!(
            fs::metadata(config_path(&home))?.permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(home.join("platform"))?.permissions().mode() & 0o777,
            0o700
        );
        save(
            &home,
            Command::Unset {
                key: "microsoft-sdk-license".into(),
            },
        )?;
        assert!(load(&home)?.values.is_empty());
        fs::remove_dir_all(home)?;
        Ok(())
    }
}
