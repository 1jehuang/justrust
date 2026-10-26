//! Bring the outputs of a remote `build` back to this machine (FINDINGS 45).
//!
//! After a successful remote `build`, the client runs `justrust __artifacts`
//! on the machine over the shared ssh connection. It lists the uplifted
//! artifacts in the profile dir (`target/<profile>/` executables, shared
//! libraries, and `examples/`), skips the ones this machine already has,
//! and streams the rest as zstd frames. Each is a `--patch-from` delta
//! against the copy the machine sent last time when this machine still has
//! that copy unchanged, else a full zstd copy. A one-line edit to a 10 MB
//! release binary comes back as ~2 MB in ~0.4 s.
//!
//! ```text
//! ~/.justrust/remote/fetched.json   client: path -> {id, size, mtime} of each file it wrote
//! ~/.justrust/sent/<path key>       machine: the last copy sent of each artifact
//! ~/.justrust/sent/<path key>.id    machine: its id (size-mtime_ns on the machine)
//! ```
//!
//! Safety: names are checked to stay inside the profile dir, every file is
//! verified (zstd content checksum, size) and checked with `ldd` for
//! missing libraries or too-new glibc before it atomically replaces the
//! local file. Any failure is an error: the router then builds locally.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// Hidden subcommand the machine runs (`justrust __artifacts`).
pub const ARG: &str = "__artifacts";
/// Clock slack between this machine and the build machine.
const SLACK_SECS: f64 = 5.0;

#[derive(Serialize, Deserialize, Debug)]
struct Request {
    cwd: String,
    /// The cargo args of the build, subcommand first.
    args: Vec<String>,
    /// When the client started the build (Unix seconds).
    start: f64,
    have: Vec<Have>,
}

/// An artifact the client already has.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Have {
    name: String,
    /// The machine id it was fetched as, when the file is unchanged since.
    id: Option<String>,
    mtime: f64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "t")]
enum Frame {
    Hello {
        arch: String,
        dir: String,
    },
    File {
        name: String,
        id: String,
        mode: u32,
        size: u64,
        patch: bool,
        len: u64,
    },
    End,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Record {
    id: String,
    size: u64,
    mtime: f64,
}

fn mtime_of(m: &std::fs::Metadata) -> f64 {
    crate::sys::mtime_f64(m)
}

fn id_of(m: &std::fs::Metadata) -> String {
    let (s, ns) = crate::sys::mtime(m);
    format!("{}-{s}{ns:09}", m.len())
}

/// `foo-0123456789abcdef` (cargo's hashed copies in `examples/`).
fn is_hashed(n: &str) -> bool {
    let stem = n.split('.').next().unwrap_or(n);
    stem.rsplit_once('-')
        .is_some_and(|(_, h)| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// An uplifted artifact worth bringing back: an executable or a shared
/// library, not a dep-info file, rlib, or our own temp files.
fn artifact_name(n: &str, mode: u32) -> bool {
    if n.starts_with('.')
        || [".d", ".rlib", ".rmeta", ".a", ".jrtmp", ".zst"]
            .iter()
            .any(|e| n.ends_with(e))
    {
        return false;
    }
    n.ends_with(".so") || n.contains(".so.") || mode & 0o111 != 0
}

/// Artifacts in a profile dir: top-level files and `examples/`.
fn candidates(dir: &Path) -> Vec<(String, std::fs::Metadata)> {
    let mut out = Vec::new();
    for sub in ["", "examples"] {
        let d = if sub.is_empty() {
            dir.to_path_buf()
        } else {
            dir.join(sub)
        };
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(m) = std::fs::metadata(e.path()) else {
                continue;
            };
            let n = e.file_name().to_string_lossy().into_owned();
            if !m.is_file()
                || !artifact_name(&n, crate::sys::mode(&m, &e.path()))
                || (!sub.is_empty() && is_hashed(&n))
            {
                continue;
            }
            out.push((
                if sub.is_empty() {
                    n
                } else {
                    format!("{sub}/{n}")
                },
                m,
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// What the machine should send: missing here or newer than the client's copy.
/// Cargo may reuse an old output on the host for a fresh CI checkout.
fn needed(have: Option<&Have>, rid: &str, rmtime: f64, start: f64) -> bool {
    if have.and_then(|h| h.id.as_deref()) == Some(rid) {
        return false;
    }
    have.is_none()
        || rmtime >= start - SLACK_SECS
        || have.is_some_and(|h| h.id.is_some() || h.mtime < rmtime)
}

/// A relative name that stays inside the profile dir.
fn safe_name(n: &str) -> bool {
    let p = Path::new(n);
    !n.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

fn write_frame(w: &mut impl Write, f: &Frame, data: &[u8]) -> Result<()> {
    serde_json::to_writer(&mut *w, f)?;
    w.write_all(b"\n")?;
    w.write_all(data)?;
    Ok(())
}

fn profile_dir(cwd: &Path, args: &[String]) -> Result<PathBuf> {
    let os: Vec<OsString> = args.iter().map(OsString::from).collect();
    let target = crate::slots::target_dir_in_result(cwd, &os)
        .context("could not resolve the target dir (cargo metadata)")?;
    Ok(target.join(crate::slots::profile_dir(&os)))
}

/// Cargo's explicit target selector, before any trailing command arguments.
/// Empty means malformed or multiple selectors, which routing must reject.
pub(crate) fn build_target(args: &[String]) -> Option<&str> {
    let mut args = args.iter().take_while(|a| a.as_str() != "--");
    let mut target = None;
    while let Some(arg) = args.next() {
        if arg == "--target" {
            if target.is_some() {
                return Some("");
            }
            target = Some(args.next().map(String::as_str).unwrap_or(""));
        } else if let Some(value) = arg.strip_prefix("--target=") {
            if target.is_some() {
                return Some("");
            }
            target = Some(value);
        }
    }
    target
}

pub(crate) fn musl_target(target: &str) -> bool {
    matches!(
        target,
        "x86_64-unknown-linux-musl" | "aarch64-unknown-linux-musl"
    )
}

/// Do not execute cross binaries or trust `ldd`'s success on the wrong CPU.
/// Read the ELF program headers, including stripped/static-PIE binaries.
fn check_static_musl(data: &[u8], target: &str) -> Result<()> {
    use object::read::elf::{Dyn, ElfFile64, FileHeader, ProgramHeader};
    let expected = match target {
        "x86_64-unknown-linux-musl" => object::elf::EM_X86_64,
        "aarch64-unknown-linux-musl" => object::elf::EM_AARCH64,
        _ => bail!("unsupported musl artifact target {target}"),
    };
    let elf =
        ElfFile64::<object::Endianness>::parse(data).context("expected ELF64 musl artifact")?;
    let endian = elf.endian();
    if elf.elf_header().e_machine(endian) != expected {
        bail!("artifact architecture does not match {target}");
    }
    if !matches!(
        elf.elf_header().e_type(endian),
        object::elf::ET_EXEC | object::elf::ET_DYN
    ) {
        bail!("musl artifact is not an executable ELF");
    }
    if elf.elf_program_headers().is_empty() {
        bail!("musl artifact has no program headers");
    }
    for header in elf.elf_program_headers() {
        if header.p_type(endian) == object::elf::PT_INTERP {
            bail!("musl release artifact has a dynamic interpreter");
        }
        if let Some(dynamic) = header.dynamic(endian, data)?
            && dynamic
                .iter()
                .any(|entry| entry.tag32(endian) == Some(object::elf::DT_NEEDED))
        {
            bail!("musl release artifact depends on shared libraries");
        }
    }
    Ok(())
}

/// Machine side: `justrust __artifacts`, request on stdin, frames on stdout.
pub fn serve_main() -> ! {
    let code = match serve(std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("justrust artifacts: {e:#}");
            1
        }
    };
    std::process::exit(code)
}

fn serve(mut inp: impl Read, out: impl Write) -> Result<()> {
    let mut s = String::new();
    inp.read_to_string(&mut s)?;
    let req: Request = serde_json::from_str(&s).context("bad request")?;
    let dir = profile_dir(Path::new(&req.cwd), &req.args)?;
    let sent_root = crate::paths::home()?.join("sent");
    std::fs::create_dir_all(&sent_root)?;
    serve_dir(&dir, &req, &sent_root, out)
}

fn serve_dir(dir: &Path, req: &Request, sent_root: &Path, out: impl Write) -> Result<()> {
    let mut out = BufWriter::new(out);
    write_frame(
        &mut out,
        &Frame::Hello {
            arch: std::env::consts::ARCH.into(),
            dir: dir.display().to_string(),
        },
        &[],
    )?;
    let have: BTreeMap<&str, &Have> = req.have.iter().map(|h| (h.name.as_str(), h)).collect();
    for (name, m) in candidates(dir) {
        let rid = id_of(&m);
        let h = have.get(name.as_str()).copied();
        if !needed(h, &rid, mtime_of(&m), req.start) {
            continue;
        }
        let path = dir.join(&name);
        let key = path.display().to_string().replace('/', "%");
        let sent = sent_root.join(&key);
        let sent_id = sent_root.join(format!("{key}.id"));
        let patch = h
            .and_then(|h| h.id.as_deref())
            .is_some_and(|id| std::fs::read_to_string(&sent_id).ok().as_deref() == Some(id))
            && sent.is_file();
        // Snapshot first: the build tree may be rewritten under us, and the
        // delta base must be exactly what was sent.
        let snap = sent_root.join(format!("{key}.new"));
        std::fs::copy(&path, &snap).with_context(|| format!("copy {}", path.display()))?;
        let zst = sent_root.join(format!("{key}.zst"));
        let mut c = Command::new("zstd");
        c.args(["-q", "-f", "-3", "-T0", "--long=31"]);
        if patch {
            c.arg(format!("--patch-from={}", sent.display()));
        }
        let st = c
            .arg(&snap)
            .arg("-o")
            .arg(&zst)
            .stdin(Stdio::null())
            .status()
            .context("zstd on the build machine")?;
        if !st.success() {
            bail!("zstd failed on {name}");
        }
        let data = std::fs::read(&zst)?;
        let _ = std::fs::remove_file(&zst);
        write_frame(
            &mut out,
            &Frame::File {
                name: name.clone(),
                id: rid.clone(),
                mode: crate::sys::mode(&m, &path) & 0o7777,
                size: m.len(),
                patch,
                len: data.len() as u64,
            },
            &data,
        )?;
        std::fs::rename(&snap, &sent)?;
        std::fs::write(&sent_id, &rid)?;
    }
    write_frame(&mut out, &Frame::End, &[])?;
    out.flush()?;
    Ok(())
}

fn records_path() -> Result<PathBuf> {
    Ok(crate::remote::dir()?.join("fetched.json"))
}

fn load_records() -> BTreeMap<String, Record> {
    records_path()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn have_list(dir: &Path, records: &BTreeMap<String, Record>) -> Vec<Have> {
    candidates(dir)
        .into_iter()
        .map(|(name, m)| {
            let p = dir.join(&name).display().to_string();
            let id = records
                .get(&p)
                .filter(|r| r.size == m.len() && r.mtime == mtime_of(&m))
                .map(|r| r.id.clone());
            Have {
                name,
                id,
                mtime: mtime_of(&m),
            }
        })
        .collect()
}

/// Missing shared libraries or symbol versions (a machine with a newer
/// glibc) make the binary useless here. `ldd` prints "not found" for both.
fn check_libs(p: &Path) -> Result<()> {
    let Ok(o) = Command::new("ldd").arg(p).stdin(Stdio::null()).output() else {
        return Ok(());
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let missing: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("not found"))
        .map(str::trim)
        .collect();
    if !missing.is_empty() {
        bail!(
            "the remote binary does not run here: {}",
            missing.join("; ")
        );
    }
    Ok(())
}

fn apply(
    dir: &Path,
    name: &str,
    mode: u32,
    size: u64,
    patch: bool,
    data: &[u8],
    musl: Option<&str>,
) -> Result<PathBuf> {
    if !safe_name(name) {
        bail!("unsafe artifact name {name:?}");
    }
    let dst = dir.join(name);
    let parent = dst.parent().context("no parent")?;
    std::fs::create_dir_all(parent)?;
    let file = dst.file_name().context("no file name")?.to_string_lossy();
    let tmp = parent.join(format!(".{file}.jrtmp"));
    let zst = parent.join(format!(".{file}.zst"));
    std::fs::write(&zst, data)?;
    let mut c = Command::new("zstd");
    c.args(["-q", "-d", "-f", "--long=31"]);
    if patch {
        c.arg(format!("--patch-from={}", dst.display()));
    }
    let st = c
        .arg(&zst)
        .arg("-o")
        .arg(&tmp)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = std::fs::remove_file(&zst);
    let res = (|| {
        if !st.context("zstd")?.success() {
            bail!(
                "could not decode {name}{}",
                if patch { " (delta)" } else { "" }
            );
        }
        let got = std::fs::metadata(&tmp)?.len();
        if got != size {
            bail!("{name}: {got} bytes, expected {size}");
        }
        crate::sys::set_mode(&tmp, mode)?;
        if let Some(target) = musl {
            check_static_musl(&std::fs::read(&tmp)?, target)?;
        } else if mode & 0o111 != 0 || name.contains(".so") {
            check_libs(&tmp)?;
        }
        std::fs::rename(&tmp, &dst)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map(|()| dst)
}

/// Client side: bring back what the remote `build` (args, subcommand
/// first) produced. Returns a one-line summary.
pub fn fetch(args: &[String], start: f64) -> Result<String> {
    let t0 = Instant::now();
    let cwd = std::env::current_dir()?;
    let dir = profile_dir(&cwd, args)?;
    let musl = (args.first().map(String::as_str) == Some("zigbuild"))
        .then(|| build_target(args).filter(|target| musl_target(target)))
        .flatten();
    let mut records = load_records();
    // A newer locally built file is not evidence that it is this hosted
    // release. Only reuse files with known remote provenance for musl.
    let have = have_list(&dir, &records)
        .into_iter()
        .filter(|h| musl.is_none() || h.id.is_some())
        .collect();
    let backend = crate::remote_backend::load().context("no remote machine configured")?;
    let t = backend.ensure_ready()?;
    let req = Request {
        cwd: cwd.display().to_string(),
        args: args.to_vec(),
        start,
        have,
    };
    let mut child = t
        .command()
        .arg(format!("~/.cargo/bin/justrust {ARG}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("ssh")?;
    {
        let mut si = child.stdin.take().context("stdin")?;
        serde_json::to_writer(&mut si, &req)?;
    }
    let mut r = BufReader::new(child.stdout.take().context("stdout")?);
    let mut stderr = child.stderr.take().context("stderr")?;
    let mut files = 0;
    let mut patched = 0;
    let mut bytes = 0u64;
    let res = (|| -> Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            if r.read_line(&mut line)? == 0 {
                bail!("the machine closed the artifact stream early");
            }
            match serde_json::from_str::<Frame>(&line).context("bad artifact frame")? {
                Frame::Hello { arch, .. } => {
                    if musl.is_none() && arch != std::env::consts::ARCH {
                        bail!(
                            "the machine builds for {arch}, this machine is {}",
                            std::env::consts::ARCH
                        );
                    }
                }
                Frame::File {
                    name,
                    id,
                    mode,
                    size,
                    patch,
                    len,
                } => {
                    let mut data = vec![0; len as usize];
                    r.read_exact(&mut data)?;
                    let dst = apply(&dir, &name, mode, size, patch, &data, musl)?;
                    let m = std::fs::metadata(&dst)?;
                    records.insert(
                        dst.display().to_string(),
                        Record {
                            id,
                            size: m.len(),
                            mtime: mtime_of(&m),
                        },
                    );
                    files += 1;
                    patched += patch as usize;
                    bytes += len;
                }
                Frame::End => return Ok(()),
            }
        }
    })();
    let mut err = String::new();
    let _ = stderr.read_to_string(&mut err);
    let st = child.wait()?;
    if let Ok(p) = records_path() {
        let _ = std::fs::write(p, serde_json::to_vec(&records)?);
    }
    res.map_err(|e| {
        let err = err.trim();
        if err.is_empty() {
            e
        } else {
            e.context(err.to_string())
        }
    })?;
    if !st.success() {
        bail!("{}", err.trim());
    }
    Ok(if files == 0 {
        format!(
            "artifacts already up to date here ({:.2}s)",
            t0.elapsed().as_secs_f64()
        )
    } else {
        format!(
            "brought back {files} artifact{} ({:.1} MB, {patched} as delta) in {:.2}s",
            if files == 1 { "" } else { "s" },
            bytes as f64 / 1e6,
            t0.elapsed().as_secs_f64()
        )
    })
}

/// `zstd` is needed on this machine to unpack artifacts.
pub fn available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        Command::new("zstd")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elf_fixture(machine: u16, kind: u32, needed: bool) -> Vec<u8> {
        let mut bytes = vec![0; 136];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16..18].copy_from_slice(&object::elf::ET_EXEC.to_le_bytes());
        bytes[18..20].copy_from_slice(&machine.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
        bytes[64..68].copy_from_slice(&kind.to_le_bytes());
        bytes[72..80].copy_from_slice(&120u64.to_le_bytes());
        bytes[96..104].copy_from_slice(&16u64.to_le_bytes());
        if needed {
            bytes[120..128].copy_from_slice(&(object::elf::DT_NEEDED as u64).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn static_musl_requires_matching_elf_without_dynamic_dependencies() {
        for (target, machine) in [
            ("x86_64-unknown-linux-musl", object::elf::EM_X86_64),
            ("aarch64-unknown-linux-musl", object::elf::EM_AARCH64),
        ] {
            assert!(
                check_static_musl(&elf_fixture(machine, object::elf::PT_LOAD, false), target)
                    .is_ok()
            );
            assert!(
                check_static_musl(
                    &elf_fixture(machine, object::elf::PT_DYNAMIC, false),
                    target
                )
                .is_ok()
            );
            assert!(
                check_static_musl(&elf_fixture(machine, object::elf::PT_INTERP, false), target)
                    .is_err()
            );
            assert!(
                check_static_musl(&elf_fixture(machine, object::elf::PT_DYNAMIC, true), target)
                    .is_err()
            );
            assert!(
                check_static_musl(
                    &elf_fixture(object::elf::EM_ARM, object::elf::PT_LOAD, false),
                    target
                )
                .is_err()
            );
            assert!(check_static_musl(b"not ELF", target).is_err());
        }
    }

    #[test]
    fn explicit_target_forms_and_separator() {
        let s = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(
            build_target(&s(&["build", "--target", "x86_64-unknown-linux-gnu"])),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(
            build_target(&s(&["build", "--target=x86_64-unknown-linux-gnu"])),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(build_target(&s(&["build", "--", "--target=ignored"])), None);
        assert_eq!(build_target(&s(&["build", "--target"])), Some(""));
        assert_eq!(
            build_target(&s(&["build", "--target=a", "--target=b"])),
            Some("")
        );
        assert_eq!(
            build_target(&s(&["build", "--target", "a", "--target=a"])),
            Some("")
        );
    }

    fn have(id: Option<&str>, mtime: f64) -> Have {
        Have {
            name: "x".into(),
            id: id.map(str::to_string),
            mtime,
        }
    }

    #[test]
    fn names() {
        assert!(artifact_name("justrust", 0o755));
        assert!(artifact_name("libfoo.so", 0o644));
        assert!(!artifact_name("justrust.d", 0o755));
        assert!(!artifact_name("libfoo.rlib", 0o644));
        assert!(!artifact_name("notes.txt", 0o644));
        assert!(!artifact_name(".justrust.jrtmp", 0o755));
        assert!(is_hashed("demo-0123456789abcdef"));
        assert!(is_hashed("libdemo-0123456789abcdef.so"));
        assert!(!is_hashed("demo"));
        assert!(!is_hashed("my-tool"));
        assert!(safe_name("examples/demo"));
        assert!(!safe_name("../x"));
        assert!(!safe_name("/etc/passwd"));
        assert!(!safe_name(""));
    }

    #[test]
    fn needed_rules() {
        // Same id: never.
        assert!(!needed(Some(&have(Some("a"), 1.0)), "a", 100.0, 100.0));
        // Built by this run: always.
        assert!(needed(None, "a", 100.0, 100.0));
        // Fresh CI runner must receive cached output even when Cargo did no work.
        assert!(needed(None, "a", 10.0, 100.0));
        // Client copy came from us and differs: send.
        assert!(needed(Some(&have(Some("b"), 50.0)), "a", 10.0, 100.0));
        // Client built it itself, older than ours: send. Newer: keep.
        assert!(needed(Some(&have(None, 5.0)), "a", 10.0, 100.0));
        assert!(!needed(Some(&have(None, 20.0)), "a", 10.0, 100.0));
    }

    /// Full round trip through serve and apply, then a delta against the
    /// copy sent the first time. Needs `zstd`. Unix only: artifacts are
    /// picked by their executable bit, which Windows files do not have.
    #[cfg(unix)]
    #[test]
    fn round_trip_full_then_delta() {
        if !available() {
            return;
        }
        let root = std::env::temp_dir().join(format!("jr-art-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (remote, local, sent) = (root.join("r"), root.join("l"), root.join("sent"));
        for d in [&remote, &local, &sent] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mut body: Vec<u8> = (0..400_000u32)
            .flat_map(|i| (i * 7).to_le_bytes())
            .collect();
        let write = |p: &Path, b: &[u8]| {
            std::fs::write(p, b).unwrap();
            crate::sys::set_mode(p, 0o755).unwrap();
        };
        write(&remote.join("app"), &body);
        std::fs::write(remote.join("app.d"), b"dep info").unwrap();
        let run = |have: Vec<Have>| -> Vec<(String, u64, bool)> {
            let req = Request {
                cwd: String::new(),
                args: vec![],
                // All outputs predate this request, as in a warm-host CI build.
                start: crate::sys::mtime_f64(&std::fs::metadata(remote.join("app")).unwrap())
                    + 60.0,
                have,
            };
            let mut out = Vec::new();
            serve_dir(&remote, &req, &sent, &mut out).unwrap();
            let mut r = BufReader::new(&out[..]);
            let mut got = Vec::new();
            let mut line = String::new();
            loop {
                line.clear();
                r.read_line(&mut line).unwrap();
                match serde_json::from_str::<Frame>(&line).unwrap() {
                    Frame::Hello { .. } => {}
                    Frame::File {
                        name,
                        id: _,
                        mode,
                        size,
                        patch,
                        len,
                    } => {
                        let mut d = vec![0; len as usize];
                        r.read_exact(&mut d).unwrap();
                        apply(&local, &name, mode, size, patch, &d, None).unwrap();
                        got.push((name, len, patch));
                    }
                    Frame::End => return got,
                }
            }
        };
        let first = run(vec![]);
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(!first[0].2);
        assert_eq!(std::fs::read(local.join("app")).unwrap(), body);
        // The client now holds the sent copy: an unchanged artifact is skipped.
        let id = id_of(&std::fs::metadata(remote.join("app")).unwrap());
        let mt = mtime_of(&std::fs::metadata(local.join("app")).unwrap());
        let h = vec![Have {
            name: "app".into(),
            id: Some(id.clone()),
            mtime: mt,
        }];
        assert!(run(h.clone()).is_empty());
        // A small change comes back as a delta and reproduces the file.
        body[1000..1010].copy_from_slice(b"0123456789");
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&remote.join("app"), &body);
        let second = run(h);
        assert_eq!(second.len(), 1);
        assert!(second[0].2, "{second:?}");
        assert!(second[0].1 < first[0].1 / 10, "{second:?} vs {first:?}");
        assert_eq!(std::fs::read(local.join("app")).unwrap(), body);
        let _ = std::fs::remove_dir_all(&root);
    }
}
