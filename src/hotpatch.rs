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
use object::read::elf::{ElfFile64, FileHeader, ProgramHeader, Rela, SectionHeader, Sym};
use object::{Object, ObjectSection, ObjectSymbol, SectionKind, SymbolKind};
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
        let dir = state_dir(&la.out);
        if record_base(&la, &dir).is_ok() {
            prebuild_index(&la.out, &dir);
        }
    }
}

/// Build the base's symbol index in a detached child (~0.3 s for the
/// Desktop UI test binary), so the first patch after a full link does not
/// pay for it. Nothing waits for it: a patch that starts first builds its
/// own copy (both renames are atomic). The child's stdio goes to
/// /dev/null so rustc, which reads the linker's output to EOF, does not
/// wait either.
fn prebuild_index(out: &Path, dir: &Path) {
    if std::env::var_os("JUSTRUST_HOTPATCH_SYNC_INDEX").is_some() {
        let _ = mmap(out).and_then(|m| index_for(out, &m, dir));
        return;
    }
    // SAFETY: the ld dispatcher is single-threaded here; the child only
    // redirects fds, builds the index and exits without unwinding.
    unsafe {
        let pid = libc::fork();
        if pid != 0 {
            return;
        }
        libc::setsid();
        if !isolate_index_child_fds() {
            // Optional speculation must not retain a build/eviction lock or
            // an output pipe. Older kernels or sandbox policies can reject
            // close_range: skip prebuilding, not descriptor isolation.
            libc::_exit(0);
        }
        libc::nice(10);
        let _ = mmap(out).and_then(|m| index_for(out, &m, dir));
        libc::_exit(0);
    }
}

/// Called only in the fork child. Use async-signal-safe operations before
/// indexing, including when tests fork from a multithreaded test harness.
unsafe fn isolate_index_child_fds() -> bool {
    // SAFETY: these syscalls operate only on the calling child's fd table.
    unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null < 0 {
            return false;
        }
        for fd in 0..3 {
            if libc::dup2(null, fd) < 0 {
                return false;
            }
        }
        // CLOEXEC does not help: this child never execs. Inherited flock
        // descriptors otherwise keep RootLock held until indexing finishes.
        #[cfg(target_os = "linux")]
        {
            libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) == 0
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
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

// ---------------------------------------------------------- base index

/// On-disk symbol index of a base executable (`<exe>.jrhot/index`), built
/// on the first patch after a base link and mapped by every later patch.
/// Names are not copied: entries point at the base's own string table.
///
/// Layout (little-endian u64s): header `MAGIC, inode, size, shdr hash,
/// tls block size, n entries, n starts`, then entries `hash(name), value,
/// name offset, group offset (u64::MAX: none), flags` sorted by hash, then
/// the sorted start addresses of every function.
const INDEX_MAGIC: u64 = u64::from_le_bytes(*b"JRHIDX02");
const HDR: usize = 7;
const ENT: usize = 5;
const F_GLOBAL: u64 = 1;
const F_TLS: u64 = 2;

fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &c in b {
        h = (h ^ c as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// What ties an index to one base binary: inode, size and a hash of its
/// section header table (offsets and sizes of everything).
fn base_key(out: &Path, base: &[u8]) -> Res<[u64; 3]> {
    let m = std::fs::metadata(out).map_err(|e| e.to_string())?;
    let shoff = u64::from_le_bytes(base.get(0x28..0x30).ok_or("short ELF")?.try_into().unwrap());
    let n = u16::from_le_bytes(base.get(0x3c..0x3e).ok_or("short ELF")?.try_into().unwrap());
    let shdrs = base
        .get(shoff as usize..shoff as usize + n as usize * 64)
        .ok_or("bad section headers")?;
    Ok([m.ino(), m.size(), fnv(shdrs)])
}

fn build_index(base_mm: &[u8], key: [u64; 3]) -> Res<Vec<u8>> {
    let base = ElfFile64::<object::Endianness>::parse(base_mm).map_err(|e| e.to_string())?;
    let endian = base.endian();
    // x86_64 TLS variant II: a thread-local at offset `v` of the TLS
    // segment lives at %fs:(v - round_up(memsz, align)).
    let mut tls_size = 0u64;
    for ph in base
        .elf_program_headers()
        .iter()
        .filter(|p| p.p_type(endian) == elf::PT_TLS)
    {
        let al = ph.p_align(endian).max(1);
        tls_size = ph.p_memsz(endian).div_ceil(al) * al;
    }
    let off = |s: &[u8]| s.as_ptr() as u64 - base_mm.as_ptr() as u64;
    let mut ents: Vec<[u64; ENT]> = Vec::with_capacity(base.symbols().size_hint().0);
    let mut starts = Vec::new();
    let mut group: Option<&[u8]> = None;
    for sym in base.symbols() {
        let Ok(name) = sym.name_bytes() else { continue };
        if sym.kind() == SymbolKind::File {
            group = Some(name);
            continue;
        }
        if sym.is_undefined() || name.is_empty() || sym.kind() == SymbolKind::Section {
            continue;
        }
        let a = sym.address();
        if sym.kind() == SymbolKind::Text && a != 0 {
            starts.push(a);
        }
        let global = sym.is_global() || sym.is_weak();
        let flags = if global { F_GLOBAL } else { 0 }
            | if sym.kind() == SymbolKind::Tls {
                F_TLS
            } else {
                0
            };
        ents.push([
            fnv(name),
            a,
            off(name),
            group.filter(|_| !global).map_or(u64::MAX, off),
            flags,
        ]);
    }
    ents.sort_unstable_by_key(|e| e[0]);
    starts.sort_unstable();
    starts.dedup();
    let hdr = [
        INDEX_MAGIC,
        key[0],
        key[1],
        key[2],
        tls_size,
        ents.len() as u64,
        starts.len() as u64,
    ];
    let mut v = Vec::with_capacity(8 * (HDR + ents.len() * ENT + starts.len()));
    for x in hdr.iter().chain(ents.iter().flatten()).chain(&starts) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    Ok(v)
}

/// The index for this base: mapped from `<dir>/index` when it matches,
/// else built and saved. `true` when it was built now.
fn index_for(out: &Path, base_mm: &[u8], dir: &Path) -> Res<(memmap2::Mmap, bool)> {
    let key = base_key(out, base_mm)?;
    let p = dir.join("index");
    if let Ok(m) = mmap(&p)
        && m.len() >= HDR * 8
        && u64s(&m, 0, 4) == [INDEX_MAGIC, key[0], key[1], key[2]]
    {
        return Ok((m, false));
    }
    let v = build_index(base_mm, key)?;
    let tmp = dir.join(format!("index.{}", std::process::id()));
    std::fs::write(&tmp, &v).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok((mmap(&p)?, true))
}

fn u64s(b: &[u8], at: usize, n: usize) -> Vec<u64> {
    (0..n)
        .map(|i| u64::from_le_bytes(b[(at + i) * 8..(at + i) * 8 + 8].try_into().unwrap()))
        .collect()
}

/// Lookups in a mapped base index (see `build_index`).
struct Index<'a> {
    idx: &'a [u8],
    base: &'a [u8],
    tls_size: u64,
    n: usize,
    nstarts: usize,
}

impl<'a> Index<'a> {
    fn new(idx: &'a [u8], base: &'a [u8]) -> Res<Index<'a>> {
        let h = u64s(idx, 0, HDR);
        let (n, nstarts) = (h[5] as usize, h[6] as usize);
        if idx.len() != 8 * (HDR + n * ENT + nstarts) {
            return Err("truncated base index".into());
        }
        Ok(Index {
            idx,
            base,
            tls_size: h[4],
            n,
            nstarts,
        })
    }

    fn word(&self, i: usize) -> u64 {
        u64::from_le_bytes(self.idx[i * 8..i * 8 + 8].try_into().unwrap())
    }

    fn ent(&self, i: usize) -> [u64; ENT] {
        std::array::from_fn(|k| self.word(HDR + i * ENT + k))
    }

    fn cstr(&self, off: u64) -> &'a [u8] {
        let s = self.base.get(off as usize..).unwrap_or(b"");
        &s[..memchr::memchr(0, s).unwrap_or(s.len())]
    }

    /// Every entry named `name`.
    fn named(&self, name: &[u8]) -> impl Iterator<Item = [u64; ENT]> + '_ {
        let h = fnv(name);
        let (mut lo, mut hi) = (0, self.n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.word(HDR + mid * ENT) < h {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let name = name.to_vec();
        (lo..self.n)
            .map(|i| self.ent(i))
            .take_while(move |e| e[0] == h)
            .filter(move |e| self.cstr(e[2]) == name.as_slice())
    }

    /// Same resolution as `Syms::find`: the local of this CGU group, else
    /// the global, else the only definition with this name.
    fn pick(&self, group: &[u8], name: &[u8], tls: bool) -> Option<u64> {
        let (mut global, mut any, mut count) = (None, None, 0);
        for e in self.named(name).filter(|e| (e[4] & F_TLS != 0) == tls) {
            if e[4] & F_GLOBAL == 0 && e[3] != u64::MAX && self.cstr(e[3]) == group {
                return Some(e[1]);
            }
            if e[4] & F_GLOBAL != 0 {
                global = Some(e[1]);
            }
            any = Some(e[1]);
            count += 1;
        }
        global.or(if count == 1 { any } else { None })
    }

    fn find(&self, group: &[u8], name: &[u8]) -> Option<u64> {
        self.pick(group, name, false)
    }

    /// Thread-pointer offset of a thread-local of the base.
    fn tpoff(&self, group: &[u8], name: &[u8]) -> Option<i64> {
        let v = self.pick(group, name, true)?;
        Some(v as i64 - self.tls_size as i64)
    }

    /// The first function start after `a`.
    fn next_start(&self, a: u64) -> Option<u64> {
        let at = HDR + self.n * ENT;
        let (mut lo, mut hi) = (0, self.nstarts);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.word(at + mid) <= a {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo < self.nstarts).then(|| self.word(at + lo))
    }
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
    // Thread-local accesses, per changed object (rewritten in a copy below).
    let mut tls: Vec<Vec<TlsFix>> = Vec::with_capacity(maps.len());
    for (p, mm) in changed.iter().zip(&maps) {
        let obj = ElfFile64::<object::Endianness>::parse(&**mm)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        let mut group = Vec::new();
        // A thread-local *definition* is harmless: the base keeps its copy,
        // and every access in the patch is rewritten to the base's offset.
        tls.push(tls_fixups(&obj).map_err(|e| format!("{e} in {}", p.display()))?);
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

    // Base executable: addresses of everything the patch needs, from the
    // symbol index built once per base (reading the base's 960k-symbol
    // table on every patch was 130-160 ms of a ~200 ms patch step).
    let base_mm = mmap(&la.out)?;
    let base = ElfFile64::<object::Endianness>::parse(&*base_mm).map_err(|e| e.to_string())?;
    if base.elf_header().e_type(base.endian()) != elf::ET_DYN {
        return Err("base executable is not PIE".into());
    }
    lap("objs", &mut t);
    let (idx_mm, built) = index_for(&la.out, &base_mm, dir)?;
    let bsyms = Index::new(&idx_mm, &base_mm)?;
    lap(if built { "basesyms(built)" } else { "basesyms" }, &mut t);
    // Objects with thread-local accesses are linked from a rewritten copy.
    let tls_names: HashSet<&[u8]> = tls.iter().flatten().map(|f| f.name.as_slice()).collect();
    let mut inputs = changed.clone();
    let mut tls_fixed = 0;
    for (i, fixes) in tls.iter().enumerate() {
        if fixes.is_empty() {
            continue;
        }
        let mut bytes = maps[i].to_vec();
        apply_tls(&mut bytes, fixes, &|g, n| bsyms.tpoff(g, n))?;
        let p = dir.join(format!("tls-{i}.o"));
        std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
        inputs[i] = p;
        tls_fixed += fixes.len();
    }
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
        if defined.contains(n) || tls_names.contains(n.as_slice()) {
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
        .args(&inputs)
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
        let room = bsyms.next_start(old).map_or(u64::MAX, |n| n - old);
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
        "{}/{} units, {redirected} functions redirected, {new_fns} new, {bound} statics bound, {tls_fixed} tls, {dyn_imports} imports, ms:{phases}",
        changed.len(),
        la.cgus.len()
    ))
}

/// One thread-local access in a changed object, to be bound to the base
/// executable's TLS block. Offsets are into the object file.
#[derive(Debug, Clone, PartialEq)]
struct TlsFix {
    /// The `Elf64_Rela` entry (its type becomes `R_X86_64_NONE`).
    rela: usize,
    /// The bytes the relocation applies to.
    site: usize,
    ty: u32,
    addend: i64,
    group: Vec<u8>,
    name: Vec<u8>,
}

/// Thread-local relocations of an object. Local-exec (`TPOFF32/64`) and
/// initial-exec (`GOTTPOFF`) accesses are bound statically: in an
/// executable a thread-local's offset from the thread pointer is a link-time
/// constant, so the patch can use the base's. General/local-dynamic models
/// (never emitted for an executable's own code) are refused.
fn tls_fixups(obj: &ElfFile64<object::Endianness>) -> Res<Vec<TlsFix>> {
    let endian = obj.endian();
    let data = obj.data();
    let sections = obj.elf_section_table();
    let mut out = Vec::new();
    let mut groups: Option<Vec<Vec<u8>>> = None;
    for sh in sections.iter() {
        if sh.sh_type(endian) != elf::SHT_RELA {
            continue;
        }
        let relas = sh
            .data_as_array::<elf::Rela64<object::Endianness>, _>(endian, data)
            .map_err(|e| e.to_string())?;
        let target = sections
            .section(object::SectionIndex(sh.sh_info(endian) as usize))
            .map_err(|e| e.to_string())?;
        let symtab = sections
            .symbol_table_by_index(
                endian,
                data,
                object::SectionIndex(sh.sh_link(endian) as usize),
            )
            .map_err(|e| e.to_string())?;
        for (k, r) in relas.iter().enumerate() {
            let ty = r.r_type(endian, false);
            match ty {
                elf::R_X86_64_TPOFF32 | elf::R_X86_64_TPOFF64 | elf::R_X86_64_GOTTPOFF => {}
                elf::R_X86_64_TLSGD
                | elf::R_X86_64_TLSLD
                | elf::R_X86_64_DTPOFF32
                | elf::R_X86_64_DTPOFF64
                | elf::R_X86_64_GOTPC32_TLSDESC
                | elf::R_X86_64_TLSDESC_CALL => {
                    return Err(format!("dynamic thread-local access (type {ty})"));
                }
                _ => continue,
            }
            let si = r.r_sym(endian, false) as usize;
            let sym = symtab
                .symbol(object::SymbolIndex(si))
                .map_err(|e| e.to_string())?;
            let name = symtab
                .symbol_name(endian, sym)
                .map_err(|e| e.to_string())?
                .to_vec();
            // The CGU group of a local: the last FILE symbol before it.
            let g = groups.get_or_insert_with(|| {
                let mut cur = Vec::new();
                symtab
                    .iter()
                    .map(|s| {
                        if s.st_type() == elf::STT_FILE {
                            cur = symtab.symbol_name(endian, s).unwrap_or(b"").to_vec();
                        }
                        cur.clone()
                    })
                    .collect()
            });
            let group = if sym.st_bind() == elf::STB_LOCAL {
                g.get(si).cloned().unwrap_or_default()
            } else {
                Vec::new()
            };
            out.push(TlsFix {
                rela: sh.sh_offset(endian) as usize
                    + k * std::mem::size_of::<elf::Rela64<object::Endianness>>(),
                site: target.sh_offset(endian) as usize + r.r_offset(endian) as usize,
                ty,
                addend: r.r_addend(endian),
                group,
                name,
            });
        }
    }
    Ok(out)
}

/// Thread-pointer offset of a base thread-local by (CGU group, name).
type TpOff<'a> = dyn Fn(&[u8], &[u8]) -> Option<i64> + 'a;

/// Apply TLS fixups to a copy of an object: write the thread-pointer
/// offset (`tpoff(group, name)`) into the code and neutralize the
/// relocation. `GOTTPOFF` loads are relaxed to immediates the way linkers
/// relax initial-exec to local-exec (`mov x@gottpoff(%rip),%r` ->
/// `mov $off,%r`, `add x@gottpoff(%rip),%r` -> `add $off,%r`).
fn apply_tls(obj: &mut [u8], fixes: &[TlsFix], tpoff: &TpOff) -> Res<()> {
    for f in fixes {
        let off = tpoff(&f.group, &f.name).ok_or_else(|| {
            format!(
                "thread-local {} is not in the base binary",
                String::from_utf8_lossy(&f.name)
            )
        })?;
        let site = f.site;
        match f.ty {
            elf::R_X86_64_TPOFF32 => {
                let v = i32::try_from(off + f.addend).map_err(|_| "TLS offset out of range")?;
                obj.get_mut(site..site + 4)
                    .ok_or("TLS site out of range")?
                    .copy_from_slice(&v.to_le_bytes());
            }
            elf::R_X86_64_TPOFF64 => {
                obj.get_mut(site..site + 8)
                    .ok_or("TLS site out of range")?
                    .copy_from_slice(&(off + f.addend).to_le_bytes());
            }
            elf::R_X86_64_GOTTPOFF => {
                let ins = obj
                    .get_mut(site.checked_sub(3).ok_or("TLS site")?..site + 4)
                    .ok_or("TLS site out of range")?;
                let (rex, op, modrm) = (ins[0], ins[1], ins[2]);
                if rex & 0xf0 != 0x40 || modrm & 0xc7 != 0x05 {
                    return Err("unexpected initial-exec TLS instruction".into());
                }
                let reg = (modrm >> 3) & 7;
                ins[0] = (rex & !0x04) | ((rex & 0x04) >> 2);
                ins[1] = match op {
                    0x8b => 0xc7,
                    0x03 => 0x81,
                    _ => return Err(format!("unexpected initial-exec TLS opcode {op:#x}")),
                };
                ins[2] = 0xc0 | reg;
                let v = i32::try_from(off).map_err(|_| "TLS offset out of range")?;
                ins[3..7].copy_from_slice(&v.to_le_bytes());
            }
            _ => return Err(format!("unexpected TLS relocation {}", f.ty)),
        }
        // r_info: symbol in the high 32 bits, type in the low 32.
        let info = obj
            .get_mut(f.rela + 8..f.rela + 16)
            .ok_or("TLS rela out of range")?;
        let sym = u64::from_le_bytes(info.try_into().unwrap()) >> 32;
        info.copy_from_slice(&((sym << 32) | elf::R_X86_64_NONE as u64).to_le_bytes());
    }
    Ok(())
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

    #[cfg(target_os = "linux")]
    #[test]
    fn detached_index_child_drops_inherited_flock_and_pipe() {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        const CHILD: &str = "JUSTRUST_HOTPATCH_FD_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // A raw fork in the shared test harness would briefly duplicate
            // other tests' flocks before isolation runs, racing their
            // nonblocking eviction probes. Exercise it in a fresh process.
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "hotpatch::tests::detached_index_child_drops_inherited_flock_and_pipe",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "isolated descriptor regression failed:\n{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let path = std::env::temp_dir().join(format!("jrhot-fd-lock-{}", std::process::id()));
        let lock = std::fs::File::create(&path).unwrap();
        // SAFETY: flock takes an open fd, and pipe initializes both fds.
        let (read, write) = unsafe {
            assert_eq!(libc::flock(lock.as_raw_fd(), libc::LOCK_EX), 0);
            let mut pipe = [-1; 2];
            assert_eq!(libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC), 0);
            (OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1]))
        };
        // SAFETY: the child uses only async-signal-safe calls and never
        // returns into the test harness or runs inherited destructors.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            unsafe {
                libc::alarm(10);
                if !isolate_index_child_fds() {
                    libc::_exit(77);
                }
                loop {
                    libc::pause();
                }
            }
        }
        struct Child(Option<libc::pid_t>);
        impl Drop for Child {
            fn drop(&mut self) {
                // SAFETY: this is our fork child, reaped even on panic.
                if let Some(pid) = self.0 {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                        libc::waitpid(pid, std::ptr::null_mut(), 0);
                    }
                }
            }
        }
        let mut child = Child(Some(pid));
        drop(write);
        let mut poll = libc::pollfd {
            fd: read.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // EOF must arrive while the child is alive, not only at its exit.
        assert_eq!(unsafe { libc::poll(&mut poll, 1, 2000) }, 1);
        assert_ne!(poll.revents & libc::POLLHUP, 0);
        let probe = std::fs::File::open(&path).unwrap();
        assert_eq!(
            unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            -1,
            "isolating the child must not release the parent's lock"
        );
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(lock);
        assert_eq!(
            unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "detached index child retained a build/eviction flock"
        );
        let mut status = 0;
        let exited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if exited == pid {
            child.0 = None;
        }
        assert!(
            exited == 0
                || (exited == pid && libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 77),
            "child should be alive after isolation, or skip on unsupported close_range"
        );
        std::fs::remove_file(path).unwrap();
    }

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

    fn rela(sym: u64, ty: u32) -> Vec<u8> {
        let mut v = vec![0u8; 24];
        v[8..16].copy_from_slice(&((sym << 32) | ty as u64).to_le_bytes());
        v
    }

    #[test]
    fn rewrites_thread_local_accesses_to_base_offsets() {
        // mov x@gottpoff(%rip),%rax ; lea x@tpoff(%rax),%rdi ; then 2 relas.
        let mut obj = vec![0x48, 0x8b, 0x05, 0, 0, 0, 0, 0x48, 0x8d, 0xb8, 0, 0, 0, 0];
        let r0 = obj.len();
        obj.extend(rela(7, elf::R_X86_64_GOTTPOFF));
        obj.extend(rela(9, elf::R_X86_64_TPOFF32));
        let fix = |rela, site, ty, addend| TlsFix {
            rela,
            site,
            ty,
            addend,
            group: b"g".to_vec(),
            name: b"x".to_vec(),
        };
        let fixes = [
            fix(r0, 3, elf::R_X86_64_GOTTPOFF, -4),
            fix(r0 + 24, 10, elf::R_X86_64_TPOFF32, 0),
        ];
        apply_tls(&mut obj, &fixes, &|g, n| {
            (g == b"g" && n == b"x").then_some(-0xa70)
        })
        .unwrap();
        // Relaxed to `mov $-0xa70,%rax` (48 c7 c0 imm32), like ld's IE->LE.
        assert_eq!(&obj[..7], &[0x48, 0xc7, 0xc0, 0x90, 0xf5, 0xff, 0xff]);
        assert_eq!(&obj[10..14], &(-0xa70i32).to_le_bytes());
        // Both relocations are now R_X86_64_NONE, symbols kept.
        assert_eq!(&obj[r0 + 8..r0 + 16], &(7u64 << 32).to_le_bytes());
        assert_eq!(&obj[r0 + 32..r0 + 40], &(9u64 << 32).to_le_bytes());
        // %r9 destination: REX.R moves to REX.B.
        let mut o2 = vec![0x4c, 0x8b, 0x0d, 0, 0, 0, 0];
        o2.extend(rela(1, elf::R_X86_64_GOTTPOFF));
        apply_tls(
            &mut o2,
            &[fix(7, 3, elf::R_X86_64_GOTTPOFF, -4)],
            &|_, _| Some(-8),
        )
        .unwrap();
        assert_eq!(&o2[..3], &[0x49, 0xc7, 0xc1]);
        // A thread-local the base does not have: fall back to a relink.
        assert!(
            apply_tls(
                &mut o2.clone(),
                &[fix(7, 3, elf::R_X86_64_TPOFF32, 0)],
                &|_, _| None
            )
            .is_err()
        );
    }

    // Parses this test binary as ELF.
    #[cfg(target_os = "linux")]
    #[test]
    fn base_index_resolves_like_the_symbol_table() {
        // This test binary is a real executable with a full symbol table.
        let exe = std::env::current_exe().unwrap();
        let mm = mmap(&exe).unwrap();
        let file = ElfFile64::<object::Endianness>::parse(&*mm).unwrap();
        let syms = Syms::read(&file, None);
        let dir = std::env::temp_dir().join(format!("jrhot-idx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (im, built) = index_for(&exe, &mm, &dir).unwrap();
        assert!(built);
        let idx = Index::new(&im, &mm).unwrap();
        let mut checked = 0;
        let mut group: &[u8] = b"";
        for sym in file.symbols() {
            let name = sym.name_bytes().unwrap_or(b"");
            if sym.kind() == SymbolKind::File {
                group = name;
                continue;
            }
            if name.is_empty() || sym.is_undefined() || sym.kind() == SymbolKind::Tls {
                continue;
            }
            assert_eq!(
                idx.find(group, name),
                syms.find(group, name),
                "{}",
                String::from_utf8_lossy(name)
            );
            checked += 1;
            if sym.kind() == SymbolKind::Text && sym.address() != 0 {
                let i = syms.starts.partition_point(|&a| a <= sym.address());
                assert_eq!(idx.next_start(sym.address()), syms.starts.get(i).copied());
            }
        }
        assert!(checked > 1000);
        // A second patch maps the saved index instead of rebuilding it.
        drop(im);
        assert!(!index_for(&exe, &mm, &dir).unwrap().1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_hooks_do_nothing_without_the_link_marker() {
        assert!(std::env::var_os(LINK_ENV).is_none());
        assert_eq!(before_link(&os(&["-o", "/nonexistent/x"])), None);
        after_link(&os(&["-o", "/nonexistent/x"]));
    }
}
