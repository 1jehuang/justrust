//! The splice for an executable of PE, as `link.exe` and `lld-link` write it for x86-64.
//!
//! The same three additions as for ELF, each a section of its own behind the last one the
//! linker wrote, their headers in the room the linker left after its own:
//!
//! 1. Read-only: the patch's constants and unwind information, and the table of functions
//!    the unwinder searches (`.pdata`) with the patch's added.
//! 2. Code: the patch's functions, and a stub for each import they call that the linker made
//!    none for.
//! 3. Data: the patch's writable sections.
//!
//! And a fourth of the table of addresses the loader moves with the image (`.reloc`) with
//! the patch's added, which `dumpbin` reads only when it is all its section has. The two
//! tables are found through the header, which is pointed at the copies: the linker's own
//! stay where they are, unused. A PE has no table of symbols. The names come
//! from the public symbols of the PDB the linker wrote beside it, which neither says how
//! large anything is: a function is as long as its entry of `.pdata` says, and a static has
//! the room that the object it was linked from has before the next thing in its section.
//! The PDB is left as it is, and knows nothing of what is added.

use std::{
    mem::offset_of,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use object::{
    LittleEndian as LE, Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationTarget,
    SectionIndex, SymbolIndex, SymbolKind, pe,
    read::{
        coff::CoffFile,
        pe::{ImageNtHeaders, ImageOptionalHeader, PeFile64},
    },
};
use pdb::FallibleIterator;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{Addition, relative};

/// What a splice has to know of the executable, every address relative to where it is
/// loaded. Nothing of the file is read again.
pub(super) struct Image {
    pub(super) len: u64,
    /// Everything ahead of the first section.
    headers: Vec<u8>,
    /// Where `headers` has the file header, the optional header and the table of sections.
    file_header: usize,
    optional_header: usize,
    section_table: usize,
    sections: Vec<pe::ImageSectionHeader>,
    /// Where the image is loaded when the loader does not move it.
    base: u64,
    file_alignment: u64,
    section_alignment: u64,
    symbols: FxHashMap<Box<[u8]>, Named>,
    /// How many bytes each static has to itself, `None` for one that shares its place.
    rooms: FxHashMap<Box<[u8]>, Option<u64>>,
    /// `.pdata`: the functions that have unwind information, sorted by where they start.
    functions: Vec<u8>,
    /// `.reloc`: where the image stores addresses. `None` for an image that is not moved.
    fixups: Option<Vec<u64>>,
    /// The GUID of the PDB the linker wrote with it, which a program that runs it reads to
    /// know which executable it is.
    pub(super) guid: [u8; 16],
}

enum Named {
    One(Defined),
    /// The name is of several places.
    Many,
}

#[derive(Clone, Copy)]
struct Defined {
    address: u64,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Code of that many bytes, as far as is known: `None` for code that another name has
    /// too, which the linker made one of two functions that were alike.
    Code(Option<u64>),
    Data,
    ThreadLocal,
}

const SECTION_HEADER: usize = size_of::<pe::ImageSectionHeader>();
const DIRECTORY: usize = size_of::<pe::ImageDataDirectory>();
/// A `RUNTIME_FUNCTION`: where a function starts, ends, and has its unwind information.
const FUNCTION: usize = 12;

impl Image {
    /// Reads the executable at `path` that was linked from `objects`, which define what
    /// they share a name for in that order.
    pub(super) fn read(bytes: &[u8], path: &Path, objects: &[&[u8]]) -> anyhow::Result<Image> {
        let file = PeFile64::parse(bytes)?;
        let nt = file.nt_headers();
        let (header, optional) = (nt.file_header(), nt.optional_header());
        ensure!(header.machine.get(LE) == pe::IMAGE_FILE_MACHINE_AMD64, "not for x86-64");
        let directory = |index: usize| {
            let directory = file.data_directories().get(index)?;
            let (address, size) = (directory.virtual_address.get(LE), directory.size.get(LE));
            (size != 0).then_some((u64::from(address), u64::from(size)))
        };
        ensure!(directory(pe::IMAGE_DIRECTORY_ENTRY_SECURITY).is_none(), "it is signed");
        ensure!(
            directory(pe::IMAGE_DIRECTORY_ENTRY_COM_DESCRIPTOR).is_none(),
            "it is a .NET assembly"
        );
        ensure!(
            optional.dll_characteristics() & pe::IMAGE_DLLCHARACTERISTICS_GUARD_CF == 0,
            "its indirect calls are checked against a table of the functions it had"
        );

        let file_header = file.dos_header().nt_headers_offset() as usize + size_of::<u32>();
        let optional_header = file_header + size_of::<pe::ImageFileHeader>();
        let section_table = optional_header + usize::from(header.size_of_optional_header.get(LE));
        let sections: Vec<_> = file.section_table().iter().copied().collect();
        let headers = bytes
            .get(..optional.size_of_headers() as usize)
            .context("headers past the end of the file")?
            .to_vec();
        let (file_alignment, section_alignment) =
            (u64::from(optional.file_alignment()), u64::from(optional.section_alignment()));
        ensure!(
            file_alignment != 0 && section_alignment.is_multiple_of(file_alignment),
            "sections aligned to {section_alignment} in memory and {file_alignment} in the file"
        );

        let mut image = Image {
            len: bytes.len() as u64,
            headers,
            file_header,
            optional_header,
            section_table,
            sections,
            base: optional.image_base(),
            file_alignment,
            section_alignment,
            symbols: FxHashMap::default(),
            rooms: FxHashMap::default(),
            functions: Vec::new(),
            fixups: None,
            guid: [0; 16],
        };
        if let Some((address, size)) = directory(pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION) {
            image.functions = image.bytes_at(bytes, address, size)?.to_vec();
        }
        ensure!(image.functions.len().is_multiple_of(FUNCTION), "a table of functions cut short");
        let moved = header.characteristics.get(LE) & pe::IMAGE_FILE_RELOCS_STRIPPED == 0;
        if let Some((address, size)) =
            directory(pe::IMAGE_DIRECTORY_ENTRY_BASERELOC).filter(|_| moved)
        {
            image.fixups = Some(fixed_up(image.bytes_at(bytes, address, size)?)?);
        }
        ensure!(
            image.fixups.is_some()
                || optional.dll_characteristics() & pe::IMAGE_DLLCHARACTERISTICS_DYNAMIC_BASE == 0,
            "an executable that is moved and says nothing of what to fix up"
        );

        let debug = file.pdb_info()?.context("it names no PDB to read its symbols from")?;
        image.guid = debug.guid();
        image.read_symbols(path, &debug)?;
        for object in objects {
            for (name, room) in rooms(&CoffFile::parse(*object)?)? {
                image.rooms.entry(name.into()).or_insert(room);
            }
        }
        Ok(image)
    }

    /// The public symbols of the PDB, which are the ones objects refer to each other by.
    fn read_symbols(&mut self, path: &Path, debug: &object::CodeView<'_>) -> anyhow::Result<()> {
        // Where the linker wrote it, or beside the executable when that says only its name.
        let named = PathBuf::from(String::from_utf8_lossy(debug.path()).into_owned());
        let beside = path.with_file_name(named.file_name().context("a PDB without a name")?);
        let found = if beside.exists() { beside } else { named };
        let file = std::fs::File::open(&found)
            .with_context(|| format!("reading the symbols of {}", found.display()))?;
        let mut pdb = pdb::PDB::open(file)?;
        ensure!(
            pdb.pdb_information()?.guid.to_bytes_le() == debug.guid(),
            "{} is of another link",
            found.display()
        );
        let address_map = pdb.address_map()?;
        let globals = pdb.global_symbols()?;
        let mut symbols = globals.iter();
        let mut names_of: FxHashMap<u64, u32> = FxHashMap::default();
        let mut found = Vec::new();
        while let Some(symbol) = symbols.next()? {
            let Ok(pdb::SymbolData::Public(public)) = symbol.parse() else { continue };
            let Some(address) = public.offset.to_rva(&address_map) else { continue };
            let address = u64::from(address.0);
            *names_of.entry(address).or_default() += 1;
            found.push((public.name.as_bytes().into(), address));
        }

        let ends: FxHashMap<u64, u64> = (self.functions.chunks_exact(FUNCTION))
            .map(|it| (u64::from(read_u32(it)), u64::from(read_u32(&it[4..]))))
            .collect();
        for (name, address) in found {
            let Some(section) = self.section_of(address) else { continue };
            let kind = if section.characteristics.get(LE) & pe::IMAGE_SCN_MEM_EXECUTE != 0 {
                // A function without an entry is a leaf, of which nothing says where it ends.
                let len = ends.get(&address).map(|end| end - address);
                Kind::Code(len.filter(|_| names_of[&address] == 1))
            } else if section.raw_name() == b".tls" {
                Kind::ThreadLocal
            } else {
                Kind::Data
            };
            let defined = Defined { address, kind };
            self.symbols
                .entry(name)
                .and_modify(|named| match named {
                    Named::One(other) if other.address == address => {}
                    _ => *named = Named::Many,
                })
                .or_insert(Named::One(defined));
        }
        Ok(())
    }

    fn section_of(&self, address: u64) -> Option<&pe::ImageSectionHeader> {
        self.sections.iter().find(|it| {
            let start = u64::from(it.virtual_address.get(LE));
            let len = it.virtual_size.get(LE).max(it.size_of_raw_data.get(LE));
            (start..start + u64::from(len)).contains(&address)
        })
    }

    /// Where the file has what is loaded at `address`.
    fn offset_of(&self, address: u64) -> anyhow::Result<u64> {
        let section = self.section_of(address).context("an address outside every section")?;
        let within = address - u64::from(section.virtual_address.get(LE));
        ensure!(
            within < u64::from(section.size_of_raw_data.get(LE)),
            "an address the file has no bytes for"
        );
        Ok(u64::from(section.pointer_to_raw_data.get(LE)) + within)
    }

    fn bytes_at<'a>(&self, bytes: &'a [u8], address: u64, len: u64) -> anyhow::Result<&'a [u8]> {
        let offset = self.offset_of(address)? as usize;
        bytes.get(offset..offset + len as usize).context("a table past the end of the file")
    }

    /// Where the last section ends in memory.
    fn end(&self) -> u64 {
        (self.sections.iter())
            .map(|it| u64::from(it.virtual_address.get(LE)) + u64::from(it.virtual_size.get(LE)))
            .max()
            .unwrap_or(0)
    }
}

/// A patch object, and what of the executable it needs.
pub(super) struct Patch<'a> {
    file: CoffFile<'a>,
    /// The sections that are loaded, by the section of ours they go to.
    sections: [Vec<SectionIndex>; 3],
    /// What each symbol a relocation refers to stands for.
    targets: FxHashMap<SymbolIndex, Target>,
    /// The imports the code calls that the executable has no stub for, in the order of ours.
    stubs: Vec<SymbolIndex>,
    /// How many addresses the patch stores, each of which the loader has to fix up.
    stored: usize,
    /// How many functions it has unwind information for.
    functions: usize,
    /// The functions and statics it defines, with what the executable has under the name.
    replaced: Vec<Replaced<'a>>,
    /// The statics among them that go where the executable has them.
    in_place: Vec<InPlace>,
}

#[derive(Clone, Copy)]
enum Added {
    Read = 0,
    Code = 1,
    Data = 2,
    Fixups = 3,
}

impl Added {
    const ALL: [Added; 4] = [Added::Read, Added::Code, Added::Data, Added::Fixups];

    fn name(self) -> [u8; 8] {
        match self {
            Added::Read => *b".rajitr\0",
            Added::Code => *b".rajitx\0",
            Added::Data => *b".rajitw\0",
            Added::Fixups => *b".rajitf\0",
        }
    }

    fn characteristics(self) -> u32 {
        match self {
            Added::Read => pe::IMAGE_SCN_CNT_INITIALIZED_DATA | pe::IMAGE_SCN_MEM_READ,
            Added::Code => {
                pe::IMAGE_SCN_CNT_CODE | pe::IMAGE_SCN_MEM_EXECUTE | pe::IMAGE_SCN_MEM_READ
            }
            Added::Data => {
                pe::IMAGE_SCN_CNT_INITIALIZED_DATA
                    | pe::IMAGE_SCN_MEM_READ
                    | pe::IMAGE_SCN_MEM_WRITE
            }
            // Read once by the loader, as the linker's own says of itself.
            Added::Fixups => {
                pe::IMAGE_SCN_CNT_INITIALIZED_DATA
                    | pe::IMAGE_SCN_MEM_READ
                    | pe::IMAGE_SCN_MEM_DISCARDABLE
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    /// At an offset in a section of the patch.
    Ours(SectionIndex, u64),
    /// At an address of the executable.
    Theirs(u64),
    /// In another library: the loader writes its address to the slot.
    Import(u64),
}

struct Replaced<'a> {
    name: &'a [u8],
    section: SectionIndex,
    offset: u64,
    /// Where the executable has the function this is the new one of.
    jump_from: Option<u64>,
}

/// A static of the patch that is written over the executable's.
struct InPlace {
    section: SectionIndex,
    /// Where its section has it, and how many bytes of the section are its own.
    offset: u64,
    len: u64,
    /// Where the executable has the old one, and how many bytes that has.
    old: u64,
    room: u64,
}

impl InPlace {
    fn holds(&self, section: SectionIndex, offset: u64) -> bool {
        self.section == section && (self.offset..self.offset + self.len).contains(&offset)
    }
}

/// How a relocation of the patch gets its value. What it adds is in the field itself.
enum Fixup {
    /// The address itself, in 8 bytes.
    Absolute,
    /// The distance to it from that many bytes behind the field, in 4 bytes.
    Relative(u64),
    /// The address without where the image is loaded, in 4 bytes.
    InImage,
}

impl Fixup {
    fn of(relocation: &object::Relocation) -> anyhow::Result<Fixup> {
        let RelocationFlags::Coff { typ } = relocation.flags() else {
            bail!("a relocation that is not COFF's")
        };
        Ok(match typ {
            pe::IMAGE_REL_AMD64_ADDR64 => Fixup::Absolute,
            pe::IMAGE_REL_AMD64_REL32..=pe::IMAGE_REL_AMD64_REL32_5 => {
                Fixup::Relative(u64::from(typ - pe::IMAGE_REL_AMD64_REL32))
            }
            pe::IMAGE_REL_AMD64_ADDR32NB => Fixup::InImage,
            other => bail!("a relocation of type {other}, which only the linker handles"),
        })
    }
}

impl<'a> Patch<'a> {
    const JUMP: usize = 5;
    /// `jmp *slot(%rip)`.
    const STUB: usize = 6;
    /// A block of `.reloc` that has one address: its page and length, the address, padding.
    const FIXUP: usize = 12;

    pub(super) fn read(bytes: &'a [u8], image: &Image) -> anyhow::Result<Patch<'a>> {
        let mut patch = Patch {
            file: CoffFile::parse(bytes)?,
            sections: Default::default(),
            targets: FxHashMap::default(),
            stubs: Vec::new(),
            stored: 0,
            functions: 0,
            replaced: Vec::new(),
            in_place: Vec::new(),
        };
        for section in patch.file.sections() {
            let object::SectionFlags::Coff { characteristics } = section.flags() else { continue };
            let unloaded =
                pe::IMAGE_SCN_LNK_REMOVE | pe::IMAGE_SCN_LNK_INFO | pe::IMAGE_SCN_MEM_DISCARDABLE;
            if characteristics & unloaded != 0 {
                continue;
            }
            let name = merged(section.name_bytes()?);
            ensure!(name != b".tls", "the patch has thread-local data");
            ensure!(name != b".CRT", "the patch has something to run before main");
            let added = if characteristics & pe::IMAGE_SCN_MEM_EXECUTE != 0 {
                Added::Code
            } else if characteristics & pe::IMAGE_SCN_MEM_WRITE != 0 {
                Added::Data
            } else {
                Added::Read
            };
            patch.sections[added as usize].push(section.index());
            if name == b".pdata" {
                patch.functions += section.size() as usize / FUNCTION;
            }
        }
        let loaded: FxHashSet<SectionIndex> = patch.sections.iter().flatten().copied().collect();
        let code: FxHashSet<SectionIndex> =
            patch.sections[Added::Code as usize].iter().copied().collect();

        let rooms = rooms(&patch.file)?;
        for symbol in patch.file.symbols() {
            if !symbol.is_definition() || symbol.is_local() {
                continue;
            }
            let name = symbol.name_bytes()?;
            let shown = || String::from_utf8_lossy(name);
            let section = symbol.section_index().context("a symbol in no section")?;
            let jump_from = match image.symbols.get(name) {
                None => None,
                // One that says what a patch left behind is of that patch, which the executable
                // was linked with, and says nothing of this one's.
                Some(_) if super::lasting::left_behind(name).is_some() => None,
                Some(Named::One(Defined { address, kind: Kind::Code(len) }))
                    if code.contains(&section) =>
                {
                    let len = len.with_context(|| {
                        format!("{} shares its code or has no unwind information", shown())
                    })?;
                    ensure!(len >= Self::JUMP as u64, "{} is too short to jump from", shown());
                    Some(*address)
                }
                Some(Named::One(Defined { address, kind: Kind::Data }))
                    if !code.contains(&section) =>
                {
                    let new = (patch.in_place_of(name, &symbol, &rooms, *address, image))
                        .with_context(|| format!("the static {} has another value", shown()))?;
                    patch.in_place.push(new);
                    None
                }
                Some(_) => bail!("{} is not what the executable has under that name", shown()),
            };
            patch.replaced.push(Replaced { name, section, offset: symbol.address(), jump_from });
        }

        for section in patch.file.sections() {
            if !loaded.contains(&section.index()) {
                continue;
            }
            for (_, relocation) in section.relocations() {
                let RelocationTarget::Symbol(index) = relocation.target() else {
                    bail!("a relocation against no symbol")
                };
                let target = match patch.targets.get(&index) {
                    Some(target) => *target,
                    None => {
                        let target = patch.target(index, image, &loaded)?;
                        patch.targets.insert(index, target);
                        target
                    }
                };
                match (Fixup::of(&relocation)?, target) {
                    (Fixup::Relative(_), Target::Import(_)) => {
                        ensure!(
                            code.contains(&section.index()),
                            "data that stores the distance to an import"
                        );
                        if !patch.stubs.contains(&index) {
                            patch.stubs.push(index);
                        }
                    }
                    (_, Target::Import(_)) => bail!("data that stores the address of an import"),
                    (Fixup::Absolute, _) => patch.stored += 1,
                    _ => {}
                }
            }
        }
        Ok(patch)
    }

    /// A static to go where the executable has the one it replaces at `old`: what refers to
    /// it there is not looked for.
    fn in_place_of(
        &self,
        name: &[u8],
        symbol: &object::read::coff::CoffSymbol<'a, '_>,
        rooms: &FxHashMap<&[u8], Option<u64>>,
        old: u64,
        image: &Image,
    ) -> anyhow::Result<InPlace> {
        let shared = "that shares its place with something else";
        let len = rooms.get(name).copied().flatten().context(shared)?;
        let room = (image.rooms.get(name).copied())
            .context("that is of no object the executable is known to be linked from")?
            .context(shared)?;
        ensure!(len <= room, "that is larger: {len} bytes after {room}");
        let index = symbol.section_index().context("a symbol in no section")?;
        let section = self.file.section_by_index(index)?;
        // What a type is aligned to divides its size, and where its section has it.
        let align = (symbol.address() | len | section.align().max(1)).isolate_lowest_one();
        ensure!(old.is_multiple_of(align), "that may be aligned to {align}");
        let there = image.section_of(old).context("that is in no section")?;
        ensure!(
            old - u64::from(there.virtual_address.get(LE)) + len
                <= u64::from(there.size_of_raw_data.get(LE)),
            "that the file has no bytes for"
        );
        let written = |characteristics: u32| characteristics & pe::IMAGE_SCN_MEM_WRITE != 0;
        let object::SectionFlags::Coff { characteristics } = section.flags() else {
            bail!("no flags")
        };
        ensure!(
            !written(characteristics) || written(there.characteristics.get(LE)),
            "that is written to, where the old one is not"
        );
        Ok(InPlace { section: index, offset: symbol.address(), len, old, room })
    }

    /// The names the patch defines.
    pub(super) fn defined(&self) -> impl Iterator<Item = &'a [u8]> {
        self.replaced.iter().map(|it| it.name)
    }

    fn target(
        &self,
        index: SymbolIndex,
        image: &Image,
        loaded: &FxHashSet<SectionIndex>,
    ) -> anyhow::Result<Target> {
        let symbol = self.file.symbol_by_index(index)?;
        if let Some(section) = symbol.section_index() {
            ensure!(loaded.contains(&section), "a reference to a section that is not loaded");
            // A static written in place is where the executable has it: its section is
            // added as a whole, with a copy of it that nothing may use.
            let mut in_place = self.in_place.iter().filter(|it| it.section == section);
            if symbol.kind() == SymbolKind::Section {
                ensure!(in_place.next().is_none(), "a reference to a static through its section");
            } else if let Some(it) = in_place.find(|it| it.offset == symbol.address()) {
                return Ok(Target::Theirs(it.old));
            }
            return Ok(Target::Ours(section, symbol.address()));
        }
        let name = symbol.name_bytes()?;
        let shown = || String::from_utf8_lossy(name);
        ensure!(symbol.is_undefined(), "{} is neither defined nor undefined", shown());
        match image.symbols.get(name) {
            Some(Named::One(Defined { kind: Kind::ThreadLocal, .. })) => {
                bail!("{} is thread-local", shown())
            }
            Some(Named::One(defined)) => Ok(Target::Theirs(defined.address)),
            Some(Named::Many) => bail!("{} names several things in the executable", shown()),
            // What a library exports is called through a stub of the linker's, which there
            // is none of when everything so far went through the slot the loader fills.
            None => match image.symbols.get(&*[b"__imp_", name].concat()) {
                Some(Named::One(slot)) => Ok(Target::Import(slot.address)),
                _ => bail!("{} is not in the executable", shown()),
            },
        }
    }

    /// Lays the patch out behind the executable and gives everything in it its address.
    pub(super) fn place(&self, image: &Image) -> anyhow::Result<Addition> {
        let page = image.section_alignment;
        // The file has each section as far behind the first as memory does: a file is
        // aligned to no more than memory is.
        let offset = image.len.next_multiple_of(image.file_alignment);
        let base = image.end().next_multiple_of(page);
        let fixed_up = if image.fixups.is_some() { self.stored } else { 0 };

        // Where everything goes, as offsets from `base`.
        let mut end = 0u64;
        let mut reserve = |len: usize, align: u64| {
            let start = end.next_multiple_of(align);
            end = start + len as u64;
            start
        };
        let mut placed = FxHashMap::default();
        let mut extents = [(0, 0); 4];
        let (mut functions_at, mut fixups_at, mut stubs_at) = (0, 0, 0);
        for added in Added::ALL {
            let start = reserve(0, page);
            for &index in self.sections.get(added as usize).into_iter().flatten() {
                let section = self.file.section_by_index(index)?;
                placed.insert(index, reserve(section.size() as usize, section.align().max(1)));
            }
            match added {
                Added::Read => {
                    let functions = image.functions.len() + self.functions * FUNCTION;
                    functions_at = reserve(functions, 4);
                }
                Added::Code => stubs_at = reserve(self.stubs.len() * Self::STUB, 1),
                Added::Data => {}
                // As long as it is when no two addresses are on one page.
                Added::Fixups => {
                    let fixups = image.fixups.as_ref().map_or(0, |it| Self::fixups(it).len());
                    fixups_at = reserve(fixups + fixed_up * Self::FIXUP, 4);
                }
            }
            extents[added as usize] = (start, reserve(0, 1));
        }
        ensure!(u32::try_from(base + end).is_ok(), "the program is too large");
        let mut appended = vec![0u8; end.next_multiple_of(image.file_alignment) as usize];
        let (mut fixed, functions) = self.fill(image, &mut appended, &placed, base, stubs_at)?;

        let mut overwritten = Vec::new();
        for it in &self.in_place {
            let new = &appended[(placed[&it.section] + it.offset) as usize..][..it.len as usize];
            overwritten.push((image.offset_of(it.old)?, new.to_vec()));
        }
        let mut redirected = 0;
        for replaced in &self.replaced {
            let Some(old) = replaced.jump_from else { continue };
            let new = base + placed[&replaced.section] + replaced.offset;
            let mut jump = vec![0xe9];
            jump.extend_from_slice(&relative(new, old + Self::JUMP as u64)?.to_le_bytes());
            overwritten.push((image.offset_of(old)?, jump));
            redirected += 1;
        }

        // The unwinder searches the table by halves: the patch's functions are all behind
        // the executable's, and in order among themselves once sorted.
        let functions = [&image.functions[..], &sorted(&functions)].concat();
        appended[functions_at as usize..][..functions.len()].copy_from_slice(&functions);

        let mut headers = image.headers.clone();
        let mut point = |index: usize, at: u64, len: usize| {
            // The directories follow what else the optional header has.
            let first = offset_of!(pe::ImageOptionalHeader64, number_of_rva_and_sizes) + 4;
            let entry = &mut headers[image.optional_header + first + index * DIRECTORY..];
            entry[..4].copy_from_slice(&((base + at) as u32).to_le_bytes());
            entry[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        };
        point(pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION, functions_at, functions.len());
        if let Some(old) = &image.fixups {
            // What a static that is written over stored is an address no longer.
            let over =
                |at: &u64| self.in_place.iter().any(|it| (it.old..it.old + it.room).contains(at));
            fixed.extend(old.iter().copied().filter(|at| !over(at)));
            fixed.sort_unstable();
            let fixups = Self::fixups(&fixed);
            appended[fixups_at as usize..][..fixups.len()].copy_from_slice(&fixups);
            point(pe::IMAGE_DIRECTORY_ENTRY_BASERELOC, fixups_at, fixups.len());
            // Fewer blocks than there was room for: the end of everything moves up.
            extents[Added::Fixups as usize].1 = fixups_at + fixups.len() as u64;
            let end = extents.iter().map(|it| it.1).max().unwrap_or(0);
            appended.truncate(end.next_multiple_of(image.file_alignment) as usize);
        }
        let image_end = base + (appended.len() as u64).next_multiple_of(page);

        // The headers of the new sections, in the room behind the linker's.
        let mut count = image.sections.len();
        for added in Added::ALL {
            let (start, end) = extents[added as usize];
            if start == end {
                continue;
            }
            let len = (end - start) as u32;
            let raw = len.next_multiple_of(image.file_alignment as u32);
            let header = pe::ImageSectionHeader {
                name: added.name(),
                virtual_size: object::U32::new(LE, len),
                virtual_address: object::U32::new(LE, (base + start) as u32),
                size_of_raw_data: object::U32::new(LE, raw),
                pointer_to_raw_data: object::U32::new(LE, u32::try_from(offset + start)?),
                pointer_to_relocations: object::U32::new(LE, 0),
                pointer_to_linenumbers: object::U32::new(LE, 0),
                number_of_relocations: object::U16::new(LE, 0),
                number_of_linenumbers: object::U16::new(LE, 0),
                characteristics: object::U32::new(LE, added.characteristics()),
            };
            let room = headers
                .get_mut(image.section_table + count * SECTION_HEADER..)
                .and_then(|it| it.get_mut(..SECTION_HEADER))
                .filter(|it| it.iter().all(|&it| it == 0))
                .context("no room for another section in its headers")?;
            room.copy_from_slice(object::bytes_of(&header));
            count += 1;
        }
        let sections = offset_of!(pe::ImageFileHeader, number_of_sections);
        headers[image.file_header + sections..][..2]
            .copy_from_slice(&u16::try_from(count)?.to_le_bytes());
        let size = offset_of!(pe::ImageOptionalHeader64, size_of_image);
        headers[image.optional_header + size..][..4]
            .copy_from_slice(&(image_end as u32).to_le_bytes());
        overwritten.push((0, headers));

        Ok(Addition {
            offset,
            appended,
            // The headers of a PE image are among what is written over.
            headers_offset: 0,
            headers: Vec::new(),
            overwritten,
            redirected,
            described: Default::default(),
            live_data_at: 0,
            live_frames: (0, 0),
            live_pointers: Vec::new(),
            defined: Vec::new(),
        })
    }

    /// Copies the patch's sections into `out`, which is loaded at `base`, each where `placed`
    /// says, gives each relocation its value and writes the stubs at `stubs_at`. Says where an
    /// address is stored, which the loader moves with the image, and what the patch has of
    /// `.pdata`.
    fn fill(
        &self,
        image: &Image,
        out: &mut [u8],
        placed: &FxHashMap<SectionIndex, u64>,
        base: u64,
        stubs_at: u64,
    ) -> anyhow::Result<(Vec<u64>, Vec<u8>)> {
        let stub_of: FxHashMap<SymbolIndex, u64> = (self.stubs.iter().copied())
            .zip((0..).map(|at| base + stubs_at + at * Self::STUB as u64))
            .collect();
        let address = |target: Target| match target {
            Target::Ours(section, at) => base + placed[&section] + at,
            Target::Theirs(address) | Target::Import(address) => address,
        };

        let mut fixed = Vec::with_capacity(self.stored);
        let mut functions = Vec::with_capacity(self.functions * FUNCTION);
        for section in self.file.sections() {
            let Some(&start) = placed.get(&section.index()) else { continue };
            let data = section.data()?;
            out[start as usize..][..data.len()].copy_from_slice(data);
            for (at, relocation) in section.relocations() {
                let RelocationTarget::Symbol(index) = relocation.target() else { continue };
                let target = self.targets[&index];
                let place = base + start + at;
                let in_place = self.in_place.iter().find(|it| it.holds(section.index(), at));
                let fixup = Fixup::of(&relocation)?;
                let len = if let Fixup::Absolute = fixup { 8 } else { 4 };
                let field = out
                    .get_mut((start + at) as usize..)
                    .and_then(|it| it.get_mut(..len))
                    .context("a relocation past its section")?;
                match fixup {
                    Fixup::Absolute => {
                        let added = i64::from_le_bytes((&*field).try_into()?);
                        let value = (image.base + address(target)).wrapping_add_signed(added);
                        field.copy_from_slice(&value.to_le_bytes());
                        fixed.push(in_place.map_or(place, |it| it.old + at - it.offset));
                    }
                    Fixup::Relative(_) if in_place.is_some() => {
                        bail!("a static that stores a distance")
                    }
                    Fixup::Relative(behind) => {
                        let added = i64::from(i32::from_le_bytes((&*field).try_into()?));
                        let to = match target {
                            Target::Import(_) => stub_of[&index],
                            _ => address(target),
                        };
                        let value = relative(to.wrapping_add_signed(added), place + 4 + behind)?;
                        field.copy_from_slice(&value.to_le_bytes());
                    }
                    Fixup::InImage => {
                        let added = i64::from(i32::from_le_bytes((&*field).try_into()?));
                        let value = u32::try_from(address(target).wrapping_add_signed(added))
                            .context("the program is too large")?;
                        field.copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
            if merged(section.name_bytes()?) == b".pdata" {
                functions.extend_from_slice(&out[start as usize..][..data.len()]);
            }
        }
        for (&index, &stub) in &stub_of {
            let Target::Import(slot) = self.targets[&index] else { continue };
            let code = &mut out[(stub - base) as usize..][..Self::STUB];
            code[..2].copy_from_slice(&[0xff, 0x25]);
            code[2..].copy_from_slice(&relative(slot, stub + Self::STUB as u64)?.to_le_bytes());
        }
        Ok((fixed, functions))
    }

    /// The patch laid out at `base` of a program that runs the executable, every address as
    /// the linker has it, for the program to map there: the constants, code, stubs and table of
    /// functions to read and run, the writable sections behind, and a jump over the entry of
    /// each function it replaces. With where each new function starts and where the
    /// executable has the old one.
    pub(super) fn place_live(
        &self,
        image: &Image,
        base: u64,
    ) -> anyhow::Result<(ra_jit_live::format::Code, Vec<(u64, u64)>)> {
        const PAGE: u64 = 4096;
        // ponytail: a static with another value asks for a restart; writing it over the old
        // one, with the addresses it stores moved, is what the splice does and the upgrade.
        ensure!(self.in_place.is_empty(), "a static has another value");
        let start = base.checked_sub(image.base).context("room below the executable")?;
        let mut end = 0u64;
        let mut reserve = |len: usize, align: u64| {
            let start = end.next_multiple_of(align);
            end = start + len as u64;
            start
        };
        let mut placed = FxHashMap::default();
        for added in [Added::Read, Added::Code] {
            for &index in &self.sections[added as usize] {
                let section = self.file.section_by_index(index)?;
                placed.insert(index, reserve(section.size() as usize, section.align().max(1)));
            }
        }
        let stubs_at = reserve(self.stubs.len() * Self::STUB, 1);
        let functions_at = reserve(self.functions * FUNCTION, 4);
        let data_at = reserve(0, PAGE);
        for &index in &self.sections[Added::Data as usize] {
            let section = self.file.section_by_index(index)?;
            placed.insert(index, reserve(section.size() as usize, section.align().max(1)));
        }
        ensure!(u32::try_from(start + end).is_ok(), "the program is too large");
        let mut text = vec![0u8; end as usize];
        let (fixed, functions) = self.fill(image, &mut text, &placed, start, stubs_at)?;
        // The unwinder searches a table that is added by halves too.
        let functions = sorted(&functions);
        text[functions_at as usize..][..functions.len()].copy_from_slice(&functions);
        let data = text.split_off(data_at as usize);

        let mut writes = Vec::new();
        let mut moved = Vec::new();
        for replaced in &self.replaced {
            let Some(old) = replaced.jump_from else { continue };
            let new = start + placed[&replaced.section] + replaced.offset;
            let mut bytes = vec![0xe9];
            bytes.extend_from_slice(&relative(new, old + Self::JUMP as u64)?.to_le_bytes());
            moved.push((image.base + new, image.base + old));
            let address = image.base + old;
            writes.push(ra_jit_live::format::Write { address, bytes, pointers: Vec::new() });
        }
        let code = ra_jit_live::format::Code {
            base,
            text,
            data,
            frames: (functions_at, functions.len() as u64),
            pointers: fixed.iter().map(|it| it - start).collect(),
            redirected: writes.len() as u64,
            statics: 0,
            writes,
            // ponytail: a static new in a patch is the patch's own here, as before lasting
            // statics; it starts again from its first value with each patch.
            lasting: Vec::new(),
        };
        Ok((code, moved))
    }

    /// The blocks of `.reloc` for whole addresses stored at `fixed`, which is sorted: one for
    /// each page, of the page, how long the block is, and where on the page each address is.
    fn fixups(fixed: &[u64]) -> Vec<u8> {
        let mut blocks = Vec::new();
        for page in fixed.chunk_by(|a, b| a >> 12 == b >> 12) {
            let entries = page.len().next_multiple_of(2);
            blocks.extend_from_slice(&((page[0] & !0xfff) as u32).to_le_bytes());
            blocks.extend_from_slice(&((8 + entries * 2) as u32).to_le_bytes());
            for at in page {
                let entry = pe::IMAGE_REL_BASED_DIR64 << 12 | (at & 0xfff) as u16;
                blocks.extend_from_slice(&entry.to_le_bytes());
            }
            // A block is a whole number of 4 bytes: an entry of no kind fills it.
            blocks.resize(blocks.len() + (entries - page.len()) * 2, 0);
        }
        blocks
    }
}

/// The section of the executable a section of an object goes to: the linker sorts by what
/// is behind the `$` and leaves it out.
fn merged(name: &[u8]) -> &[u8] {
    name.split(|&it| it == b'$').next().unwrap_or(name)
}

/// How many bytes each static of an object has to itself. A COFF symbol does not say how
/// large it is: what has a name of its own is as large as the next thing in its section is
/// far, which has one too in an object of ours. `None` for what shares its place.
fn rooms<'a>(file: &CoffFile<'a>) -> anyhow::Result<FxHashMap<&'a [u8], Option<u64>>> {
    let mut places: FxHashMap<SectionIndex, Vec<u64>> = FxHashMap::default();
    for symbol in file.symbols() {
        if let Some(section) = symbol.section_index()
            && symbol.kind() != SymbolKind::Section
        {
            places.entry(section).or_default().push(symbol.address());
        }
    }
    places.values_mut().for_each(|it| it.sort_unstable());
    let mut rooms = FxHashMap::default();
    for symbol in file.symbols() {
        let Some(index) = symbol.section_index() else { continue };
        let section = file.section_by_index(index)?;
        let object::SectionFlags::Coff { characteristics } = section.flags() else { continue };
        if symbol.is_local() || characteristics & pe::IMAGE_SCN_MEM_EXECUTE != 0 {
            continue;
        }
        let (places, at) = (&places[&index], symbol.address());
        let first = places.partition_point(|&it| it < at);
        let next = places.partition_point(|&it| it <= at);
        let end = places.get(next).copied().unwrap_or(section.size());
        // The linker keeps a section of its own as it is, and may drop one it is told it has
        // another of.
        let own = next - first == 1 && characteristics & pe::IMAGE_SCN_LNK_COMDAT == 0;
        rooms.insert(symbol.name_bytes()?, own.then(|| end - at));
    }
    Ok(rooms)
}

/// Where a table of `.reloc` says whole addresses are stored, sorted.
fn fixed_up(table: &[u8]) -> anyhow::Result<Vec<u64>> {
    let mut fixed = Vec::new();
    let mut rest = table;
    while rest.len() >= 8 {
        let (page, len) = (u64::from(read_u32(rest)), read_u32(&rest[4..]) as usize);
        let block = rest.get(8..len).context("a block of .reloc past its table")?;
        for entry in block.chunks_exact(2) {
            let entry = u16::from_le_bytes([entry[0], entry[1]]);
            match entry >> 12 {
                pe::IMAGE_REL_BASED_ABSOLUTE => {}
                pe::IMAGE_REL_BASED_DIR64 => fixed.push(page + u64::from(entry & 0xfff)),
                other => bail!("an address the loader moves in the way {other}"),
            }
        }
        rest = &rest[len..];
    }
    fixed.sort_unstable();
    Ok(fixed)
}

/// The entries of `.pdata` in `functions` in the order of where they start.
fn sorted(functions: &[u8]) -> Vec<u8> {
    let mut entries: Vec<&[u8]> = functions.chunks_exact(FUNCTION).collect();
    entries.sort_by_key(|it| read_u32(it));
    entries.concat()
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
