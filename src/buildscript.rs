//! Cached build-script *runs* for non-local packages.
//!
//! Cargo runs build scripts itself, never through `RUSTC`, so the shim cannot
//! intercept a run directly. Instead, when the shim compiles (or restores) a
//! registry/git build script, it moves the real binary to
//! `.jr-real-<name>` and puts a small `/bin/sh` wrapper in its place. Cargo
//! then hardlinks and runs the wrapper, which calls
//! `justrust __build-script <real> ...`, or runs the real script directly when
//! justrust is gone.
//!
//! ```text
//! ~/.justrust/cache/depcache/          (shared with depcache.rs and its eviction)
//!   m/<base>.jsonl     known input sets for one script binary + environment
//!   o/<key>/meta.json  replayed stdout/stderr, OUT_DIR file list, run time
//!   o/<key>/out/...    OUT_DIR contents (reflinks)
//! ```
//!
//! Key, two levels like the rustc cache:
//! - base: scheme, content hash of the real script binary, cwd (manifest dir),
//!   `rustc -vV`, and the environment cargo and the usual C toolchain crates
//!   read (`CARGO_*`, `DEP_*`, `TARGET`, `HOST`, `PROFILE`, `OPT_LEVEL`,
//!   `DEBUG`, `OUT_DIR`, `PATH`, `CC*`/`CFLAGS*`/`AR*`..., `PKG_CONFIG*`,
//!   `RUSTFLAGS`, `RUSTC_*`), with the profile dir normalized, wrapper and
//!   linker paths keyed by content.
//! - result: base + the value of every `rerun-if-env-changed` variable + a
//!   signature of every `rerun-if-changed` path (content hash, or size and
//!   mtime for files inside `CARGO_HOME`, which cargo never rewrites).
//!
//! Only stored when the script is as deterministic as cargo itself assumes:
//! exit 0, at least one `rerun-if-*` directive (scripts without any declare
//! nothing, so cargo reruns them on any package change and they may probe the
//! system freely; they are also cheap: 29 of 76 Desktop scripts, 1.2s of 40s),
//! every `rerun-if-changed` path exists and lies outside the target dir, UTF-8
//! output, only regular files in `OUT_DIR`, and no binary file in `OUT_DIR`
//! containing the target dir (text files are normalized and rewritten).
//!
//! This trusts the same thing cargo trusts in an existing target dir: that a
//! script with `rerun-if` directives only depends on what it declares (plus the
//! toolchain env above). Disable with `JUSTRUST_BUILD_SCRIPT_CACHE=0` or
//! `JUSTRUST_DEPCACHE=0`. Every failure runs the real script.

use crate::depcache::{self, H128, Norm};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCHEME: &str = "justrust-buildscript-1";
const REAL_PREFIX: &str = ".jr-real-";
const WRAPPER_MARK: &str = "# justrust build-script wrapper";

pub fn enabled() -> bool {
    depcache::enabled()
        && !matches!(
            std::env::var("JUSTRUST_BUILD_SCRIPT_CACHE").as_deref(),
            Ok("0") | Ok("off") | Ok("false") | Ok("no")
        )
}

/// Replace a freshly compiled build-script binary with the caching wrapper.
/// Called by the shim for non-local build scripts only.
pub fn wrap(out_dir: &Path, crate_name: &str, extra: &str) -> Option<()> {
    if !enabled() {
        return None;
    }
    let me = std::env::current_exe().ok()?.canonicalize().ok()?;
    wrap_with(out_dir, crate_name, extra, &me)
}

/// `wrap` with an explicit justrust path. The wrapper's contract with that
/// binary (`__build-script <real> [args]`, dispatched in `main.rs` before
/// argument parsing) must stay stable: wrappers outlive the binary that
/// wrote them and run under whatever justrust is installed later.
fn wrap_with(out_dir: &Path, crate_name: &str, extra: &str, me: &Path) -> Option<()> {
    if !crate_name.starts_with("build_script_") {
        return None;
    }
    let name = format!("{crate_name}{extra}");
    let bin = out_dir.join(&name);
    let mut magic = [0u8; 4];
    std::fs::File::open(&bin)
        .ok()?
        .read_exact(&mut magic)
        .ok()?;
    if &magic != b"\x7fELF" {
        return None;
    }
    let me = me.to_str()?;
    if me.contains('\'') || name.contains('\'') {
        return None;
    }
    let script = format!(
        "#!/bin/sh\n{WRAPPER_MARK} (src/buildscript.rs): replays cached runs.\n\
         r=\"$(dirname \"$0\")/{REAL_PREFIX}{name}\"\n\
         [ -x '{me}' ] && exec '{me}' __build-script \"$r\" \"$@\"\n\
         exec \"$r\" \"$@\"\n"
    );
    let real = out_dir.join(format!("{REAL_PREFIX}{name}"));
    let tmp = out_dir.join(format!(".jr-wrap-{name}.{}", std::process::id()));
    std::fs::write(&tmp, script).ok()?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).ok()?;
    if std::fs::rename(&bin, &real).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    if std::fs::rename(&tmp, &bin).is_err() {
        // Put the real binary back: never leave the unit without a script.
        let _ = std::fs::rename(&real, &bin);
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    Some(())
}

/// Env vars that are part of the base key.
fn keyed_env(name: &str) -> bool {
    if matches!(
        name,
        "CARGO_MAKEFLAGS" | "RUSTC" | "RUSTDOC" | "CARGO" | "NUM_JOBS" | "JOBSERVER_FDS"
    ) || name.starts_with("CARGO_TERM_")
        || name.starts_with("JUSTRUST_")
    {
        return false;
    }
    const EXACT: &[&str] = &[
        "TARGET",
        "HOST",
        "PROFILE",
        "OPT_LEVEL",
        "DEBUG",
        "OUT_DIR",
        "PATH",
        "RUSTFLAGS",
        "CRATE_CC_NO_DEFAULTS",
        "SDKROOT",
        "LIBRARY_PATH",
        "CPATH",
        "C_INCLUDE_PATH",
        "CPLUS_INCLUDE_PATH",
        "LD_LIBRARY_PATH",
        "SOURCE_DATE_EPOCH",
    ];
    const TOOLS: &[&str] = &[
        "CC", "CXX", "AR", "RANLIB", "LD", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "ARFLAGS",
        "NM", "OBJCOPY", "CMAKE", "NINJA", "PERL", "GO", "NASM", "CLANG", "LLVM",
    ];
    let bare = name
        .strip_prefix("HOST_")
        .or_else(|| name.strip_prefix("TARGET_"))
        .unwrap_or(name);
    EXACT.contains(&name)
        || name.starts_with("CARGO_")
        || name.starts_with("DEP_")
        || name.starts_with("RUSTC_")
        || name.starts_with("PKG_CONFIG")
        || name.starts_with("CMAKE_")
        || TOOLS
            .iter()
            .any(|t| bare == *t || bare.starts_with(&format!("{t}_")))
}

/// Env values naming an executable that can differ by checkout (repo-local
/// wrapper and linker scripts) are keyed by content instead of path.
fn by_content(name: &str) -> bool {
    matches!(
        name,
        "RUSTC_WRAPPER" | "RUSTC_WORKSPACE_WRAPPER" | "RUSTC_LINKER"
    )
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct Entry {
    /// (normalized path, signature) for every `rerun-if-changed` path.
    inputs: Vec<(String, String)>,
    /// (name, normalized value) for every `rerun-if-env-changed` variable.
    env: Vec<(String, Option<String>)>,
    out: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OutFile {
    path: String,
    /// UTF-8 text with placeholders, rewritten on restore.
    text: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Meta {
    files: Vec<OutFile>,
    stdout: Vec<String>,
    stderr: Vec<String>,
    wall: f64,
    package: String,
}

struct Plan {
    real: PathBuf,
    base: String,
    root: PathBuf,
    norm: Norm,
    out_dir: PathBuf,
    cwd: PathBuf,
    cargo_home: PathBuf,
}

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
        .unwrap_or_default()
}

fn plan(real: &Path, args: &[OsString]) -> Option<Plan> {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR")?);
    // <profile>/build/<pkg>-<hash>/out
    let profile = out_dir.parent()?.parent()?.parent()?;
    if out_dir.parent()?.parent()?.file_name()? != "build" {
        return None;
    }
    let norm = Norm::new(profile)?;
    let cwd = std::env::current_dir().ok()?;
    let cargo_home = cargo_home();
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR")?);
    if !manifest.starts_with(&cargo_home) {
        return None; // local package: always run
    }
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let mut h = H128::new();
    h.field(SCHEME.as_bytes());
    h.field(depcache::file_hash(real, true)?.as_bytes());
    h.field(cwd.to_str()?.as_bytes());
    h.field(depcache::rustc_version(Path::new(&rustc))?.as_bytes());
    for a in args {
        h.field(norm.norm(a.to_str()?).as_bytes());
    }
    h.field(system_stamp().as_bytes());
    let mut env: Vec<(String, String)> = std::env::vars().filter(|(k, _)| keyed_env(k)).collect();
    env.sort();
    for (k, v) in &env {
        h.field(k.as_bytes());
        if by_content(k) && Path::new(v).is_file() {
            h.field(depcache::file_hash(Path::new(v), true)?.as_bytes());
        } else {
            h.field(norm.norm(v).as_bytes());
        }
    }
    Some(Plan {
        real: real.to_path_buf(),
        base: h.hex(),
        root: depcache::cache_dir()?,
        norm,
        out_dir,
        cwd,
        cargo_home,
    })
}

/// Changes whenever system packages or shared libraries change. Scripts that
/// probe the system (pkg-config, a C compiler found on `PATH`, library
/// headers) usually declare only their env vars, and cargo would not rerun
/// them in an existing target dir either. A fresh target dir is where users
/// expect a re-probe after an upgrade, so any package install misses.
fn system_stamp() -> String {
    let mut s = String::new();
    for p in [
        "/etc/ld.so.cache",
        "/var/lib/pacman/local",
        "/var/lib/dpkg/status",
        "/var/lib/rpm",
        "/nix/var/nix/db/db.sqlite",
    ] {
        if let Ok(m) = std::fs::metadata(p) {
            s.push_str(&format!("{p}:{}.{};", m.mtime(), m.mtime_nsec()));
        }
    }
    s
}

/// Signature of a `rerun-if-changed` path. Files inside `CARGO_HOME`
/// (extracted registry crates, git checkouts) are immutable once written, so
/// size and mtime stand in for content. Directories cover every file below.
fn input_sig(path: &Path, cargo_home: &Path) -> Option<String> {
    // Follows symlinks: the target's content is what the script reads.
    let m = std::fs::metadata(path).ok()?;
    if m.is_dir() {
        let mut h = H128::new();
        h.field(b"dir");
        let mut stack = vec![path.to_path_buf()];
        let mut files = Vec::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).ok()?.flatten() {
                let p = e.path();
                match e.file_type().ok()? {
                    t if t.is_dir() => stack.push(p),
                    _ => files.push(p),
                }
            }
        }
        files.sort();
        for f in files {
            h.field(f.strip_prefix(path).ok()?.to_str()?.as_bytes());
            h.field(input_sig(&f, cargo_home)?.as_bytes());
        }
        return Some(h.hex());
    }
    if path.starts_with(cargo_home) && !std::fs::symlink_metadata(path).ok()?.is_symlink() {
        return Some(format!("m:{}:{}.{}", m.len(), m.mtime(), m.mtime_nsec()));
    }
    depcache::file_hash(path, false)
}

impl Plan {
    fn manifest(&self) -> PathBuf {
        self.root.join("m").join(format!("{}.jsonl", self.base))
    }

    fn matches(&self, e: &Entry) -> bool {
        e.env
            .iter()
            .all(|(k, v)| std::env::var(k).ok().map(|x| self.norm.norm(&x)).as_ref() == v.as_ref())
            && e.inputs.iter().all(|(p, sig)| {
                input_sig(&self.cwd.join(self.norm.denorm(p)), &self.cargo_home).as_deref()
                    == Some(sig.as_str())
            })
    }

    /// Put a cached run's OUT_DIR in place. Returns the recorded output.
    fn restore(&self) -> Option<Meta> {
        let text = std::fs::read_to_string(self.manifest()).ok()?;
        let entry = text
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
            .find(|e| self.matches(e))?;
        let dir = self.root.join("o").join(&entry.out);
        depcache::touch(&dir.join("meta.json"));
        let meta: Meta =
            serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
        for f in &meta.files {
            let rel = Path::new(&f.path);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return None;
            }
            let src = dir.join("out").join(rel);
            let dst = self.out_dir.join(rel);
            std::fs::create_dir_all(dst.parent()?).ok()?;
            let tmp = dst.with_file_name(format!(
                ".{}.jr{}",
                dst.file_name()?.to_str()?,
                std::process::id()
            ));
            let ok = if f.text {
                std::fs::read_to_string(&src)
                    .ok()
                    .and_then(|t| std::fs::write(&tmp, self.norm.denorm(&t)).ok())
                    .is_some()
            } else {
                depcache::reflink_or_copy(&src, &tmp)
            };
            if !ok || std::fs::rename(&tmp, &dst).is_err() {
                let _ = std::fs::remove_file(&tmp);
                return None;
            }
        }
        Some(meta)
    }

    fn store(&self, stdout: &[u8], stderr: &[u8], wall: f64) -> Option<()> {
        let stdout = std::str::from_utf8(stdout).ok()?;
        let stderr = std::str::from_utf8(stderr).ok()?;
        let (changed, env_names) = directives(stdout);
        if changed.is_empty() && env_names.is_empty() {
            return None;
        }
        let mut entry = Entry {
            inputs: Vec::new(),
            env: Vec::new(),
            out: String::new(),
        };
        let mut h = H128::new();
        h.field(self.base.as_bytes());
        for p in changed {
            let abs = self.cwd.join(&p);
            if abs.starts_with(&self.norm.target) {
                return None;
            }
            let sig = input_sig(&abs, &self.cargo_home)?;
            let np = self.norm.norm(&p);
            h.field(np.as_bytes());
            h.field(sig.as_bytes());
            entry.inputs.push((np, sig));
        }
        for k in env_names {
            let v = std::env::var(&k).ok().map(|v| self.norm.norm(&v));
            h.field(k.as_bytes());
            h.field(v.as_deref().unwrap_or("\0unset").as_bytes());
            entry.env.push((k, v));
        }
        entry.out = h.hex();

        let final_dir = self.root.join("o").join(&entry.out);
        if final_dir.exists() {
            depcache::touch(&final_dir.join("meta.json"));
        } else {
            let tmp = self
                .root
                .join("o")
                .join(format!(".{}.tmp{}", entry.out, std::process::id()));
            let meta = Meta {
                files: Vec::new(),
                stdout: stdout.lines().map(|l| self.norm.norm(l)).collect(),
                stderr: stderr.lines().map(|l| self.norm.norm(l)).collect(),
                wall,
                package: std::env::var("CARGO_PKG_NAME").unwrap_or_default(),
            };
            let r = self.write_outputs(&tmp, meta);
            if r.is_none() || std::fs::rename(&tmp, &final_dir).is_err() {
                let _ = std::fs::remove_dir_all(&tmp);
                if !final_dir.exists() {
                    return None;
                }
            }
        }
        std::fs::create_dir_all(self.root.join("m")).ok()?;
        let line = serde_json::to_string(&entry).ok()?;
        let known = std::fs::read_to_string(self.manifest()).unwrap_or_default();
        if known.lines().any(|l| l == line) {
            return Some(());
        }
        paths::append_line(&self.manifest(), &line).ok()
    }

    fn write_outputs(&self, dir: &Path, mut meta: Meta) -> Option<()> {
        let out = dir.join("out");
        std::fs::create_dir_all(&out).ok()?;
        let mut stack = vec![PathBuf::new()];
        while let Some(rel) = stack.pop() {
            for e in std::fs::read_dir(self.out_dir.join(&rel)).ok()?.flatten() {
                let r = rel.join(e.file_name());
                let t = e.file_type().ok()?;
                if t.is_dir() {
                    std::fs::create_dir_all(out.join(&r)).ok()?;
                    stack.push(r);
                    continue;
                }
                if !t.is_file() {
                    return None; // symlinks, sockets: not reproducible
                }
                let src = self.out_dir.join(&r);
                let bytes = std::fs::read(&src).ok()?;
                let text = !bytes.contains(&0) && std::str::from_utf8(&bytes).is_ok();
                if text {
                    let s = std::str::from_utf8(&bytes).ok()?;
                    std::fs::write(out.join(&r), self.norm.norm(s)).ok()?;
                } else {
                    // Binary data naming this target dir would point the next
                    // build at our files: never cache it.
                    if !depcache::target_refs_are_inputs(&bytes, &self.norm.target, &[]) {
                        return None;
                    }
                    if !depcache::reflink_or_copy(&src, &out.join(&r)) {
                        return None;
                    }
                }
                meta.files.push(OutFile {
                    path: r.to_str()?.to_owned(),
                    text,
                });
            }
        }
        std::fs::write(dir.join("meta.json"), serde_json::to_vec(&meta).ok()?).ok()
    }
}

/// `rerun-if-changed` paths and `rerun-if-env-changed` names from a build
/// script's stdout (both the `cargo:` and `cargo::` forms).
fn directives(stdout: &str) -> (Vec<String>, Vec<String>) {
    let mut changed = Vec::new();
    let mut env = Vec::new();
    for l in stdout.lines() {
        let Some(d) = l
            .strip_prefix("cargo::")
            .or_else(|| l.strip_prefix("cargo:"))
        else {
            continue;
        };
        if let Some(p) = d.strip_prefix("rerun-if-changed=") {
            if !changed.iter().any(|x| x == p) {
                changed.push(p.to_owned());
            }
        } else if let Some(n) = d.strip_prefix("rerun-if-env-changed=")
            && !env.iter().any(|x| x == n)
        {
            env.push(n.to_owned());
        }
    }
    (changed, env)
}

fn exec_real(real: &Path, args: &[OsString]) -> ! {
    let err = Command::new(real).args(args).exec();
    eprintln!("justrust: failed to exec {}: {err}", real.display());
    std::process::exit(127);
}

fn record(package: &str, hit: bool, wall: f64, saved: f64) {
    let Some(dir) = std::env::var_os("JUSTRUST_RUN_DIR") else {
        return;
    };
    let line = serde_json::json!({
        "package": package,
        "hit": hit,
        "wall": wall,
        "saved_secs": saved,
    });
    let _ = paths::append_line(&Path::new(&dir).join("scripts.jsonl"), &line.to_string());
}

/// Tee a child pipe to our own stream while capturing it.
fn tee<R: Read + Send + 'static>(mut r: R, to_stdout: bool) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            all.extend_from_slice(&buf[..n]);
            let _ = if to_stdout {
                std::io::stdout().write_all(&buf[..n])
            } else {
                std::io::stderr().write_all(&buf[..n])
            };
        }
        all
    })
}

/// `justrust __build-script <real> [args]`: run by the wrapper.
pub fn main(real: PathBuf, args: Vec<OsString>) -> ! {
    if !enabled() {
        exec_real(&real, &args);
    }
    let Some(plan) = plan(&real, &args) else {
        exec_real(&real, &args);
    };
    let package = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
    let start = paths::now();
    if let Some(meta) = plan.restore() {
        let mut out = std::io::stdout().lock();
        for l in &meta.stdout {
            let _ = writeln!(out, "{}", plan.norm.denorm(l));
        }
        let _ = out.flush();
        let mut err = std::io::stderr().lock();
        for l in &meta.stderr {
            let _ = writeln!(err, "{}", plan.norm.denorm(l));
        }
        record(&package, true, paths::now() - start, meta.wall);
        std::process::exit(0);
    }
    let child = Command::new(&plan.real)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        exec_real(&real, &args);
    };
    let t_out = tee(child.stdout.take().expect("piped"), true);
    let t_err = tee(child.stderr.take().expect("piped"), false);
    let status = child.wait();
    let out = t_out.join().unwrap_or_default();
    let err = t_err.join().unwrap_or_default();
    let wall = paths::now() - start;
    record(&package, false, wall, 0.0);
    let status = match status {
        Ok(s) => s,
        Err(_) => std::process::exit(1),
    };
    if status.success() {
        let _ = plan.store(&out, &err, wall);
    }
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    // Killed by a signal: die the same way so cargo reports what it would
    // have reported for the real script.
    use std::os::unix::process::ExitStatusExt;
    let sig = status.signal().unwrap_or(libc::SIGKILL);
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
    std::process::exit(128 + sig);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_directives() {
        let (c, e) = directives(
            "cargo:rerun-if-changed=build.rs\ncargo::rerun-if-env-changed=CC\n\
             cargo:rustc-cfg=foo\ncargo:rerun-if-changed=build.rs\nnoise\n",
        );
        assert_eq!(c, vec!["build.rs"]);
        assert_eq!(e, vec!["CC"]);
    }

    #[test]
    fn keys_toolchain_env_but_not_noise() {
        for k in [
            "CC",
            "CFLAGS_x86_64_unknown_linux_gnu",
            "HOST_CC",
            "TARGET_AR",
            "CARGO_FEATURE_STD",
            "CARGO_CFG_TARGET_OS",
            "DEP_Z_INCLUDE",
            "PKG_CONFIG_PATH",
            "OUT_DIR",
            "PATH",
        ] {
            assert!(keyed_env(k), "{k}");
        }
        for k in [
            "CARGO_MAKEFLAGS",
            "NUM_JOBS",
            "RUSTC",
            "JCODE_SESSION_ID",
            "TMUX_PANE",
            "CCACHE_DIR_X",
            "JUSTRUST_RUN_DIR",
        ] {
            assert!(!keyed_env(k), "{k}");
        }
    }

    /// Store a run from one target dir, restore it into another: text files
    /// are rewritten for the new dir, a changed input misses, and binary
    /// files naming the target dir are never stored.
    #[test]
    fn round_trip_between_target_dirs() {
        let dir = std::env::temp_dir().join(format!("jr-bs-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("pkg");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("in.txt"), "v1").unwrap();
        let mk = |t: &str| {
            let profile = dir.join(t).join("debug");
            let out_dir = profile.join("build/pkg-1/out");
            std::fs::create_dir_all(out_dir.join("sub")).unwrap();
            Plan {
                real: dir.join("real"),
                base: "b".repeat(32),
                root: dir.join("cache"),
                norm: Norm::new(&profile).unwrap(),
                out_dir,
                cwd: src.clone(),
                cargo_home: dir.join("no-cargo-home"),
            }
        };
        let p1 = mk("t1");
        let od1 = p1.out_dir.display().to_string();
        std::fs::write(p1.out_dir.join("gen.rs"), format!("// {od1}/gen.rs")).unwrap();
        std::fs::write(p1.out_dir.join("sub/lib.a"), b"\x00\x01ar").unwrap();
        let stdout =
            format!("cargo:rerun-if-changed=in.txt\ncargo:rustc-link-search=native={od1}\n");
        p1.store(stdout.as_bytes(), b"note\n", 3.0).unwrap();

        let p2 = mk("t2");
        let meta = p2.restore().expect("hit");
        let od2 = p2.out_dir.display().to_string();
        assert_eq!(
            std::fs::read_to_string(p2.out_dir.join("gen.rs")).unwrap(),
            format!("// {od2}/gen.rs")
        );
        assert_eq!(
            std::fs::read(p2.out_dir.join("sub/lib.a")).unwrap(),
            b"\x00\x01ar"
        );
        let lines: Vec<String> = meta.stdout.iter().map(|l| p2.norm.denorm(l)).collect();
        assert_eq!(lines[1], format!("cargo:rustc-link-search=native={od2}"));
        assert_eq!(meta.wall, 3.0);

        std::fs::write(src.join("in.txt"), "v2").unwrap();
        assert!(p2.restore().is_none());

        // No rerun-if directives: not stored.
        let p3 = Plan {
            base: "c".repeat(32),
            ..mk("t1")
        };
        assert!(p3.store(b"cargo:rustc-cfg=x\n", b"", 1.0).is_none());
        // Binary file embedding the target dir: not stored.
        let mut bin = b"\x00".to_vec();
        bin.extend_from_slice(od1.as_bytes());
        std::fs::write(p3.out_dir.join("sub/lib.a"), bin).unwrap();
        assert!(p3.store(stdout.as_bytes(), b"", 1.0).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrapper_replaces_elf_and_falls_back() {
        let dir = std::env::temp_dir().join(format!("jr-bs-wrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("build_script_build-abc");
        std::fs::copy("/bin/true", &bin).unwrap();
        wrap(&dir, "build_script_build", "-abc").unwrap();
        let w = std::fs::read_to_string(&bin).unwrap();
        assert!(w.starts_with("#!/bin/sh\n") && w.contains(WRAPPER_MARK));
        assert!(dir.join(".jr-real-build_script_build-abc").is_file());
        // Already a wrapper: left alone.
        assert!(wrap(&dir, "build_script_build", "-abc").is_none());
        // Not a build script: left alone.
        assert!(wrap(&dir, "serde", "-abc").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The wrapper must behave like the real script when justrust is gone:
    /// same stdout, stderr, args, cwd, and exit code.
    #[test]
    fn wrapper_runs_real_script_when_justrust_is_missing() {
        let dir = std::env::temp_dir().join(format!("jr-bs-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cwd")).unwrap();
        let bin = dir.join("build_script_build-abc");
        // An ELF stand-in: /bin/sh with a script supplied through argv is not
        // possible, so wrap a copy of sh and replace the real file afterwards.
        std::fs::copy("/bin/sh", &bin).unwrap();
        wrap_with(
            &dir,
            "build_script_build",
            "-abc",
            &dir.join("no-such-justrust"),
        )
        .unwrap();
        let real = dir.join(".jr-real-build_script_build-abc");
        std::fs::write(
            &real,
            "#!/bin/sh\necho \"cargo:rustc-cfg=a$1\"\necho err >&2\npwd\nexit 7\n",
        )
        .unwrap();
        let out = Command::new(&bin)
            .arg("X")
            .current_dir(dir.join("cwd"))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(7));
        let so = String::from_utf8(out.stdout).unwrap();
        assert!(so.starts_with("cargo:rustc-cfg=aX\n"), "{so}");
        assert!(so.trim_end().ends_with("/cwd"), "{so}");
        assert_eq!(out.stderr, b"err\n");

        // With a justrust that exists but has the cache disabled: same.
        let jr = dir.join("fake-justrust");
        std::fs::write(&jr, "#!/bin/sh\n[ \"$1\" = __build-script ] || exit 99\nshift\nr=$1\nshift\nexec \"$r\" \"$@\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&jr, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::copy("/bin/sh", &bin).unwrap();
        std::fs::remove_file(&real).unwrap();
        wrap_with(&dir, "build_script_build", "-abc", &jr).unwrap();
        std::fs::write(&real, "#!/bin/sh\necho \"$1 $2\"\nexit 3\n").unwrap();
        let out = Command::new(&bin).args(["a b", "c"]).output().unwrap();
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.stdout, b"a b c\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_stamp_is_stable() {
        assert_eq!(system_stamp(), system_stamp());
    }

    /// Outputs that cannot be replayed faithfully are never stored.
    #[test]
    fn non_cacheable_runs_are_not_stored() {
        let dir = std::env::temp_dir().join(format!("jr-bs-nc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("pkg");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("in.txt"), "v1").unwrap();
        let profile = dir.join("t").join("debug");
        let out_dir = profile.join("build/pkg-1/out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let p = Plan {
            real: dir.join("real"),
            base: "d".repeat(32),
            root: dir.join("cache"),
            norm: Norm::new(&profile).unwrap(),
            out_dir: out_dir.clone(),
            cwd: src.clone(),
            cargo_home: dir.join("no-cargo-home"),
        };
        // rerun-if-changed on a missing path (cargo always reruns those).
        assert!(
            p.store(b"cargo:rerun-if-changed=gone.txt\n", b"", 1.0)
                .is_none()
        );
        // rerun-if-changed inside the target dir.
        let inside = format!("cargo:rerun-if-changed={}/x\n", out_dir.display());
        std::fs::write(out_dir.join("x"), "1").unwrap();
        assert!(p.store(inside.as_bytes(), b"", 1.0).is_none());
        std::fs::remove_file(out_dir.join("x")).unwrap();
        // Non-UTF-8 stdout.
        assert!(
            p.store(b"cargo:rerun-if-changed=in.txt\n\xff\n", b"", 1.0)
                .is_none()
        );
        // Symlink in OUT_DIR.
        std::os::unix::fs::symlink("/etc/hostname", out_dir.join("link")).unwrap();
        assert!(
            p.store(b"cargo:rerun-if-changed=in.txt\n", b"", 1.0)
                .is_none()
        );
        std::fs::remove_file(out_dir.join("link")).unwrap();
        assert!(!p.manifest().exists());
        // Clean run: stored, and an env-only directive keys on the value.
        let ok = b"cargo:rerun-if-env-changed=JR_BS_TEST_UNSET_VAR\n";
        p.store(ok, b"", 1.0).unwrap();
        assert!(p.restore().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_inputs_track_contents() {
        let dir = std::env::temp_dir().join(format!("jr-bs-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("d/sub")).unwrap();
        std::fs::write(dir.join("d/sub/a"), "1").unwrap();
        let ch = dir.join("no-cargo-home");
        let s1 = input_sig(&dir.join("d"), &ch).unwrap();
        std::fs::write(dir.join("d/sub/a"), "2").unwrap();
        let s2 = input_sig(&dir.join("d"), &ch).unwrap();
        std::fs::write(dir.join("d/b"), "").unwrap();
        let s3 = input_sig(&dir.join("d"), &ch).unwrap();
        assert!(s1 != s2 && s2 != s3);
        // Symlinked file: content of the target counts.
        std::os::unix::fs::symlink(dir.join("d/sub/a"), dir.join("l")).unwrap();
        let l1 = input_sig(&dir.join("l"), &ch).unwrap();
        std::fs::write(dir.join("d/sub/a"), "3").unwrap();
        assert_ne!(l1, input_sig(&dir.join("l"), &ch).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
