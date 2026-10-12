//! Hot patching of test binaries: rerun tests after an edit without
//! relinking the test binary (opt-in, `JUSTRUST_HOTPATCH=1`, FINDINGS 42).
//!
//! The ported form of the `bench/hotpatch` prototype (FINDINGS 35), wired into
//! the rustc shim and the `ld` dispatcher of `linker.rs`:
//!
//! - **Shim.** For a local `--test` unit that links, it adds
//!   `--no-gc-sections` (a later patch may call an instance nothing used yet)
//!   and the loader (`loader.c`, compiled once into
//!   `~/.justrust/hotpatch/loader-<hash>.so`) as a `DT_NEEDED` library by
//!   absolute path, and sets `JUSTRUST_HOTPATCH_LINK=1` for the link. Link
//!   args are untracked, so no unit is rebuilt by turning it on or off.
//! - **Base link.** The `ld` dispatcher links normally (wild) and records the
//!   inode of each codegen unit object next to the binary
//!   (`<exe>.jrhot/base`).
//! - **Patch.** On the next link of the same binary, rustc has hardlinked
//!   every reused unit from the incremental cache, so a new inode is exactly a
//!   regenerated unit. Only those are linked by `ld.lld -shared` against the
//!   base binary's addresses (a generated linker script), and a map for the
//!   loader is written. The binary is not rewritten, only its mtime is bumped
//!   so cargo sees it as current. Anything unexpected (no base, TLS access in
//!   a changed unit, a missing symbol, lld error, more than half the units
//!   changed) runs the full link instead: fail open.
//! - **Loader.** Its constructor runs before any initializer of the
//!   executable, whoever starts it (cargo, the test pool, a test that
//!   re-executes itself). With a map for exactly this executable it maps the
//!   patch, redirects every replaced function with `jmp rel32` and registers
//!   the patch's unwind tables. Without a map it does nothing.
//!
//! Outcomes go to `<run dir>/hotpatch.jsonl` (one line per link) and
//! `~/.justrust/hotpatch.jsonl`.

use object::elf;
use object::read::elf::{ElfFile64, FileHeader, Rela, SectionHeader, Sym};
use object::{Object, ObjectSection, ObjectSymbol, RelocationFlags, SectionKind, SymbolKind};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Where a PIE runs with ASLR off (x86_64 Linux, 4-level paging). Map
/// addresses are relative to it; the loader adds the real load bias.
const PIE_BASE: u64 = 0x5555_5555_4000;
/// Patch images go 256 MiB below the executable, within `jmp rel32` reach.
const PATCH_SPAN: u64 = 0x1000_0000;
const LOADER_SRC: &str = include_str!("hotpatch/loader.c");
/// Set by the shim on the rustc that links a hot-patchable test binary.
const LINK_ENV: &str = "JUSTRUST_HOTPATCH_LINK";

type Res<T> = Result<T, String>;

/// `JUSTRUST_HOTPATCH=1` turns it on (off by default).
pub fn enabled() -> bool {
    matches!(
        std::env::var("JUSTRUST_HOTPATCH").as_deref(),
        Ok("1") | Ok("on") | Ok("true") | Ok("yes")
    )
}

/// Extra rustc args for a unit (`link_env` marks its link). Only local
/// test units that link, only when wild's `-B` dir is in use (the `ld`
/// dispatcher is where patching happens), only on x86_64 Linux.
pub fn rustc_args(local: bool, test: bool, linker_args: bool) -> Vec<OsString> {
    if !enabled() || !local || !test || !linker_args || std::env::consts::ARCH != "x86_64" {
        return Vec::new();
    }
    let Some(loader) = loader_path() else {
        return Vec::new();
    };
    vec![
        "-C".into(),
        "link-arg=-Wl,--no-gc-sections".into(),
        "-C".into(),
        format!(
            "link-arg=-Wl,--push-state,--no-as-needed,{},--pop-state",
            loader.display()
        )
        .into(),
    ]
}

/// Env for the rustc process whose link we may patch.
pub fn link_env() -> (&'static str, &'static str) {
    (LINK_ENV, "1")
}

/// The loader library, compiled from `loader.c` on first use. `None` (no
/// hot patching) when it cannot be built.
fn loader_path() -> Option<PathBuf> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in LOADER_SRC.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    let dir = crate::paths::home().ok()?.join("hotpatch");
    let so = dir.join(format!("loader-{h:016x}.so"));
    if so.is_file() {
        return Some(so);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join(format!("loader-{h:016x}.c"));
    std::fs::write(&src, LOADER_SRC).ok()?;
    let tmp = dir.join(format!(".loader-{h:016x}.{}.so", std::process::id()));
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-O2", "-Wall", "-o"])
        .arg(&tmp)
        .arg(&src)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    std::fs::rename(&tmp, &so).ok()?;
    Some(so)
}

fn state_dir(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_owned();
    s.push(".jrhot");
    PathBuf::from(s)
}

/// Inode, size, mtime (s, ns): what the loader checks before patching.
fn identity(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    Some(format!(
        "X {} {} {} {}",
        m.ino(),
        m.size(),
        m.mtime(),
        m.mtime_nsec()
    ))
}

fn log(out: &Path, outcome: &str, detail: &str, ms: f64) {
    let line = serde_json::json!({
        "time": crate::paths::now(),
        "out": out,
        "outcome": outcome,
        "detail": detail,
        "ms": (ms * 10.0).round() / 10.0,
    })
    .to_string();
    if let Some(d) = std::env::var_os("JUSTRUST_RUN_DIR") {
        let _ = crate::paths::append_line(&Path::new(&d).join("hotpatch.jsonl"), &line);
    }
    if let Ok(h) = crate::paths::home() {
        let _ = crate::paths::append_line(&h.join("hotpatch.jsonl"), &line);
    }
}

struct LinkArgs {
    out: PathBuf,
    /// This crate's codegen units (`<out>.<cgu>.<hash>.rcgu.o`).
    cgus: Vec<PathBuf>,
}

fn expand(args: &[OsString]) -> Vec<String> {
    let mut v = Vec::new();
    for a in args {
        let s = a.to_string_lossy().into_owned();
        if let Some(f) = s.strip_prefix('@')
            && let Ok(body) = std::fs::read_to_string(f)
        {
            v.extend(split_response_file(&body));
            continue;
        }
        v.push(s);
    }
    v
}

/// Arguments of a GNU-style response file (what gcc, clang and rustc's
/// linker invocation write): whitespace separates, single and double quotes
/// group, backslash escapes the next character.
fn split_response_file(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut quote: Option<char> = None;
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    have = true;
                }
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                have = true;
            }
            (None, c) if c.is_whitespace() => {
                if have {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            (None, c) => {
                cur.push(c);
                have = true;
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

fn parse(args: &[String]) -> Option<LinkArgs> {
    let i = args.iter().position(|a| a == "-o")?;
    let out = PathBuf::from(args.get(i + 1)?);
    let stem = out.file_name()?.to_str()?.to_owned();
    let prefix = format!("{stem}.");
    let cgus: Vec<PathBuf> = args
        .iter()
        .filter(|a| a.ends_with(".rcgu.o"))
        .map(PathBuf::from)
        .filter(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f.starts_with(&prefix))
        })
        .collect();
    (!cgus.is_empty()).then_some(LinkArgs { out, cgus })
}

/// CGU name from `<stem>.<cgu>.<sessionhash>.rcgu.o`.
fn cgu_name(p: &Path, stem: &str) -> String {
    let f = p.file_name().unwrap_or_default().to_string_lossy();
    let rest = f
        .get(stem.len() + 1..f.len().saturating_sub(".rcgu.o".len()))
        .unwrap_or("");
    rest.rsplit_once('.').map_or(rest, |(a, _)| a).to_owned()
}

/// Called by the `ld` dispatcher before linking. `Some(0)`: patched, do not
/// link. `None`: link normally (then call `after_link`).
pub fn before_link(raw: &[OsString]) -> Option<i32> {
    std::env::var_os(LINK_ENV)?;
    let args = expand(raw);
    if let Some(p) = std::env::var_os("JUSTRUST_HOTPATCH_DUMP") {
        let _ = std::fs::write(p, args.join("\n"));
    }
    let Some(la) = parse(&args) else {
        let out = args
            .iter()
            .position(|a| a == "-o")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_default();
        log(
            Path::new(&out),
            "skipped",
            "no codegen units of the output in the link",
            0.0,
        );
        return None;
    };
    let t = Instant::now();
    match patch(&la, &state_dir(&la.out)) {
        Ok(summary) => {
            log(
                &la.out,
                "patched",
                &summary,
                t.elapsed().as_secs_f64() * 1e3,
            );
            Some(0)
        }
        Err(why) => {
            log(&la.out, "full-link", &why, t.elapsed().as_secs_f64() * 1e3);
            None
        }
    }
}

/// After a successful full link: remember each unit's inode and the
/// executable, so the next link can patch.
pub fn after_link(raw: &[OsString]) {
    if std::env::var_os(LINK_ENV).is_none() {
        return;
    }
    if let Some(la) = parse(&expand(raw)) {
        let _ = record_base(&la, &state_dir(&la.out));
    }
}

fn record_base(la: &LinkArgs, dir: &Path) -> Res<()> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(dir.join("map"));
    let stem = la
        .out
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut s = String::new();
    writeln!(s, "{}", identity(&la.out).ok_or("no output")?).unwrap();
    for c in &la.cgus {
        let ino = std::fs::metadata(c).map_err(|e| e.to_string())?.ino();
        writeln!(s, "{} {ino}", cgu_name(c, &stem)).unwrap();
    }
    std::fs::write(dir.join("base"), s).map_err(|e| e.to_string())
}

/// Bump the executable's mtime (cargo's freshness check compares it with
/// the sources) and rewrite the recorded identity to match.
fn touch_and_rebase_identity(out: &Path, dir: &Path, base_txt: &str) -> Res<String> {
    let f = std::fs::File::options()
        .append(true)
        .open(out)
        .map_err(|e| e.to_string())?;
    f.set_modified(std::time::SystemTime::now())
        .map_err(|e| e.to_string())?;
    drop(f);
    let id = identity(out).ok_or("no base executable")?;
    let rest = base_txt.split_once('\n').map_or("", |(_, r)| r);
    std::fs::write(dir.join("base"), format!("{id}\n{rest}")).map_err(|e| e.to_string())?;
    Ok(id)
}

/// A GNU-flavor lld: `(program, leading args)`. Skips justrust's own `ld`
/// names and rustc's `gcc-ld/ld.lld` wrapper (needs rustc's environment).
fn lld() -> Option<(PathBuf, Vec<&'static str>)> {
    if let Some(p) = std::env::var_os("JUSTRUST_HOTPATCH_LLD") {
        return Some((PathBuf::from(p), Vec::new()));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for (n, pre) in [("ld.lld", vec![]), ("rust-lld", vec!["-flavor", "gnu"])] {
        for d in std::env::split_paths(&path) {
            let p = d.join(n);
            if p.is_file() && !crate::paths::is_justrust(&p) && !d.ends_with("gcc-ld") {
                return Some((p, pre));
            }
        }
    }
    None
}

// ---------------------------------------------------------------- patch

/// Symbols of one ELF file, keyed by name and by (CGU file group, name).
/// Borrowed from the mapped file: the base test binary of the Desktop UI
/// crate has 640k symbols, so no per-symbol allocation.
struct Syms<'a> {
    global: HashMap<&'a [u8], u64>,
    local: HashMap<(&'a [u8], &'a [u8]), u64>,
    /// Name -> (address, number of definitions).
    any: HashMap<&'a [u8], (u64, u32)>,
    /// Sorted start addresses of every function.
    starts: Vec<u64>,
}

impl<'a> Syms<'a> {
    /// `wanted`: only index these names (all function starts are kept).
    fn read(file: &ElfFile64<'a>, wanted: Option<&HashSet<&[u8]>>) -> Syms<'a> {
        let n = if wanted.is_some() {
            4096
        } else {
            file.symbols().size_hint().0
        };
        let mut s = Syms {
            global: HashMap::with_capacity(n / 2),
            local: HashMap::with_capacity(n / 2),
            any: HashMap::with_capacity(n),
            starts: Vec::with_capacity(n / 2),
        };
        let mut group: &[u8] = b"";
        for sym in file.symbols() {
            let Ok(name) = sym.name_bytes() else { continue };
            if sym.kind() == SymbolKind::File {
                group = name;
                continue;
            }
            if sym.is_undefined() || name.is_empty() || sym.kind() == SymbolKind::Section {
                continue;
            }
            let a = sym.address();
            if sym.kind() == SymbolKind::Text && a != 0 {
                s.starts.push(a);
            }
            if wanted.is_some_and(|w| !w.contains(name)) {
                continue;
            }
            if sym.is_global() || sym.is_weak() {
                s.global.insert(name, a);
            } else {
                s.local.insert((group, name), a);
            }
            s.any.entry(name).and_modify(|e| e.1 += 1).or_insert((a, 1));
        }
        s.starts.sort_unstable();
        s.starts.dedup();
        s
    }

    fn find(&self, group: &[u8], name: &[u8]) -> Option<u64> {
        if let Some(a) = self.local.get(&(group, name)) {
            return Some(*a);
        }
        if let Some(a) = self.global.get(name) {
            return Some(*a);
        }
        match self.any.get(name) {
            Some(&(a, 1)) => Some(a),
            _ => None,
        }
    }
}

fn mmap(p: &Path) -> Res<memmap2::Mmap> {
    let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    unsafe { memmap2::Mmap::map(&f) }.map_err(|e| e.to_string())
}

fn patch(la: &LinkArgs, dir: &Path) -> Res<String> {
    let base_txt = std::fs::read_to_string(dir.join("base")).map_err(|_| "no recorded base")?;
    let mut lines = base_txt.lines();
    let header = lines.next().ok_or("empty base")?;
    if identity(&la.out).as_deref() != Some(header) {
        return Err("base executable changed since it was recorded".into());
    }
    let base_inodes: HashMap<&str, u64> = lines
        .filter_map(|l| l.split_once(' '))
        .filter_map(|(n, i)| Some((n, i.parse().ok()?)))
        .collect();
    let stem = la.out.file_name().unwrap().to_string_lossy().into_owned();
    let mut changed = Vec::new();
    for c in &la.cgus {
        let ino = std::fs::metadata(c).map_err(|e| e.to_string())?.ino();
        if base_inodes.get(cgu_name(c, &stem).as_str()) != Some(&ino) {
            changed.push(c.clone());
        }
    }
    if changed.is_empty() {
        // Nothing to patch: the base binary is current. Drop an old patch.
        let _ = std::fs::remove_file(dir.join("map"));
        touch_and_rebase_identity(&la.out, dir, &base_txt)?;
        return Ok("0 units changed".into());
    }
    if changed.len() * 2 > la.cgus.len().max(2) {
        return Err(format!("{}/{} units changed", changed.len(), la.cgus.len()));
    }

    let t0 = Instant::now();
    let mut phases = String::new();
    let mut lap = |what: &str, t: &mut Instant| {
        let _ = write!(phases, " {what} {:.0}", t.elapsed().as_secs_f64() * 1e3);
        *t = Instant::now();
    };
    let mut t = t0;
    // Changed objects: what they define, what they need.
    let maps: Vec<memmap2::Mmap> = changed.iter().map(|p| mmap(p)).collect::<Res<_>>()?;
    let mut need: HashSet<Vec<u8>> = HashSet::new();
    let mut defined: HashSet<Vec<u8>> = HashSet::new();
    // (cgu group, name) of functions to redirect, and writable data to bind.
    let mut funcs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut data: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for (p, mm) in changed.iter().zip(&maps) {
        let obj = ElfFile64::<object::Endianness>::parse(&**mm)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        let mut group = Vec::new();
        for sec in obj.sections() {
            let n = sec.name().unwrap_or("");
            // A thread-local *definition* is harmless: the base keeps its copy
            // and nothing in the patch can reach the patch's copy without a
            // TLS relocation, which falls back below.
            let _ = n;
            for (_, r) in sec.relocations() {
                if let RelocationFlags::Elf { r_type } = r.flags()
                    && matches!(
                        r_type,
                        elf::R_X86_64_TPOFF32
                            | elf::R_X86_64_TPOFF64
                            | elf::R_X86_64_GOTTPOFF
                            | elf::R_X86_64_TLSGD
                            | elf::R_X86_64_TLSLD
                            | elf::R_X86_64_DTPOFF32
                            | elf::R_X86_64_DTPOFF64
                    )
                {
                    let target = match r.target() {
                        object::RelocationTarget::Symbol(i) => obj
                            .symbol_by_index(i)
                            .ok()
                            .and_then(|s| s.name().ok().map(str::to_owned))
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    return Err(format!(
                        "thread-local access (type {r_type}) to {target} in {}",
                        p.display()
                    ));
                }
            }
        }
        for sym in obj.symbols() {
            let Ok(name) = sym.name_bytes() else { continue };
            if sym.kind() == SymbolKind::File {
                group = name.to_vec();
                continue;
            }
            if name.is_empty() || sym.kind() == SymbolKind::Section {
                continue;
            }
            if sym.is_undefined() {
                need.insert(name.to_vec());
                continue;
            }
            defined.insert(name.to_vec());
            match sym.kind() {
                SymbolKind::Text if sym.size() > 0 => funcs.push((group.clone(), name.to_vec())),
                SymbolKind::Data => {
                    let writable = sym
                        .section_index()
                        .and_then(|i| obj.section_by_index(i).ok())
                        .is_some_and(|s| {
                            matches!(s.kind(), SectionKind::Data | SectionKind::UninitializedData)
                                && !s.name().unwrap_or("").starts_with(".data.rel.ro")
                        });
                    if writable && !(sym.is_global() || sym.is_weak()) {
                        // A local static is only reachable from its own unit,
                        // and every function of a changed unit is replaced, so
                        // the patch's fresh copy is the only one in use (test
                        // processes start fresh, the loader runs before any
                        // initializer of the executable).
                        continue;
                    }
                    if writable {
                        data.push((group.clone(), name.to_vec()));
                    }
                }
                _ => {}
            }
        }
    }

    // Base executable: addresses of everything the patch needs.
    let base_mm = mmap(&la.out)?;
    let base = ElfFile64::<object::Endianness>::parse(&*base_mm).map_err(|e| e.to_string())?;
    if base.elf_header().e_type(base.endian()) != elf::ET_DYN {
        return Err("base executable is not PIE".into());
    }
    lap("objs", &mut t);
    let wanted: HashSet<&[u8]> = need
        .iter()
        .chain(funcs.iter().map(|(_, n)| n))
        .chain(data.iter().map(|(_, n)| n))
        .map(|v| v.as_slice())
        .collect();
    let bsyms = Syms::read(&base, Some(&wanted));
    lap("basesyms", &mut t);
    let patch_base = PIE_BASE - PATCH_SPAN;
    let mut script = String::new();
    let mut dyn_imports = 0;
    let sym_line = |s: &mut String, name: &[u8], addr: u64| {
        let n = String::from_utf8_lossy(name).replace('"', "\\\"");
        let _ = writeln!(
            s,
            "\"{n}\" = __ehdr_start + 0x{:x};",
            (PIE_BASE + addr) - patch_base
        );
    };
    // Shared-library imports of the base (the loader resolves them).
    let imports: HashSet<Vec<u8>> = base
        .dynamic_symbols()
        .filter(|s| s.is_undefined())
        .filter_map(|s| s.name_bytes().ok())
        .map(|n| n.split(|&b| b == b'@').next().unwrap_or(n).to_vec())
        .collect();
    for n in &need {
        if defined.contains(n) {
            continue;
        }
        match bsyms.find(b"", n) {
            Some(a) if a != 0 => sym_line(&mut script, n, a),
            _ if imports.contains(n) => dyn_imports += 1,
            // Garbage-collected from the base link, or new: needs a relink.
            _ => {
                return Err(format!(
                    "{} is not in the base binary",
                    String::from_utf8_lossy(n)
                ));
            }
        }
    }
    // Writable statics keep one copy: the base binary's. A static the
    // base does not have (new, or gc'd) would split state: relink.
    let mut bound = 0;
    for (group, n) in &data {
        let Some(a) = bsyms.find(group, n) else {
            return Err(format!(
                "static {} is not in the base binary",
                String::from_utf8_lossy(n)
            ));
        };
        sym_line(&mut script, n, a);
        bound += 1;
    }
    // Make the eh_frame list zero-terminated for __register_frame.
    script.push_str("SECTIONS { .jr_eh_end : { LONG(0) } } INSERT AFTER .eh_frame;\n");

    let seq = std::fs::read_to_string(dir.join("seq"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
        + 1;
    let _ = std::fs::write(dir.join("seq"), seq.to_string());
    let script_path = dir.join("defs.ld");
    std::fs::write(&script_path, &script).map_err(|e| e.to_string())?;
    let image = dir.join(format!("patch-{seq}.so"));
    lap("script", &mut t);
    let (lld, lld_pre) = lld().ok_or("no ld.lld or rust-lld on PATH")?;
    let st = Command::new(lld)
        .args(lld_pre)
        .args([
            "-shared",
            "-Bsymbolic",
            "-z",
            "now",
            "-z",
            "norelro",
            "--no-gc-sections",
        ])
        .args([
            "-z",
            "nopack-relative-relocs",
            "--hash-style=gnu",
            "-nostdlib",
        ])
        .arg(format!("--image-base=0x{patch_base:x}"))
        .arg("-o")
        .arg(&image)
        .args(&changed)
        .arg(&script_path)
        .output()
        .map_err(|e| format!("ld.lld: {e}"))?;
    if !st.status.success() {
        let err = String::from_utf8_lossy(&st.stderr);
        return Err(format!(
            "ld.lld failed on the patch: {}",
            err.lines().take(3).collect::<Vec<_>>().join(" | ")
        ));
    }
    lap("lld", &mut t);

    // Patch image: relocations for the loader, new addresses, unwind tables.
    let pm = mmap(&image)?;
    let pf = ElfFile64::<object::Endianness>::parse(&*pm).map_err(|e| e.to_string())?;
    let mut map = String::new();
    writeln!(map, "X 0 0 0 0").unwrap();
    writeln!(map, "B {PIE_BASE:x}").unwrap();
    writeln!(map, "I {}", image.display()).unwrap();
    let eh = pf.section_by_name(".eh_frame").map_or(0, |s| s.address());
    writeln!(map, "E {eh:x}").unwrap();
    relocs(&pf, &mut map)?;

    let psyms = Syms::read(&pf, None);
    let starts = &bsyms.starts;
    let mut redirected = 0;
    let mut new_fns = 0;
    let mut seen = HashSet::new();
    for (group, name) in &funcs {
        let Some(old) = bsyms.find(group, name) else {
            new_fns += 1;
            continue;
        };
        if !seen.insert(old) {
            continue;
        }
        let new = psyms
            .find(group, name)
            .ok_or_else(|| format!("{} missing from patch", String::from_utf8_lossy(name)))?;
        let next = starts.partition_point(|&a| a <= old);
        let room = starts.get(next).map_or(u64::MAX, |n| n - old);
        if room < 5 {
            // Too small for `jmp rel32`, and too small to hold any rel32
            // reference, so its bytes are position-independent: identical
            // bytes mean identical behavior, nothing to redirect.
            let len = room.min(psyms_size(&pf, new).unwrap_or(u64::MAX));
            let same = (|| {
                let a = base.data_range_at(old, len)?;
                let b = pf.data_range_at(new, len)?;
                Some(a == b)
            })();
            if same == Some(true) {
                continue;
            }
            return Err(format!(
                "{} is too small for a jump",
                String::from_utf8_lossy(name)
            ));
        }
        writeln!(map, "T {:x} {new:x}", PIE_BASE + old).unwrap();
        redirected += 1;
    }
    // cargo compares the binary's mtime with the sources: bump it, then
    // write the map for exactly the bumped identity (atomically, a test
    // process may be starting).
    let id = touch_and_rebase_identity(&la.out, dir, &base_txt)?;
    let map = map.replacen("X 0 0 0 0", &id, 1);
    let tmp = dir.join("map.tmp");
    std::fs::write(&tmp, map).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join("map")).map_err(|e| e.to_string())?;
    lap("map", &mut t);
    // Older images are no longer referenced.
    if seq > 1 {
        let _ = std::fs::remove_file(dir.join(format!("patch-{}.so", seq - 1)));
    }
    Ok(format!(
        "{}/{} units, {redirected} functions redirected, {new_fns} new, {bound} statics bound, {dyn_imports} imports, ms:{phases}",
        changed.len(),
        la.cgus.len()
    ))
}

/// Bytes at a virtual address of an ELF image.
trait DataAt {
    fn data_range_at(&self, addr: u64, len: u64) -> Option<&[u8]>;
}

impl DataAt for ElfFile64<'_, object::Endianness> {
    fn data_range_at(&self, addr: u64, len: u64) -> Option<&[u8]> {
        self.sections()
            .find(|s| s.address() <= addr && addr + len <= s.address() + s.size())
            .and_then(|s| s.data_range(addr, len).ok().flatten())
    }
}

/// Size of the function symbol at `addr` in the patch image.
fn psyms_size(pf: &ElfFile64<object::Endianness>, addr: u64) -> Option<u64> {
    pf.symbols()
        .find(|s| s.address() == addr && s.kind() == SymbolKind::Text && s.size() > 0)
        .map(|s| s.size())
}

/// Dynamic relocations of the patch image as loader map lines. The image is
/// mapped at its link address, so RELATIVE values are final here.
fn relocs(pf: &ElfFile64<object::Endianness>, map: &mut String) -> Res<()> {
    let endian = pf.endian();
    let data = pf.data();
    let sections = pf.elf_section_table();
    for sh in sections.iter() {
        if sh.sh_type(endian) != elf::SHT_RELA {
            continue;
        }
        let relas = sh
            .data_as_array::<elf::Rela64<object::Endianness>, _>(endian, data)
            .map_err(|e| e.to_string())?;
        let symtab = sections
            .symbol_table_by_index(
                endian,
                data,
                object::SectionIndex(sh.sh_link(endian) as usize),
            )
            .map_err(|e| e.to_string())?;
        for r in relas {
            let off = r.r_offset(endian);
            let add = r.r_addend(endian);
            let ty = r.r_type(endian, false);
            let si = r.r_sym(endian, false);
            match ty {
                elf::R_X86_64_RELATIVE => {
                    writeln!(map, "A {off:x} {:x}", add as u64).unwrap();
                }
                elf::R_X86_64_GLOB_DAT | elf::R_X86_64_JUMP_SLOT | elf::R_X86_64_64 => {
                    let sym = symtab
                        .symbol(object::SymbolIndex(si as usize))
                        .map_err(|e| e.to_string())?;
                    let name = symtab.symbol_name(endian, sym).map_err(|e| e.to_string())?;
                    if sym.st_shndx(endian) != elf::SHN_UNDEF {
                        let v = sym.st_value(endian).wrapping_add(add as u64);
                        writeln!(map, "A {off:x} {v:x}").unwrap();
                    } else {
                        let kind = if sym.st_bind() == elf::STB_WEAK {
                            'W'
                        } else {
                            'D'
                        };
                        writeln!(
                            map,
                            "{kind} {off:x} {:x} {}",
                            add as u64,
                            String::from_utf8_lossy(name)
                        )
                        .unwrap();
                    }
                }
                elf::R_X86_64_NONE => {}
                other => return Err(format!("unsupported dynamic relocation {other}")),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn finds_own_codegen_units_and_names() {
        let args: Vec<String> = [
            "-pie",
            // rustc's names: <out stem>.<cgu name>.<session hash>.rcgu.o
            "/t/deps/ui-abc.07gvcskbr3mjc6lxb6mccnjzv.0xyzvhv.rcgu.o",
            "/t/deps/ui-abc.0fj73i5q2a1r0jj2a1qj2kqy8.0xyzvhv.rcgu.o",
            "/t/deps/libdep-x.rlib",
            "/tmp/rustcXYZ/symbols.o",
            "/t/deps/other-def.other.q-cgu.0.rcgu.o",
            "-o",
            "/t/deps/ui-abc",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let la = parse(&args).unwrap();
        assert_eq!(la.out, PathBuf::from("/t/deps/ui-abc"));
        assert_eq!(la.cgus.len(), 2);
        // The CGU name, stable across sessions, is the inode map's key.
        assert_eq!(cgu_name(&la.cgus[0], "ui-abc"), "07gvcskbr3mjc6lxb6mccnjzv");
        assert_eq!(cgu_name(&la.cgus[1], "ui-abc"), "0fj73i5q2a1r0jj2a1qj2kqy8");
        // A link without any of its own units (a dependency dylib, a C
        // program) is never a patch candidate.
        assert!(parse(&["-o".into(), "/t/x".into(), "/t/a.o".into()]).is_none());
    }

    #[test]
    fn splits_response_files_like_gcc() {
        assert_eq!(
            split_response_file("\"-o\" \"/a b/x\" plain 'q\"x' a\\ b\n-pie \"\"\n"),
            vec!["-o", "/a b/x", "plain", "q\"x", "a b", "-pie", ""]
        );
        // rustc's own response files: one argument per line.
        assert_eq!(split_response_file("-o\n/t/x\n"), vec!["-o", "/t/x"]);
    }

    #[test]
    fn off_by_default_and_only_for_local_linking_tests() {
        // SAFETY: tests in this module are the only readers of this var.
        unsafe { std::env::remove_var("JUSTRUST_HOTPATCH") };
        assert!(rustc_args(true, true, true).is_empty());
        unsafe { std::env::set_var("JUSTRUST_HOTPATCH", "1") };
        assert!(rustc_args(false, true, true).is_empty());
        assert!(rustc_args(true, false, true).is_empty());
        assert!(rustc_args(true, true, false).is_empty());
        unsafe { std::env::remove_var("JUSTRUST_HOTPATCH") };
    }

    #[test]
    fn link_hooks_do_nothing_without_the_link_marker() {
        assert!(std::env::var_os(LINK_ENV).is_none());
        assert_eq!(before_link(&os(&["-o", "/nonexistent/x"])), None);
        after_link(&os(&["-o", "/nonexistent/x"]));
    }
}
