//! Portable, trusted-input test executable bundles (Python helper schema v1).
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

const MAX_JSON: u64 = 4 * 1024 * 1024;
const MAX_TESTS: usize = 4096;

#[derive(Subcommand)]
pub(crate) enum BundleCmd {
    /// Package a successful Cargo JSON stream without executing tests.
    #[command(alias = "bundle")]
    Pack {
        #[arg(long)]
        target: String,
        #[arg(long)]
        messages: PathBuf,
        #[arg(long)]
        artifact_root: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Execute TRUSTED bundle code on a matching host, recording a new result.
    Run {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        test_args: Vec<OsString>,
    },
}

// Cargo events have additional fields, but duplicate contract fields must not
// silently turn an unsuccessful stream into a successful one.
#[derive(Deserialize)]
struct CargoEvent {
    reason: String,
    profile: Option<CargoProfile>,
    executable: Option<String>,
    success: Option<bool>,
}
#[derive(Deserialize)]
struct CargoProfile {
    #[serde(default)]
    test: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    target: String,
    build_status: String,
    test_status: String,
    tests: Vec<Test>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Test {
    path: String,
    sha256: String,
    size: u64,
}
#[derive(Serialize)]
struct Results {
    schema_version: u32,
    target: String,
    manifest_sha256: String,
    test_status: &'static str,
    tests: Vec<TestResult>,
}
#[derive(Serialize)]
struct TestResult {
    path: String,
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn target_host(target: &str) -> Result<(&'static str, &'static str)> {
    Ok(match target {
        "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => ("linux", "x86_64"),
        "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => ("linux", "aarch64"),
        "x86_64-pc-windows-gnu" | "x86_64-pc-windows-msvc" => ("windows", "x86_64"),
        "aarch64-pc-windows-msvc" => ("windows", "aarch64"),
        "x86_64-apple-darwin" => ("macos", "x86_64"),
        "aarch64-apple-darwin" => ("macos", "aarch64"),
        _ => bail!("unsupported target triple: {target}"),
    })
}

// Walk the original path before canonicalizing: normalization must not hide a
// symlink in e.g. link/../file. Reject parent components rather than guessing.
fn checked_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut part = PathBuf::new();
    for component in absolute.components() {
        ensure!(
            !matches!(component, Component::ParentDir),
            "parent path components are not permitted"
        );
        part.push(component);
        match fs::symlink_metadata(&part) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "symlink not permitted: {}",
                part.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(absolute)
}
fn regular(path: &Path) -> Result<PathBuf> {
    let path = checked_path(path)?;
    ensure!(
        fs::symlink_metadata(&path)?.is_file(),
        "not a regular file: {}",
        path.display()
    );
    Ok(path)
}
fn write_json(file: &mut File, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer_pretty(&mut *file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}
fn new_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(checked_path(path)?)?)
}

fn pack(target: String, messages: PathBuf, artifact_root: PathBuf, out: PathBuf) -> Result<()> {
    let (os, _) = target_host(&target)?;
    let root = checked_path(&artifact_root)?.canonicalize()?;
    ensure!(root.is_dir(), "artifact root must be a directory");
    let mut reader = BufReader::new(File::open(regular(&messages)?)?);
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    let mut finished = false;
    loop {
        let mut line = Vec::new();
        let count = reader
            .by_ref()
            .take(MAX_JSON + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        ensure!(count as u64 <= MAX_JSON, "Cargo JSON line too large");
        let value: serde_json::Value =
            serde_json::from_slice(&line).context("invalid Cargo JSON")?;
        ensure!(value.is_object(), "Cargo event must be an object");
        ensure!(
            value
                .get("profile")
                .is_none_or(|p| p.is_null() || p.is_object()),
            "Cargo profile must be an object"
        );
        let event: CargoEvent = serde_json::from_slice(&line).context("invalid Cargo JSON")?;
        if event.reason == "build-finished" {
            ensure!(
                event.success == Some(true),
                "requires successful build-finished event(s)"
            );
            finished = true;
        }
        if event.reason == "compiler-artifact"
            && event.profile.is_some_and(|profile| profile.test)
            && let Some(executable) = event.executable.as_deref().filter(|s| !s.is_empty())
        {
            let source = regular(Path::new(executable))?.canonicalize()?;
            ensure!(
                source.starts_with(&root),
                "test executable is outside artifact root"
            );
            if seen.insert(source.clone()) {
                sources.push(source);
            }
            ensure!(sources.len() <= MAX_TESTS, "too many test executables");
        }
    }
    ensure!(finished, "requires successful build-finished event(s)");
    ensure!(!sources.is_empty(), "no test executables in Cargo messages");
    let out = checked_path(&out)?;
    fs::create_dir(&out)?;
    let result = (|| -> Result<()> {
        fs::create_dir(out.join("bin"))?;
        let mut tests = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            let path = format!(
                "bin/test-{index:04}{}",
                if os == "windows" { ".exe" } else { "" }
            );
            let destination = out.join(&path);
            let mut dest = new_file(&destination)?;
            std::io::copy(&mut File::open(regular(source)?)?, &mut dest)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                dest.set_permissions(fs::Permissions::from_mode(0o755))?;
            }
            tests.push(Test {
                path,
                sha256: crate::toolchain::sha256(&destination)?,
                size: dest.metadata()?.len(),
            });
        }
        let manifest = Manifest {
            schema_version: 1,
            target,
            build_status: "compiled".into(),
            test_status: "not-tested".into(),
            tests,
        };
        write_json(&mut new_file(&out.join("manifest.json"))?, &manifest)?;
        println!(
            "Bundled {} executable(s): compiled, NOT TESTED",
            manifest.tests.len()
        );
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&out);
    }
    result
}

fn load_manifest(directory: &Path) -> Result<Manifest> {
    let path = regular(&directory.join("manifest.json"))?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_JSON, "manifest too large");
    // Serde structs also accept positional arrays. The wire schema only permits
    // objects, so check shapes before typed parsing preserves duplicate checks.
    let value: serde_json::Value = serde_json::from_slice(&bytes).context("invalid manifest")?;
    ensure!(value.is_object(), "manifest must be an object");
    ensure!(
        value
            .get("tests")
            .and_then(|v| v.as_array())
            .is_some_and(|tests| tests.iter().all(|test| test.is_object())),
        "test entries must be objects"
    );
    // Typed deserialization rejects duplicate known fields as well as unknown fields.
    let manifest: Manifest = serde_json::from_slice(&bytes).context("invalid manifest")?;
    ensure!(manifest.schema_version == 1, "unsupported schema version");
    target_host(&manifest.target)?;
    ensure!(
        manifest.build_status == "compiled" && manifest.test_status == "not-tested",
        "invalid build/test status"
    );
    ensure!(
        (1..=MAX_TESTS).contains(&manifest.tests.len()),
        "invalid test count"
    );
    let mut seen = HashSet::new();
    for test in &manifest.tests {
        let parts: Vec<_> = test.path.split('/').collect();
        ensure!(
            test.path.len() <= 256
                && !test.path.contains(['\\', ':'])
                && parts.len() == 2
                && parts[0] == "bin"
                && !matches!(parts[1], "" | "." | ".."),
            "unsafe test path"
        );
        ensure!(seen.insert(&test.path), "duplicate test path");
        ensure!(
            test.sha256.len() == 64
                && test
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid sha256"
        );
        let path = regular(&directory.join(&test.path))?;
        ensure!(
            fs::metadata(&path)?.len() == test.size
                && crate::toolchain::sha256(&path)? == test.sha256,
            "hash/size mismatch: {}",
            test.path
        );
    }
    Ok(manifest)
}

fn run(
    bundle: PathBuf,
    results: PathBuf,
    cwd: Option<PathBuf>,
    args: Vec<OsString>,
) -> Result<i32> {
    for arg in &args {
        let arg = arg.to_string_lossy();
        let flag = arg.split('=').next().unwrap_or_default();
        ensure!(
            !matches!(flag, "--list" | "--help" | "-h" | "--bench")
                && !(flag.starts_with('-') && !flag.starts_with("--") && flag[1..].contains('h')),
            "non-test invocation is not permitted"
        );
    }
    let directory = checked_path(&bundle)?.canonicalize()?;
    let manifest = load_manifest(&directory)?;
    ensure!(
        target_host(&manifest.target)? == (std::env::consts::OS, std::env::consts::ARCH),
        "target OS/architecture does not match this host"
    );
    let results = checked_path(&results)?;
    let parent = results
        .parent()
        .context("results need a parent directory")?
        .canonicalize()?;
    ensure!(
        !parent.starts_with(&directory),
        "results must be outside the immutable bundle"
    );
    let mut output = new_file(&results)?;
    let mut result = Results {
        schema_version: 1,
        target: manifest.target,
        manifest_sha256: crate::toolchain::sha256(&directory.join("manifest.json"))?,
        test_status: "failed",
        tests: Vec::new(),
    };
    // Persist a conservative partial record even if execution is interrupted.
    write_json(&mut output, &result)?;
    eprintln!("WARNING: executing trusted bundle code. Never run an untrusted bundle.");
    for test in manifest.tests {
        let mut command = Command::new(directory.join(&test.path));
        command.args(&args);
        if let Some(cwd) = &cwd {
            command.current_dir(cwd);
        }
        let entry = match command.status() {
            Ok(status) => TestResult {
                path: test.path,
                exit_code: status.code(),
                error: None,
            },
            Err(error) => TestResult {
                path: test.path,
                exit_code: None,
                error: Some(error.to_string()),
            },
        };
        result.tests.push(entry);
    }
    let passed = result.tests.iter().all(|t| t.exit_code == Some(0));
    if passed {
        result.test_status = "passed";
    }
    use std::io::{Seek, SeekFrom};
    output.seek(SeekFrom::Start(0))?;
    output.set_len(0)?;
    write_json(&mut output, &result)?;
    Ok(if passed { 0 } else { 1 })
}

pub(crate) fn main(cmd: BundleCmd) -> ! {
    let result = match cmd {
        BundleCmd::Pack {
            target,
            messages,
            artifact_root,
            out,
        } => pack(target, messages, artifact_root, out).map(|()| 0),
        BundleCmd::Run {
            bundle,
            results,
            cwd,
            test_args,
        } => run(bundle, results, cwd, test_args),
    };
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("justrust test-bundle: {error:#}");
            2
        }
    };
    std::process::exit(code)
}
