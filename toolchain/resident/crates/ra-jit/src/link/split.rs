//! The standard library's object as an archive of several, which the link reads with
//! `--whole-archive`. lld does most of its work on an input file by itself, one file to a
//! thread: one object of every function of the standard library had it parse, scan and
//! demote 140,000 sections on one thread (0.12 s of the 0.16 s link of hello), where rustc's
//! standard library is many objects.
//!
//! The functions' and statics' own sections are spread over the parts in the order they are
//! in, which keeps a function near what it refers to. The rest stays in the first part: the
//! unwind table, what a debugger reads, the thread-locals and the initializers. A local
//! symbol, or a section, that another part refers to becomes a hidden global of its part, and
//! the other part refers to it by name: the program links the same.

use std::borrow::Cow;

use anyhow::{Context, bail};
use object::elf;
use object::read::elf::{ElfFile64, FileHeader as _, SectionHeader as _, Sym as _};
use object::write;
use object::{
    Endianness, Object as _, ObjectSection as _, ObjectSymbol as _, RelocationFlags,
    RelocationTarget, SectionFlags, SectionIndex, SectionKind, SymbolFlags, SymbolIndex,
    SymbolKind, SymbolScope,
};
use rustc_hash::FxHashMap;

/// How many parts the functions are spread over, past the first: as many as lld has threads
/// to give them on a machine that builds, and few enough that each is worth a thread.
pub(crate) const PARTS: usize = 16;

/// The start of an archive.
const MAGIC: &[u8; 8] = b"!<arch>\n";

/// Whether `bytes` are those of an archive, which the link reads every member of.
pub(crate) fn is_archive(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// The prefixes of the names of the sections that are each a function's or a static's own.
const SPREAD: [&str; 6] =
    [".text.", ".rodata.", ".data.rel.ro.", ".data.", ".bss.", ".gcc_except_table"];

struct Section<'data> {
    name: &'data [u8],
    kind: SectionKind,
    sh_flags: u64,
    align: u64,
    size: u64,
    data: Cow<'data, [u8]>,
    part: usize,
    relocations: Vec<Relocation>,
}

#[derive(Clone, Copy)]
struct Relocation {
    offset: u64,
    /// The symbol it refers to, by its index among [`Symbol`]s.
    target: usize,
    addend: i64,
    r_type: u32,
}

struct Symbol<'data> {
    name: &'data [u8],
    value: u64,
    size: u64,
    st_info: u8,
    st_other: u8,
    /// The section it is defined in, by its index among [`Section`]s.
    section: Option<usize>,
    /// Whether it is defined with no section: a file's name or an absolute value.
    sectionless: bool,
}

impl Symbol<'_> {
    fn bind(&self) -> u8 {
        self.st_info >> 4
    }

    fn st_type(&self) -> u8 {
        self.st_info & 0xf
    }
}

/// `object` as an archive of `1 + parts` objects that link to the same program.
pub(crate) fn split(object: &[u8], parts: usize) -> anyhow::Result<Vec<u8>> {
    let file = ElfFile64::<Endianness>::parse(object).context("reading the object")?;
    let endian = file.endian();
    let header = file.elf_header();
    if header.e_type(endian) != elf::ET_REL {
        bail!("not a relocatable object");
    }
    let mut sections: Vec<Section<'_>> = Vec::new();
    let mut read = Vec::new();
    let mut at: FxHashMap<SectionIndex, usize> = FxHashMap::default();
    for section in file.sections() {
        let raw = section.elf_section_header();
        let sh_type = raw.sh_type(endian);
        match sh_type {
            elf::SHT_NULL
            | elf::SHT_RELA
            | elf::SHT_REL
            | elf::SHT_SYMTAB
            | elf::SHT_STRTAB
            | elf::SHT_SYMTAB_SHNDX => continue,
            elf::SHT_GROUP => bail!("the object has section groups"),
            _ => {}
        }
        let sh_flags = raw.sh_flags(endian);
        if raw.sh_link(endian) != 0 || sh_flags & u64::from(elf::SHF_LINK_ORDER) != 0 {
            bail!("a section is linked to another");
        }
        let name = section.name_bytes()?;
        let kind = match sh_type {
            elf::SHT_PROGBITS if sh_flags & u64::from(elf::SHF_STRINGS) != 0 => {
                SectionKind::OtherString
            }
            elf::SHT_PROGBITS => SectionKind::Other,
            elf::SHT_NOBITS if sh_flags & u64::from(elf::SHF_TLS) != 0 => {
                SectionKind::UninitializedTls
            }
            elf::SHT_NOBITS => SectionKind::UninitializedData,
            elf::SHT_NOTE => SectionKind::Note,
            other => SectionKind::Elf(other),
        };
        let spread = sh_flags & u64::from(elf::SHF_ALLOC) != 0
            && sh_flags & u64::from(elf::SHF_TLS | elf::SHF_MERGE) == 0
            && matches!(sh_type, elf::SHT_PROGBITS | elf::SHT_NOBITS)
            && SPREAD.iter().any(|it| name.starts_with(it.as_bytes()));
        let data =
            Cow::Borrowed(if sh_type == elf::SHT_NOBITS { &[][..] } else { section.data()? });
        at.insert(section.index(), sections.len());
        sections.push(Section {
            name,
            kind,
            sh_flags,
            align: section.align(),
            size: section.size(),
            data,
            part: usize::from(spread),
            relocations: Vec::new(),
        });
        read.push(section.relocations().collect::<Vec<_>>());
    }

    let mut symbols: Vec<Symbol<'_>> = Vec::new();
    let mut symbol_at: FxHashMap<SymbolIndex, usize> = FxHashMap::default();
    for symbol in file.symbols() {
        let raw = symbol.elf_symbol();
        let section = match symbol.section() {
            object::SymbolSection::Section(index) => {
                Some(*at.get(&index).context("a symbol is in a section that is not kept")?)
            }
            object::SymbolSection::Undefined => None,
            object::SymbolSection::Absolute | object::SymbolSection::None => None,
            other => bail!("a symbol is defined in {other:?}"),
        };
        let sectionless = matches!(
            symbol.section(),
            object::SymbolSection::Absolute | object::SymbolSection::None
        );
        if sectionless && raw.st_type() != elf::STT_FILE {
            bail!("a symbol has an absolute value");
        }
        symbol_at.insert(symbol.index(), symbols.len());
        symbols.push(Symbol {
            name: symbol.name_bytes()?,
            value: symbol.address(),
            size: symbol.size(),
            st_info: raw.st_info(),
            st_other: raw.st_other(),
            section,
            sectionless,
        });
    }
    for (section, read) in sections.iter_mut().zip(read) {
        for (offset, relocation) in read {
            let RelocationTarget::Symbol(index) = relocation.target() else {
                bail!("a relocation is not to a symbol");
            };
            let RelocationFlags::Elf { r_type } = relocation.flags() else {
                bail!("a relocation that is not ELF's");
            };
            section.relocations.push(Relocation {
                offset,
                target: *symbol_at.get(&index).context("a relocation to no symbol")?,
                addend: relocation.addend(),
                r_type,
            });
        }
    }
    // Contiguous runs of about the same size, each with its relocations, which lld reads too.
    let weight = |it: &Section<'_>| it.size + 24 * it.relocations.len() as u64 + 64;
    let total: u64 = sections.iter().filter(|it| it.part != 0).map(weight).sum();
    let mut before = 0;
    for section in sections.iter_mut().filter(|it| it.part != 0) {
        section.part = 1 + (before * parts as u64 / total.max(1)) as usize;
        before += weight(section);
    }
    if endian != Endianness::Little {
        bail!("a big-endian object");
    }
    spread_unwind(&mut sections, &symbols, parts)?;
    spread_described(&mut sections, &symbols);

    // What one part refers to of another's that the other has to name for it.
    let mut named_symbol = vec![false; symbols.len()];
    let mut named_section = vec![false; sections.len()];
    for from in 0..sections.len() {
        for relocation in &sections[from].relocations {
            let target = relocation.target;
            let symbol = &symbols[target];
            let Some(home) = symbol.section else { continue };
            if sections[home].part == sections[from].part {
                continue;
            }
            if symbol.st_type() == elf::STT_SECTION {
                named_section[home] = true;
            } else if symbol.bind() == elf::STB_LOCAL {
                named_symbol[target] = true;
            }
        }
    }
    // A place in strings the linker merges is not an offset from a symbol at their start:
    // strings another part names are kept as they are, which only a debugger reads.
    for (section, &named) in sections.iter_mut().zip(&named_section) {
        let merged = u64::from(elf::SHF_MERGE | elf::SHF_STRINGS);
        if named && section.sh_flags & merged != 0 {
            if section.sh_flags & u64::from(elf::SHF_ALLOC) != 0 {
                bail!("a part refers into another's strings by place");
            }
            section.sh_flags &= !merged;
            section.kind = SectionKind::Other;
        }
    }

    let architecture = file.architecture();
    let flags = object::FileFlags::Elf {
        os_abi: header.e_ident().os_abi,
        abi_version: header.e_ident().abi_version,
        e_flags: header.e_flags(endian),
    };
    let input = Input { sections: &sections, symbols: &symbols, named_symbol, named_section };
    let objects: Vec<anyhow::Result<Vec<u8>>> = std::thread::scope(|scope| {
        let built: Vec<_> = (0..=parts)
            .map(|part| {
                let input = &input;
                scope.spawn(move || input.part(part, architecture, endian, flags))
            })
            .collect();
        built.into_iter().map(|it| it.join().expect("a part panicked")).collect()
    });
    let mut archive = MAGIC.to_vec();
    for (part, object) in objects.into_iter().enumerate() {
        let object = object.with_context(|| format!("writing part {part}"))?;
        member(&mut archive, &format!("std.{part}.o"), &object);
    }
    Ok(archive)
}

/// An entry of the unwind table: a common entry, or a function's, which names its common
/// entry by where that starts.
struct Entry {
    start: usize,
    end: usize,
    common: Option<usize>,
    relocations: std::ops::Range<usize>,
}

/// Spreads the unwind table over the parts: each function's entry goes to the part of the
/// function, with a copy of the common entry it names, and its exception table with it. An
/// entry another part refers to would be most of what the parts name of each other.
fn spread_unwind(
    sections: &mut Vec<Section<'_>>,
    symbols: &[Symbol<'_>],
    parts: usize,
) -> anyhow::Result<()> {
    let Some(at) = sections.iter().position(|it| it.name == b".eh_frame") else {
        return Ok(());
    };
    let data = sections[at].data.clone();
    let mut relocations = std::mem::take(&mut sections[at].relocations);
    relocations.sort_by_key(|it| it.offset);
    let word = |at: usize| -> anyhow::Result<usize> {
        let bytes = data.get(at..at + 4).context("the unwind table ends inside an entry")?;
        Ok(u32::from_le_bytes(bytes.try_into()?) as usize)
    };
    let mut entries = Vec::new();
    let (mut start, mut next) = (0, 0);
    while start < data.len() {
        let len = word(start)?;
        if len == 0 {
            // The end of the table, which the linker writes its own of.
            if start + 4 != data.len() {
                bail!("the unwind table goes on past its end");
            }
            break;
        }
        if len == 0xffff_ffff {
            bail!("an unwind table entry of 64-bit length");
        }
        let end = start + 4 + len;
        let id = word(start + 4)?;
        let common = match id {
            0 => None,
            id => Some((start + 4).checked_sub(id).context("an entry names no common entry")?),
        };
        let first = next;
        while next < relocations.len() && (relocations[next].offset as usize) < end {
            next += 1;
        }
        entries.push(Entry { start, end, common, relocations: first..next });
        start = end;
    }
    if next != relocations.len() {
        bail!("a relocation of the unwind table is past its entries");
    }
    let target = |relocation: &Relocation| symbols[relocation.target].section;
    let mut part_of = vec![0; entries.len()];
    for (entry, part) in entries.iter().zip(&mut part_of) {
        if entry.common.is_none() {
            continue;
        }
        let mut relocations = relocations[entry.relocations.clone()].iter();
        let Some(function) = relocations.next().and_then(target) else { continue };
        *part = sections[function].part;
        for table in relocations.filter_map(target) {
            if sections[table].name.starts_with(b".gcc_except_table") && sections[table].part != 0 {
                sections[table].part = *part;
            }
        }
    }
    let common_at: FxHashMap<usize, usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, it)| it.common.is_none())
        .map(|(at, it)| (it.start, at))
        .collect();
    let mut built: Vec<(Vec<u8>, Vec<Relocation>)> = vec![Default::default(); parts + 1];
    let mut copied: Vec<FxHashMap<usize, usize>> = vec![FxHashMap::default(); parts + 1];
    let copy = |into: &mut (Vec<u8>, Vec<Relocation>), entry: &Entry| {
        let new = into.0.len();
        into.0.extend_from_slice(&data[entry.start..entry.end]);
        for relocation in &relocations[entry.relocations.clone()] {
            let offset = relocation.offset - entry.start as u64 + new as u64;
            into.1.push(Relocation { offset, ..*relocation });
        }
        new
    };
    for (entry, &part) in entries.iter().zip(&part_of) {
        let Some(common) = entry.common else { continue };
        let named = *common_at.get(&common).context("an entry names no common entry")?;
        let new_common = match copied[part].get(&common) {
            Some(&new) => new,
            None => {
                let new = copy(&mut built[part], &entries[named]);
                copied[part].insert(common, new);
                new
            }
        };
        let new = copy(&mut built[part], entry);
        let pointer = u32::try_from(new + 4 - new_common)?;
        built[part].0[new + 4..new + 8].copy_from_slice(&pointer.to_le_bytes());
    }
    for (common, &at) in &common_at {
        if !copied.iter().any(|it| it.contains_key(common)) {
            copy(&mut built[0], &entries[at]);
        }
    }
    let mut built = built.into_iter();
    let (data, relocations) = built.next().expect("there is a first part");
    let template = &mut sections[at];
    template.size = data.len() as u64;
    template.data = Cow::Owned(data);
    template.relocations = relocations;
    let (name, kind, sh_flags, align) =
        (template.name, template.kind, template.sh_flags, template.align);
    for (part, (data, relocations)) in built.enumerate().map(|(at, it)| (at + 1, it)) {
        if data.is_empty() {
            continue;
        }
        sections.push(Section {
            name,
            kind,
            sh_flags,
            align,
            size: data.len() as u64,
            data: Cow::Owned(data),
            part,
            relocations,
        });
    }
    Ok(())
}

/// Puts what a debugger reads beside the code it describes: the sections that refer to each
/// other (a unit's entries, its abbreviations, lines and ranges, which ra-jit writes for each
/// unit apart) go to the part most of the code they describe is in. Strings, which every unit
/// shares, stay in the first part.
fn spread_described(sections: &mut [Section<'_>], symbols: &[Symbol<'_>]) {
    let alloc = u64::from(elf::SHF_ALLOC);
    let movable =
        |it: &Section<'_>| it.part == 0 && it.sh_flags & (alloc | u64::from(elf::SHF_MERGE)) == 0;
    let mut parent: Vec<usize> = (0..sections.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    for from in 0..sections.len() {
        if !movable(&sections[from]) {
            continue;
        }
        for relocation in &sections[from].relocations {
            let Some(to) = symbols[relocation.target].section else { continue };
            if movable(&sections[to]) {
                let (a, b) = (root(&mut parent, from), root(&mut parent, to));
                parent[a] = b;
            }
        }
    }
    let mut votes: FxHashMap<usize, FxHashMap<usize, usize>> = FxHashMap::default();
    for from in 0..sections.len() {
        if !movable(&sections[from]) {
            continue;
        }
        let cluster = root(&mut parent, from);
        for relocation in &sections[from].relocations {
            let Some(to) = symbols[relocation.target].section else { continue };
            if sections[to].sh_flags & alloc != 0 {
                *votes.entry(cluster).or_default().entry(sections[to].part).or_default() += 1;
            }
        }
    }
    let chosen: FxHashMap<usize, usize> = votes
        .into_iter()
        .filter_map(|(cluster, votes)| {
            let most = votes.into_iter().max_by_key(|&(part, count)| (count, usize::MAX - part))?;
            Some((cluster, most.0))
        })
        .collect();
    for at in 0..sections.len() {
        if movable(&sections[at])
            && let Some(&part) = chosen.get(&root(&mut parent, at))
        {
            sections[at].part = part;
        }
    }
}

struct Input<'a, 'data> {
    sections: &'a [Section<'data>],
    symbols: &'a [Symbol<'data>],
    named_symbol: Vec<bool>,
    named_section: Vec<bool>,
}

impl Input<'_, '_> {
    /// The name a local symbol another part refers to has.
    fn named(&self, at: usize) -> Vec<u8> {
        let mut name = self.symbols[at].name.to_vec();
        name.extend_from_slice(format!(".part{at}").as_bytes());
        name
    }

    fn part(
        &self,
        part: usize,
        architecture: object::Architecture,
        endian: Endianness,
        flags: object::FileFlags,
    ) -> anyhow::Result<Vec<u8>> {
        let mut object = write::Object::new(object::BinaryFormat::Elf, architecture, endian);
        object.flags = flags;
        let mut ids: FxHashMap<usize, write::SectionId> = FxHashMap::default();
        let mut stack = false;
        for (at, section) in self.sections.iter().enumerate().filter(|it| it.1.part == part) {
            stack |= section.name == b".note.GNU-stack";
            let id = object.add_section(Vec::new(), section.name.to_vec(), section.kind);
            object.section_mut(id).flags = SectionFlags::Elf { sh_flags: section.sh_flags };
            if matches!(
                section.kind,
                SectionKind::UninitializedData | SectionKind::UninitializedTls
            ) {
                object.append_section_bss(id, section.size, section.align);
            } else {
                object.set_section_data(id, &*section.data, section.align);
            }
            ids.insert(at, id);
        }
        if !stack {
            let id =
                object.add_section(Vec::new(), b".note.GNU-stack".to_vec(), SectionKind::Other);
            object.section_mut(id).flags = SectionFlags::Elf { sh_flags: 0 };
        }
        let mut defined: FxHashMap<usize, write::SymbolId> = FxHashMap::default();
        for (at, symbol) in self.symbols.iter().enumerate() {
            let home = match symbol.section {
                Some(home) if self.sections[home].part == part => ids[&home],
                Some(_) | None if !(symbol.sectionless && part == 0) => continue,
                _ => {
                    let id = object.add_symbol(write::Symbol {
                        name: symbol.name.to_vec(),
                        value: symbol.value,
                        size: symbol.size,
                        kind: SymbolKind::File,
                        scope: SymbolScope::Compilation,
                        weak: false,
                        section: write::SymbolSection::None,
                        flags: SymbolFlags::Elf {
                            st_info: symbol.st_info,
                            st_other: symbol.st_other,
                        },
                    });
                    defined.insert(at, id);
                    continue;
                }
            };
            if symbol.st_type() == elf::STT_SECTION {
                defined.insert(at, object.section_symbol(home));
                continue;
            }
            let promoted = self.named_symbol[at];
            let (st_info, st_other, name) = if promoted {
                let st_info = (elf::STB_GLOBAL << 4) | symbol.st_type();
                (st_info, (symbol.st_other & !3) | elf::STV_HIDDEN, self.named(at))
            } else {
                (symbol.st_info, symbol.st_other, symbol.name.to_vec())
            };
            let id = object.add_symbol(write::Symbol {
                name,
                value: symbol.value,
                size: symbol.size,
                kind: kind_of(symbol.st_type()),
                scope: scope_of(st_info >> 4, st_other),
                weak: st_info >> 4 == elf::STB_WEAK,
                section: write::SymbolSection::Section(home),
                flags: SymbolFlags::Elf { st_info, st_other },
            });
            defined.insert(at, id);
        }
        for (at, _) in self.sections.iter().enumerate().filter(|it| it.1.part == part) {
            if self.named_section[at] {
                object.add_symbol(write::Symbol {
                    name: section_name(at),
                    value: 0,
                    size: 0,
                    kind: SymbolKind::Label,
                    scope: SymbolScope::Linkage,
                    weak: false,
                    section: write::SymbolSection::Section(ids[&at]),
                    flags: SymbolFlags::Elf {
                        st_info: (elf::STB_GLOBAL << 4) | elf::STT_NOTYPE,
                        st_other: elf::STV_HIDDEN,
                    },
                });
            }
        }
        let mut referred: FxHashMap<Vec<u8>, write::SymbolId> = FxHashMap::default();
        let mut refer = |object: &mut write::Object<'_>, name: Vec<u8>, st_info: u8| {
            *referred.entry(name.clone()).or_insert_with(|| {
                object.add_symbol(write::Symbol {
                    name,
                    value: 0,
                    size: 0,
                    kind: SymbolKind::Unknown,
                    scope: SymbolScope::Unknown,
                    weak: st_info >> 4 == elf::STB_WEAK,
                    section: write::SymbolSection::Undefined,
                    flags: SymbolFlags::Elf { st_info, st_other: elf::STV_DEFAULT },
                })
            })
        };
        for (at, section) in self.sections.iter().enumerate().filter(|it| it.1.part == part) {
            for relocation in &section.relocations {
                let target = relocation.target;
                let symbol = &self.symbols[target];
                let id = match defined.get(&target) {
                    Some(&id) => id,
                    None if symbol.st_type() == elf::STT_SECTION => {
                        let home = symbol.section.expect("a section symbol is of a section");
                        refer(&mut object, section_name(home), elf::STB_GLOBAL << 4)
                    }
                    None if self.named_symbol[target] => {
                        refer(&mut object, self.named(target), elf::STB_GLOBAL << 4)
                    }
                    None => {
                        let weak = symbol.section.is_none() && symbol.bind() == elf::STB_WEAK;
                        let bind = if weak { elf::STB_WEAK } else { elf::STB_GLOBAL };
                        refer(&mut object, symbol.name.to_vec(), bind << 4)
                    }
                };
                object.add_relocation(
                    ids[&at],
                    write::Relocation {
                        offset: relocation.offset,
                        symbol: id,
                        addend: relocation.addend,
                        flags: RelocationFlags::Elf { r_type: relocation.r_type },
                    },
                )?;
            }
        }
        Ok(object.write()?)
    }
}

/// The name a section another part refers to by place has.
fn section_name(at: usize) -> Vec<u8> {
    format!("ra-jit.part.section.{at}").into_bytes()
}

fn kind_of(st_type: u8) -> SymbolKind {
    match st_type {
        elf::STT_FUNC => SymbolKind::Text,
        elf::STT_OBJECT => SymbolKind::Data,
        elf::STT_TLS => SymbolKind::Tls,
        _ => SymbolKind::Label,
    }
}

fn scope_of(bind: u8, st_other: u8) -> SymbolScope {
    match (bind, st_other & 3) {
        (elf::STB_LOCAL, _) => SymbolScope::Compilation,
        (_, elf::STV_DEFAULT) => SymbolScope::Dynamic,
        _ => SymbolScope::Linkage,
    }
}

/// Appends a member to an archive: a name of at most 15 bytes, no symbol index, as the link
/// reads every member.
fn member(archive: &mut Vec<u8>, name: &str, bytes: &[u8]) {
    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
        format!("{name}/"),
        0,
        0,
        0,
        644,
        bytes.len()
    );
    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(bytes);
    if bytes.len() % 2 == 1 {
        archive.push(b'\n');
    }
}

#[cfg(test)]
mod tests {
    use object::read::archive::ArchiveFile;
    use object::write::{Object, Relocation, StandardSegment, Symbol, SymbolSection};
    use object::{
        Architecture, BinaryFormat, Endianness, Object as _, ObjectSection as _, ObjectSymbol as _,
        RelocationFlags, RelocationTarget, SectionKind, SymbolFlags, SymbolKind, SymbolScope, elf,
    };
    use rustc_hash::FxHashMap;

    /// Where each relocation of an object lands and what it reaches: the section and offset
    /// of the place, and the section and offset of the target, or the name of a symbol no
    /// section of the objects defines. A section is told by its contents' first bytes.
    fn reached(objects: &[Vec<u8>]) -> Vec<(Vec<u8>, u64, String, i64)> {
        let files: Vec<_> = objects.iter().map(|it| object::File::parse(&**it).unwrap()).collect();
        let tag = |file: &object::File<'_>, index| {
            let section = file.section_by_index(index).unwrap();
            [section.name_bytes().unwrap(), &section.data().unwrap()[..2]].concat()
        };
        let mut globals = FxHashMap::default();
        for file in &files {
            for symbol in file.symbols() {
                if let (Some(index), false) = (symbol.section_index(), symbol.is_local()) {
                    globals.insert(
                        symbol.name().unwrap().to_owned(),
                        (tag(file, index), symbol.address()),
                    );
                }
            }
        }
        let mut out = Vec::new();
        for file in &files {
            for section in file.sections() {
                for (offset, relocation) in section.relocations() {
                    let RelocationTarget::Symbol(index) = relocation.target() else { panic!() };
                    let symbol = file.symbol_by_index(index).unwrap();
                    let addend = relocation.addend();
                    let target = match symbol.section_index() {
                        Some(index) => format!("{:?}+{}", tag(file, index), symbol.address()),
                        None => match globals.get(symbol.name().unwrap()) {
                            Some((tag, at)) => format!("{tag:?}+{at}"),
                            None => symbol.name().unwrap().to_owned(),
                        },
                    };
                    out.push((tag(file, section.index()), offset, target, addend));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn a_functions_unwind_entry_goes_with_it() {
        let mut object = Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
        let mut add = |name: &str, kind, data: Vec<u8>| {
            let id = object.add_section(Vec::new(), name.as_bytes().to_vec(), kind);
            object.set_section_data(id, data, 8);
            id
        };
        // Large enough that c and the table are in the second part, until the table goes to a.
        let a = add(".text.a", SectionKind::Text, vec![0xc3; 1024]);
        let c = add(".text.c", SectionKind::Text, vec![0xc3; 512]);
        let table = add(".gcc_except_table", SectionKind::ReadOnlyData, vec![0xff; 8]);
        let word = |it: u32| it.to_le_bytes();
        let mut eh = Vec::new();
        // A common entry, whose personality is at 8; then a's entry, with its table at 32, and
        // c's.
        eh.extend([word(12), word(0), word(0), word(0)].concat());
        eh.extend([word(16), word(20), word(0), word(512), word(0)].concat());
        eh.extend([word(12), word(40), word(0), word(512)].concat());
        let eh_frame = add(".eh_frame", SectionKind::Elf(elf::SHT_X86_64_UNWIND), eh);
        object.section_mut(eh_frame).flags = object::SectionFlags::Elf { sh_flags: 2 };
        let personality = object.add_symbol(Symbol {
            name: b"personality".to_vec(),
            value: 0,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Unknown,
            weak: false,
            section: SymbolSection::Undefined,
            flags: SymbolFlags::None,
        });
        let pc32 = RelocationFlags::Elf { r_type: elf::R_X86_64_PC32 };
        for (offset, symbol) in [
            (8, personality),
            (24, object.section_symbol(a)),
            (32, object.section_symbol(table)),
            (44, object.section_symbol(c)),
        ] {
            let relocation = Relocation { offset, symbol, addend: 0, flags: pc32 };
            object.add_relocation(eh_frame, relocation).unwrap();
        }
        let whole = object.write().unwrap();

        let archive = super::split(&whole, 2).unwrap();
        let archive_file = ArchiveFile::parse(&*archive).unwrap();
        let (mut functions, mut tables) = (Vec::new(), 0);
        for member in archive_file.members() {
            let bytes = member.unwrap().data(&*archive).unwrap();
            let file = object::File::parse(bytes).unwrap();
            let Some(eh_frame) = file.section_by_name(".eh_frame").filter(|it| it.size() > 0)
            else {
                continue;
            };
            tables += 1;
            let data = eh_frame.data().unwrap();
            let relocations: FxHashMap<u64, String> = eh_frame
                .relocations()
                .map(|(offset, it)| {
                    let RelocationTarget::Symbol(index) = it.target() else { panic!() };
                    let symbol = file.symbol_by_index(index).unwrap();
                    let name = match symbol.section_index() {
                        Some(index) => file.section_by_index(index).unwrap().name().unwrap(),
                        None => symbol.name().unwrap(),
                    };
                    (offset, name.to_owned())
                })
                .collect();
            let word = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
            let mut start = 0;
            while start < data.len() {
                let id = word(start + 4) as usize;
                if id != 0 {
                    let common = start + 4 - id;
                    assert_eq!(word(common + 4), 0, "an entry names its common entry");
                    assert_eq!(relocations[&(common as u64 + 8)], "personality");
                    let function = relocations[&(start as u64 + 8)].clone();
                    if function == ".text.a" {
                        assert_eq!(relocations[&(start as u64 + 16)], ".gcc_except_table");
                    }
                    functions.push(function);
                }
                start += 4 + word(start) as usize;
            }
        }
        functions.sort();
        assert_eq!(functions, [".text.a", ".text.c"]);
        assert_eq!(tables, 2, "the two functions are in parts of their own");
    }

    #[test]
    fn the_parts_refer_to_what_the_object_did() {
        let mut object = Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
        let mut sections = Vec::new();
        for (at, name) in
            [".text.a", ".text.b", ".text.c", ".rodata.d", ".debug_info"].iter().enumerate()
        {
            let kind = if name.starts_with(".text") {
                SectionKind::Text
            } else if *name == ".debug_info" {
                SectionKind::Debug
            } else {
                SectionKind::ReadOnlyData
            };
            let segment = object.segment_name(StandardSegment::Text).to_vec();
            let id = object.add_section(segment, name.as_bytes().to_vec(), kind);
            object.set_section_data(id, vec![at as u8 + 1; 64], 16);
            sections.push(id);
        }
        let symbol = |object: &mut Object<'_>, name: &str, section, scope| {
            object.add_symbol(Symbol {
                name: name.as_bytes().to_vec(),
                value: 8,
                size: 8,
                kind: SymbolKind::Text,
                scope,
                weak: false,
                section: SymbolSection::Section(section),
                flags: SymbolFlags::None,
            })
        };
        let a = symbol(&mut object, "a", sections[0], SymbolScope::Dynamic);
        let b = symbol(&mut object, "local_b", sections[1], SymbolScope::Compilation);
        let c = symbol(&mut object, "hidden_c", sections[2], SymbolScope::Linkage);
        let memcpy = object.add_symbol(Symbol {
            name: b"memcpy".to_vec(),
            value: 0,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Unknown,
            weak: false,
            section: SymbolSection::Undefined,
            flags: SymbolFlags::None,
        });
        let rodata = object.section_symbol(sections[3]);
        let pc32 = RelocationFlags::Elf { r_type: elf::R_X86_64_PC32 };
        let abs64 = RelocationFlags::Elf { r_type: elf::R_X86_64_64 };
        let mut relocate = |section: usize, offset, symbol, addend, flags| {
            object
                .add_relocation(sections[section], Relocation { offset, symbol, addend, flags })
                .unwrap();
        };
        relocate(0, 0, b, -4, pc32);
        relocate(0, 8, c, -4, pc32);
        relocate(0, 16, rodata, 12, pc32);
        relocate(1, 0, memcpy, -4, pc32);
        relocate(2, 0, a, -4, pc32);
        relocate(2, 8, rodata, 40, pc32);
        relocate(4, 0, a, 0, abs64);
        relocate(4, 8, b, 0, abs64);
        relocate(4, 16, rodata, 3, abs64);
        let whole = object.write().unwrap();

        // Four parts past the first: each section of code or data is a part of its own.
        let archive = super::split(&whole, 4).unwrap();
        assert!(super::is_archive(&archive));
        let archive_file = ArchiveFile::parse(&*archive).unwrap();
        let members: Vec<Vec<u8>> = archive_file
            .members()
            .map(|it| it.unwrap().data(&*archive).unwrap().to_vec())
            .collect();
        assert_eq!(members.len(), 5);
        assert_eq!(reached(&members), reached(&[whole]));
        // What each defines once: the local is a hidden global where another part refers to it.
        let mut names: Vec<String> = Vec::new();
        for member in &members {
            let file = object::File::parse(&**member).unwrap();
            for symbol in file.symbols().filter(|it| !it.is_undefined() && !it.is_local()) {
                names.push(symbol.name().unwrap().to_owned());
            }
        }
        names.sort();
        assert!(names.contains(&"a".to_owned()) && names.contains(&"hidden_c".to_owned()));
        assert!(names.iter().any(|it| it.starts_with("local_b.part")));
        assert!(names.windows(2).all(|it| it[0] != it[1]), "{names:?}");
    }
}
