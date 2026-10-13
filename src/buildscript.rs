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
    // ELF, or Mach-O (thin 64-bit, or universal) on macOS.
    if !matches!(
        &magic,
        b"\x7fELF" | b"\xcf\xfa\xed\xfe" | b"\xca\xfe\xba\xbe"
    ) {
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
    crate::sys::set_mode(&tmp, 0o755).ok()?;
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
        "CARGO_MAKEFLAGS"
            | "RUSTC"
            | "RUSTDOC"
            | "CARGO"
            | "NUM_JOBS"
            | "CARGO_BUILD_JOBS"
            | "JOBSERVER_FDS"
            // Set only by some callers, value a placeholder anyway (as in
            // depcache::keyed_env).
            | "CARGO_HOME"
            | "CARGO_TARGET_DIR"
            | "CARGO_BUILD_TARGET_DIR"
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
    /// Keyed by the C toolchain instead of the host package database
    /// (`HERMETIC`): portable, so the prebuilt store may carry it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hermetic: bool,
}

struct Plan {
    real: PathBuf,
    base: String,
    root: PathBuf,
    norm: Norm,
    out_dir: PathBuf,
    cwd: PathBuf,
    cargo_home: PathBuf,
    hermetic: bool,
}

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
        .unwrap_or_default()
}

/// C and C++ flags that keep machine paths out of objects a registry build
/// script compiles (cc crate, cmake honor `CFLAGS`/`CXXFLAGS`): the cargo
/// home (and its resolved registry/git dirs) and the target dir map to the
/// same fixed paths rustc uses for dependency units. Without them, static
/// archives like ring's and aws-lc's embed the source path in DWARF line
/// tables, so their bytes, and every dependent's depcache key, differ per
/// user (FINDINGS 30: 25 of 833 Desktop units, aws_sdk_bedrock 11 s).
/// Appended to what the user set, set only for this script's process, and
/// part of the key (the env is keyed after this runs).
fn path_map_flags(norm: &Norm) -> String {
    let mut maps = Vec::new();
    if let Some(c) = &norm.cargo {
        maps.push(format!(
            "-ffile-prefix-map={c}={}",
            depcache::REMAPPED_CARGO_HOME
        ));
        // Same set on every machine, like rustc's (depcache::remap_args).
        for sub in ["registry", "git"] {
            let p = Path::new(c).join(sub);
            let real = p.canonicalize().unwrap_or(p);
            maps.push(format!(
                "-ffile-prefix-map={}={}/{sub}",
                real.display(),
                depcache::REMAPPED_CARGO_HOME
            ));
        }
    }
    maps.push(format!(
        "-ffile-prefix-map={}={}",
        norm.target,
        depcache::REMAPPED_TARGET
    ));
    maps.join(" ")
}

fn add_path_maps(norm: &Norm) {
    if matches!(
        std::env::var("JUSTRUST_C_PATH_MAP").as_deref(),
        Ok("0") | Ok("off")
    ) {
        return;
    }
    let Ok(target) = std::env::var("TARGET") else {
        return;
    };
    let flags = path_map_flags(norm);
    for base in ["CFLAGS", "CXXFLAGS"] {
        // cc reads `<VAR>_<target>` before `<VAR>`; use the most specific
        // one already set, else the target-specific one.
        let names = [
            format!("{base}_{target}"),
            format!("{base}_{}", target.replace('-', "_")),
        ];
        let name = names
            .iter()
            .find(|n| std::env::var_os(n).is_some())
            .cloned()
            .unwrap_or_else(|| names[1].clone());
        let old = std::env::var(&name)
            .ok()
            .or_else(|| std::env::var(base).ok())
            .unwrap_or_default();
        let new = if old.is_empty() {
            flags.clone()
        } else {
            format!("{old} {flags}")
        };
        // SAFETY: single-threaded at this point (before any spawn), and the
        // value is only read by this process and the script it starts.
        unsafe { std::env::set_var(&name, new) };
    }
}

fn plan(real: &Path, args: &[OsString]) -> Option<Plan> {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR")?);
    let profile = depcache::build_script_profile_dir(&out_dir)?;
    let norm = Norm::new(&profile)?;
    let cwd = std::env::current_dir().ok()?;
    let cargo_home = cargo_home();
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR")?);
    if !manifest.starts_with(&cargo_home) {
        return None; // local package: always run
    }
    add_path_maps(&norm);
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let package = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
    if hermetic(&package)
        && let Ok(real_dir) = manifest.canonicalize()
        && real_dir != manifest
    {
        // cc names objects by a hash of the source dir relative to
        // CARGO_MANIFEST_DIR. aws-lc compiles from its canonicalized dir,
        // so with a symlinked registry the prefix did not match and object
        // names (and the archive bytes) depended on the cargo home spelling
        // (FINDINGS 30: the last 26 differing Desktop keys). Same dir,
        // resolved spelling.
        // SAFETY: single-threaded here, before the script is spawned.
        unsafe { std::env::set_var("CARGO_MANIFEST_DIR", &real_dir) };
    }
    let mut h = H128::new();
    let mut hermetic_run = false;
    let mut explain = Vec::new();
    let mut field = |name: &str, v: &str| {
        h.field(v.as_bytes());
        explain.push(format!("{name} {v}"));
    };
    field("scheme", SCHEME);
    // Hermetic scripts: keyed by what they are built from, not the binary.
    // The binary embeds `env!("CARGO_MANIFEST_DIR")` (libsqlite3-sys prints
    // `cargo:include=<that>`) and string-table offsets shift with its
    // length, so its bytes differ per cargo home (measured: 3178472 vs
    // 3178632 bytes). The registry package (name, version, checksum-pinned
    // sources) and the script's compile flags in args/env below pin it, and
    // the build-script binary itself was compiled by the depcache-keyed
    // rustc unit from the same sources.
    if hermetic(&package) {
        let v = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
        let src = manifest.join("build.rs");
        let build_rs = depcache::file_hash(&src, false).unwrap_or_default();
        field("script", &format!("{package} {v} build.rs {build_rs}"));
    } else {
        field("script", &depcache::file_hash(real, true)?);
    }
    // The package dir in the registry: a placeholder, so the key is the
    // same in every cargo home (the script hash already pins the version).
    field("cwd", &norm.norm(cwd.to_str()?));
    field("rustc", &depcache::rustc_version(Path::new(&rustc))?);
    for a in args {
        field("arg", &norm.norm(a.to_str()?));
    }
    // What the host can change about the run: for packages that build
    // vendored C and probe nothing, only the C toolchain; for the rest,
    // any system package change.
    match hermetic_stamp(&package) {
        Some(s) => {
            hermetic_run = true;
            field("c-toolchain", &s)
        }
        None => field("system", &system_stamp()),
    }
    let mut env: Vec<(String, String)> = std::env::vars().filter(|(k, _)| keyed_env(k)).collect();
    env.sort();
    for (k, v) in &env {
        if by_content(k) && Path::new(v).is_file() {
            field(k, &depcache::file_hash(Path::new(v), true)?);
        } else if k == "PATH" && hermetic_run {
            // Which compiler `cc` resolves to is in the stamp; the rest of
            // PATH (per-user dirs) must not split keys between machines.
        } else {
            field(k, &norm.norm(v));
        }
    }
    if let Some(dir) = std::env::var_os("JUSTRUST_DEPCACHE_EXPLAIN") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            Path::new(&dir).join(format!("script-{package}-{}.txt", h.hex())),
            explain.join("\n"),
        );
    }
    Some(Plan {
        real: real.to_path_buf(),
        base: h.hex(),
        root: depcache::cache_dir()?,
        norm,
        out_dir,
        cwd,
        cargo_home,
        hermetic: hermetic_run,
    })
}

/// Packages whose build script compiles vendored C sources and, in the
/// configuration justrust sees, takes nothing from the system but the C
/// compiler and libc headers. Their runs are keyed by the C toolchain
/// (`hermetic_stamp`) instead of the package database, so they can be
/// shared across machines and published to the prebuilt store. Each entry
/// names the pkg-config modules the script probes before falling back to
/// its vendored copy: their presence is part of the key, so a host where
/// the probe would succeed never reuses a vendored run (and the reverse).
///
/// Measured on Jcode Desktop (FINDINGS 30): aws-lc-sys 12.8 s, libsqlite3-sys
/// 7.6 s, ring 4.8 s, onig_sys 2.3 s on the critical path of a cold build.
const HERMETIC: &[(&str, &[&str])] = &[
    ("ring", &[]),
    // `bundled` feature: compiles sqlite3.c, pkg-config only on request.
    ("libsqlite3-sys", &["sqlite3"]),
    ("onig_sys", &["oniguruma"]),
    (
        "aws-lc-sys",
        &["openssl", "aws-lc", "libcrypto", "libcrypto-awslc"],
    ),
];

fn hermetic(package: &str) -> bool {
    !matches!(
        std::env::var("JUSTRUST_HERMETIC_SCRIPTS").as_deref(),
        Ok("0") | Ok("off")
    ) && HERMETIC.iter().any(|(p, _)| *p == package)
}

/// The C toolchain a hermetic script compiles with: the resolved `cc`
/// (or `$CC`) and `ar` by content, the compiler's `--version`, and which of
/// the package's fallback pkg-config modules this host has. `None` when the
/// package is not hermetic or the toolchain cannot be identified.
fn hermetic_stamp(package: &str) -> Option<String> {
    if !hermetic(package) {
        return None;
    }
    let modules = HERMETIC.iter().find(|(p, _)| *p == package)?.1;
    let target = std::env::var("TARGET").unwrap_or_default();
    let tool = |names: &[&str], default: &str| -> Option<PathBuf> {
        let v = names
            .iter()
            .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
            .unwrap_or_else(|| default.to_owned());
        // Only a bare program or a path, no wrapper with arguments.
        if v.contains(char::is_whitespace) {
            return None;
        }
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
        paths::find_on_path(&v, Path::new("/nonexistent"))
    };
    let u = target.replace('-', "_");
    let cc = tool(
        &[
            &format!("CC_{target}"),
            &format!("CC_{u}"),
            "TARGET_CC",
            "CC",
        ],
        "cc",
    )?
    .canonicalize()
    .ok()?;
    let ar = tool(
        &[
            &format!("AR_{target}"),
            &format!("AR_{u}"),
            "TARGET_AR",
            "AR",
        ],
        "ar",
    )?
    .canonicalize()
    .ok()?;
    let version = Command::new(&cc)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .to_owned()
        })?;
    let mut s = format!(
        "cc {} {version}; ar {}",
        depcache::file_hash(&cc, true)?,
        depcache::file_hash(&ar, true)?
    );
    if !modules.is_empty() {
        let found: Vec<&str> = modules
            .iter()
            .copied()
            .filter(|m| {
                Command::new("pkg-config")
                    .args(["--exists", m])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            })
            .collect();
        s.push_str(&format!("; pkg-config {}", found.join(",")));
    }
    Some(s)
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
        // macOS: OS and Command Line Tools updates, Homebrew installs.
        "/System/Library/CoreServices/SystemVersion.plist",
        "/Library/Developer/CommandLineTools/usr/bin",
        "/opt/homebrew/Cellar",
        "/usr/local/Cellar",
    ] {
        if let Ok(m) = std::fs::metadata(p) {
            let (s_, ns) = crate::sys::mtime(&m);
            s.push_str(&format!("{p}:{s_}.{ns};"));
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
    if in_cargo_home(path, cargo_home) && !std::fs::symlink_metadata(path).ok()?.is_symlink() {
        let (s, ns) = crate::sys::mtime(&m);
        return Some(format!("m:{}:{s}.{ns}", m.len()));
    }
    depcache::file_hash(path, false)
}

/// Inside the cargo home, including its `registry`/`git` dirs by their
/// resolved path: build scripts run in the resolved package dir, so with
/// a symlinked registry the same file was signed by content on one machine
/// and by size+mtime on another, and no entry ever matched across them
/// (FINDINGS 30: ring, aws-lc-sys, libsqlite3-sys missed the store).
fn in_cargo_home(path: &Path, cargo_home: &Path) -> bool {
    path.starts_with(cargo_home)
        || ["registry", "git"].iter().any(|sub| {
            cargo_home
                .join(sub)
                .canonicalize()
                .is_ok_and(|real| path.starts_with(real))
        })
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
                hermetic: self.hermetic,
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

// ------------------------------------------------------- prebuilt store tier
//
// Hermetic build-script runs go to the prebuilt store under `v1/s/`:
//   v1/s/m/<base>.jsonl   manifest lines (`Entry`)
//   v1/s/o/<out>.jrbs     bundle: gzip of `JRBS1\n` then per file
//                          `u32 path_len, path, u64 len, bytes`, meta.json first
// Only entries whose meta.json says `hermetic` are published.

const BUNDLE_MAGIC: &[u8] = b"JRBS1\n";
const BUNDLE_MAX_FILES: usize = 4096;
const BUNDLE_MAX_BYTES: u64 = 256 << 20;

/// A relative path with only normal components.
fn safe_rel(p: &str) -> bool {
    !p.is_empty()
        && !p.contains('\0')
        && Path::new(p).is_relative()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

pub(crate) fn pack_entry(dir: &Path, dst: &Path) -> Option<()> {
    let meta: Meta = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
    if !meta.hermetic {
        return None;
    }
    let f = std::fs::File::create(dst).ok()?;
    let mut gz =
        flate2::write::GzEncoder::new(std::io::BufWriter::new(f), flate2::Compression::fast());
    gz.write_all(BUNDLE_MAGIC).ok()?;
    let mut put = |rel: &str, src: &Path| -> Option<()> {
        let len = std::fs::metadata(src).ok()?.len();
        gz.write_all(&(rel.len() as u32).to_le_bytes()).ok()?;
        gz.write_all(rel.as_bytes()).ok()?;
        gz.write_all(&len.to_le_bytes()).ok()?;
        std::io::copy(&mut std::fs::File::open(src).ok()?, &mut gz).ok()?;
        Some(())
    };
    put("meta.json", &dir.join("meta.json"))?;
    for f in &meta.files {
        if !safe_rel(&f.path) {
            return None;
        }
        put(&format!("out/{}", f.path), &dir.join("out").join(&f.path))?;
    }
    gz.finish().ok()?.flush().ok()
}

/// Unpack into a fresh `dst`, accepting only the files meta.json lists.
pub(crate) fn unpack_entry(src: &Path, dst: &Path) -> Option<()> {
    let f = std::fs::File::open(src).ok()?;
    let mut r = flate2::read::GzDecoder::new(std::io::BufReader::new(f)).take(BUNDLE_MAX_BYTES);
    let mut magic = [0u8; 6];
    r.read_exact(&mut magic).ok()?;
    if magic != BUNDLE_MAGIC {
        return None;
    }
    let next = |r: &mut dyn Read| -> Option<Option<(String, Vec<u8>)>> {
        let mut n = [0u8; 4];
        match r.read(&mut n[..1]) {
            Ok(0) => return Some(None),
            Ok(_) => r.read_exact(&mut n[1..]).ok()?,
            Err(_) => return None,
        }
        let n = u32::from_le_bytes(n) as usize;
        if n == 0 || n > 4096 {
            return None;
        }
        let mut name = vec![0u8; n];
        r.read_exact(&mut name).ok()?;
        let mut len = [0u8; 8];
        r.read_exact(&mut len).ok()?;
        let len = u64::from_le_bytes(len);
        if len > BUNDLE_MAX_BYTES {
            return None;
        }
        let mut body = Vec::with_capacity(len as usize);
        r.take(len).read_to_end(&mut body).ok()?;
        if body.len() as u64 != len {
            return None;
        }
        Some(Some((String::from_utf8(name).ok()?, body)))
    };
    let (name, body) = next(&mut r)??;
    if name != "meta.json" {
        return None;
    }
    let meta: Meta = serde_json::from_slice(&body).ok()?;
    if !meta.hermetic || meta.files.len() > BUNDLE_MAX_FILES {
        return None;
    }
    let mut want: std::collections::HashSet<String> = meta
        .files
        .iter()
        .map(|f| f.path.clone())
        .collect::<std::collections::HashSet<_>>();
    if want.len() != meta.files.len() || !want.iter().all(|p| safe_rel(p)) {
        return None;
    }
    std::fs::create_dir_all(dst.join("out")).ok()?;
    std::fs::write(dst.join("meta.json"), &body).ok()?;
    while let Some((name, body)) = next(&mut r)? {
        let rel = name.strip_prefix("out/")?;
        if !want.remove(rel) {
            return None;
        }
        let p = dst.join("out").join(rel);
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::write(&p, body).ok()?;
    }
    want.is_empty().then_some(())
}

/// For `justrust cache --publish`: if `line` is a hermetic build-script
/// entry of the local cache at `root`, copy its bundle to `v1/s/o` (the
/// caller appends the line to `v1/s/m/<base>.jsonl`). Returns the bundle
/// bytes written (0 when it already existed), `None` for lines that are not
/// publishable script entries.
pub(crate) fn publish_entry(root: &Path, line: &str, v1: &Path) -> Option<u64> {
    let e: Entry = serde_json::from_str(line).ok()?;
    let dir = root.join("o").join(&e.out);
    let meta: Meta = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
    if !meta.hermetic {
        return None;
    }
    let o = v1.join("s/o");
    std::fs::create_dir_all(&o).ok()?;
    let bundle = o.join(format!("{}.jrbs", e.out));
    if bundle.is_file() {
        return Some(0);
    }
    let t = o.join(format!(".{}.{}", e.out, std::process::id()));
    if pack_entry(&dir, &t).is_none() {
        let _ = std::fs::remove_file(&t);
        return None;
    }
    let n = std::fs::metadata(&t).ok()?.len();
    std::fs::rename(&t, &bundle).ok()?;
    Some(n)
}

impl Plan {
    /// After a local miss: fetch a hermetic run from the prebuilt store
    /// into the local cache, then restore it locally.
    fn fetch_prebuilt(&self) -> Option<Meta> {
        use crate::depcache_prebuilt as pb;
        let base = pb::base()?;
        let root = &self.root;
        if pb::fresh_file(&pb::breaker(root), pb::BREAKER_SECS) {
            return None;
        }
        let m = pb::tmp(root, "sm");
        let got = pb::get(&base, &format!("s/m/{}.jsonl", self.base), &m, 8, 1 << 20);
        let text = std::fs::read_to_string(&m).unwrap_or_default();
        let _ = std::fs::remove_file(&m);
        match got {
            pb::Got::Ok => pb::note_success(root),
            pb::Got::Missing => return None,
            pb::Got::Failed(why) => {
                pb::note_failure(root, &format!("script manifest: {why}"));
                return None;
            }
        }
        let (out, line) = text.lines().rev().find_map(|l| {
            let e: Entry = serde_json::from_str(l).ok()?;
            (pb::valid_key(&e.out) && self.matches(&e)).then(|| (e.out, l.to_owned()))
        })?;
        let final_dir = root.join("o").join(&out);
        if !final_dir.join("meta.json").is_file() {
            let bundle = pb::tmp(root, "sb");
            let stage = root
                .join("o")
                .join(format!(".{out}.pbs{}", std::process::id()));
            let ok = matches!(
                pb::get(
                    &base,
                    &format!("s/o/{out}.jrbs"),
                    &bundle,
                    120,
                    BUNDLE_MAX_BYTES
                ),
                pb::Got::Ok
            ) && unpack_entry(&bundle, &stage).is_some();
            let _ = std::fs::remove_file(&bundle);
            if !ok || std::fs::rename(&stage, &final_dir).is_err() {
                let _ = std::fs::remove_dir_all(&stage);
                if !final_dir.join("meta.json").is_file() {
                    return None;
                }
            }
        }
        let known = std::fs::read_to_string(self.manifest()).unwrap_or_default();
        if !known.lines().any(|l| l == line) {
            std::fs::create_dir_all(root.join("m")).ok()?;
            paths::append_line(&self.manifest(), &line).ok()?;
        }
        self.restore()
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
    let err = crate::sys::exec(Command::new(real).args(args));
    eprintln!("justrust: failed to exec {}: {err}", real.display());
    std::process::exit(127);
}

fn record(package: &str, hit: bool, wall: f64, saved: f64, prebuilt: bool) {
    let Some(dir) = std::env::var_os("JUSTRUST_RUN_DIR") else {
        return;
    };
    let mut line = serde_json::json!({
        "package": package,
        "hit": hit,
        "wall": wall,
        "saved_secs": saved,
    });
    if prebuilt {
        line["prebuilt"] = true.into();
    }
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
    let mut prebuilt = false;
    if let Some(meta) = plan.restore().or_else(|| {
        let m = plan.fetch_prebuilt();
        prebuilt = m.is_some();
        m
    }) {
        let mut out = std::io::stdout().lock();
        for l in &meta.stdout {
            let _ = writeln!(out, "{}", plan.norm.denorm(l));
        }
        let _ = out.flush();
        let mut err = std::io::stderr().lock();
        for l in &meta.stderr {
            let _ = writeln!(err, "{}", plan.norm.denorm(l));
        }
        record(&package, true, paths::now() - start, meta.wall, prebuilt);
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
    record(&package, false, wall, 0.0, false);
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
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let sig = status.signal().unwrap_or(libc::SIGKILL);
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
        std::process::exit(128 + sig);
    }
    #[cfg(not(unix))]
    std::process::exit(1);
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
                hermetic: false,
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

    // Runs `sh` scripts.
    #[cfg(unix)]
    #[test]
    fn wrapper_replaces_elf_and_falls_back() {
        let dir = std::env::temp_dir().join(format!("jr-bs-wrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("build_script_build-abc");
        // A native executable (ELF on Linux, Mach-O on macOS).
        std::fs::copy("/bin/sh", &bin).unwrap();
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
    // Runs `sh` scripts.
    #[cfg(unix)]
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
        let out = retry_busy(|| {
            Command::new(&bin)
                .arg("X")
                .current_dir(dir.join("cwd"))
                .output()
        });
        assert_eq!(out.status.code(), Some(7));
        let so = String::from_utf8(out.stdout).unwrap();
        assert!(so.starts_with("cargo:rustc-cfg=aX\n"), "{so}");
        assert!(so.trim_end().ends_with("/cwd"), "{so}");
        assert_eq!(out.stderr, b"err\n");

        // With a justrust that exists but has the cache disabled: same.
        let jr = dir.join("fake-justrust");
        std::fs::write(&jr, "#!/bin/sh\n[ \"$1\" = __build-script ] || exit 99\nshift\nr=$1\nshift\nexec \"$r\" \"$@\"\n").unwrap();
        crate::sys::set_mode(&jr, 0o755).unwrap();
        std::fs::copy("/bin/sh", &bin).unwrap();
        std::fs::remove_file(&real).unwrap();
        wrap_with(&dir, "build_script_build", "-abc", &jr).unwrap();
        std::fs::write(&real, "#!/bin/sh\necho \"$1 $2\"\nexit 3\n").unwrap();
        let out = retry_busy(|| Command::new(&bin).args(["a b", "c"]).output());
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.stdout, b"a b c\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Exec a file this test just wrote. A test forking concurrently can
    /// briefly inherit our write fd, which makes exec fail with ETXTBSY.
    #[cfg(unix)]
    fn retry_busy(
        mut f: impl FnMut() -> std::io::Result<std::process::Output>,
    ) -> std::process::Output {
        for _ in 0..50 {
            match f() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                r => return r.unwrap(),
            }
        }
        f().unwrap()
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
            hermetic: false,
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
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hostname", out_dir.join("link")).unwrap();
            assert!(
                p.store(b"cargo:rerun-if-changed=in.txt\n", b"", 1.0)
                    .is_none()
            );
            std::fs::remove_file(out_dir.join("link")).unwrap();
        }
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
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("d/sub/a"), dir.join("l")).unwrap();
            let l1 = input_sig(&dir.join("l"), &ch).unwrap();
            std::fs::write(dir.join("d/sub/a"), "3").unwrap();
            assert_ne!(l1, input_sig(&dir.join("l"), &ch).unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn script_entry(dir: &Path, hermetic: bool) -> PathBuf {
        let e = dir.join("o/e");
        std::fs::create_dir_all(e.join("out/include/openssl")).unwrap();
        std::fs::write(e.join("out/libcrypto.a"), b"!<arch>\n\0bin").unwrap();
        std::fs::write(e.join("out/include/openssl/base.h"), "#define X 1\n").unwrap();
        let meta = Meta {
            files: vec![
                OutFile {
                    path: "libcrypto.a".into(),
                    text: false,
                },
                OutFile {
                    path: "include/openssl/base.h".into(),
                    text: true,
                },
            ],
            stdout: vec!["cargo:rustc-link-lib=static=crypto".into()],
            stderr: vec![],
            wall: 12.5,
            package: "aws-lc-sys".into(),
            hermetic,
        };
        std::fs::write(e.join("meta.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        e
    }

    #[test]
    fn script_bundle_round_trip_and_hermetic_only() {
        let dir = std::env::temp_dir().join(format!("jr-bs-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = script_entry(&dir, true);
        let b = dir.join("b.gz");
        pack_entry(&e, &b).unwrap();
        let out = dir.join("u");
        unpack_entry(&b, &out).unwrap();
        for f in ["meta.json", "out/libcrypto.a", "out/include/openssl/base.h"] {
            assert_eq!(
                std::fs::read(out.join(f)).unwrap(),
                std::fs::read(e.join(f)).unwrap(),
                "{f}"
            );
        }
        // Host-keyed runs are never packed or accepted.
        let e2 = script_entry(&dir.join("x"), false);
        assert!(pack_entry(&e2, &dir.join("b2.gz")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_bundle_rejects_hostile_input() {
        let dir = std::env::temp_dir().join(format!("jr-bs-hostile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = |files: &[&str]| {
            serde_json::to_vec(&Meta {
                files: files
                    .iter()
                    .map(|p| OutFile {
                        path: (*p).into(),
                        text: false,
                    })
                    .collect(),
                stdout: vec![],
                stderr: vec![],
                wall: 1.0,
                package: "ring".into(),
                hermetic: true,
            })
            .unwrap()
        };
        let write = |name: &str, entries: &[(&str, &[u8])]| {
            let p = dir.join(name);
            let mut gz = flate2::write::GzEncoder::new(
                std::fs::File::create(&p).unwrap(),
                flate2::Compression::fast(),
            );
            gz.write_all(BUNDLE_MAGIC).unwrap();
            for (n, b) in entries {
                gz.write_all(&(n.len() as u32).to_le_bytes()).unwrap();
                gz.write_all(n.as_bytes()).unwrap();
                gz.write_all(&(b.len() as u64).to_le_bytes()).unwrap();
                gz.write_all(b).unwrap();
            }
            gz.finish().unwrap();
            p
        };
        // Traversal listed in meta.json.
        let m = meta(&["../escape"]);
        let b = write("t1", &[("meta.json", &m), ("out/../escape", b"x")]);
        assert!(unpack_entry(&b, &dir.join("o1")).is_none());
        assert!(!dir.join("escape").exists());
        // A file meta.json does not list.
        let m = meta(&["a"]);
        let b = write("t2", &[("meta.json", &m), ("out/a", b"x"), ("out/b", b"y")]);
        assert!(unpack_entry(&b, &dir.join("o2")).is_none());
        // A listed file missing.
        let m = meta(&["a", "b"]);
        let b = write("t3", &[("meta.json", &m), ("out/a", b"x")]);
        assert!(unpack_entry(&b, &dir.join("o3")).is_none());
        // meta.json not first.
        let b = write("t4", &[("out/a", b"x")]);
        assert!(unpack_entry(&b, &dir.join("o4")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hermetic_list_is_vendored_c_only() {
        for p in ["ring", "aws-lc-sys", "libsqlite3-sys", "onig_sys"] {
            assert!(HERMETIC.iter().any(|(n, _)| *n == p), "{p}");
        }
        // System probes stay keyed by the package database.
        for p in ["alsa-sys", "freetype-sys", "wayland-sys", "openssl-sys"] {
            assert!(!HERMETIC.iter().any(|(n, _)| *n == p), "{p}");
        }
    }
}
