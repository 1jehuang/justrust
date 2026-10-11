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
/// 3: `--extern` keyed by crate metadata only, `CARGO_HOME` not keyed, and
/// the C library checked for compatibility at restore instead of keyed.
/// 4: `CARGO_TARGET_DIR` not keyed.
/// 5: dependency units compile with the target dir remapped.
/// 2: the cargo home is a placeholder (portable across machines), and
/// proc macros and bins key the C library they link against.
const SCHEME: &str = "justrust-depcache-5";
const PROFILE_TOKEN: &str = "@@JUSTRUST_PROFILE_DIR@@";
const TARGET_TOKEN: &str = "@@JUSTRUST_TARGET_DIR@@";
/// The cargo home (registry sources, git checkouts), so keys and manifest
/// lines are the same on every machine and can be shared remotely.
const CARGO_HOME_TOKEN: &str = "@@JUSTRUST_CARGO_HOME@@";
const XATTR: &str = "user.justrust.h";
/// Memoized `extern_hash` (metadata only), same `<stamp>|<hash>` format.
const XATTR_META: &str = "user.justrust.m";

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
pub(crate) struct H128(
    #[allow(deprecated)] std::hash::SipHasher,
    #[allow(deprecated)] std::hash::SipHasher,
);

impl H128 {
    #[allow(deprecated)]
    pub(crate) fn new() -> H128 {
        H128(
            std::hash::SipHasher::new_with_keys(0x6a75_7374, 0x7275_7374),
            std::hash::SipHasher::new_with_keys(0x6465_7063, 0x6163_6865),
        )
    }
    /// Length-prefixed, so ("ab","c") and ("a","bc") differ.
    pub(crate) fn field(&mut self, b: &[u8]) {
        self.0.write_u64(b.len() as u64);
        self.1.write_u64(b.len() as u64);
        self.raw(b);
    }
    fn raw(&mut self, b: &[u8]) {
        self.0.write(b);
        self.1.write(b);
    }
    pub(crate) fn hex(&self) -> String {
        format!("{:016x}{:016x}", self.0.finish(), self.1.finish())
    }
}

/// `CARGO_HOME`, else `~/.cargo`.
pub(crate) fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
        .unwrap_or_default()
}

/// Maps the build's profile and target dirs, and the cargo home, to
/// placeholders and back.
#[derive(Debug, Clone)]
pub struct Norm {
    profile: String,
    pub(crate) target: String,
    pub(crate) cargo: Option<String>,
}

impl Norm {
    pub fn new(profile_dir: &Path) -> Option<Norm> {
        Norm::with_cargo_home(profile_dir, &cargo_home())
    }
    pub fn with_cargo_home(profile_dir: &Path, cargo_home: &Path) -> Option<Norm> {
        let profile = profile_dir.to_str()?.trim_end_matches('/').to_owned();
        let target = profile_dir.parent()?.to_str()?.to_owned();
        if profile.len() < 2 || target.len() < 2 {
            return None;
        }
        let cargo = cargo_home
            .to_str()
            .map(|c| c.trim_end_matches('/').to_owned())
            // A target dir inside the cargo home would be rewritten twice.
            .filter(|c| c.len() >= 2 && !target.starts_with(c.as_str()));
        Some(Norm {
            profile,
            target,
            cargo,
        })
    }
    pub fn norm(&self, s: &str) -> String {
        let mut s = if s.contains(&self.target) {
            s.replace(&self.profile, PROFILE_TOKEN)
                .replace(&self.target, TARGET_TOKEN)
        } else {
            s.to_owned()
        };
        if let Some(c) = &self.cargo
            && s.contains(c.as_str())
        {
            s = s.replace(c.as_str(), CARGO_HOME_TOKEN);
        }
        s
    }
    pub fn denorm(&self, s: &str) -> String {
        let s = s
            .replace(PROFILE_TOKEN, &self.profile)
            .replace(TARGET_TOKEN, &self.target);
        match &self.cargo {
            Some(c) => s.replace(CARGO_HOME_TOKEN, c),
            None => s,
        }
    }
}

/// Profile dir for a rustc `--out-dir`:
/// - classic layout: `<P>/deps`, or `<P>/build/<pkg>-<hash>` (build scripts);
/// - new build-dir layout (cargo nightly, `-Zbuild-dir-new-layout`, default in
///   the pinned toolchains): `<P>/build/<pkg>/<hash>/out` for every unit,
///   libraries and build scripts alike.
pub fn profile_dir(out_dir: &Path) -> Option<PathBuf> {
    if out_dir.file_name()? == "deps" {
        return Some(out_dir.parent()?.to_path_buf());
    }
    let parent = out_dir.parent()?;
    if parent.file_name()? == "build" {
        return Some(parent.parent()?.to_path_buf());
    }
    if out_dir.file_name()? == "out" {
        let build = parent.parent()?.parent()?;
        if build.file_name()? == "build" && is_unit_hash(parent.file_name()?.to_str()?) {
            return Some(build.parent()?.to_path_buf());
        }
    }
    None
}

/// Cargo's 16-hex-digit unit hash (`-C extra-filename=-<hash>`).
fn is_unit_hash(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Profile dir for a build script's `OUT_DIR`: `<P>/build/<pkg>-<hash>/out`
/// (classic) or `<P>/build/<pkg>/<hash>/out` (new build-dir layout).
pub fn build_script_profile_dir(out_dir: &Path) -> Option<PathBuf> {
    if out_dir.file_name()? != "out" {
        return None;
    }
    let unit = out_dir.parent()?;
    let up = unit.parent()?;
    if up.file_name()? == "build" {
        return Some(up.parent()?.to_path_buf());
    }
    let build = up.parent()?;
    if build.file_name()? == "build" && is_unit_hash(unit.file_name()?.to_str()?) {
        return Some(build.parent()?.to_path_buf());
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
    pub(crate) root: PathBuf,
    /// Extra rustc arguments the unit must be compiled with when it misses
    /// (keyed): the target dir remapped to a fixed path, so outputs of
    /// crates that `include!` generated `OUT_DIR` files are the same in
    /// every target dir and on every machine.
    pub extra_args: Vec<String>,
}

/// Fixed path the target dir is remapped to in dependency outputs.
pub const REMAPPED_TARGET: &str = "/justrust-target";

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
    if s.out_dir.is_empty() || s.crate_name.is_empty() {
        return None;
    }
    // Outputs are found by their `-C extra-filename` suffix. The new layout
    // compiles build scripts without one, but gives every unit its own out
    // dir, so there the whole dir is the unit's.
    if s.extra.len() < 2 && !(s.extra.is_empty() && is_unit_out_dir(Path::new(&s.out_dir))) {
        return None;
    }
    Some(s)
}

/// `<P>/build/<pkg>/<hash>/out`: a per-unit out dir of the new layout.
fn is_unit_out_dir(out_dir: &Path) -> bool {
    out_dir.file_name().is_some_and(|n| n == "out")
        && out_dir
            .parent()
            .and_then(|u| u.file_name())
            .and_then(|u| u.to_str())
            .is_some_and(is_unit_hash)
        && profile_dir(out_dir).is_some()
}

/// Env vars that are part of the base key. `CARGO_MAKEFLAGS` carries
/// jobserver fds, which differ every run.
fn keyed_env(name: &str) -> bool {
    // CARGO_HOME and the target dir: their values are always placeholders,
    // but they reach rustc only when the user (or a build slot) set them, so
    // keying them would split builds that set them from builds that do not.
    if matches!(
        name,
        "CARGO_MAKEFLAGS"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "CARGO_TERM_VERBOSE"
            | "CARGO_HOME"
            | "CARGO_TARGET_DIR"
            | "CARGO_BUILD_TARGET_DIR"
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

/// The host's glibc version as (major, minor), `None` on other C libraries.
fn host_glibc() -> Option<(u32, u32)> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: returns a pointer to a static NUL-terminated string.
        let p = unsafe { libc::gnu_get_libc_version() };
        if !p.is_null() {
            // SAFETY: non-null static C string.
            let v = unsafe { std::ffi::CStr::from_ptr(p) };
            return parse_glibc(&v.to_string_lossy());
        }
    }
    None
}

/// `2.39` (or `2.39.9000`) as (2, 39).
fn parse_glibc(v: &str) -> Option<(u32, u32)> {
    let mut it = v.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// C library family the host uses: part of the key for dynamically linked
/// outputs (a musl host cannot load a glibc proc macro at any version).
fn libc_family() -> &'static str {
    if cfg!(target_env = "gnu") {
        "glibc"
    } else if cfg!(target_env = "musl") {
        "musl"
    } else {
        "other"
    }
}

/// Newest `GLIBC_x.y` symbol version a dynamically linked ELF file
/// requires. `Some((0, 0))` when it needs none (not ELF, static), `None`
/// if an ELF file cannot be parsed. Only the dynamic string table is
/// scanned: version names live there, and only when referenced, while the
/// same bytes in ordinary data (`.rodata`) mean nothing.
pub(crate) fn needed_glibc(path: &Path) -> Option<(u32, u32)> {
    let mut head = [0u8; 4];
    {
        use std::io::Read;
        let mut f = std::fs::File::open(path).ok()?;
        if f.read_exact(&mut head).is_err() || &head != b"\x7fELF" {
            return Some((0, 0));
        }
    }
    let Some(b) = elf_section(path, ".dynstr") else {
        return Some((0, 0));
    };
    let mut best = (0, 0);
    let pat = b"GLIBC_";
    let mut i = 0;
    while let Some(off) = b[i..].windows(pat.len()).position(|w| w == pat) {
        let start = i + off + pat.len();
        let end = b[start..]
            .iter()
            .position(|c| !(c.is_ascii_digit() || *c == b'.'))
            .map_or(b.len(), |n| start + n);
        if let Some(v) = std::str::from_utf8(&b[start..end])
            .ok()
            .and_then(parse_glibc)
        {
            best = best.max(v);
        }
        i = end.max(start);
    }
    Some(best)
}

pub(crate) fn rustc_version(real: &Path) -> Option<String> {
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
    // Paths under the target dir (generated `OUT_DIR` sources) are written
    // into crate metadata and debuginfo. Remapping them makes those bytes,
    // and so every dependent's key, independent of where the target dir is.
    // Only when the caller does not remap paths itself.
    let extra_args = if args.iter().any(|a| a.starts_with("--remap-path-prefix")) {
        Vec::new()
    } else {
        vec![format!(
            "--remap-path-prefix={}={REMAPPED_TARGET}",
            norm.target
        )]
    };
    for a in &extra_args {
        h.field(norm.norm(a).as_bytes());
    }
    // Proc macros and bins are dynamically linked against the C library.
    // Only its family is keyed: the version an output needs is recorded
    // when it is stored and checked against this host on restore, so a
    // build machine with an older glibc can still use them when they load.
    if args.iter().any(|a| {
        a == "proc-macro" || a == "bin" || a.ends_with("=proc-macro") || a.ends_with("=bin")
    }) {
        h.field(libc_family().as_bytes());
    }
    let mut env: Vec<&(String, String)> = env.iter().filter(|(k, _)| keyed_env(k)).collect();
    env.sort();
    for (k, v) in &env {
        h.field(k.as_bytes());
        h.field(norm.norm(v).as_bytes());
    }
    // Diagnosis: `JUSTRUST_DEPCACHE_EXPLAIN=<dir>` writes the normalized key
    // inputs of every unit, so two misses of the same crate can be diffed.
    if let Some(dir) = std::env::var_os("JUSTRUST_DEPCACHE_EXPLAIN") {
        let mut lines = vec![format!("cwd {}", norm.norm(cwd.to_str()?))];
        lines.extend(args.iter().map(|a| format!("arg {}", norm.norm(a))));
        lines.extend(env.iter().map(|(k, v)| format!("env {k}={}", norm.norm(v))));
        let name = format!("{}-{}.txt", s.crate_name, h.hex());
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(Path::new(&dir).join(name), lines.join("\n"));
    }
    Some(Plan {
        base: h.hex(),
        out_dir,
        extra: s.extra,
        crate_name: s.crate_name,
        cwd,
        norm,
        root,
        extra_args,
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
            h.field(extern_hash(&cwd.join(path))?.as_bytes());
        }
        None => h.field(v.as_bytes()),
    }
    Some(())
}

/// Key of an `--extern` artifact: its crate metadata, which is all a
/// dependent reads from it. Object code (debuginfo naming the sysroot) and
/// linker or C runtime differences (another host's gcc) then stop cascading
/// through dependents' keys, while any metadata change (SVH, spans) still
/// changes them, so mismatched artifacts can never meet (E0460).
/// - `.rmeta`: the whole file;
/// - `.rlib`: the `lib.rmeta` archive member;
/// - `.so` (proc macro, dylib): the ELF `.rustc` section.
///
/// Anything unparsable falls back to the whole-file content hash. Memoized
/// in the `user.justrust.m` xattr, validated like `file_hash`.
pub(crate) fn extern_hash(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let st = stamp(&meta);
    if let Some(v) = xattr_get(path, XATTR_META)
        && let Some((s, hash)) = v.rsplit_once('|')
        && s == st
    {
        return Some(hash.to_owned());
    }
    let hash = match metadata_bytes(path) {
        Some(b) => {
            let mut h = H128::new();
            h.field(b"meta");
            h.field(&b);
            h.hex()
        }
        None => format!("f{}", hash_file_content(path)?),
    };
    xattr_set(path, XATTR_META, &format!("{st}|{hash}"));
    Some(hash)
}

/// The crate metadata embedded in a compiled artifact, or `None` if the
/// container cannot be parsed.
fn metadata_bytes(path: &Path) -> Option<Vec<u8>> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "rmeta" => {
            let b = std::fs::read(path).ok()?;
            b.starts_with(b"rust").then_some(b)
        }
        "rlib" => ar_member(path, "lib.rmeta"),
        "so" | "dylib" => elf_section(path, ".rustc"),
        _ => None,
    }
}

/// Content of a named member of an `ar` archive (GNU or BSD short names).
fn ar_member(path: &Path, want: &str) -> Option<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut magic = [0u8; 8];
    f.read_exact_at(&mut magic, 0).ok()?;
    if &magic != b"!<arch>\n" {
        return None;
    }
    let mut off = 8u64;
    while off + 60 <= len {
        let mut hdr = [0u8; 60];
        f.read_exact_at(&mut hdr, off).ok()?;
        if &hdr[58..60] != b"`\n" {
            return None;
        }
        let name = std::str::from_utf8(&hdr[..16]).ok()?.trim_end();
        let size: u64 = std::str::from_utf8(&hdr[48..58])
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let data = off + 60;
        if data + size > len {
            return None;
        }
        if name.trim_end_matches('/') == want {
            let mut buf = vec![0u8; usize::try_from(size).ok()?];
            f.read_exact_at(&mut buf, data).ok()?;
            return Some(buf);
        }
        off = data + size + (size & 1);
    }
    None
}

/// Content of a named section of a 64-bit little-endian ELF file.
fn elf_section(path: &Path, want: &str) -> Option<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut eh = [0u8; 64];
    f.read_exact_at(&mut eh, 0).ok()?;
    // ELF magic, ELFCLASS64, little endian.
    if &eh[..4] != b"\x7fELF" || eh[4] != 2 || eh[5] != 1 {
        return None;
    }
    let u16_at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u64_at = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let shoff = u64_at(&eh, 0x28);
    let shentsize = u64::from(u16_at(&eh, 0x3a));
    let shnum = u64::from(u16_at(&eh, 0x3c));
    let shstrndx = u64::from(u16_at(&eh, 0x3e));
    if shentsize < 64 || shnum == 0 || shstrndx >= shnum || shoff + shnum * shentsize > len {
        return None;
    }
    let mut shdrs = vec![0u8; usize::try_from(shnum * shentsize).ok()?];
    f.read_exact_at(&mut shdrs, shoff).ok()?;
    let sh = |i: u64| {
        let o = usize::try_from(i * shentsize).unwrap();
        (
            u32_at(&shdrs, o),
            u64_at(&shdrs, o + 0x18),
            u64_at(&shdrs, o + 0x20),
        )
    };
    let read = |off: u64, size: u64| -> Option<Vec<u8>> {
        if off.checked_add(size)? > len {
            return None;
        }
        let mut buf = vec![0u8; usize::try_from(size).ok()?];
        f.read_exact_at(&mut buf, off).ok()?;
        Some(buf)
    };
    let (_, str_off, str_size) = sh(shstrndx);
    let names = read(str_off, str_size)?;
    for i in 0..shnum {
        let (name, off, size) = sh(i);
        let name = names.get(name as usize..)?;
        let end = name.iter().position(|&c| c == 0)?;
        if &name[..end] == want.as_bytes() {
            return read(off, size);
        }
    }
    None
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

fn xattr_get(path: &Path, attr: &str) -> Option<String> {
    let p = CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = CString::new(attr).ok()?;
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

fn xattr_set(path: &Path, attr: &str, value: &str) {
    let (Ok(p), Ok(name)) = (
        CString::new(path.as_os_str().as_bytes()),
        CString::new(attr),
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
        && let Some(v) = xattr_get(path, XATTR)
        && let Some((s, hash)) = v.rsplit_once('|')
        && s == st
    {
        return Some(hash.to_owned());
    }
    let hash = hash_file_content(path)?;
    if memo {
        xattr_set(path, XATTR, &format!("{st}|{hash}"));
    }
    Some(hash)
}

pub(crate) fn hash_file_content(path: &Path) -> Option<String> {
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
pub(crate) struct Entry {
    /// (normalized path, content hash)
    pub(crate) inputs: Vec<(String, String)>,
    /// (name, normalized value) for env vars the unit read.
    pub(crate) env: Vec<(String, Option<String>)>,
    pub(crate) out: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct OutFile {
    pub(crate) name: String,
    pub(crate) hash: String,
    /// Text with placeholders (dep-info), rewritten on restore.
    pub(crate) text: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Outputs {
    pub(crate) files: Vec<OutFile>,
    /// Normalized stderr lines, replayed on a hit.
    stderr: Vec<String>,
    /// Wall seconds the original compile took.
    wall: f64,
    /// Newest glibc any output needs (dynamically linked proc macros and
    /// bins), e.g. `[2, 39]`. Restored only where the host has at least it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    glibc: Option<(u32, u32)>,
}

pub struct Hit {
    pub saved_secs: f64,
    /// Result key (`o/<out>`) of the restored entry.
    #[allow(dead_code)] // read by tests; the line carries it too
    pub out: String,
    /// The matched manifest line, exactly as stored.
    pub line: String,
}

impl Plan {
    pub(crate) fn manifest(&self) -> PathBuf {
        self.root.join("m").join(format!("{}.jsonl", self.base))
    }

    /// Look up and restore. On success the outputs are in place and the
    /// recorded stderr has been written to `err`. Matching lines are tried
    /// newest first. One whose outputs are missing or broken (evicted, or a
    /// peer appended a line before its outputs arrived) is skipped in favor
    /// of the next older match.
    pub fn restore(&self, err: &mut dyn Write) -> Option<Hit> {
        let text = std::fs::read_to_string(self.manifest()).ok()?;
        let mut memo: HashMap<String, Option<String>> = HashMap::new();
        let mut tried = Vec::new();
        for line in text.lines().rev() {
            let Ok(entry) = serde_json::from_str::<Entry>(line) else {
                continue;
            };
            if tried.contains(&entry.out) || !self.matches(&entry, &mut memo) {
                continue;
            }
            if let Some(saved_secs) = self.restore_entry(&entry, err) {
                return Some(Hit {
                    saved_secs,
                    out: entry.out,
                    line: line.to_owned(),
                });
            }
            tried.push(entry.out);
        }
        None
    }

    /// Restore one entry's outputs. `None` (nothing replayed) if they are
    /// missing or unreadable.
    fn restore_entry(&self, entry: &Entry, err: &mut dyn Write) -> Option<f64> {
        let dir = self.root.join("o").join(&entry.out);
        // Mark the entry used before reading it, so a concurrent eviction
        // that already picked it sees the touch and puts it back.
        touch(&dir.join("meta.json"));
        let outs: Outputs =
            serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
        // Built against a newer glibc than this host has: it would not load.
        if let Some(need) = outs.glibc
            && host_glibc().is_some_and(|have| have < need)
        {
            return None;
        }
        // Check every file first, so a broken entry leaves the out dir alone.
        if outs.files.iter().any(|f| {
            f.name.contains('/') || f.name.starts_with('.') || !dir.join(&f.name).is_file()
        }) {
            return None;
        }
        // Restore every file before replaying anything, so a partial restore
        // just falls back to a normal compile that overwrites it.
        for f in &outs.files {
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
                xattr_set(&dst, XATTR, &format!("{}|{}", stamp(&m), f.hash));
            }
        }
        for l in &outs.stderr {
            let _ = err.write_all(self.norm.denorm(l).as_bytes());
            let _ = err.write_all(b"\n");
        }
        let _ = err.flush();
        Some(outs.wall)
    }

    pub(crate) fn matches(&self, e: &Entry, memo: &mut HashMap<String, Option<String>>) -> bool {
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
    /// Returns the manifest line (for the remote upload queue).
    pub fn store(&self, start: f64, wall: f64, stderr: &[Vec<u8>]) -> Option<String> {
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
            return Some(line);
        }
        paths::append_line(&self.manifest(), &line).ok()?;
        Some(line)
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
            glibc: None,
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
            if !text && !name.ends_with(".rlib") && !name.ends_with(".rmeta") {
                let need = needed_glibc(&src)?;
                if need > (0, 0) {
                    out.glibc = Some(out.glibc.map_or(need, |g| g.max(need)));
                }
            }
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
pub(crate) fn target_refs_are_inputs(bytes: &[u8], target: &str, inputs: &[String]) -> bool {
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
pub(crate) fn touch(path: &Path) {
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
pub(crate) fn reflink_or_copy(src: &Path, dst: &Path) -> bool {
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

    #[test]
    fn glibc_needs_are_read_from_the_dynamic_string_table() {
        let d = std::env::temp_dir().join(format!("jr-glibc-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.so");
        // Not ELF: needs nothing, even if the bytes look like a version.
        std::fs::write(&f, b"not elf GLIBC_9.99").unwrap();
        assert_eq!(needed_glibc(&f), Some((0, 0)));
        assert_eq!(parse_glibc("2.39.9000"), Some((2, 39)));
        assert!(parse_glibc("x").is_none());
        // The test binary has "GLIBC_9.99" in its data (above) but needs at
        // most the host's glibc, and at least 2.2.5 on x86_64.
        if let (Some(have), Ok(me)) = (host_glibc(), std::env::current_exe()) {
            let need = needed_glibc(&me).unwrap();
            assert!(need >= (2, 2) && need <= have, "{need:?} vs {have:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn extern_keys_use_only_the_crate_metadata() {
        let d = std::env::temp_dir().join(format!("jr-extern-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // An ar archive with a lib.rmeta member and an object file.
        let ar = |obj: &[u8], meta: &[u8]| {
            let mut v = b"!<arch>\n".to_vec();
            for (name, data) in [("x.o/", obj), ("lib.rmeta/", meta)] {
                v.extend(
                    format!(
                        "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                        0,
                        0,
                        0,
                        644,
                        data.len()
                    )
                    .as_bytes(),
                );
                v.extend_from_slice(data);
                if data.len() % 2 == 1 {
                    v.push(b'\n');
                }
            }
            v
        };
        let a = d.join("liba.rlib");
        let b = d.join("libb.rlib");
        std::fs::write(&a, ar(b"object code /home/alice/sysroot", b"rust-meta-1")).unwrap();
        std::fs::write(&b, ar(b"other object code, other paths!", b"rust-meta-1")).unwrap();
        assert_eq!(ar_member(&a, "lib.rmeta").unwrap(), b"rust-meta-1");
        // Same metadata, different object code: same key.
        assert_eq!(extern_hash(&a), extern_hash(&b));
        // Different metadata: different key (and the memo follows the file).
        std::fs::write(&b, ar(b"other object code, other paths!", b"rust-meta-2")).unwrap();
        assert_ne!(extern_hash(&a), extern_hash(&b));
        // Unparsable: whole-content fallback, still distinct.
        let c = d.join("libc.rlib");
        std::fs::write(&c, b"garbage").unwrap();
        assert!(extern_hash(&c).unwrap().starts_with('f'));
        // A real ELF with a .rustc section: this test binary has none, the
        // parser must say so rather than misread.
        if let Ok(me) = std::env::current_exe() {
            assert!(elf_section(&me, ".text").is_some());
            assert!(elf_section(&me, ".rustc").is_none());
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cargo_home_is_not_keyed() {
        assert!(!keyed_env("CARGO_HOME"));
        assert!(!keyed_env("CARGO_TARGET_DIR"));
        assert!(keyed_env("CARGO_PKG_NAME"));
        assert!(keyed_env("CARGO_ENCODED_RUSTFLAGS"));
    }

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
    fn cargo_home_is_portable_across_machines() {
        let a = Norm::with_cargo_home(Path::new("/w/t/debug"), Path::new("/home/alice/.cargo"))
            .unwrap();
        let b = Norm::with_cargo_home(Path::new("/x/y/debug"), Path::new("/opt/cargo/")).unwrap();
        let src = "/home/alice/.cargo/registry/src/idx/serde-1.0.0/src/lib.rs";
        let n = a.norm(src);
        assert_eq!(
            n,
            format!("{CARGO_HOME_TOKEN}/registry/src/idx/serde-1.0.0/src/lib.rs")
        );
        assert_eq!(
            n,
            b.norm("/opt/cargo/registry/src/idx/serde-1.0.0/src/lib.rs")
        );
        assert_eq!(
            b.denorm(&n),
            "/opt/cargo/registry/src/idx/serde-1.0.0/src/lib.rs"
        );
        // Both kinds of path in one string.
        let mixed = a.norm("-L dependency=/w/t/debug/deps /home/alice/.cargo/x");
        assert_eq!(
            b.denorm(&mixed),
            "-L dependency=/x/y/debug/deps /opt/cargo/x"
        );
        // A target dir inside the cargo home disables the cargo token.
        let c = Norm::with_cargo_home(Path::new("/c/target/debug"), Path::new("/c")).unwrap();
        assert!(c.cargo.is_none());
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
        // New build-dir layout (pinned nightly cargo): every unit's out dir.
        assert_eq!(
            profile_dir(Path::new("/t/debug/build/serde_json/8ada6c1221654e94/out")).unwrap(),
            Path::new("/t/debug")
        );
        assert_eq!(
            profile_dir(Path::new(
                "/t/debug/build/aho-corasick/054c51e9873a2c10/out"
            ))
            .unwrap(),
            Path::new("/t/debug")
        );
        // Classic build-script OUT_DIR is not an rustc out dir.
        assert!(profile_dir(Path::new("/t/debug/build/serde-abc/out")).is_none());
        assert!(profile_dir(Path::new("/t/debug/build/x/not-a-hash/out")).is_none());
    }

    #[test]
    fn finds_build_script_profile_dir() {
        assert_eq!(
            build_script_profile_dir(Path::new("/t/debug/build/serde-abc/out")).unwrap(),
            Path::new("/t/debug")
        );
        assert_eq!(
            build_script_profile_dir(Path::new("/t/debug/build/serde/4e5510cdae32d8e3/out"))
                .unwrap(),
            Path::new("/t/debug")
        );
        assert!(build_script_profile_dir(Path::new("/t/debug/build/serde-abc")).is_none());
        assert!(build_script_profile_dir(Path::new("/t/debug/x/serde/zz/out")).is_none());
    }

    /// The new layout keys the same as the classic one: the profile dir is
    /// the placeholder, so two target dirs share entries.
    #[test]
    fn new_layout_out_dir_normalizes() {
        let a = Path::new("/a/t1/debug/build/itoa/6a6d3f3d53caa1d6/out");
        let b = Path::new("/b/t2/debug/build/itoa/6a6d3f3d53caa1d6/out");
        let na = Norm::new(&profile_dir(a).unwrap()).unwrap();
        let nb = Norm::new(&profile_dir(b).unwrap()).unwrap();
        assert_eq!(na.norm(a.to_str().unwrap()), nb.norm(b.to_str().unwrap()));
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
        // New layout build script: no extra-filename, own unit out dir.
        let bs = |out: &str| {
            shape(&s(&[
                "--crate-name",
                "build_script_build",
                "build.rs",
                "--crate-type",
                "bin",
                "--emit=dep-info,link",
                "--out-dir",
                out,
            ]))
        };
        assert!(bs("/t/debug/build/serde/c95beadfc4874447/out").is_some());
        assert!(bs("/t/debug/deps").is_none());
        assert!(bs("/t/debug/build/serde-c95beadfc4874447").is_none());
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
                extra_args: Vec::new(),
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
        let stored = std::fs::read_to_string(p2.manifest()).unwrap();
        assert_eq!(stored.lines().last().unwrap(), hit.line);
        assert_eq!(entry_out(&hit.line).as_deref(), Some(hit.out.as_str()));
        let deps2 = &p2.out_dir;
        assert_eq!(
            std::fs::read(deps2.join("libx-abc.rmeta")).unwrap(),
            b"rmeta"
        );
        let d2 = std::fs::read_to_string(deps2.join("x-abc.d")).unwrap();
        assert!(d2.starts_with(&format!("{}/x-abc.d", deps2.display())));
        let err = String::from_utf8(err).unwrap();
        assert!(err.contains(&format!("{}/libx-abc.rmeta", deps2.display())));

        // An entry needing a newer glibc than this host has is skipped.
        if host_glibc().is_some() {
            let mp = p2.root.join("o").join(&hit.out).join("meta.json");
            let orig = std::fs::read(&mp).unwrap();
            let mut v: serde_json::Value = serde_json::from_slice(&orig).unwrap();
            v["glibc"] = serde_json::json!([99, 0]);
            std::fs::write(&mp, v.to_string()).unwrap();
            assert!(p2.restore(&mut Vec::new()).is_none());
            v["glibc"] = serde_json::json!([2, 2]);
            std::fs::write(&mp, v.to_string()).unwrap();
            assert!(p2.restore(&mut Vec::new()).is_some());
            std::fs::write(&mp, orig).unwrap();
        }

        // Changed source: miss.
        std::fs::write(&src, "pub fn g() {}").unwrap();
        assert!(p2.restore(&mut Vec::new()).is_none());
        std::fs::write(&src, "pub fn f() {}").unwrap();
        assert!(p2.restore(&mut Vec::new()).is_some());

        // A newer matching line whose outputs never arrived (a peer's line,
        // or an evicted entry): skipped, the older line still restores.
        let good = std::fs::read_to_string(p2.manifest()).unwrap();
        let good_line = good.lines().last().unwrap().to_owned();
        let mut e: Entry = serde_json::from_str(&good_line).unwrap();
        e.out = "f".repeat(32);
        let newer = serde_json::to_string(&e).unwrap();
        paths::append_line(&p2.manifest(), &newer).unwrap();
        let mut err = Vec::new();
        let hit = p2.restore(&mut err).expect("older line restores");
        assert_eq!(hit.line, good_line);
        assert!(!err.is_empty());
        // A newer line whose meta.json lists a file that is gone: skipped too.
        let broken = dir.join("cache/o").join("e".repeat(32));
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(
            broken.join("meta.json"),
            r#"{"files":[{"name":"libx-abc.rmeta","hash":"h","text":false}],"stderr":["bad"],"wall":9.0}"#,
        )
        .unwrap();
        e.out = "e".repeat(32);
        paths::append_line(&p2.manifest(), &serde_json::to_string(&e).unwrap()).unwrap();
        let mut err = Vec::new();
        let hit = p2.restore(&mut err).expect("older line restores");
        assert_eq!(hit.line, good_line);
        assert!(!String::from_utf8(err).unwrap().contains("bad"));

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
