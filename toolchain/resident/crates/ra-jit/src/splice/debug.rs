//! What a debugger reads of a spliced executable: the lines of the patch's code.
//!
//! The sections a debugger reads are not loaded, and each is one run of the file that whoever
//! reads it finds through the section headers. A section cannot be added to where it is, the
//! next one follows it, so each section the patch has a part of is written again behind the
//! program headers: the executable's, then the patch's, which describes a unit of its own and
//! refers to the other sections by how far into them its part now is. The section headers
//! follow, with those sections where they now are.
//!
//! What the executable says of the functions the patch replaces stays: it is true of their
//! old code, which is still there behind the jump.
//!
//! The sections are copied there once the executable is linked ([`Described::laid_out`]),
//! each with room behind it, and the section headers point at the copies from then on: the
//! linker's are let go of, so the file holds each section once also where the filesystem
//! shares no blocks. Every splice finds the sections where the last one left them. One that
//! a patch adds more to than its room takes, or that the code a splice appends reaches, moves
//! behind every other, and so never over a section still to be read; its old place is let go
//! of. Of a program with hundreds of megabytes of them, the first copy is seconds.

use anyhow::{Context, bail, ensure};
use object::{
    Endianness, Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationTarget,
    SectionIndex, SymbolKind, elf,
    read::elf::{ElfFile64, FileHeader, SectionHeader},
};
use rustc_hash::FxHashMap;

use super::{Machine, Patch, Target};

/// The section headers of an executable, and the sections among them a debugger reads.
pub(super) struct Described {
    headers: Vec<elf::SectionHeader64<Endianness>>,
    /// Where the linker put the section headers.
    table: u64,
    sections: FxHashMap<Box<[u8]>, usize>,
    /// Which of the headers is of the section their names are in.
    names: usize,
    /// The sections a splice writes again behind everything else, in the order the file has
    /// them, whether or not a patch adds to each: those a debugger reads, but not one that
    /// is loaded or compressed.
    laid: Vec<usize>,
}

/// What a section is given to grow into, at the least, before the one behind it moves.
const ROOM: u64 = 1 << 20;

/// A filesystem block: a run written again as far into a block as it was can share the
/// blocks of the original, where the filesystem shares blocks between files.
pub(super) const BLOCK: u64 = 4096;

/// A run of the file that is written again further on.
#[derive(PartialEq)]
pub(super) struct Moved {
    pub(super) from: u64,
    pub(super) len: u64,
    pub(super) to: u64,
}

/// Where each section of [`Described::laid`] is copied to, and where the room behind it ends.
#[derive(Clone, Default)]
pub(super) struct Copies {
    at: Vec<(u64, u64)>,
    /// Where the last room ends: what is behind it is written again by every splice.
    end: u64,
}

/// What a splice writes behind everything else for a debugger.
#[derive(Default)]
pub(super) struct Tail {
    /// What is copied further on before anything else is written: what the patch outgrew or
    /// the code it appends reaches. Each run is let go of once copied.
    pub(super) moved: Vec<Moved>,
    /// What is copied behind the last room from where the linker left it, which stays.
    pub(super) copied: Vec<Moved>,
    pub(super) written: Vec<(u64, Vec<u8>)>,
    /// Where the section headers are and how many, when they are among what is written.
    pub(super) headers: Option<(u64, u16)>,
    /// Where the sections are once the splice is through.
    pub(super) copies: Copies,
}

/// What a patch adds to the sections a debugger reads.
struct Added {
    headers: Vec<elf::SectionHeader64<Endianness>>,
    /// The names of the sections the executable has none of.
    names: Vec<u8>,
    /// By the header of the section.
    data: FxHashMap<usize, Vec<u8>>,
    /// The sections the patch has that are not laid, in the order they come.
    others: Vec<usize>,
}

impl Described {
    pub(super) fn read(file: &ElfFile64<'_, Endianness>) -> anyhow::Result<Described> {
        let endian = file.endian();
        let headers = file.elf_header().section_headers(endian, file.data())?.to_vec();
        let mut sections = FxHashMap::default();
        for section in file.sections() {
            let name = section.name_bytes()?;
            if is_debug(name) {
                sections.insert(name.into(), section.index().0);
            }
        }
        let names = file.elf_header().shstrndx(endian, file.data())? as usize;
        ensure!(names < headers.len(), "the names of the sections are in none of them");
        let flags = u64::from(elf::SHF_ALLOC | elf::SHF_COMPRESSED);
        let mut laid: Vec<usize> = (sections.values().copied())
            .filter(|&it| {
                headers[it].sh_flags(endian) & flags == 0 && headers[it].sh_size(endian) != 0
            })
            .collect();
        laid.sort_unstable_by_key(|&it| headers[it].sh_offset(endian));
        let table = file.elf_header().e_shoff(endian);
        Ok(Described { headers, table, sections, names, laid })
    }

    fn says_lines(&self) -> bool {
        self.sections.contains_key(&b".debug_info"[..])
    }

    /// The runs a splice copies behind `offset` before a patch adds anything, and where they
    /// are then: the splices that come find them there. Nothing, of an executable that tells
    /// a debugger nothing.
    pub(super) fn laid_out(&self, offset: u64) -> (Vec<Moved>, Copies) {
        let mut copies = Copies { at: Vec::new(), end: offset };
        if !self.says_lines() {
            return (Vec::new(), copies);
        }
        let e = Endianness::Little;
        let moved = (self.laid.iter())
            .map(|&it| {
                let (from, len) = (self.headers[it].sh_offset(e), self.headers[it].sh_size(e));
                Moved { from, len, to: copies.behind(from, len, 0) }
            })
            .collect();
        (moved, copies)
    }

    /// The sections where the linker left them, with no room behind any: what a splice finds
    /// when they could not be laid out.
    pub(super) fn as_linked(&self) -> Copies {
        let e = Endianness::Little;
        let at = (self.laid.iter())
            .map(|&it| {
                let (from, len) = (self.headers[it].sh_offset(e), self.headers[it].sh_size(e));
                (from, from + len)
            })
            .collect();
        Copies { at, end: 0 }
    }

    /// Where the linker put the section headers, and them with the sections where `copies`
    /// has them: what a debugger finds of a file no splice has written over.
    pub(super) fn pointed(&self, copies: &Copies) -> (u64, Vec<u8>) {
        let e = Endianness::Little;
        let mut headers = self.headers.clone();
        for (&index, &(at, _)) in self.laid.iter().zip(&copies.at) {
            headers[index].sh_offset.set(e, at);
        }
        (self.table, object::bytes_of_slice(&headers).to_vec())
    }
}

impl Copies {
    /// Places a section of `len` bytes, of which `added` a patch adds, behind every room,
    /// as far into a block as `from` is, with room behind it: the more there is of it the
    /// more room.
    fn behind(&mut self, from: u64, len: u64, added: u64) -> u64 {
        let to = self.end.next_multiple_of(ROOM) + from % BLOCK;
        let end = to + len + added + ROOM.max((len + added) / 16);
        self.at.push((to, end));
        self.end = end;
        to
    }

    /// Where the sections are for a patch that adds `added` to each and appends code up to
    /// `offset`, and what moves: each one the code reaches or whose room what is added
    /// outgrows, behind every room, where nothing is still to be read.
    fn make_room(&self, lens: &[u64], added: &[u64], offset: u64) -> (Copies, Vec<Moved>) {
        let mut copies =
            Copies { at: Vec::with_capacity(self.at.len()), end: self.end.max(offset) };
        let mut moved = Vec::new();
        for ((&(at, room), &len), &added) in self.at.iter().zip(lens).zip(added) {
            if at >= offset && at + len + added <= room {
                copies.at.push((at, room));
            } else {
                moved.push(Moved { from: at, len, to: copies.behind(at, len, added) });
            }
        }
        (copies, moved)
    }
}

fn is_debug(name: &[u8]) -> bool {
    name.starts_with(b".debug_")
}

impl Machine {
    /// How many bytes the place in a section a debugger reads takes, which a relocation of
    /// the type fills in.
    fn described(self, r_type: u32) -> anyhow::Result<usize> {
        Ok(match (self, r_type) {
            (Machine::X86_64, elf::R_X86_64_64) | (Machine::Aarch64, elf::R_AARCH64_ABS64) => 8,
            (Machine::X86_64, elf::R_X86_64_32) | (Machine::Aarch64, elf::R_AARCH64_ABS32) => 4,
            (_, other) => bail!("a relocation of type {other} in what a debugger reads"),
        })
    }
}

impl Patch<'_> {
    /// The sections of the patch a debugger reads.
    pub(super) fn described(
        &self,
    ) -> impl Iterator<Item = object::read::elf::ElfSection64<'_, '_, Endianness>> {
        self.file.sections().filter(|it| it.size() != 0 && it.name_bytes().is_ok_and(is_debug))
    }

    /// What goes from `offset` of the file on so that a debugger finds the lines of the
    /// patch's code, whose addresses `address` gives, with the sections where `copies` has
    /// them. Nothing, for an executable that tells a debugger nothing. A patch whose lines
    /// can't be told has none, and what it appends still moves what it reaches.
    pub(super) fn describe(
        &self,
        described: &Described,
        copies: &Copies,
        machine: Machine,
        offset: u64,
        address: impl Fn(Target, i64) -> anyhow::Result<u64>,
    ) -> Tail {
        let e = Endianness::Little;
        if !described.says_lines() {
            return Tail::default();
        }
        let added = match self.described().next() {
            None => None,
            Some(_) => self.added(described, machine, address).map_or_else(
                |why| {
                    eprintln!("splice: the patch's code has no lines for a debugger: {why:#}");
                    None
                },
                Some,
            ),
        };
        let lens: Vec<u64> =
            described.laid.iter().map(|&it| described.headers[it].sh_size(e)).collect();
        let grown: Vec<u64> = (described.laid.iter())
            .map(|it| {
                added.as_ref().and_then(|added| added.data.get(it)).map_or(0, |it| it.len() as u64)
            })
            .collect();
        let (copies, moved) = copies.make_room(&lens, &grown, offset);
        let mut tail = Tail { moved, copies, ..Tail::default() };
        let Some(Added { mut headers, names, mut data, others }) = added else { return tail };
        for (&index, &(at, _)) in described.laid.iter().zip(&tail.copies.at) {
            let header = &mut headers[index];
            header.sh_offset.set(e, at);
            if let Some(data) = data.remove(&index) {
                let len = header.sh_size(e);
                header.sh_size.set(e, len + data.len() as u64);
                tail.written.push((at + len, data));
            }
        }
        // Behind the last room: what no splice finds where the last one wrote it.
        let mut end = tail.copies.end;
        for index in others {
            let header = &mut headers[index];
            let len = header.sh_size(e);
            if len != 0 {
                tail.copied.push(Moved { from: header.sh_offset(e), len, to: end });
            }
            header.sh_offset.set(e, end);
            end += len;
            if let Some(data) = data.remove(&index) {
                header.sh_size.set(e, len + data.len() as u64);
                end += data.len() as u64;
                tail.written.push((end - data.len() as u64, data));
            }
        }
        if !names.is_empty() {
            let header = &mut headers[described.names];
            let named = header.sh_size(e);
            tail.copied.push(Moved { from: header.sh_offset(e), len: named, to: end });
            header.sh_offset.set(e, end);
            header.sh_size.set(e, named + names.len() as u64);
            tail.written.push((end + named, names));
            end += header.sh_size(e);
        }
        let end = end.next_multiple_of(8);
        let count = u16::try_from(headers.len()).ok().filter(|&it| it < elf::SHN_LORESERVE);
        let Some(count) = count else {
            eprintln!(
                "splice: the patch's code has no lines for a debugger: more sections than the file's header counts"
            );
            tail.written.clear();
            tail.copied.clear();
            return tail;
        };
        tail.written.push((end, object::bytes_of_slice(&headers).to_vec()));
        tail.headers = Some((end, count));
        tail
    }

    /// The section headers with those the patch has that the executable has not, and what the
    /// patch adds to each section, its relocations applied.
    fn added(
        &self,
        described: &Described,
        machine: Machine,
        address: impl Fn(Target, i64) -> anyhow::Result<u64>,
    ) -> anyhow::Result<Added> {
        let e = Endianness::Little;
        let mut headers = described.headers.clone();
        let mut sections = described.sections.clone();
        let mut names = Vec::new();
        let named = headers[described.names].sh_size(e);
        let mut theirs = FxHashMap::<SectionIndex, u64>::default();
        for section in self.described() {
            let name = section.name_bytes()?;
            let shown = String::from_utf8_lossy(name);
            let index = *sections.entry(name.into()).or_insert_with(|| {
                let at = named + names.len() as u64;
                names.extend_from_slice(name);
                names.push(0);
                headers.push(empty(section.elf_section_header(), at as u32));
                headers.len() - 1
            });
            let header = &headers[index];
            ensure!(
                header.sh_flags(e) & u64::from(elf::SHF_COMPRESSED) == 0,
                "the executable's {shown} is compressed"
            );
            theirs.insert(section.index(), header.sh_size(e));
        }
        let mut data = FxHashMap::<usize, Vec<u8>>::default();
        let mut others = Vec::new();
        for section in self.described() {
            let mut bytes = section.data()?.to_vec();
            for (at, relocation) in section.relocations() {
                let RelocationFlags::Elf { r_type } = relocation.flags() else {
                    bail!("a relocation that is not ELF's")
                };
                let RelocationTarget::Symbol(index) = relocation.target() else {
                    bail!("a relocation against no symbol")
                };
                let symbol = self.file.symbol_by_index(index)?;
                let within = symbol.section_index().and_then(|it| theirs.get(&it));
                let value = match within {
                    Some(len) if symbol.kind() == SymbolKind::Section => {
                        len.wrapping_add_signed(relocation.addend())
                    }
                    Some(_) => bail!("a symbol in what a debugger reads"),
                    None => {
                        let target = self.targets.get(&index).context("code nothing else names")?;
                        address(*target, relocation.addend())?
                    }
                };
                let len = machine.described(r_type)?;
                let field = bytes.get_mut(at as usize..at as usize + len);
                let field = field.context("a relocation past its section")?;
                if len == 4 {
                    let value = u32::try_from(value).context("more than a debugger is told of")?;
                    field.copy_from_slice(&value.to_le_bytes());
                } else {
                    field.copy_from_slice(&value.to_le_bytes());
                }
            }
            let index = sections[section.name_bytes()?];
            if !described.laid.contains(&index) && !others.contains(&index) {
                others.push(index);
            }
            data.insert(index, bytes);
        }
        Ok(Added { headers, names, data, others })
    }
}

/// The header of a section the executable has none of yet, like the patch's and with nothing
/// in it, named by what is `name` into the names of the sections.
fn empty(like: &elf::SectionHeader64<Endianness>, name: u32) -> elf::SectionHeader64<Endianness> {
    let e = Endianness::Little;
    elf::SectionHeader64 {
        sh_name: object::U32::new(e, name),
        sh_type: like.sh_type,
        sh_flags: like.sh_flags,
        sh_addr: object::U64::new(e, 0),
        sh_offset: object::U64::new(e, 0),
        sh_size: object::U64::new(e, 0),
        sh_link: object::U32::new(e, 0),
        sh_info: object::U32::new(e, 0),
        sh_addralign: object::U64::new(e, 1),
        sh_entsize: like.sh_entsize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sizes like those of nushell's debug sections, as the linker left them.
    const NUSHELL: [(u64, u64); 6] = [
        (0x7619545, 0x25ce04),
        (0x7876349, 0xc118872),
        (0x1398ebbb, 0x2c92214),
        (0x16620dcf, 0x1e16c50),
        (0x18437a1f, 0xca5992d),
        (0x24e9134c, 0x48351d),
    ];
    const OFFSET: u64 = 0x2a00_0000;

    fn laid() -> (Vec<Moved>, Copies) {
        let mut copies = Copies { at: Vec::new(), end: OFFSET };
        let moved = (NUSHELL.iter())
            .map(|&(from, len)| Moved { from, len, to: copies.behind(from, len, 0) })
            .collect();
        (moved, copies)
    }

    fn lens() -> Vec<u64> {
        NUSHELL.iter().map(|&(_, len)| len).collect()
    }

    fn added(added: &[u64]) -> Vec<u64> {
        added.iter().copied().chain(std::iter::repeat(0)).take(NUSHELL.len()).collect()
    }

    #[test]
    fn what_fits_in_the_room_leaves_every_section_where_it_was_laid_out() {
        let (_, copies) = laid();
        let grown = added(&[0x1000, 0x8_0000, 0x4_0000, ROOM, 0x20_0000, 0x100]);
        let (after, moved) = copies.make_room(&lens(), &grown, OFFSET);
        assert!(moved.is_empty());
        assert_eq!(after.at, copies.at);
        assert_eq!(after.end, copies.end);
    }

    #[test]
    fn a_section_that_outgrows_its_room_alone_moves_behind_every_room() {
        let (_, copies) = laid();
        let (after, moved) = copies.make_room(&lens(), &added(&[0, 0, 0x100_0000]), OFFSET);
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].from, copies.at[2].0);
        assert!(moved[0].to >= copies.end);
        for (k, (before, now)) in copies.at.iter().zip(&after.at).enumerate() {
            assert_eq!(k == 2, before != now, "section {k}");
        }
        // The next splice finds it where it went, and with the same addition it stays.
        let (again, moved) = after.make_room(&lens(), &added(&[0, 0, 0x100_0000]), OFFSET);
        assert!(moved.is_empty());
        assert_eq!(again.at, after.at);
    }

    #[test]
    fn what_the_code_reaches_moves_and_no_copy_is_written_over_before_it_is_read() {
        let (_, copies) = laid();
        let offset = copies.at[1].0 + 1;
        let (after, moved) = copies.make_room(&lens(), &added(&[]), offset);
        assert_eq!(
            moved.iter().map(|it| it.from).collect::<Vec<_>>(),
            [copies.at[0].0, copies.at[1].0]
        );
        let live: Vec<(u64, u64)> =
            copies.at.iter().zip(lens()).map(|(&(at, _), len)| (at, at + len)).collect();
        for run in &moved {
            assert!(run.to >= offset);
            assert!(live.iter().all(|&(start, end)| run.to + run.len <= start || end <= run.to));
        }
        assert!(after.at.iter().all(|&(at, _)| at >= offset));
    }

    #[test]
    fn each_starts_as_far_into_a_block_as_it_was_and_behind_the_one_before() {
        let (moved, copies) = laid();
        let mut end = OFFSET;
        for (run, &(at, room)) in moved.iter().zip(&copies.at) {
            assert_eq!(run.to, at);
            assert_eq!(at % BLOCK, run.from % BLOCK);
            assert!(at >= end, "{at:#x} is before {end:#x}");
            assert!(room >= at + run.len + ROOM);
            end = room;
        }
        let (after, moved) =
            copies.make_room(&lens(), &added(&[0, 0x200_0000, 0, 0x200_0000]), OFFSET);
        for run in &moved {
            assert_eq!(run.to % BLOCK, run.from % BLOCK);
        }
        assert!(after.end > copies.end);
    }

    #[test]
    fn what_the_linker_left_moves_whole_on_the_first_splice() {
        let (_, mut copies) = laid();
        copies.at = NUSHELL.iter().map(|&(from, len)| (from, from + len)).collect();
        copies.end = 0;
        let (after, moved) = copies.make_room(&lens(), &added(&[]), OFFSET);
        assert_eq!(moved.len(), NUSHELL.len());
        assert!(after.at.iter().all(|&(at, _)| at >= OFFSET));
    }
}
