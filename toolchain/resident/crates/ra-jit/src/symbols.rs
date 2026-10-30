//! Maps Rust paths to the mangled names rustc gave them, read from rlibs and objects.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};

use anyhow::Context;
use object::{Object, ObjectSection as _, ObjectSymbol, SymbolKind, read::archive::ArchiveFile};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::dep_info::DepInfo;

#[derive(Default)]
pub(crate) struct SymbolTable {
    /// More than one symbol for the items that print as one path: of two `const _` blocks of
    /// a module, or of one name in one function. Linking one to another's symbol is a
    /// miscompile.
    by_path: FxHashMap<String, Vec<Placed>>,
    /// Symbols of items nested in closures and anonymous constants, by their path without the
    /// `{closure#0}`-like segments, with the numbers of the closures among those.
    nested: FxHashMap<String, Vec<NestedSymbol>>,
    /// rustc's `StableCrateId` of each crate named in a symbol. `None` when two crates share
    /// the name, two versions of one package say.
    crates: FxHashMap<String, Option<u64>>,
    /// The `StableCrateId` of the crate whose source starts at a file, for the crates that
    /// share a name. Their paths keep the `name[id]` rustc mangled into the symbol.
    by_root: FxHashMap<PathBuf, u64>,
    /// The crates of the link, each by its name and the file it starts at.
    linked: FxHashSet<(String, PathBuf)>,
    /// Every symbol defined or referred to, hashed: two that hash alike are not expected.
    names: FxHashSet<u64>,
    /// The paths of the functions defined under a symbol private to its object, hashed.
    private: FxHashSet<u64>,
    /// The files rustc built that cargo left a dep-info file for, each with its sources.
    built_from: Vec<(PathBuf, Vec<Source>)>,
    /// When the files started to be read.
    read: Option<SystemTime>,
    /// Which table read from files this is, counting from 1: 0 for one that is not.
    generation: u64,
    /// The copies crates made of one instance, by the symbol without the crate that made it.
    copies: FxHashMap<String, Vec<String>>,
    /// The crates that made a copy of an instance of a function, by the function's name
    /// alone: what there is to go by for an instance whose types cannot be spelled.
    makers: FxHashMap<String, FxHashSet<u64>>,
    /// Each file a crate starts at as [`SymbolTable::real`] names it.
    real: Mutex<FxHashMap<PathBuf, PathBuf>>,
    /// rustc's copies of statics, by their symbols, that a copy of ours can only share with
    /// a check or not at all ([`StaticCopy`]). One a symbol names whole is not here.
    static_copies: FxHashMap<String, StaticCopy>,
    /// How many copies of statics under names LLVM gave them or private to their objects
    /// were read, shared or not.
    copies_of_statics: usize,
}

/// What rustc's copy of a static is to code of ours that refers to the static, when LLVM gave
/// it a name of its own or kept it to its object ([`hir_ty_codegen::RustcStaticCopy`]): rustc
/// internalises a static only the code of one codegen unit uses, and LLVM then renames it as it
/// optimizes (`SERVING` to `SERVING.0` when its global optimizer splits a struct into its
/// fields, `.llvm.<hash>` when ThinLTO lets another unit use it). rustc's code reads and writes
/// that copy, so a static with state has to be that one in ours too, or the program's state
/// splits in two (`docs/live.md#statics`).
pub(crate) use hir_ty_codegen::RustcStaticCopy as StaticCopy;

struct NestedSymbol {
    name: String,
    closures: Vec<u32>,
    place: Option<u64>,
    around: Vec<u64>,
}

struct Placed {
    name: String,
    /// The place of the item among those of its name in what it is written in, which rustc
    /// mangles into the symbol and no path says. `None` for a symbol we can't read it from.
    place: Option<u64>,
    /// The places of the items it is written in, the outermost first.
    around: Vec<u64>,
}

/// A linkable symbol: its mangled name and its path with and without crate ids.
struct Symbol {
    /// Whether only the object that defines it can link to it.
    private: bool,
    name: String,
    place: Option<u64>,
    around: Vec<u64>,
    with_ids: String,
    path: String,
}

/// A file of the source of a crate rustc built.
#[derive(Debug)]
struct Source {
    path: PathBuf,
    /// What a file that a build script wrote came to when the symbols were read. cargo runs
    /// a script again without building the crate again when the script says the same, so
    /// when such a file was written says nothing.
    generated: Option<u64>,
}

impl Source {
    fn new(path: PathBuf) -> Source {
        let of_script = crate::cargo::script::in_out_dir(&path);
        let generated = of_script.then(|| content(&path)).flatten();
        Source { path, generated }
    }

    /// Whether the file is another than the one rustc read when it finished at `built`.
    fn written_since(&self, built: SystemTime) -> bool {
        match self.generated {
            Some(generated) => content(&self.path) != Some(generated),
            None => written(&self.path).is_some_and(|it| it > built),
        }
    }
}

fn written(file: &Path) -> Option<SystemTime> {
    std::fs::metadata(file).and_then(|it| it.modified()).ok()
}

fn content(file: &Path) -> Option<u64> {
    let mut hasher = DefaultHasher::new();
    std::fs::read(file).ok()?.hash(&mut hasher);
    Some(hasher.finish())
}

/// A symbol of an object, and how it demangles when it is one of Rust's.
struct Named {
    name: String,
    /// Whether only the object that defines it can link to it.
    private: bool,
    /// With the ids of the crates it names, and without them.
    demangled: Option<(String, String)>,
    /// The symbol rustc gave a static that LLVM renamed, which `name` is rustc's copy of:
    /// what the path and the places are read from.
    base: Option<String>,
    /// What that copy is to ours, when not simply the whole static.
    copy: Option<StaticCopy>,
}

impl Named {
    fn new(name: &str, private: bool) -> Named {
        Named { name: name.to_owned(), private, demangled: demangled(name), base: None, copy: None }
    }

    /// rustc's copy `name` of the static rustc named `base`.
    fn static_copy(name: &str, base: &str, copy: Option<StaticCopy>) -> Named {
        Named {
            name: name.to_owned(),
            private: false,
            demangled: demangled(base),
            base: Some(base.to_owned()),
            copy,
        }
    }
}

/// How `name` demangles, with the ids of the crates it names and without them.
fn demangled(name: &str) -> Option<(String, String)> {
    // The memory a static points to is a symbol that demangles to the static's path.
    (!name.starts_with("_RNn"))
        .then(|| rustc_demangle::try_demangle(name).ok())
        .flatten()
        // `{:#}` drops the crate disambiguator hashes.
        .map(|it| (it.to_string(), format!("{it:#}")))
}

/// What was read of a library before the table was asked for: with its length and when it
/// was written, which a library written again since has others of.
type ReadAhead = FxHashMap<PathBuf, ((u64, SystemTime), Vec<Named>)>;

static READ_AHEAD: std::sync::LazyLock<Mutex<ReadAhead>> = std::sync::LazyLock::new(Mutex::default);

/// Reads the symbols of a library cargo just built, while it builds the rest of what links
/// with it, on a quarter of the cores: the others are cargo's.
#[allow(dead_code)] // `ra-jit-wasm` builds this file too, and reads nothing ahead.
pub(crate) fn read_ahead(file: PathBuf) {
    static READERS: std::sync::LazyLock<Mutex<std::sync::mpsc::Sender<PathBuf>>> =
        std::sync::LazyLock::new(|| {
            let (send, receive) = std::sync::mpsc::channel::<PathBuf>();
            let receive = Arc::new(Mutex::new(receive));
            let cores = std::thread::available_parallelism().map_or(1, |it| it.get());
            for _ in 0..cores.div_ceil(4) {
                let receive = receive.clone();
                std::thread::spawn(move || {
                    loop {
                        let file = receive.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        let Ok(file) = file else { return };
                        let Some(stamp) = stamp_of(&file) else { continue };
                        if let Ok(names) = names_of_file(&file) {
                            lock(&READ_AHEAD).insert(file, (stamp, names));
                        }
                    }
                });
            }
            Mutex::new(send)
        });
    let _ = lock(&READERS).send(file);
}

fn lock<T>(it: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    it.lock().unwrap_or_else(|e| e.into_inner())
}

fn stamp_of(file: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(file).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The symbols of `file` as they were read ahead, unless it was written again since.
fn read_before(file: &Path) -> Option<Vec<Named>> {
    let (stamp, names) = lock(&READ_AHEAD).remove(file)?;
    let names = (stamp_of(file)? == stamp).then_some(names)?;
    READ_BEFORE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Some(names)
}

/// How many files the last table took as they were read ahead.
static READ_BEFORE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The symbols of each of `files`, which are objects or archives of them, read on every core.
fn names_of(files: &[PathBuf]) -> Vec<anyhow::Result<Vec<Named>>> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let read = || {
        let mut read = Vec::new();
        loop {
            let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(file) = files.get(at) else { return read };
            read.push((at, read_before(file).map_or_else(|| names_of_file(file), Ok)));
        }
    };
    let threads = std::thread::available_parallelism().map_or(1, |it| it.get()).min(files.len());
    let mut read: Vec<_> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..threads).map(|_| scope.spawn(read)).collect();
        threads.into_iter().flat_map(|it| it.join().expect("reading symbols panicked")).collect()
    });
    read.sort_by_key(|it| it.0);
    read.into_iter().map(|it| it.1).collect()
}

fn names_of_file(file: &Path) -> anyhow::Result<Vec<Named>> {
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let mut names = Vec::new();
    // The objects of a program's crates are linked through copies that make their statics'
    // symbols linkable (`weaken::copy_of` on Apple's systems, `weaken::shared` on ELF ones);
    // the toolchain's prebuilt library is linked as it is.
    let copied = !file.components().any(|it| it.as_os_str() == "rustlib");
    match ArchiveFile::parse(&*data) {
        Ok(archive) => {
            for member in archive.members() {
                names_of_object(member?.data(&*data)?, &mut names, copied);
            }
        }
        Err(_) => names_of_object(&data, &mut names, copied),
    }
    Ok(names)
}

/// The symbols of an object. `copied`: whether it is linked through a copy of
/// [`crate::weaken::definitions`] (on Apple's linker) or [`crate::weaken::shared`] (on an ELF
/// one), which make the symbols of its statics that are private to it linkable.
fn names_of_object(data: &[u8], names: &mut Vec<Named>, copied: bool) {
    // rlibs also hold `lib.rmeta`, which isn't an object file.
    let Ok(object) = object::File::parse(data) else { return };
    let mut copies: FxHashMap<&str, Vec<CopyOfStatic<'_>>> = FxHashMap::default();
    for symbol in object.symbols() {
        if let Some(copy) = CopyOfStatic::of(&object, &symbol) {
            copies.entry(copy.base).or_default().push(copy);
            continue;
        }
        // A reference is as good as a definition: something in the link defines it.
        // Not `is_definition`, which leaves out thread locals.
        // A wasm object's data is in segments, which are not sections.
        let in_segment = object.format() == object::BinaryFormat::Wasm
            && symbol.section() == object::SymbolSection::Unknown;
        let defined = symbol.section_index().is_some() || symbol.is_common() || in_segment;
        let linkable = symbol.is_undefined() || (defined && symbol.is_global());
        let private = defined && symbol.is_local() && symbol.kind() == SymbolKind::Text;
        if !linkable && !private {
            continue;
        }
        let Ok(name) = symbol.name() else { continue };
        // `<name>.llvm.<hash>` is a function LLVM made local and promoted for ThinLTO: its
        // signature may no longer be the Rust ABI (dead arguments removed), so it is no copy
        // to call.
        if name.contains(".llvm.") {
            continue;
        }
        // Mach-O spells every name of C and Rust with an underscore in front, which the
        // writer of an object adds back.
        let name = match object.format() {
            object::BinaryFormat::MachO => name.strip_prefix('_').unwrap_or(name),
            _ => name,
        };
        names.push(Named::new(name, private));
    }
    static_copies(&object, copies, copied, names);
}

/// A symbol of an object that may be rustc's copy of a static under a name LLVM gave it, or
/// one private to the object: Rust's symbol of data a program writes to, with something after
/// the symbol rustc gave it (`.0`, `.llvm.<hash>`), or local.
struct CopyOfStatic<'data> {
    /// Without Mach-O's `_`.
    name: &'data str,
    /// The symbol rustc gave the static.
    base: &'data str,
    local: bool,
    address: u64,
    size: u64,
}

impl<'data> CopyOfStatic<'data> {
    fn of(
        object: &object::File<'data>,
        symbol: &object::Symbol<'data, '_>,
    ) -> Option<CopyOfStatic<'data>> {
        if !matches!(symbol.kind(), SymbolKind::Data | SymbolKind::Tls) {
            return None;
        }
        let section = object.section_by_index(symbol.section_index()?).ok()?;
        if !written_at_run_time(&section) {
            return None;
        }
        let name = symbol.name().ok()?;
        let name = match object.format() {
            object::BinaryFormat::MachO => name.strip_prefix('_')?,
            _ => name,
        };
        let base = &name[..rust_symbol_length(name)?];
        let local = symbol.is_local();
        (local || base.len() != name.len()).then_some(CopyOfStatic {
            name,
            base,
            local,
            address: symbol.address(),
            size: symbol.size(),
        })
    }
}

/// Whether a section holds data a program writes to: a static with state is in one.
pub(crate) fn written_at_run_time(section: &object::Section<'_, '_>) -> bool {
    match section.flags() {
        // `__thread_vars` holds the descriptor code refers to a thread-local by.
        object::SectionFlags::MachO { .. } => {
            matches!(section.name(), Ok("__data" | "__bss" | "__common" | "__thread_vars"))
        }
        object::SectionFlags::Elf { sh_flags } => {
            sh_flags & u64::from(object::elf::SHF_WRITE) != 0
                && !section.name().is_ok_and(|it| it.starts_with(".data.rel.ro"))
        }
        _ => false,
    }
}

/// How long the symbol rustc made is at the start of `name`, a symbol LLVM may have put a
/// suffix after (`.0`, `.llvm.<hash>`): `None` for no symbol of Rust's, or the memory a
/// static points to (`_RNn`), or a thread-local's first value (`$tlv$init`).
pub(crate) fn rust_symbol_length(name: &str) -> Option<usize> {
    if name.contains('$') && !name.starts_with("_ZN") || name.starts_with("_RNn") {
        return None;
    }
    if name.starts_with("_R") {
        // v0 writes only letters, digits and `_`.
        return Some(name.find('.').unwrap_or(name.len()));
    }
    // The legacy mangling: `_ZN`, names each after its length, `E`. A name may have `.` in
    // it (`core..fmt..Debug`), and `$` escapes.
    let mut rest = name.strip_prefix("_ZN")?.as_bytes();
    let mut at = 3;
    loop {
        if rest.first() == Some(&b'E') {
            return Some(at + 1);
        }
        let digits = rest.iter().take_while(|it| it.is_ascii_digit()).count();
        let length: usize = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
        rest = rest.get(digits + length..)?;
        at += digits + length;
    }
}

/// What each copy of a static in `copies` is to ours, added to `names` under its own symbol.
/// rustc gives a static one symbol, and LLVM gives what it makes of it names after that
/// one: `<symbol>.llvm.<hash>` for the static under another name, `<symbol>.N` (and that
/// with `.llvm.<hash>`) for the parts its global optimizer split it into, `<symbol>.b` for one
/// it shrank to a `bool`. Only the static whole can be shared: a part that is the whole is
/// told by the object's symbol (ELF) or its debug information (Mach-O).
fn static_copies(
    object: &object::File<'_>,
    copies: FxHashMap<&str, Vec<CopyOfStatic<'_>>>,
    copied: bool,
    names: &mut Vec<Named>,
) {
    let macho = object.format() == object::BinaryFormat::MachO;
    // The copies the daemon links make a symbol private to its object linkable: Apple's
    // (`weaken::definitions`) and ELF's (`weaken::shared`).
    let shares_private = macho || object.format() == object::BinaryFormat::Elf;
    let mut decided = Vec::new();
    let mut parts = FxHashMap::default();
    for (base, mut found) in copies {
        found.sort_by_key(|it| it.name);
        let copy = &found[0];
        let suffix = &copy.name[base.len()..];
        let renamed = suffix.is_empty() || is_llvm_hash(suffix);
        let part = suffix
            .strip_prefix('.')
            .map(|it| it.split_once(".llvm.").map_or(it, |(part, _)| part))
            .is_some_and(|it| !it.is_empty() && it.bytes().all(|it| it.is_ascii_digit()));
        let why = if found.len() > 1 {
            Some(format!("is split by LLVM into {} parts", found.len()))
        } else if copy.local && !(copied && shares_private) {
            Some("is private to its object".to_owned())
        } else if !renamed && !part {
            Some(format!("was made something else by LLVM (`{suffix}`)"))
        } else {
            None
        };
        let copy_of = match why {
            Some(why) => Some(StaticCopy::Unshared(why)),
            None if renamed => None,
            None if macho => {
                parts.insert(base, copy.address);
                None
            }
            None => Some(StaticCopy::Sized(copy.size)),
        };
        decided.push((copy.name, base, copy_of));
    }
    let whole =
        if parts.is_empty() { FxHashSet::default() } else { whole_in_dwarf(object, &parts) };
    for (name, base, mut copy) in decided {
        if parts.contains_key(base) && !whole.contains(base) {
            let why = "is a part LLVM split off, which the debug information does not say is whole";
            copy = Some(StaticCopy::Unshared(why.to_owned()));
        }
        names.push(Named::static_copy(name, base, copy));
    }
}

/// Whether `suffix` is the `.llvm.<hash>` ThinLTO gives a symbol it promotes.
fn is_llvm_hash(suffix: &str) -> bool {
    suffix
        .strip_prefix(".llvm.")
        .is_some_and(|it| !it.is_empty() && it.bytes().all(|it| it.is_ascii_digit()))
}

/// Of the statics `parts` names by rustc's symbol, with the address of a part LLVM split off,
/// those the object's debug information says are at that address whole: a variable of that
/// linkage name whose place is that address alone, with no `DW_OP_piece` of a part.
fn whole_in_dwarf(object: &object::File<'_>, parts: &FxHashMap<&str, u64>) -> FxHashSet<String> {
    let mut whole = FxHashSet::default();
    let endian = match object.is_little_endian() {
        true => gimli::RunTimeEndian::Little,
        false => gimli::RunTimeEndian::Big,
    };
    let load = |id: gimli::SectionId| -> Result<_, gimli::Error> {
        let data = object.section_by_name(id.name()).and_then(|it| it.data().ok());
        Ok(gimli::EndianSlice::new(data.unwrap_or_default(), endian))
    };
    let Ok(dwarf) = gimli::Dwarf::load(load) else { return whole };
    let mut units = dwarf.units();
    while let Ok(Some(header)) = units.next() {
        let Ok(unit) = dwarf.unit(header) else { continue };
        let mut entries = unit.entries();
        while let Ok(Some(entry)) = entries.next_dfs() {
            if entry.tag() != gimli::DW_TAG_variable {
                continue;
            }
            let linkage = entry
                .attr_value(gimli::DW_AT_linkage_name)
                .or_else(|| entry.attr_value(gimli::DW_AT_MIPS_linkage_name));
            let Some(name) = linkage.and_then(|it| dwarf.attr_string(&unit, it).ok()) else {
                continue;
            };
            let Ok(name) = name.to_string() else { continue };
            let Some(&address) = parts.get(name) else { continue };
            let Some(gimli::AttributeValue::Exprloc(place)) =
                entry.attr_value(gimli::DW_AT_location)
            else {
                continue;
            };
            let mut operations = place.operations(unit.encoding());
            let at = match operations.next() {
                Ok(Some(gimli::Operation::Address { address })) => Some(address),
                Ok(Some(gimli::Operation::AddressIndex { index })) => {
                    dwarf.address(&unit, index).ok()
                }
                _ => None,
            };
            if at == Some(address) && matches!(operations.next(), Ok(None)) {
                whole.insert(name.to_owned());
            }
        }
    }
    whole
}

pub(crate) fn hashed(symbol: &str) -> u64 {
    use std::hash::BuildHasher;
    rustc_hash::FxBuildHasher.hash_one(symbol)
}

fn hashed_bytes(bytes: &[u8]) -> u64 {
    use std::hash::BuildHasher;
    rustc_hash::FxBuildHasher.hash_one(bytes)
}

/// Every name a symbol of `texts` (a symbol a line) may have in its path, [`hashed_bytes`]:
/// as many bytes as each decimal number in it says, after it, and after a `_` after it too.
/// Both manglings write a name after its length, and v0 puts a `_` between them when the name
/// starts with one or with a digit. A number is read from each of its digits, as one name
/// can end in digits right before the length of the next. Some of what is found is no name
/// (the digits of a hash, say), but no name is missed.
fn names_in_symbols(texts: &[&str]) -> FxHashSet<u64> {
    let chunks: Vec<&str> = texts.iter().flat_map(|text| chunks_of_lines(text, 1 << 20)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |it| it.get()).min(chunks.len());
    let found: Vec<FxHashSet<u64>> = std::thread::scope(|scope| {
        let work = || {
            let mut found = FxHashSet::default();
            loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(chunk) = chunks.get(at) else { break found };
                for line in chunk.lines() {
                    names_in_symbol(line.as_bytes(), &mut found);
                }
            }
        };
        let threads: Vec<_> = (0..threads).map(|_| scope.spawn(work)).collect();
        threads.into_iter().map(|it| it.join().expect("indexing names panicked")).collect()
    });
    let mut found = found.into_iter();
    let mut all = found.next().unwrap_or_default();
    found.for_each(|it| all.extend(it));
    all
}

fn names_in_symbol(symbol: &[u8], found: &mut FxHashSet<u64>) {
    for start in 0..symbol.len() {
        let (mut end, mut length) = (start, 0usize);
        while let Some(digit) = symbol.get(end).filter(|it| it.is_ascii_digit()) {
            length = length.saturating_mul(10).saturating_add(usize::from(digit - b'0'));
            end += 1;
        }
        if end == start || length == 0 {
            continue;
        }
        for from in [end, end + 1] {
            if let Some(name) = symbol.get(from..from.saturating_add(length)) {
                found.insert(hashed_bytes(name));
            }
            if symbol.get(end) != Some(&b'_') {
                break;
            }
        }
    }
}

/// `text` cut after a line near every `size` bytes.
fn chunks_of_lines(text: &str, size: usize) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let cut = match rest.get(size..).and_then(|it| it.find('\n')) {
            Some(at) => size + at + 1,
            None => rest.len(),
        };
        let (chunk, after) = rest.split_at(cut);
        rest = after;
        Some(chunk)
    })
}

/// The symbols of an executable rustc linked, which a patch is spliced into.
pub(crate) struct Linked {
    /// What it defines or takes from a library, [`hashed`].
    names: FxHashSet<u64>,
    /// Its functions and statics of symbols private to the object they were in, by their
    /// paths as [`SymbolTable`] spells them, with their addresses: none for a path that two
    /// have, or a symbol that two copies have, which a patch would replace one of.
    private: FxHashMap<String, Option<(String, u64)>>,
    /// The symbol of each function rustc compiled from Rust (or C++), a line each:
    /// demangled when asked, for a few.
    functions: String,
    /// The symbols of the functions left out of the others, being private to an object
    /// whose crate calls them its own way, a line each.
    unkept: String,
    /// The names in the symbols of both, as [`names_in_symbols`] has them.
    named: FxHashSet<u64>,
    /// Whether it has thread-local data of the objects of `by_symbol`.
    own_thread_locals: bool,
}

impl Linked {
    /// `by_symbol` are the objects and archives of the crates rustc compiled with a symbol for
    /// each function, which every call goes to (`cargo::plan`'s `calls_by_symbol`), and
    /// `shared` the names that more than one crate of the link has, as
    /// [`SymbolTable::shared_names`] has them.
    pub(crate) fn read(
        path: &Path,
        by_symbol: &[PathBuf],
        shared: &FxHashSet<String>,
    ) -> anyhow::Result<Linked> {
        let start = std::time::Instant::now();
        let file =
            std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
        // Only its symbols are read of it, which are a little of a program with debug info.
        // SAFETY: a linker writes a new file in place of the one it links over, not into it,
        // and nothing else writes what rustc linked.
        let data = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*data)?;
        let hidden = hidden_globals(&data);
        let own = defined_in(by_symbol);
        let mut linked = Linked {
            names: FxHashSet::default(),
            private: FxHashMap::default(),
            functions: String::new(),
            unkept: String::new(),
            named: FxHashSet::default(),
            own_thread_locals: false,
        };
        let mut local = Vec::new();
        for symbol in object.symbols().chain(object.dynamic_symbols()) {
            let Ok(name) = symbol.name() else { continue };
            let name = match object.format() {
                object::BinaryFormat::MachO => name.strip_prefix('_').unwrap_or(name),
                _ => name,
            };
            // One the linker made local that was no global of its object, hidden or not, had
            // internal linkage, and an optimized crate calls it its own way: LLVM passes a
            // reference argument by value, drops one nothing reads, shrinks a static to a
            // `bool`. A patch calls none of those, or what it prints is wrong (#52).
            let internal = symbol.is_local()
                && symbol.section_index().is_some()
                && !hidden.contains(&symbol.index().0)
                && !matches!(symbol.flags(), object::SymbolFlags::Elf { st_other, .. }
                    if st_other & 3 == object::elf::STV_HIDDEN);
            let function = (name.starts_with("_R") || name.starts_with("_ZN"))
                && symbol.kind() == SymbolKind::Text
                && symbol.section_index().is_some();
            linked.own_thread_locals |= symbol.kind() == SymbolKind::Tls
                && symbol.section_index().is_some()
                && own.contains(&hashed(name));
            if internal && !own.contains(&hashed(name)) {
                if function {
                    linked.unkept.push_str(name);
                    linked.unkept.push('\n');
                }
                continue;
            }
            linked.names.insert(hashed(name));
            if function {
                linked.functions.push_str(name);
                linked.functions.push('\n');
            }
            let private = symbol.is_local()
                && matches!(symbol.kind(), SymbolKind::Text | SymbolKind::Data)
                && symbol.section_index().is_some();
            if !private || !name.starts_with("_R") {
                continue;
            }
            let Some((with_ids, path)) = Named::new(name, true).demangled else { continue };
            local.push((with_ids, path, name.to_owned(), symbol.address()));
        }
        linked.private = by_path(local, shared);
        let read = start.elapsed();
        linked.named = names_in_symbols(&[&linked.functions, &linked.unkept]);
        if std::env::var_os("RA_JIT_STATS").is_some() {
            let (read, named) = (read.as_micros(), (start.elapsed() - read).as_micros());
            eprintln!("rustc-symbols-us: read={read} named={named}");
        }
        Ok(linked)
    }

    pub(crate) fn has(&self, symbol: &str) -> bool {
        self.names.contains(&hashed(symbol))
    }

    /// Whether the crates rustc compiled with a symbol for each function have thread-locals
    /// of their own here. A build in full has its own copies of those, and an executable has
    /// all the thread-local data it ever will: nothing of the build can be spliced into it.
    pub(crate) fn has_own_thread_locals(&self) -> bool {
        self.own_thread_locals
    }

    /// The one private symbol of the function or static at `path`.
    pub(crate) fn private(&self, path: &str) -> Option<&str> {
        self.private.get(path)?.as_ref().map(|it| &*it.0)
    }

    /// Whether no function in the executable has `name` anywhere in its symbol: it then has
    /// no copy of a function of that name, nor of a closure or instance of one.
    pub(crate) fn has_no_function_named(&self, name: &str) -> bool {
        !name.is_empty() && name.is_ascii() && !self.named.contains(&hashed_bytes(name.as_bytes()))
    }

    /// The copies of the instances of the generic function at `path`, each by its path, as
    /// `core::mem::swap::<u8>`, with its symbol. `None` when one is of more than an instance
    /// (a closure in it, say), or two have one path.
    pub(crate) fn instances_of(&self, path: &str) -> Option<Vec<(String, String)>> {
        let name = path.rsplit("::").next().filter(|it| !it.is_empty() && it.is_ascii())?;
        // How the v0 mangling writes a name, which the symbol of each copy has whole.
        let separator =
            if name.starts_with(|it: char| it.is_ascii_digit() || it == '_') { "_" } else { "" };
        let written = format!("{}{separator}{name}", name.len());
        let prefix = format!("{path}::<");
        let mut found: FxHashMap<String, String> = FxHashMap::default();
        for symbol in self.functions.lines().filter(|it| it.contains(&written)) {
            let Some((_, demangled)) = Named::new(symbol, true).demangled else { continue };
            // The legacy mangling leaves out the types of an instance.
            if symbol.starts_with("_ZN")
                && (demangled == path || demangled.starts_with(&format!("{path}::")))
            {
                return None;
            }
            let Some(args) = demangled.strip_prefix(&prefix) else { continue };
            let after = args_closed(args).map(|at| &args[at..])?;
            // A closure of the instance is compiled with it.
            if !after.is_empty() {
                if closures_only(after) {
                    continue;
                }
                return None;
            }
            if found.get(&demangled).is_some_and(|it| it != symbol) {
                return None;
            }
            found.insert(demangled, symbol.to_owned());
        }
        Some(found.into_iter().collect())
    }
}

/// Where the arguments of an instance end, as a demangled symbol has them after the `<` that
/// opens them.
fn args_closed(args: &str) -> Option<usize> {
    let mut depth = 1;
    let mut last = ' ';
    for (at, it) in args.char_indices() {
        match it {
            '<' => depth += 1,
            // The `->` of a function type.
            '>' if last == '-' => {}
            '>' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return Some(at + 1);
        }
        last = it;
    }
    None
}

/// Whether `path` is `::{closure#N}` once or more, a closure in the body of what it follows.
fn closures_only(mut path: &str) -> bool {
    while let Some(rest) = path.strip_prefix("::{closure#") {
        let Some((number, rest)) = rest.split_once('}') else { return false };
        if number.is_empty() || !number.bytes().all(|it| it.is_ascii_digit()) {
            return false;
        }
        path = rest;
    }
    path.is_empty()
}

/// The indices of the symbols of a Mach-O executable that were private externals, hidden
/// globals of their objects the linker made local, which `object` reports as plain locals.
fn hidden_globals(data: &[u8]) -> FxHashSet<usize> {
    use object::macho::{N_EXT, N_PEXT};
    use object::read::macho::{MachOFile64, Nlist};
    let Ok(file) = MachOFile64::<object::Endianness>::parse(data) else {
        return FxHashSet::default();
    };
    let table = file.macho_symbol_table();
    (0..table.len())
        .filter(|&index| {
            table
                .symbol(object::SymbolIndex(index))
                .is_ok_and(|it| it.n_type() & (N_EXT | N_PEXT) == N_PEXT)
        })
        .collect()
}

/// What the objects in `files`, and in the archives among them, define, [`hashed`] as
/// [`Linked`] names it.
fn defined_in(files: &[PathBuf]) -> FxHashSet<u64> {
    let mut defined = FxHashSet::default();
    let mut add = |data: &[u8]| {
        let Ok(object) = object::File::parse(data) else { return };
        for symbol in object.symbols().filter(|it| it.section_index().is_some()) {
            let Ok(name) = symbol.name() else { continue };
            let name = match object.format() {
                object::BinaryFormat::MachO => name.strip_prefix('_').unwrap_or(name),
                _ => name,
            };
            defined.insert(hashed(name));
        }
    };
    for file in files {
        let Ok(data) = std::fs::read(file) else { continue };
        match ArchiveFile::parse(&*data) {
            Ok(archive) => {
                for member in archive.members().flatten() {
                    if let Ok(member) = member.data(&*data) {
                        add(member);
                    }
                }
            }
            Err(_) => add(&data),
        }
    }
    defined
}

impl SymbolTable {
    /// `root` is what the paths in cargo's dep-info files are relative to.
    pub(crate) fn load(files: &[PathBuf], root: &Path) -> anyhow::Result<SymbolTable> {
        static READ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let generation = READ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut table =
            SymbolTable { read: Some(SystemTime::now()), generation, ..SymbolTable::default() };
        let mut symbols = Vec::new();
        let mut owners = Vec::new();
        let start = std::time::Instant::now();
        READ_BEFORE.store(0, std::sync::atomic::Ordering::Relaxed);
        let read = names_of(files);
        // What was read ahead of files that are not of the link is not asked for again.
        lock(&READ_AHEAD).clear();
        let read_in = start.elapsed();
        for (file, names) in files.iter().zip(read) {
            let first = symbols.len();
            for name in names? {
                table.add_name(name, &mut symbols);
            }
            if let Some((name, sources)) = crate_sources(file, root) {
                owners.push((name, table.real(&sources[0]), first..symbols.len()));
                let sources = sources.into_iter().map(Source::new).collect();
                table.built_from.push((file.clone(), sources));
            }
        }
        // A crate's symbols mention the crates of its name that it depends on (a binary
        // instantiates the generics of its library) besides itself, and never one that depends
        // on it: the id that is left once the others have theirs is its own. A crate may be
        // several files, when it is linked from its objects.
        let mut unplaced: FxHashMap<PathBuf, FxHashMap<u64, usize>> = FxHashMap::default();
        for (name, source, range) in owners {
            table.linked.insert((name.clone(), source.clone()));
            if table.crates.get(&name) != Some(&None) {
                continue;
            }
            let mentions = unplaced.entry(source).or_default();
            for symbol in &symbols[range] {
                for (_, id) in crate_ids(&symbol.with_ids).filter(|it| it.0 == name) {
                    *mentions.entry(id).or_default() += 1;
                }
            }
        }
        let added_in = start.elapsed() - read_in;
        table.place(unplaced);
        table.index(symbols);
        if std::env::var_os("RA_JIT_STATS").is_some() {
            let indexed_in = start.elapsed() - read_in - added_in;
            let ahead = READ_BEFORE.load(std::sync::atomic::Ordering::Relaxed);
            eprintln!(
                "symbols: read {read_in:.1?} ({ahead} of {} files ahead), added {added_in:.1?}, \
                 indexed {indexed_in:.1?}",
                files.len()
            );
            let unshared = table.static_copies.values();
            let unshared = unshared.filter(|it| matches!(it, StaticCopy::Unshared(_))).count();
            eprintln!(
                "symbols: {} copies of statics renamed or private, {} not to be shared",
                table.copies_of_statics, unshared
            );
        }
        Ok(table)
    }

    /// Tells tables apart for what is kept from one build to the next: two that were read
    /// from files at different times differ, and 0 is the table of no files.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// The table of the symbols named, for a link that is not read from files.
    #[allow(dead_code)] // `ra-jit-wasm` builds this file too, and has no files to read.
    pub(crate) fn of_names<'a>(names: impl IntoIterator<Item = &'a str>) -> SymbolTable {
        let mut table = SymbolTable::default();
        let mut symbols = Vec::new();
        for name in names {
            table.add_name(Named::new(name, false), &mut symbols);
        }
        table.index(symbols);
        table
    }

    /// Files the symbols under their paths, once every crate named in one is known.
    fn index(&mut self, symbols: Vec<Symbol>) {
        let table = self;
        for Symbol { private, name, place, around, with_ids, path } in symbols {
            let shared = |krate: &str| table.crates.get(krate) == Some(&None);
            let path = if crate_ids(&with_ids).any(|(krate, _)| shared(krate)) {
                without_ids(&with_ids, |krate| !shared(krate))
            } else {
                path
            };
            if private {
                table.private.insert(Self::hashed(&path));
                continue;
            }
            // A closure is not an item nested in itself.
            let unnamed = path.rsplit("::").next().is_some_and(|last| last.starts_with('{'));
            if path.contains("::{") && !unnamed {
                let named: Vec<_> = path.split("::").filter(|it| !it.starts_with('{')).collect();
                let closures = path
                    .split("::")
                    .filter_map(|it| it.strip_prefix("{closure#")?.strip_suffix('}')?.parse().ok())
                    .collect();
                let symbols = table.nested.entry(named.join("::")).or_default();
                if symbols.iter().all(|it| it.name != name) {
                    let around = around.clone();
                    symbols.push(NestedSymbol { name: name.clone(), closures, place, around });
                }
            }
            let known = table.by_path.entry(path).or_default();
            let same = |it: &Placed| {
                without_instantiating_crate(&it.name) == without_instantiating_crate(&name)
            };
            if !known.iter().any(same) {
                known.push(Placed { name, place, around });
            }
        }
    }

    fn hashed(symbol: &str) -> u64 {
        hashed(symbol)
    }

    /// What rustc's copy of a static under `symbol` is to a copy of ours, when that is not
    /// simply the whole static ([`StaticCopy`]).
    pub(crate) fn static_copy(&self, symbol: &str) -> Option<&StaticCopy> {
        self.static_copies.get(symbol)
    }

    /// Whether the function at `path` is defined under a symbol private to its object.
    pub(crate) fn private(&self, path: &str) -> bool {
        self.private.contains(&Self::hashed(path))
    }

    pub(crate) fn defines(&self, symbol: &str) -> bool {
        self.names.contains(&Self::hashed(symbol))
    }

    fn add_name(&mut self, named: Named, symbols: &mut Vec<Symbol>) {
        let Named { name, private, demangled, base, copy } = named;
        self.copies_of_statics += usize::from(base.is_some());
        if let Some(copy) = copy {
            self.static_copies.insert(name.clone(), copy);
        }
        // A copy of a static LLVM renamed has the places of the symbol rustc gave it.
        let parsed = base.as_deref().unwrap_or(&name);
        let instance = without_instantiating_crate(&name);
        if !private && self.names.insert(Self::hashed(&name)) && instance.len() != name.len() {
            self.copies.entry(instance.to_owned()).or_default().push(name.clone());
        }
        let Some((with_ids, path)) = demangled else { return };
        for (krate, id) in crate_ids(&with_ids) {
            match self.crates.get_mut(krate) {
                Some(known) if *known != Some(id) => *known = None,
                Some(_) => {}
                None => {
                    self.crates.insert(krate.to_owned(), Some(id));
                }
            }
        }
        if instance.len() != name.len()
            && let Some(maker) = V0::instantiating_crate(&name)
            && let Some(function) = function_name(&path)
        {
            self.makers.entry(function).or_default().insert(maker);
        }
        let place = V0::place(parsed);
        let around = V0::places_around(parsed).unwrap_or_default();
        symbols.push(Symbol { private, name, place, around, with_ids, path });
    }

    /// How paths spell the crate named `name` whose source starts at `root`: `name[id]` when
    /// another crate has the name too.
    pub(crate) fn crate_path(&self, name: &str, root: &Path) -> Option<String> {
        match self.crates.get(name) {
            Some(None) => Some(format!("{name}[{:x}]", self.by_root.get(&self.real(root))?)),
            _ => Some(name.to_owned()),
        }
    }

    /// Gives each crate that shares its name the id that is its own, from how often its symbols
    /// mention each id of that name.
    fn place(&mut self, mut unplaced: FxHashMap<PathBuf, FxHashMap<u64, usize>>) {
        loop {
            let taken: FxHashSet<u64> = self.by_root.values().copied().collect();
            let before = unplaced.len();
            unplaced.retain(|source, mentions| {
                let mut free = mentions.keys().filter(|id| !taken.contains(id));
                match (free.next(), free.next()) {
                    (Some(&id), None) => {
                        self.by_root.insert(source.clone(), id);
                        false
                    }
                    _ => true,
                }
            });
            if unplaced.len() == before {
                break;
            }
        }
        // The other crate of the name is none of these files (a test named `test`, and the
        // sysroot's): a guess, that a crate mentions itself more than it does the other.
        for (source, mentions) in unplaced {
            if let Some((&id, _)) = mentions.iter().max_by_key(|(_, count)| **count) {
                self.by_root.insert(source, id);
            }
        }
    }

    /// Whether the crate named `name` whose source starts at `root` is one of the link. A
    /// bench may be named as the library, start where a test does, and be another program.
    /// Whether the table has no symbol of rustc's: the build has nothing rustc built.
    pub(crate) fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub(crate) fn links(&self, name: &str, root: &Path) -> bool {
        self.linked.contains(&(name.to_owned(), self.real(root)))
    }

    /// The file at `path` as the filesystem names it, through no link. Cargo names a file of a
    /// crate outside the workspace by the path it was given (`--manifest-path /tmp/..`), the
    /// daemon its checkout by where it is (`/private/tmp/..` on macOS), and either names the
    /// one crate.
    fn real(&self, path: &Path) -> PathBuf {
        let mut known = lock(&self.real);
        let real = || std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        known.entry(path.to_owned()).or_insert_with(real).clone()
    }

    /// A source that was written after rustc built the crate it is of: what rustc built is
    /// then not what it would build now.
    pub(crate) fn written_since_built(&self) -> Option<&Path> {
        self.all_written_since_built().next()
    }

    /// Every file rustc read of the crates it built, as their dep-info files list them.
    pub(crate) fn sources(&self) -> impl Iterator<Item = &Path> {
        self.built_from.iter().flat_map(|(_, sources)| sources.iter().map(|it| &*it.path))
    }

    /// Every source that was written after rustc built the crate it is of.
    pub(crate) fn all_written_since_built(&self) -> impl Iterator<Item = &Path> {
        let finished = self.finished();
        self.built_from.iter().flat_map(move |(_, sources)| {
            let built = sources.first().and_then(|root| finished.get(&root.path).copied());
            sources
                .iter()
                .filter(move |it| built.is_some_and(|built| it.written_since(built)))
                .map(|it| &*it.path)
        })
    }

    /// When rustc finished each crate, by the crate's root file: the newest of its files. An
    /// incremental build keeps the objects of the codegen units it did not compile again as
    /// they were, with their old times, so one object of a crate built after an edit can be
    /// older than the edit.
    fn finished(&self) -> FxHashMap<PathBuf, SystemTime> {
        let mut finished: FxHashMap<PathBuf, SystemTime> = FxHashMap::default();
        for (built, sources) in &self.built_from {
            let (Some(root), Some(at)) = (sources.first(), written(built)) else { continue };
            let newest = finished.entry(root.path.clone()).or_insert(at);
            *newest = (*newest).max(at);
        }
        finished
    }

    /// Whether rustc has built one of the files again since they were read.
    pub(crate) fn built_since_read(&self) -> bool {
        self.read.is_some_and(|read| {
            self.built_from.iter().any(|(built, _)| written(built).is_some_and(|it| it > read))
        })
    }

    /// The names that more than one crate of the link has.
    pub(crate) fn shared_names(&self) -> FxHashSet<String> {
        self.crates.iter().filter(|it| it.1.is_none()).map(|it| it.0.clone()).collect()
    }

    pub(crate) fn stable_crate_id(&self, name: &str, root: &Path) -> Option<u64> {
        match *self.crates.get(name)? {
            Some(id) => Some(id),
            None => self.by_root.get(&self.real(root)).copied(),
        }
    }

    /// The symbol of the one item at `path`. A symbol that is alone at its path and is not
    /// the first of its name there is of another item than the one the path was read from:
    /// rustc wrote that item itself, as it does the `main` of a test executable, and built
    /// nothing of the one in the source.
    pub(crate) fn get(&self, path: &str) -> Option<&str> {
        match &**self.by_path.get(path)? {
            [only] if only.place.is_none_or(|it| it == 0) => Some(&only.name),
            _ => None,
        }
    }

    /// The symbol of the item at `path` that is the `place`th of its name in what it is
    /// written in. `around` are the places of the innermost items it is written in, as far
    /// out as they tell it apart: a static in the value of the second static of a name.
    pub(crate) fn get_at(&self, path: &str, place: u64, around: &[u64]) -> Option<&str> {
        let mut symbols = match &**self.by_path.get(path)? {
            [only] => {
                return Some(only)
                    .filter(|it| it.place.is_none_or(|it| it == place))
                    .map(|it| &*it.name);
            }
            symbols => {
                symbols.iter().filter(|it| it.place == Some(place) && it.around.ends_with(around))
            }
        };
        let symbol = symbols.next()?;
        symbols.next().is_none().then_some(&*symbol.name)
    }

    /// The symbol of an item whose path has unnamed segments between the named ones given.
    ///
    /// `closures` are the numbers rustc gives the closures around the item, outermost first,
    /// which tell apart the items that only differ in those, and `place` and `around` are as
    /// in `get_at`. Anonymous constants are not numbered, so two items that differ in one of
    /// those have no symbol here.
    pub(crate) fn get_nested(
        &self,
        named: &str,
        closures: &[u32],
        place: u64,
        around: &[u64],
    ) -> Option<&str> {
        let here = |it: &&NestedSymbol| it.place.is_none_or(|known| known == place);
        let mut symbols = match &**self.nested.get(named)? {
            // Its closures may be numbered otherwise than here, but are as many.
            [only] => {
                return Some(only)
                    .filter(here)
                    .filter(|it| it.closures.len() == closures.len())
                    .map(|it| &*it.name);
            }
            symbols => symbols
                .iter()
                .filter(here)
                .filter(|it| it.closures == closures && it.around.ends_with(around)),
        };
        let symbol = symbols.next()?;
        symbols.next().is_none().then_some(&*symbol.name)
    }

    /// The copies of the instance `symbol` is one of, each with rustc's `StableCrateId` of
    /// the crate that compiled it. None for a symbol that is of one crate only.
    pub(crate) fn copies(&self, symbol: &str) -> impl Iterator<Item = (&str, u64)> {
        let copies = self.copies.get(without_instantiating_crate(symbol));
        copies.into_iter().flatten().filter_map(|it| Some((&**it, V0::instantiating_crate(it)?)))
    }

    /// rustc's `StableCrateId` of each crate that made a copy of an instance of a function
    /// named `function`, in any module and of any type.
    pub(crate) fn makers(&self, function: &str) -> impl Iterator<Item = u64> {
        self.makers.get(function).into_iter().flatten().copied()
    }

    pub(crate) fn len(&self) -> usize {
        self.by_path.len()
    }
}

/// The name of the function a demangled symbol is an instance of: the last of its path that
/// is not types or a closure.
fn function_name(path: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut bare = String::new();
    for c in path.replace("->", "").chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.checked_sub(1)?,
            _ if depth == 0 => bare.push(c),
            _ => {}
        }
    }
    let name = bare.rsplit("::").find(|it| !it.is_empty() && !it.starts_with('{'))?;
    Some(name.to_owned())
}

/// The name of the crate in `libname-hash.rlib`, or in the `name-hash.<unit>.rcgu.o` of a crate
/// built as a program, and the files of its source, which the dep-info file cargo leaves
/// next to it lists, the one the crate starts at first.
fn crate_sources(rlib: &Path, root: &Path) -> Option<(String, Vec<PathBuf>)> {
    let file = rlib.file_name()?.to_str()?;
    let stem = match file.strip_suffix(".rlib") {
        Some(stem) => stem.strip_prefix("lib")?,
        None => file.strip_suffix(".rcgu.o")?.split_once('.')?.0,
    };
    // A crate's name has no `-`. cargo gives the program of an MSVC or Apple target no hash:
    // the file of its symbols is named after it.
    let name = stem.rsplit_once('-').map_or(stem, |(name, _)| name);
    let text = std::fs::read_to_string(rlib.with_file_name(format!("{stem}.d"))).ok()?;
    let sources: Vec<PathBuf> =
        DepInfo::parse(&text).files.into_iter().map(|it| root.join(it)).collect();
    (!sources.is_empty()).then(|| (name.to_owned(), sources))
}

/// A v0 symbol without the crate that instantiated it, which ends the symbol of a generic
/// instance as `Cs<id>_<len><name>` or as a back reference `B<offset>_`. Copies of one instance
/// made by two crates differ only in that.
fn without_instantiating_crate(symbol: &str) -> &str {
    let is_base62 = |c: char| c.is_ascii_alphanumeric();
    if let Some(body) = symbol.strip_suffix('_')
        && let Some(start) = body.rfind('B')
        && body[start + 1..].chars().all(is_base62)
    {
        return &symbol[..start];
    }
    // `Cs<id>_<len><name>`: the name is the last `len` bytes.
    let Some(crate_start) = symbol.rfind("Cs") else { return symbol };
    let Some((id, ident)) = symbol[crate_start + 2..].split_once('_') else { return symbol };
    let digits = ident.chars().take_while(char::is_ascii_digit).count();
    let named = ident[..digits].parse() == Ok(ident.len() - digits);
    if id.chars().all(is_base62) && named { &symbol[..crate_start] } else { symbol }
}

/// The private symbols of an executable, each as demangled with the ids of its crates and
/// without, its name and its address, by their paths: with `name[id]` for a crate of a name
/// in `shared`, as [`SymbolTable`] spells those, so that the item of one is not taken for
/// that of the other. A path that two symbols have names none.
fn by_path(
    private: Vec<(String, String, String, u64)>,
    shared: &FxHashSet<String>,
) -> FxHashMap<String, Option<(String, u64)>> {
    let shared = |krate: &str| shared.contains(krate);
    let mut found: FxHashMap<String, Option<(String, u64)>> = FxHashMap::default();
    for (with_ids, path, name, address) in &private {
        let path = if crate_ids(with_ids).any(|(krate, _)| shared(krate)) {
            without_ids(with_ids, |krate| !shared(krate))
        } else {
            path.clone()
        };
        found
            .entry(path)
            .and_modify(|it| {
                if it.as_ref().is_none_or(|it| it.0 != *name || it.1 != *address) {
                    *it = None;
                }
            })
            .or_insert_with(|| Some((name.clone(), *address)));
    }
    found
}

/// The `name[id]` a demangled v0 symbol prints for each crate in it.
fn crate_ids(demangled: &str) -> impl Iterator<Item = (&str, u64)> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut rest = demangled;
    std::iter::from_fn(move || {
        loop {
            let (before, after) = rest.split_once('[')?;
            rest = after;
            let Some((id, _)) = after.split_once(']') else { continue };
            let name_len = before.chars().rev().take_while(|&c| is_ident(c)).count();
            let name = &before[before.len() - name_len..];
            let Ok(id) = u64::from_str_radix(id, 16) else { continue };
            if !name.is_empty() {
                return Some((name, id));
            }
        }
    })
}

/// The demangled symbol without the ids of the crates `drop` picks.
fn without_ids(demangled: &str, drop: impl Fn(&str) -> bool) -> String {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut path = String::with_capacity(demangled.len());
    let mut rest = demangled;
    while let Some(open) = rest.find('[') {
        let (before, after) = rest.split_at(open);
        path.push_str(before);
        let name_len = before.chars().rev().take_while(|&c| is_ident(c)).count();
        let name = &before[before.len() - name_len..];
        match after.find(']') {
            Some(close)
                if !name.is_empty() && u64::from_str_radix(&after[1..close], 16).is_ok() =>
            {
                if !drop(name) {
                    path.push_str(&after[..=close]);
                }
                rest = &after[close + 1..];
            }
            _ => {
                path.push('[');
                rest = &after[1..];
            }
        }
    }
    path.push_str(rest);
    path
}

/// A reader of v0 symbols, for what `rustc_demangle` reads and does not print.
struct V0<'a> {
    rest: &'a [u8],
}

impl V0<'_> {
    /// The disambiguator of the last name in the path of a symbol: `0` for the first item of
    /// that name in what it is written in, `1` for the next.
    fn place(symbol: &str) -> Option<u64> {
        V0 { rest: symbol.strip_prefix("_R")?.as_bytes() }.path()
    }

    /// The `StableCrateId` of the crate that compiled a copy of an instance, which ends the
    /// symbol: see [`without_instantiating_crate`].
    fn instantiating_crate(symbol: &str) -> Option<u64> {
        let at = without_instantiating_crate(symbol).len();
        let mut reader = V0 { rest: symbol.as_bytes().get(at..)? };
        if reader.eat(b'B') {
            let to = usize::try_from(reader.base62()?).ok()?;
            reader = V0 { rest: symbol.strip_prefix("_R")?.as_bytes().get(to..)? };
        }
        reader.eat(b'C').then(|| reader.disambiguator())?
    }

    /// The disambiguators of the names in the path of a symbol before the last, outermost
    /// first: those of what an item is written in. A closure has no name.
    fn places_around(symbol: &str) -> Option<Vec<u64>> {
        let mut symbol = V0 { rest: symbol.strip_prefix("_R")?.as_bytes() };
        let mut namespaces = Vec::new();
        while symbol.eat(b'N') {
            namespaces.push(symbol.next()?);
        }
        symbol.path()?;
        let mut places = Vec::new();
        for namespace in namespaces.into_iter().rev() {
            let place = symbol.disambiguator()?;
            symbol.identifier()?;
            if namespace.is_ascii_lowercase() {
                places.push(place);
            }
        }
        places.pop();
        Some(places)
    }

    fn next(&mut self) -> Option<u8> {
        let (&first, rest) = self.rest.split_first()?;
        self.rest = rest;
        Some(first)
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.rest.first() == Some(&byte);
        if found {
            self.rest = &self.rest[1..];
        }
        found
    }

    fn base62(&mut self) -> Option<u64> {
        if self.eat(b'_') {
            return Some(0);
        }
        let mut value = 0u64;
        loop {
            let digit = match self.next()? {
                b'_' => return value.checked_add(1),
                byte @ b'0'..=b'9' => byte - b'0',
                byte @ b'a'..=b'z' => byte - b'a' + 10,
                byte @ b'A'..=b'Z' => byte - b'A' + 36,
                _ => return None,
            };
            value = value.checked_mul(62)?.checked_add(digit.into())?;
        }
    }

    fn disambiguator(&mut self) -> Option<u64> {
        if self.eat(b's') { self.base62()?.checked_add(1) } else { Some(0) }
    }

    fn identifier(&mut self) -> Option<()> {
        self.eat(b'u');
        // A length of zero is one `0`: the digits after it start the next name.
        let digits = match self.rest.first()? {
            b'0' => 1,
            _ => self.rest.iter().take_while(|it| it.is_ascii_digit()).count(),
        };
        let len: usize = str::from_utf8(&self.rest[..digits]).ok()?.parse().ok()?;
        self.rest = &self.rest[digits..];
        self.eat(b'_');
        self.rest = self.rest.get(len..)?;
        Some(())
    }

    /// Reads a path, for the disambiguator of the name it ends in.
    fn path(&mut self) -> Option<u64> {
        match self.next()? {
            b'C' => {
                self.disambiguator()?;
                self.identifier()?;
                Some(0)
            }
            b'M' => {
                self.disambiguator()?;
                self.path()?;
                self.ty()?;
                Some(0)
            }
            b'X' => {
                self.disambiguator()?;
                self.path()?;
                self.ty()?;
                self.path()?;
                Some(0)
            }
            b'Y' => {
                self.ty()?;
                self.path()?;
                Some(0)
            }
            b'N' => {
                self.next()?;
                self.path()?;
                let place = self.disambiguator()?;
                self.identifier()?;
                Some(place)
            }
            b'I' => {
                let place = self.path()?;
                while !self.eat(b'E') {
                    match self.rest.first()? {
                        b'L' => {
                            self.next();
                            self.base62()?;
                        }
                        b'K' => {
                            self.next();
                            self.constant()?;
                        }
                        _ => self.ty()?,
                    }
                }
                Some(place)
            }
            b'B' => {
                self.base62()?;
                Some(0)
            }
            _ => None,
        }
    }

    fn binder(&mut self) -> Option<()> {
        if self.eat(b'G') {
            self.base62()?;
        }
        Some(())
    }

    fn ty(&mut self) -> Option<()> {
        match *self.rest.first()? {
            b'C' | b'M' | b'X' | b'Y' | b'N' | b'I' | b'B' => return self.path().map(drop),
            _ => {}
        }
        match self.next()? {
            b'A' => {
                self.ty()?;
                self.constant()?;
            }
            b'S' | b'P' | b'O' => self.ty()?,
            b'T' => {
                while !self.eat(b'E') {
                    self.ty()?;
                }
            }
            b'R' | b'Q' => {
                if self.eat(b'L') {
                    self.base62()?;
                }
                self.ty()?;
            }
            b'F' => {
                self.binder()?;
                self.eat(b'U');
                if self.eat(b'K') && !self.eat(b'C') {
                    self.identifier()?;
                }
                while !self.eat(b'E') {
                    self.ty()?;
                }
                self.ty()?;
            }
            b'D' => {
                self.binder()?;
                while !self.eat(b'E') {
                    self.path()?;
                    while self.eat(b'p') {
                        self.identifier()?;
                        self.ty()?;
                    }
                }
                if !self.eat(b'L') {
                    return None;
                }
                self.base62()?;
            }
            // A basic type is one lowercase letter.
            byte if byte.is_ascii_lowercase() => {}
            _ => return None,
        }
        Some(())
    }

    // FIXME: the constants of `adt_const_params` are not read.
    fn constant(&mut self) -> Option<()> {
        if self.eat(b'p') {
            return Some(());
        }
        if self.eat(b'B') {
            return self.base62().map(drop);
        }
        self.ty()?;
        while self.next()? != b'_' {}
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustc_hash::{FxHashMap, FxHashSet};

    use object::{
        Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope,
        write::{Object, Symbol, SymbolSection},
    };

    use super::{
        Linked, SymbolTable, V0, by_path, crate_sources, function_name, names_in_symbols,
        names_of_object, without_ids,
    };

    #[test]
    fn a_patch_calls_no_function_an_optimized_object_kept_to_itself() {
        let dir = std::env::temp_dir().join(format!("ra-jit-linked-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |file: &str, symbols: &[(&str, SymbolScope, u8)]| {
            let mut object =
                Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
            let text = object.section_id(object::write::StandardSection::Text);
            for &(name, scope, visibility) in symbols {
                let value = object.append_section_data(text, &[0xc3], 1);
                let bind = if scope == SymbolScope::Compilation {
                    object::elf::STB_LOCAL
                } else {
                    object::elf::STB_GLOBAL
                };
                object.add_symbol(Symbol {
                    name: name.as_bytes().to_vec(),
                    value,
                    size: 1,
                    kind: SymbolKind::Text,
                    scope,
                    weak: false,
                    section: SymbolSection::Section(text),
                    flags: SymbolFlags::Elf {
                        st_info: bind << 4 | object::elf::STT_FUNC,
                        st_other: visibility,
                    },
                });
            }
            let path = dir.join(file);
            std::fs::write(&path, object.write().unwrap()).unwrap();
            path
        };
        let (local, default, hidden) =
            (SymbolScope::Compilation, object::elf::STV_DEFAULT, object::elf::STV_HIDDEN);
        // The program's crate calls by symbol; `dep` was optimized and kept `helper` to itself.
        let own = write("app.o", &[("_RNvCs1234_3app6helper", local, default)]);
        let linked = write(
            "linked",
            &[
                ("exported", SymbolScope::Dynamic, default),
                ("made_local", local, hidden),
                ("_RNvCs1234_3app6helper", local, default),
                ("_RNvCs5678_3dep6helper", local, default),
            ],
        );
        let linked = Linked::read(&linked, &[own], &Default::default()).unwrap();
        assert!(linked.has("exported"));
        assert!(linked.has("made_local"));
        assert!(linked.has("_RNvCs1234_3app6helper"));
        assert_eq!(linked.private("app::helper"), Some("_RNvCs1234_3app6helper"));
        assert!(!linked.has("_RNvCs5678_3dep6helper"));
        assert_eq!(linked.private("dep::helper"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_name_of_a_symbol_is_found() {
        let functions = [
            // v0, a name that ends in digits right before the next one's length.
            "_RNvNtCs1234_6sha2563foo",
            // Legacy, the same.
            "_ZN4core2u83add17h0123456789abcdefE",
            // v0 puts a `_` after the length of a name that starts with one.
            "_RNvCs1234_3app4__bar",
        ]
        .join("\n");
        let named = names_in_symbols(&[&functions, "_RNvCs1234_3app6unkept"]);
        let linked = Linked {
            names: FxHashSet::default(),
            private: FxHashMap::default(),
            functions,
            unkept: String::new(),
            named,
            own_thread_locals: false,
        };
        for name in ["sha256", "foo", "core", "u8", "add", "app", "_bar", "unkept"] {
            assert!(!linked.has_no_function_named(name), "{name}");
        }
        for name in ["oo", "sha", "bar", "absent"] {
            assert!(linked.has_no_function_named(name), "{name}");
        }
    }

    #[test]
    fn thin_lto_promoted_names_are_no_copies() {
        let mut object = Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
        let text = object.section_id(object::write::StandardSection::Text);
        let at = object.append_section_data(text, &[0xc3; 2], 1);
        // The same function as rustc emits it, and as ThinLTO promotes it from another unit.
        let name = "_ZN3dep4cell5World3get17h0123456789abcdefE";
        for (nth, name) in
            [name.to_owned(), format!("{name}.llvm.1234567890")].into_iter().enumerate()
        {
            object.add_symbol(Symbol {
                name: name.into_bytes(),
                value: at + nth as u64,
                size: 1,
                kind: SymbolKind::Text,
                scope: SymbolScope::Linkage,
                weak: false,
                section: SymbolSection::Section(text),
                flags: SymbolFlags::None,
            });
        }
        let mut names = Vec::new();
        names_of_object(&object.write().unwrap(), &mut names, true);
        let names: Vec<&str> = names.iter().map(|it| it.name.as_str()).collect();
        assert_eq!(names, [name]);
    }

    #[test]
    fn crate_sources_of_a_path_with_a_space() {
        let dir = std::env::temp_dir().join(format!("ra-jit symbols {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rlib = dir.join("libapp-1.rlib");
        // As the wrapper writes it beside its copy of the library, and as rustc writes it.
        let root = dir.join("my app/src/lib.rs");
        let escaped = crate::dep_info::escaped(&root.display().to_string());
        std::fs::write(dir.join("app-1.d"), format!("app-1: {escaped} my\\ app/src/a.rs\n"))
            .unwrap();
        let (name, sources) = crate_sources(&rlib, std::path::Path::new("/ws")).unwrap();
        assert_eq!(name, "app");
        assert_eq!(sources, [root, PathBuf::from("/ws/my app/src/a.rs")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn placed(crates: &[(&str, &[(u64, usize)])]) -> Vec<(String, u64)> {
        let mut table = SymbolTable::default();
        let unplaced: FxHashMap<_, _> = crates
            .iter()
            .map(|(root, mentions)| (PathBuf::from(root), mentions.iter().copied().collect()))
            .collect();
        table.place(unplaced);
        let mut placed: Vec<_> =
            table.by_root.iter().map(|(root, id)| (root.display().to_string(), *id)).collect();
        placed.sort();
        placed
    }

    #[test]
    fn the_crate_that_compiled_a_copy() {
        let of_lib = "_RINvCslFwujm09uXn_4kdep3sumhECsjPNOOTmEH7l_4klib";
        let of_app = "_RINvCslFwujm09uXn_4kdep3sumhECsjSK0TBt3L2V_4kapp";
        let by_reference = "_RNvXs_NtNtCsevLNFiNqfJP_4core3ops5arithhNtB4_3Add3addB8_";
        let table = SymbolTable::of_names([of_lib, of_app, by_reference]);
        let copies: Vec<_> = table.copies(of_app).map(|it| it.0).collect();
        assert_eq!(copies, [of_lib, of_app]);
        let ids: Vec<_> = table.copies(of_lib).map(|it| it.1).collect();
        assert_ne!(ids[0], ids[1]);
        assert_eq!(table.crates["core"], table.copies(by_reference).next().map(|it| it.1));
        assert_eq!(table.copies("_RNvCslFwujm09uXn_4kdep5small").count(), 0);
        assert_eq!(table.makers("sum").count(), 2);
    }

    #[test]
    fn private_functions_of_crates_that_share_a_name() {
        let symbol = |with_ids: &str, name: &str, address| {
            (with_ids.to_owned(), without_ids(with_ids, |_| true), name.to_owned(), address)
        };
        let shared = ["app".to_owned()].into_iter().collect();
        let found = by_path(
            vec![
                symbol("app[a1]::work", "bin_work", 1),
                symbol("app[b2]::work", "lib_work", 2),
                symbol("<app[a1]::World as dep[c3]::Greet>::name", "bin_name", 3),
                symbol("dep[c3]::value", "dep_value", 4),
                symbol("dep[c3]::twice", "one_copy", 5),
                symbol("dep[c3]::twice", "other_copy", 6),
            ],
            &shared,
        );
        let named = |path: &str| found.get(path).map(|it| it.as_ref().map(|it| &*it.0));
        assert_eq!(named("app[a1]::work"), Some(Some("bin_work")));
        assert_eq!(named("app[b2]::work"), Some(Some("lib_work")));
        assert_eq!(named("app::work"), None);
        assert_eq!(named("<app[a1]::World as dep::Greet>::name"), Some(Some("bin_name")));
        assert_eq!(named("dep::value"), Some(Some("dep_value")));
        assert_eq!(named("dep::twice"), Some(None));
    }

    #[test]
    fn a_binary_that_mentions_its_library_more_than_itself() {
        let placed = placed(&[("src/lib.rs", &[(1, 5)]), ("src/main.rs", &[(1, 9), (2, 3)])]);
        assert_eq!(placed, [("src/lib.rs".to_owned(), 1), ("src/main.rs".to_owned(), 2)]);
    }

    #[test]
    fn a_test_named_as_a_crate_of_the_sysroot() {
        let placed = placed(&[("tests/test.rs", &[(7, 6), (8, 300)])]);
        assert_eq!(placed, [("tests/test.rs".to_owned(), 8)]);
    }

    #[test]
    fn place_of_a_static_in_a_closure() {
        let in_function = "_RNvNvMNtCs7aCYEL8njSF_2h24sendNtB4_4Send5apply10___CALLSITE";
        let first = "_RNvNCNvMNtCs7aCYEL8njSF_2h24sendNtB6_4Send5apply010___CALLSITE";
        let second = "_RNvNCNvMNtCs7aCYEL8njSF_2h24sendNtB6_4Send5apply0s_10___CALLSITE";
        let third = "_RNvNCNvMNtCs7aCYEL8njSF_2h24sendNtB6_4Send5apply0s0_10___CALLSITE";
        assert_eq!(V0::place(in_function), Some(0));
        assert_eq!(V0::place(first), Some(0));
        assert_eq!(V0::place(second), Some(1));
        assert_eq!(V0::place(third), Some(2));
        let inner = "_RNvNvNCNCNvCs3XX9e4g8R7s_8filtereds_14trace_filtered00s_10___CALLSITE4META";
        assert_eq!(V0::place(inner), Some(0));
        assert_eq!(V0::places_around(inner), Some(vec![1, 1]));
        assert_eq!(V0::places_around(third), Some(vec![0]));
    }

    #[test]
    fn the_function_a_symbol_is_an_instance_of() {
        let name = |path| function_name(path).unwrap();
        assert_eq!(
            name("std::panic::catch_unwind::<&dyn core::ops::Fn<(), Output = u8>, u8>"),
            "catch_unwind"
        );
        assert_eq!(name("<core::fmt::Arguments>::new::<5, 2>"), "new");
        assert_eq!(name("<alloc::boxed::Box<fn(u8) -> u8> as core::ops::Drop>::drop"), "drop");
        assert_eq!(name("kdep::each::<u8, klib::run::{closure#0}>::{closure#1}"), "each");
    }
}
