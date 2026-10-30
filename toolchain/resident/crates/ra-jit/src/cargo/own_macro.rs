//! Proc macros compiled and linked by ra-jit (docs/no-rustc.md, step J). When rust-analyzer's
//! `cargo check --compile-time-deps` of a load into ra-jit's own target directory asks rustc
//! for a proc macro, and the load names a server that reads the list of macros ra-jit writes
//! into a library ([`SERVER`], `vendor/proc-macro-srv`), the wrapper describes the proc macro
//! and the libraries it links as it does a build script (`own_script.rs`), and ra-jit links it
//! as a shared library where rustc would have put it, with the dep-info cargo reads. The
//! load expands with it by that server. Cargo's own builds never read that directory, and
//! nothing there is rustc's (`wrapper.rs`, `for_load`): the libraries a proc macro links are
//! only described, and a proc macro without a server fails the load.
//!
//! By default a proc macro is built as a WebAssembly module instead ([`WASM`]), which ra-jit
//! runs in its own process (`crates/ra-jit-macro-wasm`) and needs no server. One that cannot
//! be built so fails the load, naming it, unless the workspace's root manifest names its
//! package in `native-proc-macros` ([`NATIVE_KEY`]): then it is expanded as it was before.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::Context as _;

use super::{
    own_script::{self, Made},
    sandboxed, say,
    units::Record,
};

/// The variable that names the proc-macro server that reads the list of macros of a library
/// ra-jit linked: a path, `1` for the one a release ships beside ra-jit ([`SHIPPED`]), which
/// is also what is used when it is not set and that server is there, or `0` for none.
pub(crate) const SERVER: &str = "RA_JIT_PROC_MACRO_SERVER";

/// The server's executable a release has beside ra-jit's (`.github/scripts/build_release.py`).
pub(crate) const SHIPPED: &str = "blaze-proc-macro-srv";

/// The variable that has proc macros expanded natively, `0` or `off`, rather than built as
/// WebAssembly modules and run by ra-jit, which is what is done when it is unset.
pub(crate) const WASM: &str = "RA_JIT_PROC_MACRO_WASM";

/// The target a proc macro's module is built for.
pub(crate) const WASM_TARGET: &str = "wasm32-wasip1";

/// Has the toolchain at `sysroot` the WASI libc a module is linked with (`link.rs`), which
/// comes with rustup's `wasm32-wasip1` target: rustup adds it when it is not there, once per
/// process. Builds that start together add it once: two rustups that download one component
/// at a time each take the other's file away. `Err` says why there is none.
pub(crate) fn wasm_target(sysroot: &Path) -> Result<(), String> {
    static ADDED: std::sync::Mutex<Vec<(PathBuf, Result<(), String>)>> =
        std::sync::Mutex::new(Vec::new());
    let libc = sysroot.join("lib/rustlib").join(WASM_TARGET).join("lib/self-contained");
    if libc.join("crt1-reactor.o").is_file() {
        return Ok(());
    }
    let mut added = ADDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, said)) = added.iter().find(|(it, _)| it == sysroot) {
        return said.clone();
    }
    let toolchain = sysroot.file_name().and_then(OsStr::to_str).unwrap_or_default();
    let mut options = std::fs::OpenOptions::new();
    let lock = options.create(true).truncate(false).write(true);
    let lock = lock.open(std::env::temp_dir().join(format!("blaze-{toolchain}-{WASM_TARGET}")));
    let locked = lock.and_then(|it| it.lock().map(|()| it));
    let said = match locked {
        Ok(_) if libc.join("crt1-reactor.o").is_file() => Ok(()),
        Ok(_locked) => {
            let mut rustup = std::process::Command::new("rustup");
            rustup.args(["target", "add", WASM_TARGET, "--toolchain", toolchain]);
            match rustup.stdin(std::process::Stdio::null()).output() {
                Ok(_) if libc.join("crt1-reactor.o").is_file() => Ok(()),
                Ok(ran) => Err(String::from_utf8_lossy(&ran.stderr).trim_end().to_owned()),
                Err(error) => Err(format!("rustup did not run: {error}")),
            }
        }
        Err(error) => Err(format!("no lock to add it under: {error}")),
    };
    let said = said.map_err(|why| {
        format!(
            "the toolchain at {} has no {WASM_TARGET} target, which a proc macro's module is \
             linked with, and `rustup target add {WASM_TARGET} --toolchain {toolchain}` did not \
             add it: {why}",
            sysroot.display()
        )
    });
    added.push((sysroot.to_owned(), said.clone()));
    said
}

/// Whether proc macros are built as WebAssembly modules ([`WASM`]): not when [`turned_off`].
pub(crate) fn wasm() -> bool {
    let off =
        matches!(std::env::var_os(WASM).as_deref().and_then(OsStr::to_str), Some("0" | "off"));
    !off && !turned_off()
}

/// The key, in `[workspace.metadata.blaze]` of a workspace's root manifest, that lists the
/// packages whose proc macros are expanded natively, not as WebAssembly.
pub(crate) const NATIVE_KEY: &str = "native-proc-macros";

/// The packages, by crate name (`-` as `_`), the root manifest of the workspace at `root`
/// names in [`NATIVE_KEY`]: none when it has no manifest or does not say.
pub(crate) fn native_proc_macros(root: &Path) -> Result<Vec<String>, String> {
    let manifest = root.join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&manifest) else { return Ok(Vec::new()) };
    let wrong = |what: &str| {
        format!("`{NATIVE_KEY}` in [workspace.metadata.blaze] of {}: {what}", manifest.display())
    };
    let value: toml::Value = toml::from_str(&text).map_err(|it| wrong(&it.to_string()))?;
    let listed = ["workspace", "metadata", "blaze", NATIVE_KEY]
        .iter()
        .try_fold(&value, |it, key| it.get(*key));
    let Some(listed) = listed else { return Ok(Vec::new()) };
    let names = listed.as_array().ok_or_else(|| wrong("a list of package names"))?;
    let names = names.iter().map(|it| it.as_str().map(|it| it.replace('-', "_")));
    names.collect::<Option<_>>().ok_or_else(|| wrong("a list of package names, as strings"))
}

/// Whether the proc macro of crate `name` that cargo asked rustc for runs as WebAssembly: when
/// the workspace cargo runs rustc in does not run it natively ([`NATIVE_KEY`]).
pub(crate) fn runs_as_wasm(name: &str) -> anyhow::Result<bool> {
    let cwd = std::env::current_dir()?;
    let root = sandboxed::workspace_root(&cwd)?.unwrap_or(cwd);
    let native = native_proc_macros(&root).map_err(anyhow::Error::msg)?;
    Ok(!native.iter().any(|it| it == &name.replace('-', "_")))
}

/// The variable ra-jit sets for the cargo of a load into its own target directory.
pub(crate) const OWN_TARGET: &str = "RA_JIT_OWN_TARGET";

/// Whether [`SERVER`] turns ra-jit's proc macros off, `0` or `off`, as the harness's reference
/// builds do: a load's cargo is then told so, and rustc builds what it asks for.
pub(crate) fn turned_off() -> bool {
    matches!(std::env::var_os(SERVER).as_deref().and_then(OsStr::to_str), Some("0" | "off"))
}

/// The directory in cargo's target directory of a load's own target directory when ra-jit
/// links the proc macros, where libraries are only described: cargo is given empty files for
/// them (`wrapper.rs`, `described_only`).
pub(crate) const OWN_DIR: &str = "rust-analyzer";

/// The same when [`turned_off`], where rustc builds every unit. Cargo does not fingerprint
/// [`SERVER`], so in one directory it would take the empty files of a load with ra-jit's proc
/// macros for rustc's libraries, and rustc then fails reading them (E0786, #460).
pub(crate) const BY_RUSTC_DIR: &str = "rust-analyzer-rustc";

/// The load's own target directory in cargo's, for the mode [`SERVER`] says now.
pub(crate) fn own_dir() -> &'static str {
    if turned_off() { BY_RUSTC_DIR } else { OWN_DIR }
}

/// What a load whose proc macro ra-jit has no server for is told: where the server was
/// looked for, and the two ways through.
pub(crate) fn missing_server() -> String {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            Some(exe.parent()?.join(format!("{SHIPPED}{}", std::env::consts::EXE_SUFFIX)))
        })
        .map_or_else(|| SHIPPED.to_owned(), |it| it.display().to_string());
    format!(
        "no proc-macro server is beside blaze (looked for {beside}), and blaze links proc \
         macros only for its own. Build it there from a blaze checkout with `ra-jit \
         vendor/proc-macro-srv --crate rust_analyzer_proc_macro_srv --crate proc_macro --out \
         <scratch>/server.o --exe {beside}` (what `.github/scripts/build_release.py --server` \
         runs; the releases ship it on Linux and macOS), or have rustc build the load's proc macros \
         with `BLAZE_PROC_MACRO_SERVER=0`"
    )
}

/// The server that expands with the libraries ra-jit links, as [`SERVER`] says, relative to
/// the current directory: `None` when it says none, or no server is beside ra-jit when it
/// says `1` or nothing. A load names it to its cargo, or takes the variable away, so the
/// wrapper links a proc macro only for a server there is.
pub(crate) fn server() -> Option<&'static Path> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND.get_or_init(find).as_deref()
}

fn find() -> Option<PathBuf> {
    let named = std::env::var_os(SERVER);
    match named.as_deref().and_then(OsStr::to_str) {
        Some("0" | "off") => return None,
        Some("1") | None => {}
        Some(_) => return named.and_then(|it| std::path::absolute(it).ok()),
    }
    let exe = std::env::current_exe().ok()?;
    let shipped = exe.parent()?.join(format!("{SHIPPED}{}", std::env::consts::EXE_SUFFIX));
    if shipped.is_file() {
        return Some(shipped);
    }
    // A build of ra-jit that has no server, as one from crates.io, links no proc macro: a load
    // into its own target directory that has one fails.
    if named.is_some() {
        eprintln!("warning: no proc-macro server is beside {}", exe.display());
    }
    None
}

/// `ra-jit proc-macro-library <sysroot> <library>`: makes ra-jit's library of the proc macro
/// rustc compiled at `<library>` and prints where it is ([`libraries_of`]).
pub(crate) const COMMAND: &str = "proc-macro-library";

/// The directory, in the record of a proc macro rustc compiled, of ra-jit's library of it.
const OWN: &str = "ra-jit";

/// How many libraries are made at once: each is a build that uses every core itself.
const AT_ONCE: usize = 4;

/// What stands for each of `libraries`, proc macros rustc compiled in a build through the
/// wrapper, which recorded them (`units.rs`), each with its crate's name, in the workspace at
/// `workspace`: the store's or a new build from source over the standard library of
/// `sysroot`, each in a process of its own. A WebAssembly module ([`wasm`]), or `Err` naming
/// the macro when it cannot be one, but for a package the workspace runs natively
/// ([`NATIVE_KEY`]), which is `None`, expanded with rustc's library. Otherwise ra-jit's
/// library for [`server`], or `None` for one ra-jit could not make, which is said.
pub(crate) fn libraries_of(
    sysroot: Option<&Path>,
    workspace: &Path,
    libraries: &[Compiled],
) -> Vec<Result<Option<PathBuf>, String>> {
    let wasm = wasm();
    if !wasm && server().is_none() {
        return vec![Ok(None); libraries.len()];
    }
    let native = match wasm {
        true => {
            let root = sandboxed::workspace_root(workspace).ok().flatten();
            match native_proc_macros(root.as_deref().unwrap_or(workspace)) {
                Ok(it) => it,
                Err(why) => return vec![Err(why); libraries.len()],
            }
        }
        false => Vec::new(),
    };
    let refused = |name: &str, why: &str| refusal(name, why, workspace);
    let Some(sysroot) = sysroot else {
        let why = "the workspace has no sysroot to build it over";
        return libraries
            .iter()
            .map(|Compiled { name, .. }| match wasm && !native.contains(name) {
                true => Err(refused(name, why)),
                false => Ok(None),
            })
            .collect();
    };
    let Ok(exe) = std::env::current_exe() else {
        return vec![Err("blaze does not know where it is".to_owned()); libraries.len()];
    };
    let next = std::sync::atomic::AtomicUsize::new(0);
    let made = std::sync::Mutex::new(vec![Ok(None); libraries.len()]);
    std::thread::scope(|scope| {
        for _ in 0..AT_ONCE.min(libraries.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(Compiled { name, library, description }) = libraries.get(at) else {
                        break;
                    };
                    let as_wasm = wasm && !native.contains(name);
                    if !as_wasm && server().is_none() {
                        continue;
                    }
                    let mut command = std::process::Command::new(&exe);
                    command.arg(COMMAND);
                    if as_wasm {
                        let Some(description) = description else {
                            let why = "the load can't describe its crate";
                            made.lock().unwrap()[at] = Err(refused(name, why));
                            continue;
                        };
                        match described_at(library, description) {
                            Ok(file) => _ = command.arg(WASM_FLAG).arg(file),
                            Err(why) => {
                                made.lock().unwrap()[at] = Err(refused(name, &why));
                                continue;
                            }
                        }
                    }
                    command.arg(sysroot).arg(library);
                    let start = std::time::Instant::now();
                    let ran = command.stdin(std::process::Stdio::null()).output();
                    if std::env::var_os("RA_JIT_STATS").is_some() {
                        eprintln!("blaze: the proc macro {name} made in {:.1?}", start.elapsed());
                    }
                    let found = match ran {
                        Ok(ran) if ran.status.success() => {
                            let said = String::from_utf8_lossy(&ran.stdout).trim_end().to_owned();
                            Ok((!said.is_empty()).then(|| PathBuf::from(said)))
                        }
                        Ok(ran) => Err(String::from_utf8_lossy(&ran.stderr).trim_end().to_owned()),
                        Err(error) => Err(error.to_string()),
                    };
                    let found = match found {
                        Err(why) if as_wasm => Err(refused(name, &why)),
                        Err(why) => {
                            let file = library.file_name().unwrap_or_default().to_string_lossy();
                            say::fallback(
                                &format!("blaze did not make the proc macro {file}: {why}"),
                                "rustc's library of it is expanded by the toolchain's server",
                            );
                            Ok(None)
                        }
                        found => found,
                    };
                    made.lock().unwrap()[at] = found;
                }
            });
        }
    });
    made.into_inner().unwrap()
}

/// A proc macro rustc compiled: its crate's name, its library, and the `rust-project.json`
/// of the crate and what it depends on, which its module is built from.
pub(crate) struct Compiled {
    pub(crate) name: String,
    pub(crate) library: PathBuf,
    pub(crate) description: Option<String>,
}

/// Why the load fails when the proc macro of crate `name` in the workspace at `workspace`
/// cannot be built or run as a WebAssembly module, and the ways out.
pub(crate) fn refusal(name: &str, why: &str, workspace: &Path) -> String {
    let root = sandboxed::workspace_root(workspace).ok().flatten();
    let root = root.as_deref().unwrap_or(workspace);
    format!(
        "the proc macro `{name}` cannot run as WebAssembly: {why}\n  name its package in \
         `{NATIVE_KEY}` under [workspace.metadata.blaze] in {} to expand it natively, or set \
         BLAZE_PROC_MACRO_WASM=0 for every proc macro",
        root.join("Cargo.toml").display(),
    )
}

/// The flag of [`COMMAND`] that has it make a WebAssembly module rather than a library, from
/// the description in the file after it.
const WASM_FLAG: &str = "--wasm";

/// The directory, beside rustc's library, of the modules ra-jit builds of it.
const WASM_DIR: &str = "ra-jit-wasm";

/// Writes `description`, of the proc macro rustc compiled at `library`, where its module is
/// built, a directory of its own for each description.
fn described_at(library: &Path, description: &str) -> Result<PathBuf, String> {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(description.as_bytes());
    let digest = crate::content::hex(&digest[..8]);
    let stem = library.file_stem().and_then(OsStr::to_str).ok_or("a library unnamed")?;
    let dir = library.parent().ok_or("a library without a directory")?;
    let dir = dir.join(WASM_DIR).join(format!("{stem}-{digest}"));
    std::fs::create_dir_all(&dir).map_err(|it| format!("making {}: {it}", dir.display()))?;
    let file = dir.join("rust-project.json");
    std::fs::write(&file, description).map_err(|it| format!("writing {}: {it}", file.display()))?;
    Ok(file)
}

/// [`COMMAND`]. It prints nothing for a library rustc compiled in a build that did not go
/// through the wrapper, which recorded nothing of it.
pub(crate) fn command(args: impl Iterator<Item = std::ffi::OsString>) -> anyhow::Result<()> {
    let mut args = args.peekable();
    let described = match args.next_if(|it| it == WASM_FLAG) {
        Some(_) => Some(PathBuf::from(args.next().context("a description after --wasm")?)),
        None => None,
    };
    let sysroot = PathBuf::from(args.next().context("the sysroot is named first")?);
    let library = PathBuf::from(args.next().context("then rustc's library")?);
    let made = match &described {
        Some(described) => module_of(&sysroot, &library, described).map(Some),
        None => library_of(&sysroot, &library),
    };
    match made {
        Ok(own) => {
            if let Some(own) = own {
                println!("{}", own.display());
            }
            Ok(())
        }
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1)
        }
    }
}

/// ra-jit's WebAssembly module of the proc macro rustc compiled at `library`, built from the
/// `rust-project.json` at `described` ([`described_at`]) over the standard library of
/// `sysroot`, beside it.
fn module_of(sysroot: &Path, library: &Path, described: &Path) -> Result<PathBuf, String> {
    let description = std::fs::read_to_string(described).map_err(|it| it.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&description).map_err(|it| it.to_string())?;
    let name = value["crates"][0]["display_name"].as_str().ok_or("a proc macro unnamed")?;
    let dir = described.parent().ok_or("a description without a directory")?;
    let stem = library.file_stem().and_then(OsStr::to_str).unwrap_or(name);
    let made = Made {
        name,
        proc_macro: true,
        wasi: false,
        wasm: true,
        exe: dir.join(format!("{stem}.wasm")),
        dep_info: dir.join(format!("{stem}.d")),
    };
    own_script::compile_described(sysroot, &description, &made, dir)?;
    // Here, as soon as it is built and beside the other modules' builds, rather than in the
    // load, which loads one module after another.
    ra_jit_macro_wasm::prepare(&made.exe, own_script::store())?;
    Ok(made.exe)
}

/// ra-jit's library of the proc macro rustc compiled at `library`, made from its record over
/// the standard library of `sysroot`, in that record. `None` when it has no record.
fn library_of(sysroot: &Path, library: &Path) -> Result<Option<PathBuf>, String> {
    let (prefix, suffix) = (std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    let file = library.file_name().and_then(OsStr::to_str).ok_or("a library unnamed")?;
    let stem = file.strip_prefix(prefix).and_then(|it| it.strip_suffix(suffix));
    let stem = stem.ok_or("not a library")?;
    let dir = library.parent().ok_or("a library without a directory")?;
    let unit = &super::layout::unit_of_file(stem, dir);
    let Some((record, records)) = Record::described_or_seeded(dir, unit) else { return Ok(None) };
    let dir = records.join(unit).join(OWN);
    let extra = record.codegen("extra-filename").unwrap_or_default();
    let made = Made {
        name: &record.crate_name,
        proc_macro: true,
        wasi: false,
        wasm: false,
        exe: dir.join(file),
        dep_info: dir.join(format!("{}{extra}.d", record.crate_name)),
    };
    std::fs::create_dir_all(&dir).map_err(|it| it.to_string())?;
    own_script::compile_record(sysroot, &record, &made, &dir)?;
    Ok(Some(made.exe))
}

/// What ra-jit makes of the proc macro `record` is of: its library, or with `wasm` its module,
/// where rustc would have put the library, and the dep-info cargo reads. `None` when cargo
/// names no place for them.
pub(crate) fn made(record: &Record, wasm: bool) -> Option<Made<'_>> {
    let (prefix, suffix) = (std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    let name = record.crate_name.as_str();
    let extra = record.codegen("extra-filename").unwrap_or_default();
    let out_dir = record.out_dir()?;
    Some(Made {
        name,
        proc_macro: true,
        wasi: false,
        wasm,
        exe: out_dir.join(format!("{prefix}{name}{extra}{suffix}")),
        dep_info: out_dir.join(format!("{name}{extra}.d")),
    })
}
