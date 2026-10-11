//! jr-hot: hot patching of Rust test binaries (prototype, FINDINGS 35).
//!
//! Two roles:
//!
//! - `jr-hot` as the linker (`CARGO_TARGET_<triple>_LINKER`): with
//!   `JR_HOT=1` and a recorded base link, it does not relink the test binary.
//!   It finds the codegen units rustc regenerated (their `.rcgu.o` is a new
//!   file: reused units are hardlinks of the incremental cache, same inode),
//!   links only those into a small patch image at a fixed address below the
//!   base executable, resolving every other symbol to its address in the base
//!   binary, and writes a map for the loader. Anything unexpected runs the
//!   real linker (fail open). Without `JR_HOT=1` it links normally and records
//!   the base.
//! - `jr-hot run <exe> <args>` as the cargo runner: when a patch map matches
//!   the executable, it disables ASLR for the process, preloads `loader.so`
//!   and execs the binary. The loader maps the patch and writes `jmp rel32`
//!   over the entry of every replaced function.
//!
//! Limits (each falls back to a full link): thread-locals in a changed unit,
//! a function too small for a 5-byte jump, more than half the units changed.

use object::elf;
use object::read::elf::{ElfFile64, FileHeader, Rela, SectionHeader, Sym};
use object::{Object, ObjectSection, ObjectSymbol, RelocationFlags, SectionKind, SymbolKind};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Where a PIE runs with ASLR off (x86_64 Linux, 4-level paging).
const PIE_BASE: u64 = 0x5555_5555_4000;
/// Patch images go 256 MiB below it, within `jmp rel32` reach.
const PATCH_SPAN: u64 = 0x1000_0000;
const LOADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/loader.so"));

type Res<T> = Result<T, String>;

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "run") {
        run(&args[1..]);
    }
    std::process::exit(link(&args));
}

fn state_dir(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_owned();
    s.push(".jrhot");
    PathBuf::from(s)
}

fn identity(p: &Path) -> Option<(u64, u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.ino(), m.size(), m.mtime()))
}

fn log(msg: &str) {
    if let Some(p) = std::env::var_os("JR_HOT_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
        {
            let _ = writeln!(f, "jr-hot: {msg}");
        }
    }
    if std::env::var_os("JR_HOT_QUIET").is_none() {
        eprintln!("jr-hot: {msg}");
    }
}

// ---------------------------------------------------------------- runner

fn run(args: &[OsString]) -> ! {
    let exe = PathBuf::from(&args[0]);
    let map = state_dir(&exe).join("map");
    let mut cmd = Command::new(&exe);
    cmd.args(&args[1..]);
    let id = identity(&exe);
    let header = std::fs::read_to_string(&map)
        .ok()
        .and_then(|s| s.lines().next().map(str::to_owned));
    let matches = match (id, header) {
        (Some((i, s, m)), Some(h)) => h == format!("X {i} {s} {m}"),
        _ => false,
    };
    if matches {
        let loader = state_dir(&exe).join("loader.so");
        if std::fs::read(&loader).ok().as_deref() != Some(LOADER) {
            let _ = std::fs::write(&loader, LOADER);
        }
        let mut pre = loader.into_os_string();
        if let Some(old) = std::env::var_os("LD_PRELOAD") {
            pre.push(":");
            pre.push(old);
        }
        cmd.env("LD_PRELOAD", pre).env("JR_HOTPATCH", &map);
        unsafe {
            cmd.pre_exec(|| {
                libc::personality(libc::ADDR_NO_RANDOMIZE as libc::c_ulong);
                Ok(())
            });
        }
    }
    let err = cmd.exec();
    eprintln!("jr-hot: exec {}: {err}", exe.display());
    std::process::exit(127)
}

// ---------------------------------------------------------------- linker

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
            v.extend(body.lines().map(str::to_owned));
            continue;
        }
        v.push(s);
    }
    v
}

fn parse(args: &[String]) -> Option<LinkArgs> {
    let i = args.iter().position(|a| a == "-o")?;
    let out = PathBuf::from(args.get(i + 1)?);
    let stem = out.file_name()?.to_str()?.to_owned();
    let cgus = args
        .iter()
        .filter(|a| a.ends_with(".rcgu.o"))
        .map(PathBuf::from)
        .filter(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f.starts_with(&format!("{stem}.")))
        })
        .collect();
    Some(LinkArgs { out, cgus })
}

/// CGU name from `<stem>.<cgu>.<sessionhash>.rcgu.o`.
fn cgu_name(p: &Path, stem: &str) -> String {
    let f = p.file_name().unwrap().to_string_lossy();
    let rest = &f[stem.len() + 1..f.len() - ".rcgu.o".len()];
    rest.rsplit_once('.').map_or(rest, |(a, _)| a).to_owned()
}

fn real_link(args: &[OsString], keep_sections: bool) -> i32 {
    let linker = std::env::var_os("JR_HOT_REAL_LINKER").unwrap_or_else(|| "cc".into());
    let mut cmd = Command::new(&linker);
    cmd.args(args);
    if keep_sections {
        // A later patch may call functions nothing referenced yet (generic
        // instances shared by dependencies, unreferenced local items).
        // --gc-sections would have dropped them from the base. Last wins.
        cmd.arg("-Wl,--no-gc-sections");
    }
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("jr-hot: cannot run {}: {e}", linker.to_string_lossy());
            1
        }
    }
}

fn link(raw: &[OsString]) -> i32 {
    let args = expand(raw);
    let Some(la) = parse(&args) else {
        return real_link(raw, false);
    };
    let hot = std::env::var_os("JR_HOT").is_some_and(|v| v == "1");
    let dir = state_dir(&la.out);
    if hot {
        let t = Instant::now();
        match patch(&la, &dir) {
            Ok(summary) => {
                log(&format!(
                    "patched instead of linking: {summary} in {:.0} ms",
                    t.elapsed().as_secs_f64() * 1e3
                ));
                return 0;
            }
            Err(why) => log(&format!("full link: {why}")),
        }
    }
    let t = Instant::now();
    let code = real_link(raw, hot);
    if code == 0 {
        let _ = record_base(&la, &dir);
        if hot {
            log(&format!(
                "base linked in {:.0} ms",
                t.elapsed().as_secs_f64() * 1e3
            ));
        }
    }
    code
}

/// After a full link: remember each unit's inode and the executable.
fn record_base(la: &LinkArgs, dir: &Path) -> Res<()> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem = la.out.file_name().unwrap().to_string_lossy().into_owned();
    let mut s = String::new();
    let (i, sz, m) = identity(&la.out).ok_or("no output")?;
    writeln!(s, "X {i} {sz} {m}").unwrap();
    for c in &la.cgus {
        let ino = std::fs::metadata(c).map_err(|e| e.to_string())?.ino();
        writeln!(s, "{} {ino}", cgu_name(c, &stem)).unwrap();
    }
    let _ = std::fs::remove_file(dir.join("map"));
    std::fs::write(dir.join("base"), s).map_err(|e| e.to_string())
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
    let (i, sz, m) = identity(&la.out).ok_or("no base executable")?;
    if header != format!("X {i} {sz} {m}") {
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
    let lld = std::env::var_os("JR_HOT_LLD").unwrap_or_else(|| "ld.lld".into());
    let st = Command::new(lld)
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
        .status()
        .map_err(|e| format!("ld.lld: {e}"))?;
    if !st.success() {
        return Err("ld.lld failed on the patch".into());
    }
    lap("lld", &mut t);

    // Patch image: relocations for the loader, new addresses, unwind tables.
    let pm = mmap(&image)?;
    let pf = ElfFile64::<object::Endianness>::parse(&*pm).map_err(|e| e.to_string())?;
    let mut map = String::new();
    writeln!(map, "X {i} {sz} {m}").unwrap();
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
    std::fs::write(dir.join("map"), map).map_err(|e| e.to_string())?;
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
