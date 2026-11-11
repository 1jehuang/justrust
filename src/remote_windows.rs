//! Native Windows hosted transport. This never enters the Unix sync daemon.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

pub const TARGET: &str = "x86_64-pc-windows-msvc";
pub const TRANSPORT: &str = "windows-powershell-v1";
const AGENT: &str = r"C:\ProgramData\Justrust\hosted-agent.ps1";
const MAX_SOURCE: usize = 64 << 20;
const MAX_FRAME: usize = 1 << 20;
const MAX_CHUNK: usize = 256 << 10;
const MAX_ARTIFACT: u64 = 512 << 20;

fn platform_value(value: &str) -> Result<bool> {
    match value {
        "linux" => Ok(false),
        "windows" => Ok(true),
        _ => bail!("unsupported hosted platform {value:?}: expected linux or windows"),
    }
}

pub fn selected() -> Result<bool> {
    if let Some(value) = std::env::var_os("JUSTRUST_HOST_PLATFORM") {
        return platform_value(value.to_str().context("host platform is not UTF-8")?);
    }
    let file = crate::paths::home()?.join("remote/hosted-platform.json");
    match std::fs::read(file) {
        Ok(data) => platform_value(
            &serde_json::from_slice::<String>(&data)
                .context("invalid hosted platform selection")?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn select(value: &str) -> Result<()> {
    platform_value(value)?;
    let dir = crate::remote::dir()?;
    let file = dir.join("hosted-platform.json");
    let tmp = dir.join("hosted-platform.json.tmp");
    let mut out = crate::sys::create_private(&tmp)?;
    out.write_all(&serde_json::to_vec(value)?)?;
    drop(out);
    std::fs::rename(tmp, file)?;
    Ok(())
}

pub fn setup_args(args: &[String]) -> Result<String> {
    let mut platform = None;
    let mut target = None;
    let mut i = 0;
    while i < args.len() {
        let (key, value, n) = if let Some((key, value)) = args[i].split_once('=') {
            (key, value, 1)
        } else {
            (
                args[i].as_str(),
                args.get(i + 1)
                    .context("expected --platform linux|windows [--target x86_64-pc-windows-msvc]")?
                    .as_str(),
                2,
            )
        };
        match key {
            "--platform" if platform.is_none() => {
                platform_value(value)?;
                platform = Some(value);
            }
            "--target" if target.is_none() => target = Some(value),
            _ => bail!("unexpected or duplicate hosted option {key:?}"),
        }
        i += n;
    }
    let platform = platform.unwrap_or("linux");
    ensure!(
        target.is_none() || (platform == "windows" && target == Some(TARGET)),
        "hosted Windows requires --platform windows --target {TARGET}"
    );
    Ok(platform.into())
}

/// Per-invocation selection is deliberately separate from Cargo's --target.
/// Stop at -- so a test or application can use its own --host-platform flag.
pub fn take_platform_flags(args: Vec<OsString>) -> Result<Vec<OsString>> {
    let mut out = Vec::new();
    let mut iter = args.into_iter();
    let mut selected = false;
    while let Some(arg) = iter.next() {
        if arg == "--" {
            out.push(arg);
            out.extend(iter);
            break;
        }
        let text = arg.to_str().unwrap_or("");
        let value = if text == "--host-platform" {
            Some(
                iter.next()
                    .context("--host-platform requires linux or windows")?,
            )
        } else {
            text.strip_prefix("--host-platform=").map(OsString::from)
        };
        if let Some(value) = value {
            ensure!(!selected, "duplicate --host-platform");
            let value = value.to_str().context("host platform is not UTF-8")?;
            platform_value(value)?;
            // CLI startup is single-threaded, before routing starts workers.
            unsafe {
                std::env::set_var("JUSTRUST_HOST_PLATFORM", value);
            }
            selected = true;
        } else {
            out.push(arg);
        }
    }
    Ok(out)
}

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub(crate) fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn decode(text: &str) -> Result<Vec<u8>> {
    ensure!(
        text.len().is_multiple_of(4) && text.len() <= MAX_CHUNK.div_ceil(3) * 4,
        "invalid or oversized native base64 chunk"
    );
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for (index, group) in text.as_bytes().chunks(4).enumerate() {
        let mut n = 0u32;
        let mut padding = 0;
        for (i, byte) in group.iter().enumerate() {
            n <<= 6;
            if *byte == b'=' {
                ensure!(
                    i >= 2 && index == text.len() / 4 - 1,
                    "invalid base64 padding"
                );
                padding += 1;
            } else {
                ensure!(padding == 0, "invalid base64 padding");
                n |= ALPHABET
                    .iter()
                    .position(|c| c == byte)
                    .context("invalid base64 byte")? as u32;
            }
        }
        ensure!(
            padding <= 2 && (padding == 0 || n & ((1 << (padding * 8)) - 1) == 0),
            "noncanonical base64 padding"
        );
        out.push((n >> 16) as u8);
        if padding < 2 {
            out.push((n >> 8) as u8);
        }
        if padding == 0 {
            out.push(n as u8);
        }
    }
    ensure!(out.len() <= MAX_CHUNK, "native output chunk exceeds bound");
    Ok(out)
}

pub fn agent_command(probe: bool) -> String {
    // This fixed path has no cmd.exe metacharacters or whitespace. Request
    // arguments belong exclusively on stdin, never in this SSH command.
    // Invoke the reviewed agent directly, without an intermediate script.
    format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -File {AGENT}{}",
        if probe { " -Probe" } else { "" }
    )
}

pub fn verify_probe(value: &Value) -> Result<()> {
    ensure!(
        value["schema"] == 1
            && value["platform"] == "windows"
            && value["target"] == TARGET
            && value["transport"] == TRANSPORT,
        "native Windows worker probe platform/target/transport mismatch"
    );
    Ok(())
}

pub fn probe(target: &crate::remote_backend::Target) -> Result<()> {
    let mut child = target
        .command()
        .arg(agent_command(true))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut data = Vec::new();
    child
        .stdout
        .take()
        .context("native probe stdout")?
        .take(16385)
        .read_to_end(&mut data)?;
    if data.len() > 16384 {
        let _ = child.kill();
        let _ = child.wait();
        bail!("native Windows probe exceeds size bound");
    }
    ensure!(
        child.wait()?.success(),
        "native Windows worker probe failed"
    );
    verify_probe(&serde_json::from_slice(&data).context("invalid native Windows worker probe")?)
}

/// Reject paths whose meaning changes across Unix and Win32. No source links
/// are followed and no unrepresentable filename is silently renamed.
fn wire_path(path: &Path) -> Result<String> {
    let mut out = Vec::new();
    for part in path.components() {
        let Component::Normal(part) = part else {
            bail!("native path must be relative without traversal");
        };
        let part = part.to_str().context("native paths must be UTF-8")?;
        ensure!(
            !part.is_empty()
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
            "unrepresentable Windows filename {part:?}"
        );
        let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !matches!(
                base.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            ) && !(base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && matches!(base.as_bytes()[3], b'1'..=b'9')),
            "reserved Windows device filename {part:?}"
        );
        out.push(part);
    }
    let out = out.join("/");
    ensure!(
        !out.is_empty() && out.len() <= 240,
        "native path is empty or too long"
    );
    Ok(out)
}

fn sensitive_path(path: &Path) -> bool {
    path.components().any(|c| {
        let name = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        name == ".env"
            || name.starts_with(".env.")
            || matches!(
                name.as_str(),
                ".ssh"
                    | ".aws"
                    | ".azure"
                    | ".gnupg"
                    | "credentials"
                    | "credentials.toml"
                    | "id_rsa"
                    | "id_ed25519"
            )
    })
}

#[derive(Serialize, Deserialize, Debug)]
struct SourceFile {
    path: String,
    data: String,
}

fn common_root(roots: &[PathBuf]) -> Result<PathBuf> {
    let mut common = roots
        .first()
        .context("no source roots for Windows build")?
        .clone();
    for root in roots.iter().skip(1) {
        while !root.starts_with(&common) {
            ensure!(common.pop(), "source roots do not share a volume");
        }
    }
    Ok(common)
}

fn project_key(cwd: &Path) -> String {
    use std::hash::{Hash, Hasher};
    // A stable namespace, not an authentication token. Tenant isolation is
    // enforced by the control plane and dedicated Windows account worker.
    (0u64..4)
        .map(|domain| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (TRANSPORT, TARGET, domain, cwd).hash(&mut h);
            format!("{:016x}", h.finish())
        })
        .collect()
}

fn native_args(sub: &str, args: &[String]) -> Result<Vec<String>> {
    ensure!(
        matches!(sub, "check" | "build" | "test" | "clippy"),
        "native Windows hosted supports check, build, test and clippy"
    );
    let mut out = vec![sub.to_owned()];
    let mut iter = args.iter();
    let mut target = false;
    while let Some(arg) = iter.next() {
        if arg == "--" {
            out.push(arg.clone());
            out.extend(iter.cloned());
            break;
        }
        ensure!(
            !matches!(
                arg.as_str(),
                "--target-dir" | "--manifest-path" | "--config"
            ) && !["--target-dir=", "--manifest-path=", "--config="]
                .iter()
                .any(|p| arg.starts_with(p)),
            "native Windows hosted does not accept caller-local path/config overrides ({arg})"
        );
        if arg == "--target" {
            ensure!(
                !target && iter.next().map(String::as_str) == Some(TARGET),
                "native Windows hosted target must be {TARGET}"
            );
            target = true;
        } else if let Some(value) = arg.strip_prefix("--target=") {
            ensure!(
                !target && value == TARGET,
                "native Windows hosted target must be {TARGET}"
            );
            target = true;
        } else {
            out.push(arg.clone());
        }
    }
    // Insert before --, never into a test binary's arguments.
    let index = out.iter().position(|a| a == "--").unwrap_or(out.len());
    out.splice(index..index, ["--target".into(), TARGET.into()]);
    Ok(out)
}

fn bundle(sub: &str, args: &[String], run: &str, cwd: &Path, roots: &[PathBuf]) -> Result<Value> {
    let args = native_args(sub, args)?;
    let common = common_root(roots)?;
    let relative_cwd = cwd.strip_prefix(&common)?;
    let relative_cwd = if relative_cwd.as_os_str().is_empty() {
        ".".into()
    } else {
        wire_path(relative_cwd)?
    };
    let mut files = Vec::new();
    let mut names: BTreeMap<String, (String, bool)> = BTreeMap::new();
    let mut size = 0usize;
    for root in roots {
        let scan = crate::remote_sync::scan(root)?;
        ensure!(
            scan.oversized.is_empty(),
            "native Windows source contains files over the 8 MiB file limit"
        );
        for relative in scan.files.keys() {
            if sensitive_path(relative) {
                continue;
            }
            let full = root.join(relative);
            let path = wire_path(full.strip_prefix(&common)?)?;
            // Case folding must also reject directory case collisions.
            for (i, _) in path.match_indices('/') {
                let prefix = &path[..i];
                let folded = prefix.to_lowercase();
                if let Some((existing, file)) = names.get(&folded) {
                    ensure!(
                        !file && existing == prefix,
                        "Windows source path case collision: {path}"
                    );
                } else {
                    names.insert(folded, (prefix.to_owned(), false));
                }
            }
            let folded = path.to_lowercase();
            ensure!(
                names.insert(folded, (path.clone(), true)).is_none(),
                "Windows source path collision: {path}"
            );
            let meta = std::fs::symlink_metadata(&full)?;
            ensure!(
                meta.file_type().is_file(),
                "native source changed type: {}",
                full.display()
            );
            let data = std::fs::read(&full)?;
            ensure!(
                data.len() <= crate::remote_sync::MAX_FILE as usize,
                "native source file grew beyond limit"
            );
            size += data.len();
            ensure!(
                size <= MAX_SOURCE && files.len() < 20000,
                "native Windows source exceeds 64 MiB/20000 file bound"
            );
            files.push(SourceFile {
                path,
                data: encode(&data),
            });
        }
    }
    ensure!(
        files
            .iter()
            .any(|f| f.path == "Cargo.toml" || f.path.ends_with("/Cargo.toml")),
        "no Cargo.toml in native source snapshot"
    );
    let env: BTreeMap<_, _> = crate::remote_build::forwarded_env(|name| std::env::var(name).ok())
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "RUSTFLAGS"
                    | "CARGO_ENCODED_RUSTFLAGS"
                    | "CARGO_INCREMENTAL"
                    | "CARGO_BUILD_JOBS"
                    | "RUST_TEST_THREADS"
                    | "CARGO_TERM_COLOR"
                    | "RUST_BACKTRACE"
                    | "SOURCE_DATE_EPOCH"
                    | "CARGO_PROFILE_RELEASE_LTO"
                    | "CARGO_PROFILE_RELEASE_CODEGEN_UNITS"
                    | "CARGO_PROFILE_RELEASE_OPT_LEVEL"
                    | "CARGO_PROFILE_RELEASE_DEBUG"
                    | "CARGO_PROFILE_RELEASE_STRIP"
            )
        })
        .collect();
    Ok(
        json!({"schema":1,"platform":"windows","target":TARGET,"project":project_key(cwd),"run":run,"cwd":relative_cwd,"args":args,"files":files,"env":env}),
    )
}

struct SshReader<'a>(&'a [u8]);
impl<'a> SshReader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        ensure!(self.0.len() >= count, "truncated SSH certificate");
        let (value, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into()?))
    }
    fn string(&mut self) -> Result<&'a [u8]> {
        let count = self.u32()? as usize;
        self.take(count)
    }
}

/// SSH authenticates the CA signature itself. Before persisting the API result,
/// independently verify that it enrolls only our freshly generated public key,
/// is a user certificate and has a narrow validity interval.
pub fn verify_certificate(certificate: &str, public_key: &str, now: u64) -> Result<()> {
    let mut words = certificate.split_whitespace();
    ensure!(
        !certificate.contains(['\n', '\r'])
            && words.next() == Some("ssh-ed25519-cert-v01@openssh.com"),
        "invalid Windows SSH certificate type"
    );
    let cert = decode(words.next().context("SSH certificate data missing")?)?;
    let mut public = public_key.split_whitespace();
    ensure!(
        public.next() == Some("ssh-ed25519"),
        "Windows hosted requires an Ed25519 client key"
    );
    let key = decode(public.next().context("SSH public key data missing")?)?;
    let mut key = SshReader(&key);
    ensure!(
        key.string()? == b"ssh-ed25519",
        "invalid SSH public key type"
    );
    let expected = key.string()?;
    ensure!(
        expected.len() == 32 && key.0.is_empty(),
        "invalid Ed25519 public key"
    );
    let mut cert = SshReader(&cert);
    ensure!(
        cert.string()? == b"ssh-ed25519-cert-v01@openssh.com",
        "SSH certificate wire type mismatch"
    );
    let _nonce = cert.string()?;
    ensure!(
        cert.string()? == expected,
        "SSH certificate does not bind our public key"
    );
    let _serial = cert.u64()?;
    ensure!(
        cert.u32()? == 1,
        "SSH certificate is not a user certificate"
    );
    let _key_id = cert.string()?;
    let principals = cert.string()?;
    ensure!(
        !principals.is_empty(),
        "SSH certificate has unrestricted principals"
    );
    let mut principals = SshReader(principals);
    while !principals.0.is_empty() {
        let principal = principals.string()?;
        ensure!(
            !principal.is_empty()
                && principal.len() <= 256
                && principal
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-:".contains(b)),
            "invalid SSH certificate principal"
        );
    }
    let after = cert.u64()?;
    let before = cert.u64()?;
    ensure!(
        after <= now.saturating_add(30)
            && after.saturating_add(600) >= now
            && before > now
            && before <= now.saturating_add(600)
            && before > after
            && before - after <= 660,
        "Windows SSH certificate is expired or not short-lived"
    );
    let _critical = cert.string()?;
    let _extensions = cert.string()?;
    let _reserved = cert.string()?;
    ensure!(
        !cert.string()?.is_empty() && !cert.string()?.is_empty() && cert.0.is_empty(),
        "SSH certificate signature missing or trailing bytes"
    );
    Ok(())
}

pub struct Prepared {
    target: crate::remote_backend::Target,
    payload: Vec<u8>,
    artifacts: PathBuf,
    require_artifact: bool,
    require_executable: bool,
}

pub fn prepare(
    sub: &str,
    args: &[String],
    run: &str,
    cwd: &Path,
    roots: &[PathBuf],
) -> Result<Prepared> {
    let payload = serde_json::to_vec(&bundle(sub, args, run, cwd, roots)?)?;
    ensure!(
        payload.len() <= 96 << 20,
        "native source request exceeds 96 MiB bound"
    );
    let target = crate::remote_hosted::ensure_ready_with_refresh(true)?;
    let artifacts = cwd
        .join("target/hosted-windows")
        .join(TARGET)
        .join(project_key(cwd))
        .join(run);
    let require_artifact = sub == "build"
        || (sub == "test"
            && args
                .iter()
                .take_while(|a| a.as_str() != "--")
                .any(|a| a == "--no-run"));
    let require_executable = (sub == "test" && require_artifact)
        || (sub == "build"
            && args.iter().take_while(|a| a.as_str() != "--").any(|a| {
                matches!(a.as_str(), "--bin" | "--bins" | "--example" | "--examples")
                    || a.starts_with("--bin=")
                    || a.starts_with("--example=")
            }));
    Ok(Prepared {
        target,
        payload,
        artifacts,
        require_artifact,
        require_executable,
    })
}

pub struct NativeResult {
    pub code: i32,
    pub summary: Option<Value>,
}

struct PendingArtifact {
    file: std::fs::File,
    path: PathBuf,
    bytes: u64,
    received: u64,
    sha256: String,
}

struct Receiver {
    run: String,
    hello: bool,
    result: Option<NativeResult>,
    pending: Option<PendingArtifact>,
    paths: BTreeSet<String>,
    stage: PathBuf,
    output: Option<std::fs::File>,
    started: std::time::Instant,
    artifact_bytes: u64,
    output_bytes: usize,
    executables: usize,
    stdout_pending: Vec<u8>,
    stderr_pending: Vec<u8>,
}

impl Receiver {
    fn new(run: &str, stage: PathBuf, dir: &Path) -> Result<Self> {
        ensure!(
            run.len() <= 128
                && !run.is_empty()
                && run
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "invalid native run identifier"
        );
        std::fs::create_dir(&stage).context("creating fresh native artifact staging directory")?;
        let output = std::fs::create_dir_all(dir)
            .and_then(|()| crate::sys::create_private(&dir.join("output.jsonl")))
            .ok();
        if output.is_none() {
            eprintln!("justrust: native output log unavailable; continuing without recording");
        }
        Ok(Self {
            run: run.into(),
            hello: false,
            result: None,
            pending: None,
            paths: BTreeSet::new(),
            stage,
            output,
            started: std::time::Instant::now(),
            artifact_bytes: 0,
            output_bytes: 0,
            executables: 0,
            stdout_pending: Vec::new(),
            stderr_pending: Vec::new(),
        })
    }

    fn journal(&mut self, stderr: bool, data: &[u8], final_chunk: bool) -> Result<()> {
        let Some(output) = self.output.as_mut() else {
            return Ok(());
        };
        let pending = if stderr {
            &mut self.stderr_pending
        } else {
            &mut self.stdout_pending
        };
        pending.extend_from_slice(data);
        let mut consumed = 0;
        for (index, byte) in pending.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let line = crate::record::OutputLine {
                t: self.started.elapsed().as_secs_f64(),
                s: if stderr { "e" } else { "o" }.into(),
                l: String::from_utf8_lossy(&pending[consumed..index])
                    .trim_end_matches('\r')
                    .to_owned(),
            };
            serde_json::to_writer(&mut *output, &line)?;
            output.write_all(b"\n")?;
            consumed = index + 1;
        }
        if consumed != 0 {
            pending.drain(..consumed);
        }
        if (final_chunk || pending.len() >= MAX_FRAME) && !pending.is_empty() {
            let line = crate::record::OutputLine {
                t: self.started.elapsed().as_secs_f64(),
                s: if stderr { "e" } else { "o" }.into(),
                l: String::from_utf8_lossy(pending).into_owned(),
            };
            serde_json::to_writer(&mut *output, &line)?;
            output.write_all(b"\n")?;
            pending.clear();
        }
        Ok(())
    }

    fn record_chunk(&mut self, stderr: bool, data: &[u8], final_chunk: bool) {
        if self.journal(stderr, data, final_chunk).is_err() {
            eprintln!("justrust: native output recording failed; compilation continues");
            self.output = None;
            self.stdout_pending.clear();
            self.stderr_pending.clear();
        }
    }

    fn frame(&mut self, frame: Value, out: &mut impl Write, err: &mut impl Write) -> Result<()> {
        ensure!(
            self.result.is_none(),
            "native protocol data follows terminal exit"
        );
        let kind = frame["type"]
            .as_str()
            .context("native protocol frame has no type")?;
        if !self.hello {
            ensure!(
                kind == "hello" && frame["run"] == self.run,
                "native protocol missing hello or run binding"
            );
            verify_probe(&frame)?;
            self.hello = true;
            return Ok(());
        }
        match kind {
            "stdout" | "stderr" => {
                let data = decode(
                    frame["data"]
                        .as_str()
                        .context("native output data missing")?,
                )?;
                self.output_bytes += data.len();
                ensure!(
                    self.output_bytes <= 64 << 20,
                    "native output exceeds 64 MiB bound"
                );
                if kind == "stdout" {
                    out.write_all(&data)?;
                    out.flush()?;
                } else {
                    err.write_all(&data)?;
                    err.flush()?;
                }
                self.record_chunk(kind == "stderr", &data, false);
            }
            "artifact-start" => {
                ensure!(self.pending.is_none(), "overlapping native artifacts");
                let name = frame["path"]
                    .as_str()
                    .context("native artifact path missing")?;
                ensure!(
                    wire_path(Path::new(name))? == name && name.starts_with(&format!("{TARGET}/")),
                    "invalid or unscoped native artifact path"
                );
                ensure!(
                    [".exe", ".dll", ".rlib", ".lib"]
                        .iter()
                        .any(|extension| name.ends_with(extension)),
                    "unsupported native artifact format"
                );
                ensure!(
                    self.paths.insert(name.to_lowercase()) && self.paths.len() <= 256,
                    "duplicate or too many native artifacts"
                );
                let bytes = frame["bytes"]
                    .as_u64()
                    .context("native artifact size missing")?;
                let sha256 = frame["sha256"]
                    .as_str()
                    .context("native artifact checksum missing")?
                    .to_owned();
                self.artifact_bytes = self
                    .artifact_bytes
                    .checked_add(bytes)
                    .context("artifact size overflow")?;
                ensure!(
                    bytes > 0
                        && self.artifact_bytes <= MAX_ARTIFACT
                        && sha256.len() == 64
                        && sha256
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "native artifact bounds/checksum invalid"
                );
                let path = self.stage.join(name);
                std::fs::create_dir_all(path.parent().context("artifact parent missing")?)?;
                let file = crate::sys::create_private(&path)?;
                self.pending = Some(PendingArtifact {
                    file,
                    path,
                    bytes,
                    received: 0,
                    sha256,
                });
            }
            "artifact-data" => {
                let artifact = self
                    .pending
                    .as_mut()
                    .context("native artifact data without start")?;
                let data = decode(
                    frame["data"]
                        .as_str()
                        .context("native artifact data missing")?,
                )?;
                artifact.received += data.len() as u64;
                ensure!(
                    !data.is_empty() && artifact.received <= artifact.bytes,
                    "native artifact exceeds announced size"
                );
                artifact.file.write_all(&data)?;
            }
            "artifact-end" => {
                let artifact = self
                    .pending
                    .take()
                    .context("native artifact end without start")?;
                ensure!(
                    artifact.received == artifact.bytes,
                    "native artifact truncated"
                );
                artifact.file.sync_all()?;
                drop(artifact.file);
                ensure!(
                    crate::toolchain::sha256(&artifact.path)? == artifact.sha256,
                    "native artifact SHA256 mismatch"
                );
                let file = std::fs::File::open(&artifact.path)?;
                // SAFETY: the file is private staging output, no writer remains.
                let data = unsafe { memmap2::Mmap::map(&file)? };
                validate_artifact(&artifact.path, &data)?;
                self.executables +=
                    usize::from(artifact.path.extension().is_some_and(|e| e == "exe"));
            }
            "exit" => {
                ensure!(
                    self.pending.is_none(),
                    "native exit during artifact transfer"
                );
                let code = frame["code"]
                    .as_i64()
                    .filter(|c| (0..=255).contains(c))
                    .context("native exit code invalid")? as i32;
                let mut summary = frame.get("summary").filter(|v| !v.is_null()).cloned();
                if let Some(value) = &summary {
                    ensure!(
                        value["id"] == self.run && value["exit"] == code,
                        "native summary run/exit mismatch"
                    );
                    if serde_json::from_value::<crate::summary::Summary>(value.clone()).is_err() {
                        eprintln!(
                            "justrust: native recorder summary is incompatible; keeping the command result without measurements"
                        );
                        summary = None;
                    }
                }
                self.record_chunk(false, &[], true);
                self.record_chunk(true, &[], true);
                if let Some(output) = self.output.as_mut() {
                    let _ = output.flush();
                }
                self.result = Some(NativeResult { code, summary });
            }
            _ => bail!("unknown native Windows protocol frame {kind:?}"),
        }
        Ok(())
    }
}

fn read_frame(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut data = Vec::new();
    let bytes = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut data)?;
    if bytes == 0 {
        return Ok(None);
    }
    ensure!(
        bytes <= MAX_FRAME && data.last() == Some(&b'\n'),
        "oversized or unterminated native protocol frame"
    );
    Ok(Some(
        serde_json::from_slice(&data).context("invalid native protocol JSON")?,
    ))
}

/// Once stdin is submitted, every failure is a hard error. Never silently run
/// the same command on Linux after an unknown Windows result.
pub fn execute(prepared: Prepared, run: &str) -> Result<NativeResult> {
    let stage = crate::remote_hosted::dir()?.join(format!("artifacts-{run}"));
    let mut receiver = Receiver::new(run, stage.clone(), &crate::paths::runs_dir()?.join(run))?;
    let result = (|| -> Result<NativeResult> {
        let mut child = prepared
            .target
            .command()
            .arg(agent_command(false))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("starting native Windows SSH transport")?;
        let mut stdin = child.stdin.take().context("native SSH stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().context("native SSH stdout")?);
        // Drain output concurrently with upload to avoid pipe deadlock if an
        // executor rejects a large request early.
        let transfer = std::thread::spawn(move || -> std::io::Result<()> {
            stdin.write_all(&prepared.payload)
        });
        let mut out = std::io::stdout().lock();
        let mut err = std::io::stderr().lock();
        let receive = (|| -> Result<()> {
            while let Some(frame) = read_frame(&mut stdout)? {
                receiver.frame(frame, &mut out, &mut err)?;
            }
            Ok(())
        })();
        if receive.is_err() {
            let _ = child.kill();
        }
        let status = child.wait()?;
        let sent = transfer
            .join()
            .map_err(|_| anyhow::anyhow!("native source upload worker panicked"))?;
        receive?;
        sent.context("uploading native source snapshot")?;
        let result = receiver.result.take().context(
            "native Windows transport ended without a terminal result, build status unknown",
        )?;
        ensure!(
            status.code() == Some(result.code) && result.code != 255,
            "native Windows SSH exit disagrees with result, build status unknown"
        );
        artifact_requirements(
            result.code,
            prepared.require_artifact,
            prepared.require_executable,
            receiver.paths.len(),
            receiver.executables,
        )?;
        if result.code == 0 && !receiver.paths.is_empty() {
            let parent = prepared
                .artifacts
                .parent()
                .context("native artifact destination parent")?;
            std::fs::create_dir_all(parent)?;
            ensure!(
                !prepared.artifacts.exists(),
                "native artifact destination already exists"
            );
            // Stage and destination can be on different mounts. Copy verified
            // files into a fresh per-run directory, never overwrite Linux output.
            let publishing = parent.join(format!(".{run}.partial"));
            std::fs::create_dir(&publishing)?;
            let published = (|| -> Result<()> {
                for entry in walk_files(&stage)? {
                    let relative = entry.strip_prefix(&stage)?;
                    let destination = publishing.join(relative);
                    std::fs::create_dir_all(destination.parent().context("artifact parent")?)?;
                    std::fs::copy(entry, destination)?;
                }
                std::fs::rename(&publishing, &prepared.artifacts)?;
                Ok(())
            })();
            if published.is_err() {
                let _ = std::fs::remove_dir_all(&publishing);
            }
            published?;
            eprintln!(
                "justrust: Windows artifacts: {}",
                prepared.artifacts.display()
            );
        }
        Ok(result)
    })();
    receiver.record_chunk(false, &[], true);
    receiver.record_chunk(true, &[], true);
    drop(receiver);
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn validate_artifact(path: &Path, data: &[u8]) -> Result<()> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("exe" | "dll") => crate::remote_artifacts::check_windows_for_target(
            data,
            path.extension().is_some_and(|e| e == "exe"),
            TARGET,
        ),
        // ar magic establishes only container format, not the architecture of
        // opaque members. Native target/probe binding provides provenance. Do
        // not extract, link or execute untrusted archive members on the client.
        Some("rlib" | "lib") => {
            ensure!(
                data.starts_with(b"!<arch>\n"),
                "native library artifact is not an ar archive"
            );
            Ok(())
        }
        _ => bail!("unsupported native artifact format"),
    }
}

fn artifact_requirements(
    code: i32,
    any: bool,
    executable: bool,
    files: usize,
    executables: usize,
) -> Result<()> {
    ensure!(
        code != 0 || !any || files != 0,
        "native Windows build succeeded without required artifacts"
    );
    ensure!(
        code != 0 || !executable || executables != 0,
        "native Windows binary build succeeded without required PE executable"
    );
    Ok(())
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "native artifact staging contains a symlink"
            );
            if kind.is_dir() {
                pending.push(entry.path());
            } else {
                ensure!(
                    kind.is_file(),
                    "native artifact staging contains a special file"
                );
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "justrust-native-{}-{}",
                crate::record::new_run_id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn receiver(&self) -> Receiver {
            Receiver::new("test-run", self.0.join("stage"), &self.0.join("run")).unwrap()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn hello() -> Value {
        json!({"type":"hello","schema":1,"platform":"windows","target":TARGET,"transport":TRANSPORT,"run":"test-run"})
    }

    #[test]
    fn base64_is_strict_and_roundtrips_every_tail() {
        for (plain, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), expected);
            assert_eq!(decode(expected).unwrap(), plain.as_bytes());
        }
        for length in 0..1024 {
            let bytes: Vec<u8> = (0..length).map(|i| (i % 256) as u8).collect();
            assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
        }
        for invalid in [
            "A", "====", "=AAA", "AA=A", "Zg=A", "Zh==", "Zm9=", "AAAA\n", "éAA",
        ] {
            assert!(decode(invalid).is_err(), "{invalid}");
        }
        assert!(decode(&"A".repeat(MAX_FRAME)).is_err());
    }

    #[test]
    fn setup_is_explicit_and_rejects_mixed_targets() {
        let strings = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(setup_args(&[]).unwrap(), "linux");
        assert_eq!(
            setup_args(&strings(&["--platform=windows", "--target", TARGET])).unwrap(),
            "windows"
        );
        for args in [
            vec!["windows"],
            vec!["--platform", "macos"],
            vec!["--platform"],
            vec!["--target", TARGET],
            vec![
                "--platform",
                "windows",
                "--target",
                "x86_64-unknown-linux-gnu",
            ],
            vec!["--platform", "windows", "--platform", "linux"],
        ] {
            assert!(setup_args(&strings(&args)).is_err(), "{args:?}");
        }
        assert!(!platform_value("linux").unwrap());
        assert!(platform_value("windows").unwrap());
        assert!(platform_value("Linux").is_err());
    }

    #[test]
    fn target_is_inserted_before_test_separator_and_never_executes_linux() {
        let args = ["--target", TARGET, "--", "--target", "application-value"].map(String::from);
        assert_eq!(
            native_args("test", &args).unwrap(),
            [
                "test",
                "--target",
                TARGET,
                "--",
                "--target",
                "application-value"
            ]
        );
        for sub in ["run", "rustc", "cargo", "zigbuild"] {
            assert!(native_args(sub, &[]).is_err());
        }
        for arg in [
            "--target-dir=/tmp/local",
            "--manifest-path=../other",
            "--config=build.rustc='evil'",
            "--target=x86_64-unknown-linux-gnu",
        ] {
            assert!(native_args("build", &[arg.into()]).is_err(), "{arg}");
        }
    }

    #[test]
    fn paths_reject_win32_aliases_devices_and_traversal() {
        assert_eq!(
            wire_path(Path::new("src/a file.rs")).unwrap(),
            "src/a file.rs"
        );
        for path in [
            "",
            "/absolute",
            "../escape",
            "src/../escape",
            "src/NUL.rs",
            "src/com1",
            "a/b.",
            "a/b ",
            "a/C:/file",
            "a\\b",
            "a?b",
            "a\nb",
            "CONIN$",
        ] {
            assert!(wire_path(Path::new(path)).is_err(), "{path:?}");
        }
        for path in [
            ".env",
            "a/.env.local",
            ".aws/credentials",
            "a/.ssh/id_rsa",
            ".cargo/credentials.toml",
        ] {
            assert!(sensitive_path(Path::new(path)), "{path}");
        }
        assert_ne!(
            project_key(Path::new("/one")),
            project_key(Path::new("/two"))
        );
        assert_eq!(project_key(Path::new("/one")).len(), 64);
    }

    #[test]
    fn bundle_preserves_sibling_dependencies_and_filters_secrets() {
        let scratch = Scratch::new();
        let app = scratch.0.join("app");
        let dep = scratch.0.join("dep");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir(&dep).unwrap();
        std::fs::write(
            app.join("Cargo.toml"),
            "[package]\nname='app'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(app.join(".env"), "DO_NOT_UPLOAD=private").unwrap();
        std::fs::write(
            dep.join("Cargo.toml"),
            "[package]\nname='dep'\nversion='0.1.0'\n",
        )
        .unwrap();
        let request = bundle("check", &[], "test-run", &app, &[app.clone(), dep]).unwrap();
        assert_eq!(request["cwd"], "app");
        let paths: Vec<_> = request["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["path"].as_str().unwrap())
            .collect();
        assert!(
            paths.contains(&"app/Cargo.toml")
                && paths.contains(&"dep/Cargo.toml")
                && paths.contains(&"app/src/main.rs")
        );
        assert!(
            !serde_json::to_string(&request)
                .unwrap()
                .contains("DO_NOT_UPLOAD")
        );
        assert!(!paths.contains(&"app/.env"));
        std::fs::create_dir(app.join("Src")).unwrap();
        std::fs::write(app.join("Src/other.rs"), "").unwrap();
        assert!(bundle("check", &[], "test-run", &app, std::slice::from_ref(&app)).is_err());
    }

    #[test]
    fn native_frames_are_bounded_and_require_newlines() {
        assert!(read_frame(&mut &b"{\"type\":\"exit\",\"code\":0}"[..]).is_err());
        assert!(read_frame(&mut &b"not json\n"[..]).is_err());
        assert!(read_frame(&mut &vec![b' '; MAX_FRAME + 1][..]).is_err());
        assert!(read_frame(&mut &b""[..]).unwrap().is_none());
        assert_eq!(
            read_frame(&mut &b"{\"type\":\"exit\",\"code\":1}\r\n"[..])
                .unwrap()
                .unwrap()["code"],
            1
        );
    }

    #[test]
    fn stream_requires_platform_bound_hello_and_terminal_exit() {
        let scratch = Scratch::new();
        let mut receiver = scratch.receiver();
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert!(
            receiver
                .frame(json!({"type":"exit","code":0}), &mut out, &mut err)
                .is_err()
        );
        let mut wrong = hello();
        wrong["target"] = json!("x86_64-unknown-linux-gnu");
        assert!(receiver.frame(wrong, &mut out, &mut err).is_err());
        receiver.frame(hello(), &mut out, &mut err).unwrap();
        receiver
            .frame(
                json!({"type":"stdout","data":encode(b"build output\n")}),
                &mut out,
                &mut err,
            )
            .unwrap();
        receiver
            .frame(
                json!({"type":"stderr","data":encode(b"error details\n")}),
                &mut out,
                &mut err,
            )
            .unwrap();
        assert_eq!(out, b"build output\n");
        assert_eq!(err, b"error details\n");
        assert!(receiver.result.is_none());
        receiver
            .frame(json!({"type":"exit","code":101}), &mut out, &mut err)
            .unwrap();
        assert_eq!(receiver.result.as_ref().unwrap().code, 101);
        assert!(receiver.frame(hello(), &mut out, &mut err).is_err());
        let log = std::fs::read_to_string(scratch.0.join("run/output.jsonl")).unwrap();
        assert!(log.contains("build output") && log.contains("error details"));
    }

    #[test]
    fn artifacts_reject_traversal_wrong_scope_duplicates_and_truncation() {
        let scratch = Scratch::new();
        let mut receiver = scratch.receiver();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        receiver.frame(hello(), &mut out, &mut err).unwrap();
        for path in [
            "../evil.exe",
            "debug/a.exe",
            "x86_64-unknown-linux-gnu/debug/a.exe",
            "x86_64-pc-windows-msvc/debug/NUL.exe",
            "x86_64-pc-windows-msvc/debug/a.sh",
        ] {
            assert!(receiver.frame(json!({"type":"artifact-start","path":path,"bytes":4,"sha256":"a".repeat(64)}), &mut out, &mut err).is_err(), "{path}");
        }
        receiver.frame(json!({"type":"artifact-start","path":format!("{TARGET}/debug/a.exe"),"bytes":4,"sha256":"a".repeat(64)}), &mut out, &mut err).unwrap();
        assert!(
            receiver
                .frame(json!({"type":"exit","code":0}), &mut out, &mut err)
                .is_err()
        );
        receiver
            .frame(
                json!({"type":"artifact-data","data":encode(b"xx")}),
                &mut out,
                &mut err,
            )
            .unwrap();
        assert!(
            receiver
                .frame(json!({"type":"artifact-end"}), &mut out, &mut err)
                .is_err()
        );
        assert!(receiver.frame(json!({"type":"artifact-start","path":format!("{TARGET}/DEBUG/A.exe"),"bytes":4,"sha256":"a".repeat(64)}), &mut out, &mut err).is_err());
    }

    #[test]
    fn pure_library_build_returns_checksum_verified_opaque_archive_with_code_zero() {
        let scratch = Scratch::new();
        let mut receiver = scratch.receiver();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let archive = b"!<arch>\n";
        let source = scratch.0.join("library.rlib");
        std::fs::write(&source, archive).unwrap();
        let sha256 = crate::toolchain::sha256(&source).unwrap();
        receiver.frame(hello(), &mut out, &mut err).unwrap();
        let name = format!("{TARGET}/debug/library.rlib");
        receiver
            .frame(
                json!({"type":"artifact-start","path":name,"bytes":archive.len(),"sha256":sha256}),
                &mut out,
                &mut err,
            )
            .unwrap();
        for chunk in archive.chunks(3) {
            receiver
                .frame(
                    json!({"type":"artifact-data","data":encode(chunk)}),
                    &mut out,
                    &mut err,
                )
                .unwrap();
        }
        receiver
            .frame(json!({"type":"artifact-end"}), &mut out, &mut err)
            .unwrap();
        receiver
            .frame(json!({"type":"exit","code":0}), &mut out, &mut err)
            .unwrap();
        assert_eq!(receiver.result.as_ref().unwrap().code, 0);
        assert_eq!(std::fs::read(receiver.stage.join(name)).unwrap(), archive);
        assert_eq!(receiver.executables, 0);
        artifact_requirements(0, true, false, receiver.paths.len(), 0).unwrap();
        assert!(artifact_requirements(0, true, true, receiver.paths.len(), 0).is_err());
        assert!(artifact_requirements(0, true, false, 0, 0).is_err());
        for extension in ["rlib", "lib"] {
            validate_artifact(Path::new(&format!("library.{extension}")), archive).unwrap();
            assert!(
                validate_artifact(
                    Path::new(&format!("library.{extension}")),
                    b"not an archive"
                )
                .is_err()
            );
        }
        assert!(validate_artifact(Path::new("library.exe"), archive).is_err());
        assert!(validate_artifact(Path::new("library.pdb"), archive).is_err());
    }

    #[test]
    fn missing_recorder_or_incompatible_optional_summary_does_not_change_exit() {
        let scratch = Scratch::new();
        let log_dir = scratch.0.join("not-a-directory");
        std::fs::write(&log_dir, "existing file").unwrap();
        let mut receiver = Receiver::new("test-run", scratch.0.join("stage"), &log_dir).unwrap();
        assert!(receiver.output.is_none());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        receiver.frame(hello(), &mut out, &mut err).unwrap();
        receiver
            .frame(
                json!({"type":"stdout","data":encode(b"native output\n")}),
                &mut out,
                &mut err,
            )
            .unwrap();
        receiver.frame(json!({"type":"exit","code":101,"summary":{"id":"test-run","exit":101,"unknown-future-schema":true}}), &mut out, &mut err).unwrap();
        let result = receiver.result.as_ref().unwrap();
        assert_eq!(result.code, 101);
        assert!(result.summary.is_none());
        assert_eq!(out, b"native output\n");
    }

    #[test]
    fn output_journal_keeps_utf8_split_across_protocol_chunks() {
        let scratch = Scratch::new();
        let mut receiver = scratch.receiver();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        receiver.frame(hello(), &mut out, &mut err).unwrap();
        let bytes = "compiling 🦀\nno newline".as_bytes();
        for byte in bytes {
            receiver
                .frame(
                    json!({"type":"stdout","data":encode(&[*byte])}),
                    &mut out,
                    &mut err,
                )
                .unwrap();
        }
        receiver
            .frame(json!({"type":"exit","code":0}), &mut out, &mut err)
            .unwrap();
        let log = std::fs::read_to_string(scratch.0.join("run/output.jsonl")).unwrap();
        let records: Vec<crate::record::OutputLine> = log
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].l, "compiling 🦀");
        assert_eq!(records[1].l, "no newline");
        assert_eq!(out, bytes);
    }

    fn ssh_string(out: &mut Vec<u8>, value: &[u8]) {
        out.extend((value.len() as u32).to_be_bytes());
        out.extend(value);
    }
    fn certificate(key: &[u8], after: u64, before: u64, cert_type: u32) -> (String, String) {
        let mut public = Vec::new();
        ssh_string(&mut public, b"ssh-ed25519");
        ssh_string(&mut public, key);
        let mut cert = Vec::new();
        ssh_string(&mut cert, b"ssh-ed25519-cert-v01@openssh.com");
        ssh_string(&mut cert, b"nonce");
        ssh_string(&mut cert, key);
        cert.extend(1u64.to_be_bytes());
        cert.extend(cert_type.to_be_bytes());
        ssh_string(&mut cert, b"test-key");
        let mut principals = Vec::new();
        ssh_string(&mut principals, b"jr-account-abc123");
        ssh_string(&mut cert, &principals);
        cert.extend(after.to_be_bytes());
        cert.extend(before.to_be_bytes());
        for value in [b"".as_slice(), b"", b"", b"signing-key", b"signature"] {
            ssh_string(&mut cert, value);
        }
        (
            format!("ssh-ed25519-cert-v01@openssh.com {} test", encode(&cert)),
            format!("ssh-ed25519 {}", encode(&public)),
        )
    }

    #[test]
    fn certificates_bind_key_type_and_short_validity() {
        let (cert, public) = certificate(&[42; 32], 980, 1300, 1);
        verify_certificate(&cert, &public, 1000).unwrap();
        let (_, wrong) = certificate(&[43; 32], 980, 1300, 1);
        assert!(verify_certificate(&cert, &wrong, 1000).is_err());
        for (after, before, kind) in [
            (980, 999, 1),
            (1100, 1300, 1),
            (980, 2000, 1),
            (980, 1300, 2),
        ] {
            let (cert, public) = certificate(&[42; 32], after, before, kind);
            assert!(verify_certificate(&cert, &public, 1000).is_err());
        }
        assert!(verify_certificate(&format!("{cert}\n"), &public, 1000).is_err());
        let wire = decode(cert.split_whitespace().nth(1).unwrap()).unwrap();
        for len in 0..wire.len() {
            let truncated = format!("ssh-ed25519-cert-v01@openssh.com {}", encode(&wire[..len]));
            assert!(verify_certificate(&truncated, &public, 1000).is_err());
        }
    }

    #[test]
    fn agent_command_only_contains_fixed_file_invocation() {
        for probe in [true, false] {
            let cmd = agent_command(probe);
            let expected = r"powershell.exe -NoLogo -NoProfile -NonInteractive -File C:\ProgramData\Justrust\hosted-agent.ps1";
            assert_eq!(
                cmd,
                format!("{expected}{}", if probe { " -Probe" } else { "" })
            );
            assert!(!cmd.contains("EncodedCommand"));
            assert!(!cmd.contains(['&', ';', '|', '<', '>', '%', '^', '\n', '\r']));
        }
    }

    #[test]
    fn agent_stdin_json_preserves_non_ascii_arguments() {
        let scratch = Scratch::new();
        std::fs::write(
            scratch.0.join("Cargo.toml"),
            "[package]\nname='app'\nversion='0.1.0'\n",
        )
        .unwrap();
        let argument = "日本語 café 🦀 & echo %PATH% | 'quoted'";
        let args = ["--".to_owned(), argument.to_owned()];
        let request = bundle("test", &args, "utf8-run", &scratch.0, &[scratch.0.clone()]).unwrap();
        // prepare serializes the bundle this way, and execute writes those
        // bytes directly to SSH stdin, without including them in the command.
        let payload = serde_json::to_vec(&request).unwrap();
        assert!(std::str::from_utf8(&payload).unwrap().contains(argument));
        assert_eq!(serde_json::from_slice::<Value>(&payload).unwrap(), request);
        assert!(!agent_command(false).contains(argument));
    }
}
