//! Shared, content-addressed cache for non-local (registry and git) rustc units.
//!
//! Fresh checkouts, scratch copies, build slots, and new target directories all
//! recompile the same dependencies from zero. sccache does not help here: its
//! key includes absolute target-dir paths (`-L dependency=`, `OUT_DIR`,
//! `CARGO_TARGET_DIR`), so two target dirs never share an entry (measured: 0 of
//! 86 hits, see FINDINGS.md). This cache normalizes the profile dir instead.
//!
//! ```text
//! ~/.justrust/cache/depcache/
//!   m/<base-key>.jsonl     one entry per known input set: source hashes, env, result
//!   o/<result-key>/        the unit's outputs (.rlib .rmeta .d .so, build-script exe)
//!   o/<result-key>/meta.json   file list with hashes, replayed stderr, compile time
//! ```
//!
//! Key, in two levels (like ccache's direct mode):
//! - base key: rustc `-vV`, cwd, every argument with the profile dir
//!   (`<target>/debug`) and target dir replaced by placeholders, the content
//!   hash of every `--extern` artifact, of the linker, and of native library
//!   dirs inside the target dir, plus the `CARGO_*`/`RUSTC_*`/`OUT_DIR` env.
//! - result key: base key plus the content of every source file in the unit's
//!   dep-info (including generated files in `OUT_DIR`) and every env var it
//!   tracked (`env!`, `option_env!`).
//!
//! Externs are keyed by content, not by name, so a dependency rebuilt with
//! different bytes can never be paired with a dependent compiled against the
//! old one. Content hashes of large artifacts are memoized in a
//! `user.justrust.h` xattr validated by inode, size, and mtime.
//!
//! On a hit the outputs are reflinked into the out dir (fresh mtimes, like a
//! real compile), `.d` files are rewritten for the new profile dir, and the
//! recorded stderr (artifact notifications, diagnostics) is replayed so cargo
//! pipelining works as usual.
//!
//! Rules: only non-local, non-incremental units emitting dep-info/metadata/link
//! into `--out-dir`. Only successful compiles are stored. Every error means
//! "compile normally". Disable with `JUSTRUST_DEPCACHE=0`.
//!
//! Size cap and LRU eviction live in `depcache_gc.rs` (`justrust cache`).
//!
//! Embedded target paths: a unit is only stored if every occurrence of the
//! target dir in its outputs is the path of one of its own (content-hashed)
//! input files, i.e. `OUT_DIR` sources pulled in with `include!`. Outputs that
//! carry a target path as data (`env!("OUT_DIR")` read at runtime) are never
//! cached. The remaining, accepted difference from a real compile: panic
//! locations, debug line tables, and diagnostics in such `OUT_DIR` code name
//! the target dir of the build that first produced them (same file content).
//! Measured in FINDINGS.md section 8b.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::{CString, OsString};
use std::hash::Hasher;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Bump to invalidate every entry when the key scheme changes.
const SCHEME: &str = "justrust-depcache-1";
const PROFILE_TOKEN: &str = "@@JUSTRUST_PROFILE_DIR@@";
const TARGET_TOKEN: &str = "@@JUSTRUST_TARGET_DIR@@";
const XATTR: &str = "user.justrust.h";

pub fn enabled() -> bool {
    !matches!(
        std::env::var("JUSTRUST_DEPCACHE").as_deref(),
        Ok("0") | Ok("off") | Ok("false") | Ok("no")
    )
}

pub fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("JUSTRUST_DEPCACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    paths::home().ok().map(|h| h.join("cache/depcache"))
}

/// 128-bit content hash from two independently keyed SipHash-2-4 streams.
/// The algorithm is fixed (unlike `DefaultHasher`), so keys are stable
/// across justrust builds.
struct H128(
    #[allow(deprecated)] std::hash::SipHasher,
    #[allow(deprecated)] std::hash::SipHasher,
);

impl H128 {
    #[allow(deprecated)]
    fn new() -> H128 {
        H128(
            std::hash::SipHasher::new_with_keys(0x6a75_7374, 0x7275_7374),
            std::hash::SipHasher::new_with_keys(0x6465_7063, 0x6163_6865),
        )
    }
    /// Length-prefixed, so ("ab","c") and ("a","bc") differ.
    fn field(&mut self, b: &[u8]) {
        self.0.write_u64(b.len() as u64);
        self.1.write_u64(b.len() as u64);
        self.raw(b);
    }
    fn raw(&mut self, b: &[u8]) {
        self.0.write(b);
        self.1.write(b);
    }
    fn hex(&self) -> String {
        format!("{:016x}{:016x}", self.0.finish(), self.1.finish())
    }
}

/// Maps the build's profile and target dirs to placeholders and back.
#[derive(Debug, Clone)]
pub struct Norm {
    profile: String,
    target: String,
}

impl Norm {
    pub fn new(profile_dir: &Path) -> Option<Norm> {
        let profile = profile_dir.to_str()?.trim_end_matches('/').to_owned();
        let target = profile_dir.parent()?.to_str()?.to_owned();
        if profile.len() < 2 || target.len() < 2 {
            return None;
        }
        Some(Norm { profile, target })
    }
    pub fn norm(&self, s: &str) -> String {
        if !s.contains(&self.target) {
            return s.to_owned();
        }
        s.replace(&self.profile, PROFILE_TOKEN)
            .replace(&self.target, TARGET_TOKEN)
    }
    pub fn denorm(&self, s: &str) -> String {
        s.replace(PROFILE_TOKEN, &self.profile)
            .replace(TARGET_TOKEN, &self.target)
    }
}

/// Profile dir for an out dir: `<P>/deps` or `<P>/build/<pkg-hash>`.
pub fn profile_dir(out_dir: &Path) -> Option<PathBuf> {
    if out_dir.file_name()? == "deps" {
        return Some(out_dir.parent()?.to_path_buf());
    }
    let parent = out_dir.parent()?;
    if parent.file_name()? == "build" {
        return Some(parent.parent()?.to_path_buf());
    }
    None
}

/// What the cache needs to know about one rustc invocation.
#[derive(Debug, Clone)]
pub struct Plan {
    pub base: String,
    pub out_dir: PathBuf,
    pub extra: String,
    pub crate_name: String,
    pub cwd: PathBuf,
    pub norm: Norm,
    root: PathBuf,
}

/// Parsed eligibility: `None` means "not cacheable, compile normally".
#[derive(Debug, Default, PartialEq)]
struct Shape {
    out_dir: String,
    extra: String,
    crate_name: String,
}

fn shape(args: &[String]) -> Option<Shape> {
    let mut s = Shape::default();
    let mut emit: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next = args.get(i + 1).map(String::as_str);
        let mut codegen = |v: &str| -> Option<()> {
            if let Some(e) = v.strip_prefix("extra-filename=") {
                s.extra = e.to_owned();
            }
            if v.starts_with("incremental") || v.starts_with("save-temps") {
                return None;
            }
            Some(())
        };
        match a {
            "-o" | "--print" | "-" => return None,
            "--out-dir" => {
                s.out_dir = next?.to_owned();
                i += 1;
            }
            "--crate-name" => {
                s.crate_name = next?.to_owned();
                i += 1;
            }
            "--crate-type" => {
                if !matches!(next?, "lib" | "rlib" | "proc-macro" | "bin") {
                    return None;
                }
                i += 1;
            }
            "--emit" => {
                emit = Some(next?.to_owned());
                i += 1;
            }
            "-C" => {
                codegen(next?)?;
                i += 1;
            }
            _ => {
                if let Some(v) = a.strip_prefix("--emit=") {
                    emit = Some(v.to_owned());
                } else if let Some(v) = a.strip_prefix("--crate-type=") {
                    if !matches!(v, "lib" | "rlib" | "proc-macro" | "bin") {
                        return None;
                    }
                } else if let Some(v) = a.strip_prefix("--out-dir=") {
                    s.out_dir = v.to_owned();
                } else if a.starts_with("--print") || a.starts_with("-o") && a.len() > 2 {
                    return None;
                } else if let Some(v) = a.strip_prefix("-C") {
                    codegen(v)?;
                }
            }
        }
        i += 1;
    }
    let emit = emit?;
    if !emit.split(',').all(|e| {
        matches!(e, "dep-info" | "metadata" | "link") // `kind=path` is not cacheable
    }) || !emit.split(',').any(|e| e == "dep-info")
    {
        return None;
    }
    if s.out_dir.is_empty() || s.extra.len() < 2 || s.crate_name.is_empty() {
        return None;
    }
    Some(s)
}

/// Env vars that are part of the base key. `CARGO_MAKEFLAGS` carries
/// jobserver fds, which differ every run.
fn keyed_env(name: &str) -> bool {
    if matches!(
        name,
        "CARGO_MAKEFLAGS" | "RUSTC_WRAPPER" | "RUSTC_WORKSPACE_WRAPPER" | "CARGO_TERM_VERBOSE"
    ) || name.starts_with("CARGO_TERM_")
    {
        return false;
    }
    name.starts_with("CARGO_")
        || name.starts_with("RUSTC_")
        || matches!(
            name,
            "OUT_DIR" | "SOURCE_DATE_EPOCH" | "LIBRARY_PATH" | "SDKROOT" | "RUST_TARGET_PATH"
        )
}

fn rustc_version(real: &Path) -> Option<String> {
    let out = std::process::Command::new(real)
        .arg("-vV")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Build the base key. `None` if the unit is not cacheable or anything fails.
pub fn plan(real: &Path, args: &[OsString], env: &[(String, String)]) -> Option<Plan> {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    let s = shape(&args)?;
    let cwd = std::env::current_dir().ok()?;
    let out_dir = cwd.join(&s.out_dir);
    let norm = Norm::new(&profile_dir(&out_dir)?)?;
    let root = cache_dir()?;

    let mut h = H128::new();
    h.field(SCHEME.as_bytes());
    h.field(rustc_version(real)?.as_bytes());
    h.field(norm.norm(cwd.to_str()?).as_bytes());
    key_args(&args, &norm, &cwd, &mut h)?;
    let mut env: Vec<&(String, String)> = env.iter().filter(|(k, _)| keyed_env(k)).collect();
    env.sort();
    for (k, v) in env {
        h.field(k.as_bytes());
        h.field(norm.norm(v).as_bytes());
    }
    Some(Plan {
        base: h.hex(),
        out_dir,
        extra: s.extra,
        crate_name: s.crate_name,
        cwd,
        norm,
        root,
    })
}

fn key_args(args: &[String], norm: &Norm, cwd: &Path, h: &mut H128) -> Option<()> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next = args.get(i + 1).map(String::as_str);
        if a.starts_with("--diagnostic-width") {
            // Terminal width: only changes how diagnostics wrap.
        } else if a == "--extern" {
            h.field(b"--extern");
            key_extern(next?, cwd, h)?;
            i += 1;
        } else if a == "-C" {
            h.field(b"-C");
            key_codegen(next?, norm, cwd, h)?;
            i += 1;
        } else if let Some(v) = a.strip_prefix("-C") {
            h.field(b"-C");
            key_codegen(v, norm, cwd, h)?;
        } else if a == "-L" {
            h.field(b"-L");
            key_lib_dir(next?, norm, cwd, h)?;
            i += 1;
        } else if let Some(v) = a.strip_prefix("-L") {
            h.field(b"-L");
            key_lib_dir(v, norm, cwd, h)?;
        } else {
            h.field(norm.norm(a).as_bytes());
        }
        i += 1;
    }
    Some(())
}

fn key_extern(v: &str, cwd: &Path, h: &mut H128) -> Option<()> {
    match v.split_once('=') {
        Some((name, path)) => {
            h.field(name.as_bytes());
            h.field(file_hash(&cwd.join(path), true)?.as_bytes());
        }
        None => h.field(v.as_bytes()),
    }
    Some(())
}

fn key_codegen(v: &str, norm: &Norm, cwd: &Path, h: &mut H128) -> Option<()> {
    if let Some(linker) = v.strip_prefix("linker=") {
        // Config-relative linkers resolve to a different absolute path in each
        // checkout. Key on the linker's content when it is a file.
        let p = cwd.join(linker);
        h.field(b"linker=");
        match p.is_file() {
            true => h.field(file_hash(&p, false)?.as_bytes()),
            false => h.field(linker.as_bytes()),
        }
    } else {
        h.field(norm.norm(v).as_bytes());
    }
    Some(())
}

/// Native library dirs inside the target dir (build-script output such as a
/// `cc`-built `libfoo.a`) get bundled into rlibs, so their content is keyed.
fn key_lib_dir(v: &str, norm: &Norm, cwd: &Path, h: &mut H128) -> Option<()> {
    h.field(norm.norm(v).as_bytes());
    let (kind, path) = v.split_once('=').unwrap_or(("all", v));
    if kind == "dependency" {
        return Some(());
    }
    let dir = cwd.join(path);
    if !dir.starts_with(&norm.target) {
        return Some(());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    for f in files {
        h.field(f.file_name()?.as_bytes());
        h.field(file_hash(&f, false)?.as_bytes());
    }
    Some(())
}

fn xattr_get(path: &Path) -> Option<String> {
    let p = CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = CString::new(XATTR).ok()?;
    let mut buf = [0u8; 128];
    // SAFETY: valid NUL-terminated strings and a buffer of the stated length.
    let n = unsafe {
        libc::getxattr(
            p.as_ptr(),
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

fn xattr_set(path: &Path, value: &str) {
    let (Ok(p), Ok(name)) = (
        CString::new(path.as_os_str().as_bytes()),
        CString::new(XATTR),
    ) else {
        return;
    };
    // SAFETY: valid NUL-terminated strings and a value buffer of stated length.
    unsafe {
        libc::setxattr(
            p.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        );
    }
}

fn stamp(m: &std::fs::Metadata) -> String {
    format!("{}:{}:{}:{}", m.ino(), m.len(), m.mtime(), m.mtime_nsec())
}

/// Content hash of a file. With `memo`, reuse a hash stored in an xattr when
/// the file's inode, size, and mtime still match.
pub fn file_hash(path: &Path, memo: bool) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let st = stamp(&meta);
    if memo
        && let Some(v) = xattr_get(path)
        && let Some((s, hash)) = v.rsplit_once('|')
        && s == st
    {
        return Some(hash.to_owned());
    }
    let hash = hash_file_content(path)?;
    if memo {
        xattr_set(path, &format!("{st}|{hash}"));
    }
    Some(hash)
}

fn hash_file_content(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = H128::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        h.raw(&buf[..n]);
    }
    h.field(&total.to_le_bytes());
    Some(h.hex())
}

/// One known input set for a base key.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct Entry {
    /// (normalized path, content hash)
    inputs: Vec<(String, String)>,
    /// (name, normalized value) for env vars the unit read.
    env: Vec<(String, Option<String>)>,
    out: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OutFile {
    name: String,
    hash: String,
    /// Text with placeholders (dep-info), rewritten on restore.
    text: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Outputs {
    files: Vec<OutFile>,
    /// Normalized stderr lines, replayed on a hit.
    stderr: Vec<String>,
    /// Wall seconds the original compile took.
    wall: f64,
}

pub struct Hit {
    pub saved_secs: f64,
}

impl Plan {
    fn manifest(&self) -> PathBuf {
        self.root.join("m").join(format!("{}.jsonl", self.base))
    }

    /// Look up and restore. On success the outputs are in place and the
    /// recorded stderr has been written to `err`.
    pub fn restore(&self, err: &mut dyn Write) -> Option<Hit> {
        let text = std::fs::read_to_string(self.manifest()).ok()?;
        let mut memo: HashMap<String, Option<String>> = HashMap::new();
        let entry = text
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
            .find(|e| self.matches(e, &mut memo))?;
        let dir = self.root.join("o").join(&entry.out);
        // Mark the entry used before reading it, so a concurrent eviction
        // that already picked it sees the touch and puts it back.
        touch(&dir.join("meta.json"));
        let outs: Outputs =
            serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
        // Restore every file before replaying anything, so a partial restore
        // just falls back to a normal compile that overwrites it.
        for f in &outs.files {
            if f.name.contains('/') || f.name.starts_with('.') {
                return None;
            }
            let src = dir.join(&f.name);
            let dst = self.out_dir.join(&f.name);
            let tmp = self
                .out_dir
                .join(format!(".{}.jr{}", f.name, std::process::id()));
            let ok = if f.text {
                std::fs::read_to_string(&src)
                    .ok()
                    .and_then(|t| std::fs::write(&tmp, self.norm.denorm(&t)).ok())
                    .is_some()
            } else {
                reflink_or_copy(&src, &tmp)
            };
            if !ok || std::fs::rename(&tmp, &dst).is_err() {
                let _ = std::fs::remove_file(&tmp);
                return None;
            }
            if !f.text
                && let Ok(m) = std::fs::metadata(&dst)
            {
                xattr_set(&dst, &format!("{}|{}", stamp(&m), f.hash));
            }
        }
        for l in &outs.stderr {
            let _ = err.write_all(self.norm.denorm(l).as_bytes());
            let _ = err.write_all(b"\n");
        }
        let _ = err.flush();
        Some(Hit {
            saved_secs: outs.wall,
        })
    }

    fn matches(&self, e: &Entry, memo: &mut HashMap<String, Option<String>>) -> bool {
        let env_ok = e
            .env
            .iter()
            .all(|(k, v)| std::env::var(k).ok().map(|x| self.norm.norm(&x)).as_ref() == v.as_ref());
        env_ok
            && e.inputs.iter().all(|(p, hash)| {
                let path = self.norm.denorm(p);
                let got = memo
                    .entry(path.clone())
                    .or_insert_with(|| file_hash(Path::new(&path), false));
                got.as_deref() == Some(hash.as_str())
            })
    }

    /// Store the outputs of a successful compile that started at `start`.
    pub fn store(&self, start: f64, wall: f64, stderr: &[Vec<u8>]) -> Option<()> {
        let dep_info = self
            .out_dir
            .join(format!("{}{}.d", self.crate_name, self.extra));
        let d = std::fs::read_to_string(&dep_info).ok()?;
        let (inputs, env_names) = parse_dep_info(&d);
        let mut entry = Entry {
            inputs: Vec::new(),
            env: Vec::new(),
            out: String::new(),
        };
        let mut h = H128::new();
        h.field(self.base.as_bytes());
        for p in inputs {
            let abs = self.cwd.join(&p);
            let hash = file_hash(&abs, false)?;
            let np = self.norm.norm(abs.to_str()?);
            h.field(np.as_bytes());
            h.field(hash.as_bytes());
            entry.inputs.push((np, hash));
        }
        for k in env_names {
            let v = std::env::var(&k).ok().map(|v| self.norm.norm(&v));
            h.field(k.as_bytes());
            h.field(v.as_deref().unwrap_or("\0unset").as_bytes());
            entry.env.push((k, v));
        }
        entry.out = h.hex();

        let mut stderr_lines = Vec::new();
        let mut artifacts = Vec::new();
        for l in stderr {
            let s = String::from_utf8_lossy(l);
            let s = s.trim_end_matches(['\n', '\r']);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(s)
                && v.get("$message_type").and_then(|m| m.as_str()) == Some("artifact")
                && let Some(a) = v.get("artifact").and_then(|a| a.as_str())
            {
                artifacts.push(self.cwd.join(a));
            }
            stderr_lines.push(self.norm.norm(s));
        }

        // The unit's outputs: files in the out dir carrying its unique
        // `-C extra-filename` suffix, written by this compile.
        let mut files = Vec::new();
        for e in std::fs::read_dir(&self.out_dir).ok()?.flatten() {
            let name = e.file_name().to_str()?.to_owned();
            if name.starts_with('.') || !name.contains(&self.extra) {
                continue;
            }
            let m = e.metadata().ok()?;
            let mtime = m.mtime() as f64 + m.mtime_nsec() as f64 / 1e9;
            if m.is_file() && mtime + 1.0 >= start {
                files.push(name);
            }
        }
        files.sort();
        // Every artifact rustc announced must be among them.
        for a in &artifacts {
            let name = a.file_name()?.to_str()?;
            if a.parent() != Some(self.out_dir.as_path()) || !files.iter().any(|f| f == name) {
                return None;
            }
        }

        // Never cache outputs that carry this target dir as data.
        let input_paths: Vec<String> = entry
            .inputs
            .iter()
            .map(|(p, _)| self.norm.denorm(p))
            .filter(|p| p.starts_with(&self.norm.target))
            .collect();
        for name in files.iter().filter(|n| !n.ends_with(".d")) {
            let bytes = std::fs::read(self.out_dir.join(name)).ok()?;
            if !target_refs_are_inputs(&bytes, &self.norm.target, &input_paths) {
                return None;
            }
        }

        let final_dir = self.root.join("o").join(&entry.out);
        if final_dir.exists() {
            touch(&final_dir.join("meta.json"));
        } else {
            let tmp = self
                .root
                .join("o")
                .join(format!(".{}.tmp{}", entry.out, std::process::id()));
            let r = self.write_outputs(&tmp, &files, stderr_lines, wall);
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

    fn write_outputs(
        &self,
        dir: &Path,
        files: &[String],
        stderr: Vec<String>,
        wall: f64,
    ) -> Option<()> {
        std::fs::create_dir_all(dir).ok()?;
        let mut out = Outputs {
            files: Vec::new(),
            stderr,
            wall,
        };
        for name in files {
            let src = self.out_dir.join(name);
            let text = name.ends_with(".d");
            if text {
                let t = std::fs::read_to_string(&src).ok()?;
                std::fs::write(dir.join(name), self.norm.norm(&t)).ok()?;
            } else if !reflink_or_copy(&src, &dir.join(name)) {
                return None;
            }
            let hash = file_hash(&src, !text)?;
            out.files.push(OutFile {
                name: name.clone(),
                hash,
                text,
            });
        }
        std::fs::write(dir.join("meta.json"), serde_json::to_vec(&out).ok()?).ok()
    }
}

/// True when every occurrence of the target dir in a compiled output is the
/// start of one of the unit's own input files (an `OUT_DIR` file pulled in
/// with `include!`). Those name a file whose content is part of the key, so
/// a restored copy only differs in which target dir panic locations, debug
/// line tables, and diagnostics name. Anything else (`env!("OUT_DIR")` kept
/// as a runtime string, a proc macro embedding a target path) would make the
/// restored artifact read another build's files, so the unit is not cached.
fn target_refs_are_inputs(bytes: &[u8], target: &str, inputs: &[String]) -> bool {
    let t = target.as_bytes();
    let Some(&first) = t.first() else {
        return true;
    };
    let mut i = 0;
    while let Some(off) = bytes[i..].iter().position(|&b| b == first) {
        let at = i + off;
        let rest = &bytes[at..];
        if rest.starts_with(t)
            && !inputs.iter().any(|p| {
                rest.starts_with(p.as_bytes())
                    && !rest
                        .get(p.len())
                        .is_some_and(|c| c.is_ascii_alphanumeric() || b"/._-+".contains(c))
            })
        {
            return false;
        }
        i = at + 1;
    }
    true
}

/// Set a file's mtime to now: the LRU clock for eviction (`depcache_gc`).
fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// The output key a manifest line points at.
pub fn entry_out(line: &str) -> Option<String> {
    serde_json::from_str::<Entry>(line).ok().map(|e| e.out)
}

/// Source paths and env var names from a rustc dep-info file.
fn parse_dep_info(d: &str) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut env = Vec::new();
    for line in d.lines() {
        if let Some(e) = line.strip_prefix("# env-dep:") {
            let name = e.split_once('=').map_or(e, |(k, _)| k);
            if !env.iter().any(|x| x == name) {
                env.push(name.to_owned());
            }
            continue;
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // `target: dep dep` with spaces escaped as `\ `.
        let Some(idx) = find_rule_colon(line) else {
            continue;
        };
        for dep in split_escaped(&line[idx + 1..]) {
            if !files.contains(&dep) {
                files.push(dep);
            }
        }
    }
    (files, env)
}

fn find_rule_colon(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    (0..b.len()).find(|&i| b[i] == b':' && (i + 1 == b.len() || b[i + 1] == b' '))
}

fn split_escaped(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&' ') => {
                cur.push(' ');
                chars.next();
            }
            ' ' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Copy-on-write clone when the filesystem supports it (btrfs, xfs), else a
/// plain copy. Keeps the source's permissions.
fn reflink_or_copy(src: &Path, dst: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let _ = std::fs::remove_file(dst);
    if let (Ok(s), Ok(d)) = (std::fs::File::open(src), std::fs::File::create(dst)) {
        // SAFETY: both fds are open for the duration of the call.
        let r = unsafe { libc::ioctl(d.as_raw_fd(), libc::FICLONE, s.as_raw_fd()) };
        if r == 0 {
            if let Ok(m) = s.metadata() {
                let _ = d.set_permissions(m.permissions());
            }
            return true;
        }
    }
    std::fs::copy(src, dst).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn normalizes_profile_and_target_dirs() {
        let n = Norm::new(Path::new("/a/t1/debug")).unwrap();
        assert_eq!(
            n.norm("dependency=/a/t1/debug/deps"),
            format!("dependency={PROFILE_TOKEN}/deps")
        );
        assert_eq!(n.norm("/a/t1"), TARGET_TOKEN);
        assert_eq!(n.norm("/home/x/.cargo/src"), "/home/x/.cargo/src");
        let m = Norm::new(Path::new("/b/other/justrust-slots/3/debug")).unwrap();
        let a = n.norm("-L dependency=/a/t1/debug/deps");
        assert_eq!(
            m.denorm(&a),
            "-L dependency=/b/other/justrust-slots/3/debug/deps"
        );
    }

    #[test]
    fn finds_profile_dir() {
        assert_eq!(
            profile_dir(Path::new("/t/debug/deps")).unwrap(),
            Path::new("/t/debug")
        );
        assert_eq!(
            profile_dir(Path::new("/t/debug/build/serde-abc")).unwrap(),
            Path::new("/t/debug")
        );
        assert!(profile_dir(Path::new("/t/debug/examples")).is_none());
    }

    #[test]
    fn only_plain_outputs_are_cacheable() {
        let base = s(&[
            "--crate-name",
            "libc",
            "src/lib.rs",
            "--crate-type",
            "lib",
            "--emit=dep-info,metadata,link",
            "-C",
            "extra-filename=-5f4f",
            "--out-dir",
            "/t/debug/deps",
        ]);
        assert!(shape(&base).is_some());
        let with = |extra: &[&str]| {
            let mut v = base.clone();
            v.extend(s(extra));
            shape(&v)
        };
        assert!(with(&["-C", "incremental=/t/inc"]).is_none());
        assert!(with(&["--print", "cfg"]).is_none());
        assert!(with(&["--crate-type", "cdylib"]).is_none());
        let mut no_dep_info = base.clone();
        no_dep_info[5] = "--emit=metadata".into();
        assert!(shape(&no_dep_info).is_none());
        let mut emit_path = base.clone();
        emit_path[5] = "--emit=dep-info,link=/x/y".into();
        assert!(shape(&emit_path).is_none());
    }

    #[test]
    fn base_key_ignores_target_dir_but_not_flags() {
        let dir = std::env::temp_dir().join(format!("jr-depcache-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = |t: &str, opt: &str| {
            let n = Norm::new(&dir.join(t).join("debug")).unwrap();
            let p = dir.join(t).join("debug/deps");
            let args = s(&[
                "--crate-name",
                "x",
                "-C",
                opt,
                "-L",
                &format!("dependency={}", p.display()),
                "--diagnostic-width=80",
            ]);
            let mut h = H128::new();
            key_args(&args, &n, &dir, &mut h).unwrap();
            h.hex()
        };
        assert_eq!(key("t1", "opt-level=0"), key("t2", "opt-level=0"));
        assert_ne!(key("t1", "opt-level=0"), key("t1", "opt-level=1"));
    }

    #[test]
    fn parses_dep_info() {
        let d = "/t/debug/deps/x-1.d: src/lib.rs /t/debug/build/x-2/out/gen\\ x.rs\n\n\
                 src/lib.rs:\n\n# env-dep:CARGO_PKG_NAME=x\n# env-dep:FOO\n";
        let (files, env) = parse_dep_info(d);
        assert_eq!(files, vec!["src/lib.rs", "/t/debug/build/x-2/out/gen x.rs"]);
        assert_eq!(env, vec!["CARGO_PKG_NAME", "FOO"]);
    }

    #[test]
    fn target_paths_only_allowed_as_input_file_names() {
        let inputs = vec!["/t/debug/build/x-1/out/gen.rs".to_string()];
        let ok = b"\x00/t/debug/build/x-1/out/gen.rs\x01 and /home/src/lib.rs";
        assert!(target_refs_are_inputs(ok, "/t", &inputs));
        assert!(target_refs_are_inputs(
            b"no paths /tmp/x",
            "/t/debug-not",
            &inputs
        ));
        // env!("OUT_DIR") as a value: the bare dir is not an input.
        let bare = b"\x00/t/debug/build/x-1/out\x00";
        assert!(!target_refs_are_inputs(bare, "/t", &inputs));
        // A longer path that only shares the input's prefix.
        let longer = b"/t/debug/build/x-1/out/gen.rs.bak";
        assert!(!target_refs_are_inputs(longer, "/t", &inputs));
        assert!(!target_refs_are_inputs(b"/t/debug/deps", "/t", &[]));
    }

    /// Store a fake compile from one target dir, restore into another, and
    /// check that a changed source, a corrupt manifest, or missing cached
    /// files all miss instead of restoring something wrong.
    #[test]
    fn round_trip_and_fail_open() {
        let dir = std::env::temp_dir().join(format!("jr-depcache-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src.rs");
        std::fs::create_dir_all(dir.join("t1/debug/deps")).unwrap();
        std::fs::create_dir_all(dir.join("t2/debug/deps")).unwrap();
        std::fs::write(&src, "pub fn f() {}").unwrap();
        let mk = |t: &str| {
            let out_dir = dir.join(t).join("debug/deps");
            Plan {
                base: "b".repeat(32),
                out_dir: out_dir.clone(),
                extra: "-abc".into(),
                crate_name: "x".into(),
                cwd: dir.clone(),
                norm: Norm::new(out_dir.parent().unwrap()).unwrap(),
                root: dir.join("cache"),
            }
        };
        let p1 = mk("t1");
        let deps1 = &p1.out_dir;
        std::fs::write(
            deps1.join("x-abc.d"),
            format!("{0}/x-abc.d: {1}\n\n{1}:\n", deps1.display(), src.display()),
        )
        .unwrap();
        std::fs::write(deps1.join("libx-abc.rmeta"), b"rmeta").unwrap();
        let notice = format!(
            r#"{{"$message_type":"artifact","artifact":"{}/libx-abc.rmeta","emit":"metadata"}}"#,
            deps1.display()
        );
        p1.store(0.0, 2.5, &[notice.into_bytes()]).unwrap();

        let p2 = mk("t2");
        let mut err = Vec::new();
        let hit = p2.restore(&mut err).expect("hit");
        assert_eq!(hit.saved_secs, 2.5);
        let deps2 = &p2.out_dir;
        assert_eq!(
            std::fs::read(deps2.join("libx-abc.rmeta")).unwrap(),
            b"rmeta"
        );
        let d2 = std::fs::read_to_string(deps2.join("x-abc.d")).unwrap();
        assert!(d2.starts_with(&format!("{}/x-abc.d", deps2.display())));
        let err = String::from_utf8(err).unwrap();
        assert!(err.contains(&format!("{}/libx-abc.rmeta", deps2.display())));

        // Changed source: miss.
        std::fs::write(&src, "pub fn g() {}").unwrap();
        assert!(p2.restore(&mut Vec::new()).is_none());
        std::fs::write(&src, "pub fn f() {}").unwrap();
        assert!(p2.restore(&mut Vec::new()).is_some());

        // Cached outputs gone: miss, nothing replayed.
        let o = dir.join("cache/o");
        for e in std::fs::read_dir(&o).unwrap().flatten() {
            let _ = std::fs::remove_file(e.path().join("libx-abc.rmeta"));
        }
        let mut err = Vec::new();
        assert!(p2.restore(&mut err).is_none());
        assert!(err.is_empty());

        // Corrupt manifest: miss.
        std::fs::write(p2.manifest(), "not json\n").unwrap();
        assert!(p2.restore(&mut Vec::new()).is_none());

        // Unusable cache root: store and restore just fail.
        let mut p3 = mk("t1");
        p3.root = PathBuf::from("/proc/justrust-no-such-dir");
        assert!(p3.store(0.0, 1.0, &[]).is_none());
        assert!(p3.restore(&mut Vec::new()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
