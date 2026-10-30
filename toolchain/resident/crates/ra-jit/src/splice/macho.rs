//! The splice into a Mach-O executable for AArch64.
//!
//! What differs from ELF is where things have to be. The loader wants `__LINKEDIT` to be the
//! last segment and the signature to be the last of it, and the kernel runs no page the
//! signature has no hash of. So the patch goes where `__LINKEDIT` was, in two segments of
//! its own, and `__LINKEDIT` is written again behind them:
//!
//! 1. `__RA_JIT_TEXT`, read and run: the patch's constants and functions, and a stub for
//!    each call that is out of a branch's reach.
//! 2. `__RA_JIT_DATA`, read and written: the patch's writable sections and those that store
//!    addresses, and an address slot for each symbol the code reaches through one.
//! 3. `__LINKEDIT`: as the linker wrote it but for the addresses the loader moves with the
//!    program, which are encoded again, and for the signature, which keeps the hashes of
//!    the pages that are as they were.
//!
//! The unwinder gets `__unwind_info` and `__eh_frame` from dyld, which on some versions of
//! macOS finds them only in the segment named `__TEXT`, by the name of the segment's load
//! command. So the patch's frames go there too: behind the program's when `__TEXT` has room
//! after `__eh_frame`, so the section only grows and no offset of `__unwind_info` moves.
//! Otherwise `__unwind_info`, which the linker puts right in front of `__eh_frame`, moves
//! into the room the header leaves, and the patch's frames take its place, so `__eh_frame`
//! starts with them and no section overlaps another, which `strip` would refuse; the table
//! is told the program's frames are that much further from where the section starts, which
//! its 24-bit offsets can't say past 16 MB. When the patch's frames are larger than the table,
//! both tables move into the header's room: the table as it is, and `__eh_frame` with the
//! program's frames where the table has them and the patch's behind. Each way the table has
//! nothing for the patch's functions, which are past its last entry, so the unwinder looks
//! for them in `__eh_frame`. A program linked with `-no_compact_unwind` has no table, and the
//! unwinder looks for every function in `__eh_frame`: the patch's frames go behind the
//! program's, or both move into the header's room.
//!
//! The kernel remembers the signature of a file it has run, so the executable is not
//! written where it is: a copy is, which the file system makes without copying where it
//! can, and takes the place of the executable when it is whole.

use std::{
    fs::{File, OpenOptions},
    path::Path,
    sync::atomic::AtomicBool,
};

use anyhow::{Context, bail, ensure};
use object::{
    Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationTarget, SectionFlags,
    SectionIndex, SymbolIndex, SymbolKind, macho,
};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{
    Spliced, Stamp,
    lasting::{Lasting, Own, is_lasting, left_behind},
};

/// What segments are aligned to on AArch64.
const PAGE: u64 = 0x4000;
const HEADER: usize = size_of::<macho::MachHeader64<object::LittleEndian>>();
const SEGMENT: usize = size_of::<macho::SegmentCommand64<object::LittleEndian>>();
const SECTION: usize = size_of::<macho::Section64<object::LittleEndian>>();
const NLIST: usize = size_of::<macho::Nlist64<object::LittleEndian>>();

fn u32_at(bytes: &[u8], at: usize) -> anyhow::Result<u32> {
    let field = bytes.get(at..at + 4).context("the file ends early")?;
    Ok(u32::from_le_bytes(field.try_into()?))
}

fn u16_at(bytes: &[u8], at: usize) -> anyhow::Result<u16> {
    let field = bytes.get(at..at + 2).context("the file ends early")?;
    Ok(u16::from_le_bytes(field.try_into()?))
}

fn u64_at(bytes: &[u8], at: usize) -> anyhow::Result<u64> {
    let field = bytes.get(at..at + 8).context("the file ends early")?;
    Ok(u64::from_le_bytes(field.try_into()?))
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn uleb(bytes: &[u8], at: &mut usize) -> anyhow::Result<u64> {
    let (mut value, mut shift) = (0u64, 0);
    loop {
        let byte = *bytes.get(*at).context("a number that ends early")?;
        *at += 1;
        value |= u64::from(byte & 0x7f).checked_shl(shift).context("a number too large")?;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}

fn push_uleb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        out.push(if value == 0 { byte } else { byte | 0x80 });
        if value == 0 {
            return;
        }
    }
}

/// What a splice has to know of the executable. Nothing of the file is read again but the
/// pages a splice writes to, for their hashes.
pub(super) struct Image {
    /// The header and the load commands.
    commands: Vec<u8>,
    /// Where the first section starts in the file: the load commands end before it.
    room: usize,
    segments: Vec<Segment>,
    /// How many sections the segments have.
    sectioned: usize,
    /// Where `commands` has what refers into `__LINKEDIT`.
    at: Commands,
    symbols: FxHashMap<Box<[u8]>, Named>,
    imports: FxHashMap<Box<[u8]>, Import>,
    unwind: Table,
    frames: Table,
    /// How much is in front of `__eh_frame` that only `__unwind_info` takes: the room the
    /// patch's frames get when the table moves into the header's.
    in_front: u64,
    /// How much of `__TEXT` is behind `__eh_frame` and no section's: the room the patch's
    /// frames get behind the program's, where no offset of the table moves.
    behind: u64,
    /// The addresses the loader adds where the program is loaded to, in order.
    rebases: Vec<u64>,
    /// `__LINKEDIT` up to the signature.
    linkedit: Vec<u8>,
    signature: Signature,
}

struct Segment {
    address: u64,
    size: u64,
    offset: u64,
    file_size: u64,
    writable: bool,
}

#[derive(Default)]
struct Commands {
    linkedit: usize,
    symbols: usize,
    indirect: usize,
    fixups: usize,
    signature: usize,
    /// The commands that have one offset into `__LINKEDIT` and its length.
    data: Vec<usize>,
}

/// A section the unwinder finds by its name.
#[derive(Default)]
struct Table {
    /// Where `commands` has its header.
    header: usize,
    /// Its number among the sections, from 0, and that of the first of its segment.
    nth: usize,
    first: usize,
    address: u64,
    bytes: Vec<u8>,
}

enum Named {
    One(Defined),
    /// Symbols private to their objects that share the name: which one is meant is lost.
    Many,
}

#[derive(Clone, Copy, PartialEq)]
struct Defined {
    address: u64,
    /// As far as the next symbol or the end of the section: no symbol has a size here.
    size: u64,
    kind: SymbolKind,
    /// Where the file has its entry of the symbol table.
    entry: u64,
}

/// What another library defines, by how the executable reaches it.
#[derive(Clone, Copy, Default)]
struct Import {
    /// The address slot the loader fills when it loads the program.
    slot: Option<u64>,
    /// The code that jumps to it.
    stub: Option<u64>,
}

/// The signature of the file: a directory of one hash for each page of the file, as the
/// linker makes it (of pages of 4 KB), or as `codesign` does (of 16 KB) with the other
/// blobs it adds (requirements, entitlements), which the directory has hashes of ahead of
/// those of the pages and which are kept as they are.
struct Signature {
    /// The directory up to its hashes.
    directory: Vec<u8>,
    hashes: Vec<u8>,
    /// How many bytes each hash is of.
    page: usize,
    /// The blobs besides the directory, by their kind, in the order they were.
    others: Vec<(u32, Vec<u8>)>,
    /// Where among them the directory was.
    at: usize,
}

impl Signature {
    const HASH: usize = 32;
    /// The header of the blob of blobs, and what it says of each blob in it.
    const HEADER: usize = 12;
    const ENTRY: usize = 8;
    const BLOBS: u32 = 0xfade_0cc0;
    const DIRECTORY: u32 = 0xfade_0c02;
    /// Where the directory says how long it is, how many hashes it has, and of how much.
    const LENGTH: usize = 4;
    const SLOTS: usize = 28;
    const LIMIT: usize = 32;

    fn read(blob: &[u8]) -> anyhow::Result<Signature> {
        let be = |bytes: &[u8], at: usize| -> anyhow::Result<u32> {
            let field = bytes.get(at..at + 4).context("a signature that ends early")?;
            Ok(u32::from_be_bytes(field.try_into()?))
        };
        let foreign = "a signature of another shape than the linker's or codesign's";
        ensure!(be(blob, 0)? == Self::BLOBS, foreign);
        let (mut directory, mut others, mut at) = (None, Vec::new(), 0);
        for nth in 0..be(blob, 8)? as usize {
            let (kind, offset) =
                (be(blob, Self::HEADER + nth * 8)?, be(blob, Self::HEADER + nth * 8 + 4)? as usize);
            let len = be(blob, offset + 4)? as usize;
            let content = blob.get(offset..offset + len).context(foreign)?;
            match kind {
                0 => {
                    (directory, at) = (Some(content), nth);
                }
                // Another directory, of other hashes, would have to be written again too.
                0x1000.. if kind != 0x10000 => bail!("a signature with a second directory"),
                _ => others.push((kind, content.to_vec())),
            }
        }
        let directory = directory.context(foreign)?;
        ensure!(be(directory, 0)? == Self::DIRECTORY, foreign);
        let (hashes, slots) = (be(directory, 16)? as usize, be(directory, Self::SLOTS)? as usize);
        // Hashes of 32 bytes, SHA-256, of pages of 4 or 16 KB.
        let shape = directory.get(36..40).context(foreign)?;
        ensure!(shape[0] == 32 && shape[1] == 2 && matches!(shape[3], 12 | 14), foreign);
        Ok(Signature {
            directory: directory.get(..hashes).context(foreign)?.to_vec(),
            hashes: directory.get(hashes..hashes + slots * Self::HASH).context(foreign)?.to_vec(),
            page: 1 << shape[3],
            others,
            at,
        })
    }

    /// How long the signature of a file is that has `limit` bytes before it.
    fn len(&self, limit: u64) -> usize {
        let pages = (limit as usize).div_ceil(self.page);
        let others: usize = self.others.iter().map(|(_, it)| it.len()).sum();
        let entries = Self::HEADER + Self::ENTRY * (self.others.len() + 1);
        (entries + self.directory.len() + pages * Self::HASH + others).next_multiple_of(16)
    }

    /// The signature of `file`, whose first `kept` bytes are what the linker wrote but for
    /// `dirty`, and `added` after them, and the hashes of the pages of `added`. A page that
    /// the last splice appended at the same place as it is now has the hash it had then.
    fn of(
        &self,
        file: &File,
        kept: u64,
        dirty: &[(u64, Vec<u8>)],
        added: &[u8],
        last: Option<&Appended>,
        give_up: &AtomicBool,
    ) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let limit = kept + added.len() as u64;
        let page_len = self.page;
        let last = last.filter(|it| it.offset == kept && it.page == page_len);
        ensure!(kept as usize % page_len == 0, "a splice behind a page the signature hashes");
        let kept = kept as usize / page_len;
        let mut hashes =
            self.hashes.get(..kept * Self::HASH).context("a short signature")?.to_vec();
        let mut page = vec![0; page_len];
        let pages: FxHashSet<usize> = (dirty.iter())
            .flat_map(|(at, bytes)| {
                *at as usize / page_len..=(*at as usize + bytes.len().max(1) - 1) / page_len
            })
            .filter(|page| *page < kept)
            .collect();
        for index in pages {
            super::read_exact_at(&file, &mut page, (index * page_len) as u64)?;
            hashes[index * Self::HASH..][..Self::HASH]
                .copy_from_slice(&super::sha256::digest(&page));
        }
        // Every page appended, tens of megabytes for a large debug build: without the
        // processor's SHA-256 instructions the pages are hashed on several threads.
        let pages: Vec<(usize, &[u8])> = added.chunks(page_len).enumerate().collect();
        let threads = match super::sha256::fast() {
            true => 1,
            false => std::thread::available_parallelism().map_or(1, |it| it.get()).min(8),
        };
        let part = pages.len().div_ceil(threads).max(1);
        let parts: Vec<Result<Vec<u8>, super::GaveUp>> = std::thread::scope(|scope| {
            let parts: Vec<_> = (pages.chunks(part))
                .map(|pages| {
                    scope.spawn(move || -> Result<Vec<u8>, super::GaveUp> {
                        let mut hashes = Vec::with_capacity(pages.len() * Self::HASH);
                        for (index, &(nth, page)) in pages.iter().enumerate() {
                            if index % 256 == 0 {
                                super::go_on(give_up)?;
                            }
                            match last.and_then(|it| it.hash_of(nth, page)) {
                                Some(hash) => hashes.extend_from_slice(hash),
                                None => hashes.extend_from_slice(&super::sha256::digest(page)),
                            }
                        }
                        Ok(hashes)
                    })
                })
                .collect();
            parts.into_iter().map(|it| it.join().expect("hashing pages panicked")).collect()
        });
        let appended_from = hashes.len();
        for part in parts {
            hashes.extend_from_slice(&part?);
        }
        let appended = hashes[appended_from..].to_vec();

        let mut directory = self.directory.clone();
        let length = u32::try_from(directory.len() + hashes.len())?;
        directory[Self::LENGTH..][..4].copy_from_slice(&length.to_be_bytes());
        let slots = (hashes.len() / Self::HASH) as u32;
        directory[Self::SLOTS..][..4].copy_from_slice(&slots.to_be_bytes());
        let limit = u32::try_from(limit).context("an executable of more than 4 GB")?;
        directory[Self::LIMIT..][..4].copy_from_slice(&limit.to_be_bytes());
        directory.extend_from_slice(&hashes);

        let mut blobs: Vec<(u32, &[u8])> =
            self.others.iter().map(|(kind, it)| (*kind, it.as_slice())).collect();
        blobs.insert(self.at.min(blobs.len()), (0, &directory));
        let len = self.len(u64::from(limit));
        let mut signature = Vec::with_capacity(len);
        let total: usize = blobs.iter().map(|(_, it)| it.len()).sum();
        let entries = Self::HEADER + Self::ENTRY * blobs.len();
        for field in [Self::BLOBS, u32::try_from(entries + total)?, blobs.len() as u32] {
            signature.extend_from_slice(&field.to_be_bytes());
        }
        let mut offset = entries;
        for (kind, blob) in &blobs {
            signature.extend_from_slice(&kind.to_be_bytes());
            signature.extend_from_slice(&u32::try_from(offset)?.to_be_bytes());
            offset += blob.len();
        }
        for (_, blob) in &blobs {
            signature.extend_from_slice(blob);
        }
        signature.resize(len, 0);
        Ok((signature, appended))
    }
}

/// What a splice appended behind what the linker wrote, and the hashes of its pages: most of
/// it is `__LINKEDIT` as the linker wrote it, which the next splice appends again.
pub(super) struct Appended {
    offset: u64,
    page: usize,
    bytes: Vec<u8>,
    hashes: Vec<u8>,
}

impl Appended {
    /// The hash of page `nth` of what is appended now, when it was appended last as it is.
    fn hash_of(&self, nth: usize, page: &[u8]) -> Option<&[u8]> {
        let at = nth * self.page;
        let was = self.bytes.get(at..(at + self.page).min(self.bytes.len()))?;
        let hash = self.hashes.get(nth * Signature::HASH..(nth + 1) * Signature::HASH)?;
        (was == page).then_some(hash)
    }
}

/// The opcodes that say which addresses the loader moves (`REBASE_OPCODE_*`).
struct Rebases;

impl Rebases {
    fn read(stream: &[u8], segments: &[Segment]) -> anyhow::Result<Vec<u64>> {
        let mut found = Vec::new();
        let (mut at, mut address) = (0, 0u64);
        let mut rebase = |address: &mut u64, times: u64, skip: u64| {
            for _ in 0..times {
                found.push(*address);
                *address = address.wrapping_add(8 + skip);
            }
        };
        while let Some(&byte) = stream.get(at) {
            at += 1;
            let immediate = u64::from(byte & 0x0f);
            match byte & 0xf0 {
                macho::REBASE_OPCODE_DONE => break,
                macho::REBASE_OPCODE_SET_TYPE_IMM => {
                    ensure!(
                        immediate == u64::from(macho::REBASE_TYPE_POINTER),
                        "code the loader fixes up"
                    );
                }
                macho::REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                    let segment =
                        segments.get(immediate as usize).context("a fixup in no segment")?;
                    address = segment.address + uleb(stream, &mut at)?;
                }
                macho::REBASE_OPCODE_ADD_ADDR_ULEB => {
                    address = address.wrapping_add(uleb(stream, &mut at)?)
                }
                macho::REBASE_OPCODE_ADD_ADDR_IMM_SCALED => address += immediate * 8,
                macho::REBASE_OPCODE_DO_REBASE_IMM_TIMES => rebase(&mut address, immediate, 0),
                macho::REBASE_OPCODE_DO_REBASE_ULEB_TIMES => {
                    rebase(&mut address, uleb(stream, &mut at)?, 0);
                }
                macho::REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB => {
                    rebase(&mut address, 1, uleb(stream, &mut at)?);
                }
                macho::REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB => {
                    let times = uleb(stream, &mut at)?;
                    rebase(&mut address, times, uleb(stream, &mut at)?);
                }
                other => bail!("a fixup of kind {other:#x}"),
            }
        }
        found.sort_unstable();
        Ok(found)
    }

    /// The opcodes for `addresses`, which are in order, by the first address of each segment.
    fn written(addresses: &[u64], segments: &[u64]) -> anyhow::Result<Vec<u8>> {
        let mut stream = vec![macho::REBASE_OPCODE_SET_TYPE_IMM | macho::REBASE_TYPE_POINTER];
        let mut rest = addresses;
        while let Some(&first) = rest.first() {
            let segment = segments.partition_point(|it| *it <= first).checked_sub(1);
            let segment = segment.filter(|it| *it < 16).context("an address in no segment")?;
            let end = segments.get(segment + 1).copied().unwrap_or(u64::MAX);
            let run = (rest.iter().zip(0..))
                .take_while(|(it, nth)| **it == first + nth * 8 && **it < end)
                .count();
            stream.push(macho::REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | segment as u8);
            push_uleb(&mut stream, first - segments[segment]);
            stream.push(macho::REBASE_OPCODE_DO_REBASE_ULEB_TIMES);
            push_uleb(&mut stream, run as u64);
            rest = &rest[run..];
        }
        stream.push(macho::REBASE_OPCODE_DONE);
        stream.resize(stream.len().next_multiple_of(8), 0);
        Ok(stream)
    }
}

impl Image {
    /// Whether a patch can call the C symbol `name`: the executable defines it, or has the
    /// address slot of the library's that defines it, as a patch reaches what it calls.
    pub(super) fn can_call(&self, name: &str) -> bool {
        self.gives(format!("_{name}").as_bytes())
    }

    /// Whether a patch that refers to the symbol `name`, as its object names it, is given
    /// what it names.
    pub(super) fn gives(&self, name: &[u8]) -> bool {
        match self.symbols.get(name) {
            Some(Named::One(defined)) => defined.kind != SymbolKind::Tls,
            Some(Named::Many) => false,
            None => self.imports.get(name).is_some_and(|it| it.slot.is_some()),
        }
    }

    pub(super) fn read(bytes: &[u8]) -> anyhow::Result<Image> {
        ensure!(
            u32_at(bytes, 0)? == macho::MH_MAGIC_64 && u32_at(bytes, 4)? == macho::CPU_TYPE_ARM64,
            "not an AArch64 executable"
        );
        ensure!(u32_at(bytes, 12)? == macho::MH_EXECUTE, "not an executable");
        let count = u32_at(bytes, 16)?;
        let commands =
            bytes.get(..HEADER + u32_at(bytes, 20)? as usize).context("the file ends early")?;

        let mut segments = Vec::new();
        // The address, size and flags of each, and which the indirect symbols start at.
        let mut sections = Vec::new();
        let mut at = Commands::default();
        let (mut unwind, mut frames) = (Table::default(), Table::default());
        let mut room = bytes.len();
        let mut command = HEADER;
        for _ in 0..count {
            let size = u32_at(commands, command + 4)? as usize;
            match u32_at(commands, command)? {
                macho::LC_SEGMENT_64 => {
                    let name = &commands[command + 8..command + 24];
                    // Whichever comes last has to be it.
                    at.linkedit = if name.starts_with(b"__LINKEDIT\0") { command } else { 0 };
                    let segment = Segment {
                        address: u64_at(commands, command + 24)?,
                        size: u64_at(commands, command + 32)?,
                        offset: u64_at(commands, command + 40)?,
                        file_size: u64_at(commands, command + 48)?,
                        writable: u32_at(commands, command + 60)? & macho::VM_PROT_WRITE != 0,
                    };
                    let first = sections.len();
                    for nth in 0..u32_at(commands, command + 64)? as usize {
                        let header = command + SEGMENT + nth * SECTION;
                        let (address, len) =
                            (u64_at(commands, header + 32)?, u64_at(commands, header + 40)?);
                        let offset = u32_at(commands, header + 48)? as usize;
                        let flags = u32_at(commands, header + 64)?;
                        if offset != 0 && len != 0 {
                            room = room.min(offset);
                        }
                        let table = match &commands[header..header + 16] {
                            b"__unwind_info\0\0\0" => Some(&mut unwind),
                            b"__eh_frame\0\0\0\0\0\0" => Some(&mut frames),
                            _ => None,
                        };
                        if let Some(table) = table {
                            let content = bytes.get(offset..offset + len as usize);
                            let bytes = content.context("the file ends early")?.to_vec();
                            *table = Table { header, nth: first + nth, first, address, bytes };
                        }
                        let indirect = u32_at(commands, header + 68)?;
                        let stub = u32_at(commands, header + 72)?;
                        sections.push((address, len, flags, indirect, stub));
                    }
                    segments.push(segment);
                }
                macho::LC_SYMTAB => at.symbols = command,
                macho::LC_DYSYMTAB => at.indirect = command,
                macho::LC_DYLD_INFO | macho::LC_DYLD_INFO_ONLY => at.fixups = command,
                macho::LC_CODE_SIGNATURE => at.signature = command,
                macho::LC_FUNCTION_STARTS | macho::LC_DATA_IN_CODE => at.data.push(command),
                macho::LC_DYLD_CHAINED_FIXUPS | macho::LC_DYLD_EXPORTS_TRIE => {
                    bail!("an executable whose addresses are fixed up in chains")
                }
                macho::LC_LOAD_DYLINKER
                | macho::LC_UUID
                | macho::LC_BUILD_VERSION
                | macho::LC_VERSION_MIN_MACOSX
                | macho::LC_SOURCE_VERSION
                | macho::LC_MAIN
                | macho::LC_LOAD_DYLIB
                | macho::LC_LOAD_WEAK_DYLIB
                | macho::LC_RPATH
                | macho::LC_DYLD_ENVIRONMENT => {}
                other => bail!("a load command of kind {other:#x}"),
            }
            command += size;
        }
        ensure!(command == commands.len(), "load commands of another length than the header says");
        let present = [at.linkedit, at.symbols, at.indirect, at.fixups, at.signature];
        ensure!(
            present.iter().all(|it| *it != 0),
            "an executable without symbols, fixups or a signature, or with segments behind `__LINKEDIT`"
        );
        // Without `__unwind_info` (`-no_compact_unwind`) the unwinder looks for every frame in
        // `__eh_frame`, to its end.
        ensure!(!frames.bytes.is_empty(), "the executable has no unwind tables");
        let (start, end) = (unwind.address, frames.address);
        let alone = (sections.iter()).all(|&(address, len, ..)| {
            len == 0 || address == start || !(start..end).contains(&address)
        });
        let in_front = match end.checked_sub(start) {
            Some(gap) if alone && !unwind.bytes.is_empty() && gap >= unwind.bytes.len() as u64 => {
                gap
            }
            _ => 0,
        };
        let frames_end = frames.address + frames.bytes.len() as u64;
        let text_end = (segments.iter())
            .find(|it| (it.address..it.address + it.file_size).contains(&frames.address))
            .map_or(frames_end, |it| it.address + it.file_size);
        let next = (sections.iter())
            .filter(|&&(address, len, ..)| len != 0 && address >= frames_end)
            .map(|&(address, ..)| address)
            .min()
            .unwrap_or(text_end);
        let behind = next.min(text_end).saturating_sub(frames_end);

        let linkedit = segments.last().context("no segments")?;
        ensure!(linkedit.offset.is_multiple_of(PAGE), "`__LINKEDIT` is not at a page of the file");
        let signed = u32_at(commands, at.signature + 8)? as usize;
        let signature = bytes.get(signed..).context("the file ends early")?;
        ensure!(
            signature.len() == u32_at(commands, at.signature + 12)? as usize
                && signed >= linkedit.offset as usize,
            "the signature is not the end of the file"
        );
        let signature = Signature::read(signature)?;
        let linkedit_at = linkedit.offset as usize;
        let content = bytes[linkedit_at..signed].to_vec();

        let (rebases, len) =
            (u32_at(commands, at.fixups + 8)? as usize, u32_at(commands, at.fixups + 12)? as usize);
        let stream = bytes.get(rebases..rebases + len).context("the file ends early")?;
        let rebases = Rebases::read(stream, &segments)?;

        let table = u32_at(commands, at.symbols + 8)? as usize;
        let names = bytes
            .get(u32_at(commands, at.symbols + 16)? as usize..)
            .context("the file ends early")?;
        let name = |entry: usize| -> anyhow::Result<&[u8]> {
            let name =
                names.get(u32_at(bytes, entry)? as usize..).context("a symbol of no name")?;
            Ok(name.split(|it| *it == 0).next().unwrap_or_default())
        };
        let mut defined = Vec::new();
        for nth in 0..u32_at(commands, at.symbols + 12)? as usize {
            let entry = table + nth * NLIST;
            let kind = *bytes.get(entry + 4).context("the file ends early")?;
            if kind & macho::N_STAB != 0 || kind & macho::N_TYPE != macho::N_SECT {
                continue;
            }
            let section = usize::from(bytes[entry + 5]).checked_sub(1);
            let &(start, len, flags, ..) =
                section.and_then(|it| sections.get(it)).context("a symbol in no section")?;
            let kind = match flags & macho::SECTION_TYPE {
                // What code reaches a thread-local by: the address of a function that gives
                // its address, called with that of the descriptor. The descriptor itself is
                // data like any other.
                macho::S_THREAD_LOCAL_VARIABLES => SymbolKind::Data,
                macho::S_THREAD_LOCAL_REGULAR..=macho::S_THREAD_LOCAL_INIT_FUNCTION_POINTERS => {
                    SymbolKind::Tls
                }
                _ if flags
                    & (macho::S_ATTR_PURE_INSTRUCTIONS | macho::S_ATTR_SOME_INSTRUCTIONS)
                    != 0 =>
                {
                    SymbolKind::Text
                }
                _ => SymbolKind::Data,
            };
            defined.push((u64_at(bytes, entry + 8)?, start + len, kind, entry));
        }
        let mut starts: Vec<u64> = defined.iter().map(|it| it.0).collect();
        starts.sort_unstable();
        starts.dedup();
        let mut symbols = FxHashMap::<Box<[u8]>, Named>::default();
        for (address, section_end, kind, entry) in defined {
            let next = starts.get(starts.partition_point(|it| *it <= address));
            let end = next.map_or(section_end, |next| section_end.min(*next));
            let defined =
                Defined { address, size: end.saturating_sub(address), kind, entry: entry as u64 };
            match symbols.get_mut(name(entry)?) {
                None => _ = symbols.insert(name(entry)?.into(), Named::One(defined)),
                Some(Named::One(known)) if *known == Defined { entry: known.entry, ..defined } => {}
                Some(known) => *known = Named::Many,
            }
        }

        let mut imports = FxHashMap::<Box<[u8]>, Import>::default();
        let indirect = u32_at(commands, at.indirect + 56)? as usize;
        for &(address, len, flags, first, stub) in &sections {
            let each = match flags & macho::SECTION_TYPE {
                macho::S_NON_LAZY_SYMBOL_POINTERS => 8,
                macho::S_SYMBOL_STUBS if stub != 0 => u64::from(stub),
                _ => continue,
            };
            for nth in 0..len / each {
                let symbol = u32_at(bytes, indirect + (first as usize + nth as usize) * 4)?;
                if symbol & (macho::INDIRECT_SYMBOL_LOCAL | macho::INDIRECT_SYMBOL_ABS) != 0 {
                    continue;
                }
                let entry = table + symbol as usize * NLIST;
                if *bytes.get(entry + 4).context("the file ends early")? & macho::N_TYPE
                    != macho::N_UNDF
                {
                    continue;
                }
                let import = imports.entry(name(entry)?.into()).or_default();
                let at = Some(address + nth * each);
                match each {
                    8 if flags & macho::SECTION_TYPE == macho::S_NON_LAZY_SYMBOL_POINTERS => {
                        import.slot = at
                    }
                    _ => import.stub = at,
                }
            }
        }

        Ok(Image {
            commands: commands.to_vec(),
            room,
            segments,
            sectioned: sections.len(),
            at,
            symbols,
            imports,
            unwind,
            frames,
            in_front,
            behind,
            rebases,
            linkedit: content,
            signature,
        })
    }

    /// Takes the signature of `bytes`, the executable as someone else signed it again after
    /// the splice `undo` puts back. Each splice starts from the file as it was before any,
    /// so the hashes of the pages `undo` puts back are of what it puts back.
    pub(super) fn signed_again(
        &mut self,
        bytes: &[u8],
        undo: &[(u64, Vec<u8>)],
    ) -> anyhow::Result<()> {
        let count = u32_at(bytes, 16)?;
        let (mut at, mut uuid, mut signature) = (HEADER, None, None);
        for _ in 0..count {
            match u32_at(bytes, at)? {
                macho::LC_UUID => uuid = bytes.get(at + 8..at + 24),
                macho::LC_CODE_SIGNATURE => {
                    let (offset, len) =
                        (u32_at(bytes, at + 8)? as usize, u32_at(bytes, at + 12)? as usize);
                    signature = Some((offset, bytes.get(offset..offset + len)));
                }
                _ => {}
            }
            at += u32_at(bytes, at + 4)? as usize;
        }
        ensure!(
            uuid.is_some() && uuid == uuid_of(&self.commands).as_ref().map(|it| &it[..]),
            "the executable was written by someone else"
        );
        let (offset, signature) = signature.context("the executable is not signed")?;
        let mut new = Signature::read(signature.context("the file ends early")?)?;
        let page = new.page;
        let pages: FxHashSet<usize> = (undo.iter())
            .flat_map(|(at, old)| {
                *at as usize / page..=(*at as usize + old.len().max(1) - 1) / page
            })
            .collect();
        for index in pages {
            let start = index * page;
            let end = (start + page).min(offset);
            let mut content = bytes.get(start..end).context("a short file")?.to_vec();
            for (at, old) in undo.iter().rev() {
                let (at, end) = (*at as usize, *at as usize + old.len());
                let (from, to) = (at.max(start), end.min(start + content.len()));
                if from < to {
                    content[from - start..to - start].copy_from_slice(&old[from - at..to - at]);
                }
            }
            let hash =
                new.hashes.get_mut(index * Signature::HASH..).context("a short signature")?;
            hash[..Signature::HASH].copy_from_slice(&super::sha256::digest(&content));
        }
        self.signature = new;
        Ok(())
    }

    fn linkedit(&self) -> &Segment {
        &self.segments[self.segments.len() - 1]
    }

    /// What is loaded from `offset` of the file.
    fn address_of(&self, offset: u64) -> anyhow::Result<u64> {
        (self.segments.iter())
            .find(|it| it.file_size != 0 && (it.offset..it.offset + it.file_size).contains(&offset))
            .map(|it| it.address + (offset - it.offset))
            .with_context(|| format!("nothing is loaded from {offset:#x} of the file"))
    }

    /// Where in the file the linker put what is loaded at `address`.
    fn offset_of(&self, address: u64) -> anyhow::Result<u64> {
        (self.segments.iter())
            .find(|it| (it.address..it.address + it.file_size).contains(&address))
            .map(|it| it.offset + (address - it.address))
            .with_context(|| format!("nothing of the file is loaded at {address:#x}"))
    }
}

/// How `__eh_frame` says an address is written (`DW_EH_PE_*`).
struct Pointer {
    at: usize,
    len: usize,
    /// From where it is written, so it changes when that does.
    relative: bool,
}

impl Pointer {
    fn encoded(encoding: u8, at: usize) -> anyhow::Result<Pointer> {
        let len = match encoding & 0x0f {
            0x00 | 0x04 | 0x0c => 8,
            0x03 | 0x0b => 4,
            other => bail!("frames whose addresses are encoded as {other:#x}"),
        };
        Ok(Pointer { at, len, relative: encoding & 0x70 == 0x10 })
    }

    fn value(&self, section: &[u8]) -> anyhow::Result<i64> {
        Ok(match self.len {
            4 => i64::from(u32_at(section, self.at)? as i32),
            _ => u64_at(section, self.at)? as i64,
        })
    }

    /// The addresses `section` has: of the code of each frame, of what its code throws
    /// to, and of the personality routine of each common entry.
    fn all(section: &[u8]) -> anyhow::Result<Vec<Pointer>> {
        let mut found = Vec::new();
        // How the frames of a common entry write addresses of code and of landing pads.
        let mut common = FxHashMap::<usize, (u8, Option<u8>)>::default();
        let mut at = 0;
        while at + 4 <= section.len() {
            let len = u32_at(section, at)? as usize;
            if len == 0 {
                break;
            }
            ensure!(len != u32::MAX as usize, "an unwind table in the 64-bit format");
            let back = u32_at(section, at + 4)? as usize;
            if back == 0 {
                let mut next = at + 9;
                let augmentation: Vec<u8> =
                    section[next..].iter().copied().take_while(|it| *it != 0).collect();
                next += augmentation.len() + 1;
                // The alignment of code and of data, the register of the return address,
                // and the length of what the augmentation adds.
                for _ in 0..3 {
                    uleb(section, &mut next)?;
                }
                let (mut code, mut pads) = (0, None);
                if augmentation.first() == Some(&b'z') {
                    uleb(section, &mut next)?;
                    for letter in &augmentation[1..] {
                        let encoding =
                            *section.get(next).context("a common entry that ends early")?;
                        match letter {
                            b'R' => code = encoding,
                            b'L' => pads = Some(encoding),
                            b'P' => {
                                let pointer = Pointer::encoded(encoding, next + 1)?;
                                next += pointer.len;
                                found.push(pointer);
                            }
                            other => bail!("a common entry augmented with {:?}", *other as char),
                        }
                        next += 1;
                    }
                }
                common.insert(at, (code, pads));
            } else {
                let entry = (at + 4).checked_sub(back).and_then(|it| common.get(&it));
                let &(code, pads) = entry.context("a frame of no common entry")?;
                let start = Pointer::encoded(code, at + 8)?;
                let mut next = start.at + 2 * start.len;
                found.push(start);
                if let Some(pads) = pads {
                    uleb(section, &mut next)?;
                    let pad = Pointer::encoded(pads, next)?;
                    // Nothing is written for code that throws to nowhere.
                    if pad.value(section)? != 0 {
                        found.push(pad);
                    }
                }
            }
            at += 4 + len;
        }
        Ok(found)
    }
}

/// The patch's frames `ours`, laid out at `from`, moved where they will be in `__eh_frame`
/// starting at `start`: the addresses in them are relative to where they are.
fn frames_moved(ours: &[u8], from: u64, start: u64) -> anyhow::Result<Option<Vec<u8>>> {
    let (mut used, mut last) = (0, 0);
    while used + 4 <= ours.len() && u32_at(ours, used)? != 0 {
        last = used;
        used += 4 + u32_at(ours, used)? as usize;
    }
    ensure!(used <= ours.len(), "frames that end past their section");
    if used == 0 {
        return Ok(None);
    }
    let mut block = ours[..used].to_vec();
    // The unwinder stops at an entry of no length, so the last is what is longer for what
    // follows to start at a multiple of eight, by instructions that do nothing.
    let padded = used.next_multiple_of(8);
    let len = u32_at(&block, last)? + (padded - used) as u32;
    put32(&mut block, last, len);
    block.resize(padded, 0);
    let moved = start.wrapping_sub(from) as i64;
    for pointer in Pointer::all(&block)? {
        ensure!(pointer.relative, "frames with an address that is not relative to them");
        let value = pointer.value(&block)?.wrapping_sub(moved);
        match pointer.len {
            4 => put32(
                &mut block,
                pointer.at,
                i32::try_from(value).context("the program is too large")? as u32,
            ),
            _ => put64(&mut block, pointer.at, value as u64),
        }
    }
    Ok(Some(block))
}

/// The patch's frames `ours`, laid out at `from`, behind the program's in the room `__TEXT`
/// has there: `__eh_frame` grows, its start and `__unwind_info` stay as they are, so no
/// offset of the table moves (they are 24 bits, which a program of more than 16 MB of
/// frames fills). The patch's functions are past the table's last entry, which sends the
/// unwinder through `__eh_frame` to the end. Gives where they start, the frames, and the new
/// size of `__eh_frame`; or nothing when the patch has no frames or they don't fit.
fn frames_behind(
    frames: &Table,
    behind: u64,
    ours: &[u8],
    from: u64,
) -> anyhow::Result<Option<(u64, Vec<u8>, u64)>> {
    // Right behind the program's last entry: anything of no length between would stop the
    // unwinder.
    let start = frames.address + frames.bytes.len() as u64;
    ensure!(start.is_multiple_of(4), "`__eh_frame` that ends off a word");
    let Some(block) = frames_moved(ours, from, start)? else { return Ok(None) };
    if block.len() as u64 > behind {
        return Ok(None);
    }
    let size = start + block.len() as u64 - frames.address;
    Ok(Some((start, block, size)))
}

/// The tables of the unwinder once the patch's frames `ours`, laid out at `from`, are in
/// front of the program's: `__unwind_info` moves into the room the header leaves, from `at`
/// of the file, and the frames go where it was. Gives what that writes where in the file
/// and where the two sections then start, `__unwind_info` first; or nothing when the patch
/// has no frames, they are larger than the table, or the table can't say the program's frames
/// are that much further in.
fn frames_in_front(
    image: &Image,
    ours: &[u8],
    from: u64,
    at: usize,
) -> anyhow::Result<Option<(Vec<(u64, Vec<u8>)>, [u64; 2])>> {
    // Its length, then laid out where it goes.
    let Some(len) = frames_moved(ours, from, from)?.map(|it| it.len()) else { return Ok(None) };
    if len as u64 > image.in_front {
        return Ok(None);
    }
    let start = image.frames.address - len as u64;
    let block = frames_moved(ours, from, start)?.context("frames that went away")?;
    let padded = block.len();
    let mut table = image.unwind.bytes.clone();
    if !shifted(&mut table, padded as u64)? {
        return Ok(None);
    }
    ensure!(
        at + table.len() <= image.room,
        "the header has no room for the unwind table: link with -Wl,-headerpad,{:#x}",
        (at + table.len()).next_multiple_of(0x1000)
    );
    let unwind = image.address_of(at as u64)?;
    Ok(Some((vec![(image.offset_of(start)?, block), (at as u64, table)], [unwind, start])))
}

/// The tables of the unwinder once both are in the room the header leaves, from `at` of the
/// file, for patch frames of any size that fits there: `__unwind_info` as it is, then
/// `__eh_frame` with the program's frames at the offsets the table has for them and the
/// patch's behind, where the unwinder finds them as it does behind the program's
/// ([`frames_behind`]). Gives what that writes where in the file, where the two sections
/// then start, `__unwind_info` first, and the size of `__eh_frame`; or nothing when the
/// patch has no frames.
fn frames_in_header(
    image: &Image,
    ours: &[u8],
    from: u64,
    at: usize,
) -> anyhow::Result<Option<(Vec<(u64, Vec<u8>)>, [u64; 2], u64)>> {
    ensure!(
        image.unwind.bytes.is_empty() || image.frames.nth == image.unwind.nth + 1,
        "a section between `__unwind_info` and `__eh_frame`"
    );
    let frames_at = (at + image.unwind.bytes.len()).next_multiple_of(8);
    let start = image.address_of(frames_at as u64)?;
    let theirs = frames_moved(&image.frames.bytes, image.frames.address, start)?
        .context("the program has no frames")?;
    let behind = start + theirs.len() as u64;
    let Some(block) = frames_moved(ours, from, behind)? else { return Ok(None) };
    let end = frames_at + theirs.len() + block.len();
    ensure!(
        end <= image.room,
        "the header has no room for the unwind tables: link with -Wl,-headerpad,{:#x}",
        end.next_multiple_of(0x1000)
    );
    let unwind = image.address_of(at as u64)?;
    let size = (theirs.len() + block.len()) as u64;
    let written =
        vec![(at as u64, image.unwind.bytes.clone()), (frames_at as u64, [theirs, block].concat())];
    Ok(Some((written, [unwind, start], size)))
}

/// Lists `table` (`__unwind_info`, or `__eh_frame` of a program without it) and the
/// `count - 1` sections behind it first of their segment's sections once they are in the room
/// the header leaves ([`frames_in_front`], [`frames_in_header`]): a segment lists its sections
/// in the order of their addresses, which dyld checks (`dyld_info` says "start address
/// overlaps previous section"). The symbols name their section by its number, so those of the
/// sections moved and passed are numbered again, in the symbol table of `linkedit`, which
/// starts at `offset` of the file.
fn tables_first(
    table: &Table,
    commands: &mut [u8],
    linkedit: &mut [u8],
    offset: u64,
    count: usize,
) -> anyhow::Result<()> {
    let Table { header, nth, first, .. } = *table;
    let start = header - (nth - first) * SECTION;
    commands[start..header + count * SECTION].rotate_right(count * SECTION);

    let lowest = u8::try_from(first + 1)?;
    let (moved, past) = (u8::try_from(nth + 1)?, u8::try_from(nth + 1 + count)?);
    let passed = u8::try_from(count)?;
    let mut command = HEADER;
    for _ in 0..u32_at(commands, 16)? {
        if u32_at(commands, command)? == macho::LC_SYMTAB {
            let table = (u64::from(u32_at(commands, command + 8)?))
                .checked_sub(offset)
                .context("a symbol table in front of `__LINKEDIT`")?;
            for nth in 0..u32_at(commands, command + 12)? as usize {
                let entry = table as usize + nth * NLIST + 5;
                let section = linkedit.get_mut(entry).context("symbols past the end")?;
                *section = match *section {
                    it if (moved..past).contains(&it) => lowest + (it - moved),
                    it if (lowest..moved).contains(&it) => it + passed,
                    it => it,
                };
            }
            return Ok(());
        }
        command += u32_at(commands, command + 4)? as usize;
    }
    bail!("no symbol table")
}

/// The unwinder's table (`__unwind_info`) for frames `by` bytes further from where
/// `__eh_frame` starts: its entries that say "look in `__eh_frame`" say where. False, and the
/// table half written, when one of them is further in than its 24 bits say.
fn shifted(table: &mut [u8], by: u64) -> anyhow::Result<bool> {
    fn shift(table: &mut [u8], at: usize, by: u64) -> anyhow::Result<bool> {
        const MODE: u32 = 0x0f00_0000;
        const DWARF: u32 = 0x0300_0000;
        const OFFSET: u32 = 0x00ff_ffff;
        let encoding = u32_at(table, at)?;
        if encoding & MODE == DWARF {
            let offset = u64::from(encoding & OFFSET) + by;
            if offset > u64::from(OFFSET) {
                return Ok(false);
            }
            put32(table, at, (encoding & !OFFSET) | offset as u32);
        }
        Ok(true)
    }
    ensure!(u32_at(table, 0)? == 1, "an unwind table of another version");
    let (common, count) = (u32_at(table, 4)? as usize, u32_at(table, 8)? as usize);
    for nth in 0..count {
        if !shift(table, common + nth * 4, by)? {
            return Ok(false);
        }
    }
    let (index, count) = (u32_at(table, 20)? as usize, u32_at(table, 24)? as usize);
    ensure!(count != 0, "an unwind table of no functions");
    // The last entry is of no page, which sends the unwinder to `__eh_frame` for whatever
    // is past it: the patch's functions.
    ensure!(
        u32_at(table, index + (count - 1) * 12 + 4)? == 0,
        "an unwind table whose last entry has a page"
    );
    for nth in 0..count - 1 {
        let page = u32_at(table, index + nth * 12 + 4)? as usize;
        let entries = usize::from(u16_at(table, page + 4)?);
        let len = usize::from(u16_at(table, page + 6)?);
        match u32_at(table, page)? {
            // Regular: each entry has its encoding behind where its function starts.
            2 => {
                for entry in 0..len {
                    if !shift(table, page + entries + entry * 8 + 4, by)? {
                        return Ok(false);
                    }
                }
            }
            // Compressed: the entries point into encodings of the page's own.
            3 => {
                let encodings = usize::from(u16_at(table, page + 8)?);
                for encoding in 0..usize::from(u16_at(table, page + 10)?) {
                    if !shift(table, page + encodings + encoding * 4, by)? {
                        return Ok(false);
                    }
                }
            }
            other => bail!("an unwind table with a page of kind {other}"),
        }
    }
    Ok(true)
}

/// The instructions a relocation is written into.
struct Code;

impl Code {
    /// How far a branch reaches.
    const REACH: i64 = 1 << 27;
    /// `adrp x16`, `add x16, x16` and `ldr x16, [x16]`, without what they add, and `br x16`.
    const PAGE_OF: u32 = 0x9000_0010;
    const ADD: u32 = 0x9100_0210;
    const LOAD: u32 = 0xf940_0210;
    const JUMP: u32 = 0xd61f_0200;
    /// A stub: one of each.
    const STUB: usize = 12;

    fn reaches(to: u64, from: u64) -> bool {
        (-Self::REACH..Self::REACH).contains(&(to.wrapping_sub(from) as i64))
    }

    /// `b` or `bl` with where it goes.
    fn branch(instruction: u32, to: u64, from: u64) -> anyhow::Result<u32> {
        ensure!(Self::reaches(to, from) && to.is_multiple_of(4), "a branch out of reach");
        Ok(instruction & 0xfc00_0000 | (to.wrapping_sub(from) >> 2) as u32 & 0x03ff_ffff)
    }

    /// `adrp` with the page of `to`.
    fn page(instruction: u32, to: u64, from: u64) -> anyhow::Result<u32> {
        let pages = (to >> 12) as i64 - (from >> 12) as i64;
        ensure!((-(1 << 20)..1 << 20).contains(&pages), "the program is too large");
        let pages = pages as u32;
        Ok(instruction & 0x9f00_001f | (pages & 3) << 29 | (pages >> 2 & 0x7_ffff) << 5)
    }

    /// An `add`, or a load or store, with where in its page `to` is.
    fn offset(instruction: u32, to: u64) -> anyhow::Result<u32> {
        let mut within = (to & 0xfff) as u32;
        if instruction & 0x3b00_0000 == 0x3900_0000 {
            // A load or store counts in what it moves, 16 bytes when the register is a vector.
            let vector = instruction & 0x0480_0000 == 0x0480_0000;
            let scale = if vector { 4 } else { instruction >> 30 };
            ensure!(within.is_multiple_of(1 << scale), "a load of what is not aligned");
            within >>= scale;
        }
        Ok(instruction & !(0xfff << 10) | within << 10)
    }

    /// The code that jumps to `to`, or to the address in the slot at `to`.
    fn stub(to: u64, from: u64, through_slot: bool) -> anyhow::Result<[u8; Self::STUB]> {
        let second = if through_slot { Self::LOAD } else { Self::ADD };
        let code = [Self::page(Self::PAGE_OF, to, from)?, Self::offset(second, to)?, Self::JUMP];
        let mut stub = [0; Self::STUB];
        for (bytes, instruction) in stub.as_chunks_mut().0.iter_mut().zip(code) {
            *bytes = instruction.to_le_bytes();
        }
        Ok(stub)
    }
}

/// A patch object, and what of the executable it needs.
pub(super) struct Patch<'a> {
    file: object::File<'a>,
    /// The sections that are loaded, by the segment they go to.
    sections: [Vec<SectionIndex>; 2],
    code: FxHashSet<SectionIndex>,
    /// Its `__eh_frame`, which goes behind the program's.
    frames: Option<SectionIndex>,
    relocations: FxHashMap<SectionIndex, Vec<Relocation>>,
    /// What each symbol a relocation refers to stands for.
    targets: FxHashMap<SymbolIndex, Target>,
    /// The symbols the code reaches through an address slot of ours, in the order of the
    /// slots.
    slots: Vec<SymbolIndex>,
    /// The symbols of the executable that the code branches to, in the order of their stubs.
    stubs: Vec<SymbolIndex>,
    /// The functions and statics it defines, with what the executable has under the name.
    replaced: Vec<Replaced<'a>>,
    /// The statics among them that go where the executable has them, by where the patch does.
    in_place: FxHashMap<SectionIndex, Vec<InPlace>>,
}

const TEXT: usize = 0;
const DATA: usize = 1;

#[derive(Clone, Copy)]
enum Target {
    /// At an offset in a section of the patch.
    Ours(SectionIndex, u64),
    /// At an address of the executable.
    Theirs(u64),
    Import(Import),
}

struct Relocation {
    at: u64,
    fixup: Fixup,
    symbol: SymbolIndex,
    addend: i64,
}

/// How a relocation of the patch gets its value.
#[derive(Clone, Copy, PartialEq)]
enum Fixup {
    /// The address itself, in 8 bytes.
    Absolute,
    /// How far the address is from that of another symbol, in this many bytes.
    Distance(SymbolIndex, usize),
    Branch,
    /// The page of the address, and where in the page it is.
    Page,
    Offset,
    /// The same of a slot that holds the address.
    SlotPage,
    SlotOffset,
    /// The distance to such a slot, in 4 bytes.
    SlotDistance,
}

impl Fixup {
    fn through_slot(self) -> bool {
        matches!(self, Fixup::SlotPage | Fixup::SlotOffset | Fixup::SlotDistance)
    }
}

pub(super) struct Replaced<'a> {
    pub(super) name: &'a [u8],
    section: SectionIndex,
    offset: u64,
    /// To the next symbol of its section, or its end: no symbol has a size.
    size: u64,
    /// `None` for what the executable does not have.
    old: Option<Defined>,
}

/// A static of the patch that is written over the executable's.
#[derive(Clone, Copy)]
struct InPlace {
    /// Where its section has it.
    offset: u64,
    size: u64,
    old: Defined,
}

impl InPlace {
    fn holds(&self, offset: u64) -> bool {
        (self.offset..self.offset + self.size).contains(&offset)
    }
}

impl<'a> Patch<'a> {
    const SLOT: usize = 8;

    fn read(bytes: &'a [u8], image: &Image) -> anyhow::Result<Patch<'a>> {
        let file = object::File::parse(bytes)?;
        let mut patch = Patch {
            file,
            sections: Default::default(),
            code: FxHashSet::default(),
            frames: None,
            relocations: FxHashMap::default(),
            targets: FxHashMap::default(),
            slots: Vec::new(),
            stubs: Vec::new(),
            replaced: Vec::new(),
            in_place: FxHashMap::default(),
        };
        for section in patch.file.sections() {
            let SectionFlags::MachO { flags } = section.flags() else {
                bail!("a patch that is not Mach-O")
            };
            let kind = flags & macho::SECTION_TYPE;
            ensure!(
                !(macho::S_THREAD_LOCAL_REGULAR..=macho::S_THREAD_LOCAL_INIT_FUNCTION_POINTERS)
                    .contains(&kind),
                "the patch has thread-local data"
            );
            if flags & macho::S_ATTR_DEBUG != 0 || section.size() == 0 {
                continue;
            }
            let relocations = Self::relocations(&section)?;
            let stores = relocations.iter().any(|it| it.fixup == Fixup::Absolute);
            let writable =
                section.segment_name_bytes()?.is_some_and(|it| it.starts_with(b"__DATA"));
            if section.name_bytes()? == b"__eh_frame" {
                ensure!(!stores && patch.frames.is_none(), "frames that store addresses");
                patch.frames = Some(section.index());
            } else if flags & (macho::S_ATTR_PURE_INSTRUCTIONS | macho::S_ATTR_SOME_INSTRUCTIONS)
                != 0
            {
                ensure!(!stores, "code that stores addresses");
                patch.code.insert(section.index());
                patch.sections[TEXT].push(section.index());
            } else if stores || writable || kind == macho::S_ZEROFILL {
                patch.sections[DATA].push(section.index());
            } else {
                patch.sections[TEXT].push(section.index());
            }
            patch.relocations.insert(section.index(), relocations);
        }

        // No symbol has a size: one reaches to the next of its section.
        let mut starts: Vec<u64> =
            patch.file.symbols().filter(|it| it.is_definition()).map(|it| it.address()).collect();
        starts.sort_unstable();
        starts.dedup();
        for symbol in patch.file.symbols() {
            if !symbol.is_definition() || symbol.is_local() {
                continue;
            }
            let name = symbol.name_bytes()?;
            let shown = || String::from_utf8_lossy(name);
            let index = symbol.section_index().context("a symbol in no section")?;
            let section = patch.file.section_by_index(index)?;
            ensure!(
                patch.relocations.contains_key(&index),
                "{} is in a section that is not loaded",
                shown()
            );
            let offset = symbol.address() - section.address();
            let code = patch.code.contains(&index);
            let next = starts.get(starts.partition_point(|it| *it <= symbol.address()));
            let end = section.address() + section.size();
            let size = next.map_or(end, |next| end.min(*next)) - symbol.address();
            let old = match image.symbols.get(name) {
                None => None,
                // The executable's is the same static with the same value: it is kept.
                Some(_) if is_lasting(name) => None,
                // One that says what a patch left behind is of that patch, which the executable
                // was linked with, and says nothing of this one's.
                Some(_) if left_behind(name).is_some() => None,
                Some(Named::One(old)) if old.kind == SymbolKind::Text && code => Some(*old),
                Some(Named::One(old)) if old.kind == SymbolKind::Data && !code => {
                    let new = InPlace { offset, size, old: *old };
                    patch
                        .fits(&new, &section, image)
                        .with_context(|| format!("the static {} has another value", shown()))?;
                    patch.in_place.entry(index).or_default().push(new);
                    Some(*old)
                }
                Some(_) => {
                    bail!("the patch replaces {}, which is not one function or static", shown())
                }
            };
            patch.replaced.push(Replaced { name, section: index, offset, size, old });
        }

        let (mut slots, mut stubs) = (FxHashSet::default(), FxHashSet::default());
        // A call of a function of the patch that replaces one of the executable goes to the
        // executable's entry of it (`lay_out`'s `entry`), which can be out of a branch's reach
        // of where the patch is put: in a large executable, patches go on past its end.
        let replacing: FxHashSet<(SectionIndex, u64)> = (patch.replaced.iter())
            .filter(|it| it.old.is_some_and(|old| old.kind == SymbolKind::Text))
            .map(|it| (it.section, it.offset))
            .collect();
        let mut targets = FxHashMap::default();
        for relocation in patch.relocations.values().flatten() {
            let symbols = match relocation.fixup {
                Fixup::Distance(from, _) => vec![relocation.symbol, from],
                _ => vec![relocation.symbol],
            };
            for symbol in symbols {
                if let std::collections::hash_map::Entry::Vacant(unseen) = targets.entry(symbol) {
                    unseen.insert(patch.target(symbol, image)?);
                }
            }
            let target = targets[&relocation.symbol];
            let shown = || {
                let symbol = patch.file.symbol_by_index(relocation.symbol);
                symbol.ok().and_then(|it| it.name().ok()).unwrap_or_default().to_owned()
            };
            match (relocation.fixup, target) {
                (
                    Fixup::Absolute | Fixup::Distance(..) | Fixup::Page | Fixup::Offset,
                    Target::Import(_),
                ) => {
                    bail!("the address of {}, which is in another library", shown())
                }
                (fixup, Target::Import(Import { slot: None, .. })) if fixup.through_slot() => {
                    bail!("{}, which the executable has no address slot of", shown())
                }
                (Fixup::Branch, Target::Import(Import { slot: None, stub: None })) => {
                    bail!("{}, which the executable does not call", shown())
                }
                (Fixup::Branch, Target::Theirs(_) | Target::Import(_)) => {
                    if stubs.insert(relocation.symbol) {
                        patch.stubs.push(relocation.symbol);
                    }
                }
                (Fixup::Branch, Target::Ours(section, at))
                    if relocation.addend == 0 && replacing.contains(&(section, at)) =>
                {
                    if stubs.insert(relocation.symbol) {
                        patch.stubs.push(relocation.symbol);
                    }
                }
                (fixup, Target::Ours(..) | Target::Theirs(_)) if fixup.through_slot() => {
                    if slots.insert(relocation.symbol) {
                        patch.slots.push(relocation.symbol);
                    }
                }
                _ => {}
            }
        }
        patch.targets = targets;
        patch.keep_lasting(|name| match image.symbols.get(name) {
            Some(Named::One(Defined { kind: SymbolKind::Data, address, .. })) => Some(*address),
            _ => None,
        })?;
        Ok(patch)
    }

    /// Has what refers to a static the patch defines anew ([`is_lasting`]) refer to the copy
    /// `kept` finds of it instead, where it finds one: the executable's, or one an earlier
    /// patch gave the running program. The patch's own copy is left unused.
    fn keep_lasting(&mut self, kept: impl Fn(&[u8]) -> Option<u64>) -> anyhow::Result<()> {
        for (&index, target) in &mut self.targets {
            if !matches!(target, Target::Ours(..)) {
                continue;
            }
            let name = self.file.symbol_by_index(index)?.name_bytes()?;
            if is_lasting(name)
                && let Some(address) = kept(name)
            {
                *target = Target::Theirs(address);
            }
        }
        Ok(())
    }

    /// Where each static the patch defines anew is once it is laid out at `base`: its own
    /// copy, or the one [`Self::keep_lasting`] kept.
    fn lasting_in(
        &self,
        base: u64,
        placed: &FxHashMap<SectionIndex, u64>,
    ) -> Vec<(Box<[u8]>, u64)> {
        let kept: FxHashMap<&[u8], u64> = (self.targets.iter())
            .filter_map(|(&index, target)| match target {
                Target::Theirs(address) => {
                    let name = self.file.symbol_by_index(index).ok()?.name_bytes().ok()?;
                    is_lasting(name).then_some((name, *address))
                }
                _ => None,
            })
            .collect();
        (self.replaced.iter())
            .filter(|it| is_lasting(it.name))
            .map(|it| {
                let own = base + placed[&it.section] + it.offset;
                (Box::from(it.name), kept.get(it.name).copied().unwrap_or(own))
            })
            .collect()
    }

    /// The statics the patch defines anew, with what they take.
    fn lasting_own(&self) -> anyhow::Result<Vec<Own>> {
        let mut own = Vec::new();
        for replaced in self.replaced.iter().filter(|it| is_lasting(it.name)) {
            let section = self.file.section_by_index(replaced.section)?.align().max(1);
            let align = (replaced.offset | section).isolate_lowest_one();
            own.push(Own { name: replaced.name.into(), size: replaced.size, align });
        }
        Ok(own)
    }

    /// The relocations of a section, with the two that make a distance as one.
    fn relocations(section: &object::Section<'_, '_>) -> anyhow::Result<Vec<Relocation>> {
        let mut found = Vec::new();
        let mut from = None;
        for (at, relocation) in section.relocations() {
            let RelocationFlags::MachO { r_type, r_pcrel, r_length } = relocation.flags() else {
                bail!("a relocation that is not Mach-O's")
            };
            let RelocationTarget::Symbol(symbol) = relocation.target() else {
                bail!("a relocation against a section")
            };
            let fixup = match (r_type, r_pcrel, r_length) {
                (macho::ARM64_RELOC_SUBTRACTOR, false, 2 | 3) => {
                    from = Some(symbol);
                    continue;
                }
                (macho::ARM64_RELOC_UNSIGNED, false, 2 | 3) if from.is_some() => {
                    Fixup::Distance(from.take().context("no symbol")?, 1 << r_length)
                }
                (macho::ARM64_RELOC_UNSIGNED, false, 3) => Fixup::Absolute,
                (macho::ARM64_RELOC_BRANCH26, true, 2) => Fixup::Branch,
                (macho::ARM64_RELOC_PAGE21, true, 2) => Fixup::Page,
                (macho::ARM64_RELOC_PAGEOFF12, false, 2) => Fixup::Offset,
                // A thread-local's descriptor is reached through a slot, as what another
                // library defines is.
                (
                    macho::ARM64_RELOC_GOT_LOAD_PAGE21 | macho::ARM64_RELOC_TLVP_LOAD_PAGE21,
                    true,
                    2,
                ) => Fixup::SlotPage,
                (
                    macho::ARM64_RELOC_GOT_LOAD_PAGEOFF12 | macho::ARM64_RELOC_TLVP_LOAD_PAGEOFF12,
                    false,
                    2,
                ) => Fixup::SlotOffset,
                (macho::ARM64_RELOC_POINTER_TO_GOT, true, 2) => Fixup::SlotDistance,
                (other, ..) => bail!("a relocation of type {other}, which only the linker handles"),
            };
            found.push(Relocation { at, fixup, symbol, addend: relocation.addend() });
        }
        ensure!(from.is_none(), "half of a distance");
        Ok(found)
    }

    /// Whether a static can go where the executable has the one it replaces: what refers to
    /// it there is not looked for.
    fn fits(
        &self,
        new: &InPlace,
        section: &object::Section<'_, '_>,
        image: &Image,
    ) -> anyhow::Result<()> {
        let old = &new.old;
        ensure!(new.size <= old.size, "that is larger: {} bytes after {}", new.size, old.size);
        let align = (new.offset | section.align().max(1) | 1 << 12).isolate_lowest_one();
        ensure!(old.address.is_multiple_of(align), "that may be aligned to {align}");
        let segment = (image.segments.iter())
            .find(|it| {
                it.address <= old.address && old.address + new.size <= it.address + it.file_size
            })
            .context("that the file has no bytes for")?;
        let writable = section.segment_name_bytes()?.is_some_and(|it| it.starts_with(b"__DATA"));
        ensure!(!writable || segment.writable, "that is written to, where the old one is not");
        Ok(())
    }

    fn target(&self, index: SymbolIndex, image: &Image) -> anyhow::Result<Target> {
        let symbol = self.file.symbol_by_index(index)?;
        let name = symbol.name_bytes()?;
        let shown = || String::from_utf8_lossy(name);
        if let Some(section) = symbol.section_index() {
            ensure!(
                self.relocations.contains_key(&section),
                "{} is in a section that is not loaded",
                shown()
            );
            let start = self.file.section_by_index(section)?.address();
            return Ok(Target::Ours(section, symbol.address() - start));
        }
        ensure!(symbol.is_undefined(), "{} is neither defined nor undefined", shown());
        match image.symbols.get(name) {
            Some(Named::One(Defined { kind: SymbolKind::Tls, .. })) => {
                bail!("{} is thread-local", shown())
            }
            Some(Named::One(defined)) => Ok(Target::Theirs(defined.address)),
            Some(Named::Many) => bail!("{} names several things in the executable", shown()),
            None => match image.imports.get(name) {
                Some(import) => Ok(Target::Import(*import)),
                None => bail!("{} is not in the executable", shown()),
            },
        }
    }

    /// Lays the patch out where `__LINKEDIT` is and gives everything in it its address.
    fn place(&self, image: &Image) -> anyhow::Result<Addition> {
        let linkedit = image.linkedit();
        let (offset, base) = (linkedit.offset, linkedit.address);
        let Laid { mut appended, placed, data_at, linkedit_at, stored, restored, statics } =
            self.lay_out(base)?;
        let mut overwritten = Vec::new();
        for (address, value) in statics {
            overwritten.push((image.offset_of(address)?, value));
        }

        // The symbols of the functions name the new code, for whoever reads a backtrace.
        let mut renamed = Vec::new();
        let mut redirected = 0;
        for (old, new, jump) in self.redirects(base, &placed)? {
            overwritten.push((image.offset_of(old.address)?, jump));
            renamed.push((old.entry, new));
            redirected += 1;
        }

        let mut rebases: Vec<u64> = image.rebases.clone();
        let statics: Vec<Defined> = self.in_place.values().flatten().map(|it| it.old).collect();
        rebases
            .retain(|at| !statics.iter().any(|it| (it.address..it.address + it.size).contains(at)));
        rebases.extend(restored);
        rebases.extend(stored);
        rebases.sort_unstable();
        rebases.dedup();

        let text = (base, data_at);
        let data = (base + data_at, linkedit_at - data_at);
        let layout = Layout { text, data, rebases, renamed };
        let (mut commands, mut linkedit) = layout.linked(image, offset + linkedit_at)?;
        ensure!(
            commands.len() <= image.room,
            "the header has no room for two more segments: link with -Wl,-headerpad,{:#x}",
            (commands.len() - image.commands.len()).next_multiple_of(0x100)
        );

        // The patch's frames: behind the program's when `__TEXT` has room there, in front of
        // them where the unwind table was, which moves into the room the header leaves, and
        // else behind them with both tables in that room.
        let mut tables = Vec::new();
        if let Some(index) = self.frames {
            let (at, len) = (placed[&index], self.file.section_by_index(index)?.size());
            let ours = &appended[at as usize..(at + len) as usize];
            let front = commands.len().next_multiple_of(16);
            let unwind = image.unwind.bytes.len() as u64;
            let moved = if let Some((start, block, size)) =
                frames_behind(&image.frames, image.behind, ours, base + at)?
            {
                put64(&mut commands, image.frames.header + 40, size);
                tables = vec![(image.offset_of(start)?, block)];
                None
            } else if let Some((written, starts)) = frames_in_front(image, ours, base + at, front)?
            {
                let end = image.frames.address + image.frames.bytes.len() as u64;
                Some((written, starts, [unwind, end - starts[1]], 1))
            } else {
                frames_in_header(image, ours, base + at, front)?
                    .map(|(written, starts, size)| (written, starts, [unwind, size], 2))
            };
            if let Some((written, starts, sizes, count)) = moved {
                let moved = [&image.unwind, &image.frames].into_iter().zip(starts).zip(sizes);
                let has_table = !image.unwind.bytes.is_empty();
                for ((table, start), size) in moved.skip(usize::from(!has_table)) {
                    put64(&mut commands, table.header + 32, start);
                    put64(&mut commands, table.header + 40, size);
                    put32(
                        &mut commands,
                        table.header + 48,
                        u32::try_from(image.offset_of(start)?)?,
                    );
                }
                let (first, count) =
                    if has_table { (&image.unwind, count) } else { (&image.frames, 1) };
                tables_first(first, &mut commands, &mut linkedit, offset + linkedit_at, count)?;
                tables = written;
            }
        }
        tables.push((0, commands));
        appended.extend_from_slice(&linkedit);
        let lasting = self.lasting_in(base, &placed);
        Ok(Addition { offset, appended, overwritten, tables, redirected, lasting })
    }

    /// Lays the patch out at `base` of a running program, whose memory it is applied to:
    /// nothing of the executable's tables is written again, and the unwinder is told of the
    /// patch's frames where they are.
    fn place_live(
        &self,
        image: &Image,
        originals: &FxHashMap<u64, Vec<u8>>,
        base: u64,
        lasting: &mut Lasting,
    ) -> anyhow::Result<(ra_jit_live::format::Code, Vec<(u64, u64)>)> {
        ensure!(base.is_multiple_of(PAGE), "a patch laid out off a page");
        let laid = self.lay_out(base)?;
        let own = self.lasting_own()?;
        let copy = |name: &[u8]| {
            let found = self.replaced.iter().find(|it| it.name == name);
            found.map(|it| laid.placed[&it.section] + it.offset)
        };
        let pointers: Vec<u64> = laid.stored.iter().map(|it| it - base).collect();
        let first = lasting.first_values(&own, copy, &laid.appended, base, &pointers)?;
        lasting.note(self.lasting_in(base, &laid.placed));
        let frames = match self.frames {
            Some(index) => (laid.placed[&index], self.file.section_by_index(index)?.size()),
            None => (0, 0),
        };
        let mut writes = Vec::new();
        let mut moved = Vec::new();
        for (old, new, jump) in self.redirects(base, &laid.placed)? {
            moved.push((new, old.address));
            writes.push(ra_jit_live::format::Write {
                address: old.address,
                bytes: jump,
                pointers: Vec::new(),
            });
        }
        let redirected = writes.len() as u64;
        for (address, bytes) in laid.statics {
            let pointers = (laid.restored.iter())
                .filter(|it| (address..address + bytes.len() as u64).contains(it))
                .map(|it| it - address)
                .collect();
            writes.push(ra_jit_live::format::Write { address, bytes, pointers });
        }
        let statics = writes.len() as u64 - redirected;
        // What a splice wrote that this patch does not is put back as the linker wrote it.
        let written: FxHashSet<u64> = writes.iter().map(|it| it.address).collect();
        for (&address, bytes) in originals {
            if written.contains(&address) {
                continue;
            }
            let after = image.rebases.partition_point(|it| *it < address);
            let pointers = (image.rebases[after..].iter())
                .take_while(|it| **it < address + bytes.len() as u64)
                .map(|it| it - address)
                .collect();
            writes.push(ra_jit_live::format::Write { address, bytes: bytes.clone(), pointers });
        }
        let mut appended = laid.appended;
        let data = appended.split_off(laid.data_at as usize);
        let code = ra_jit_live::format::Code {
            base,
            text: appended,
            data,
            frames,
            pointers,
            writes,
            redirected,
            statics,
            lasting: first,
        };
        Ok((code, moved))
    }

    /// The jump over the entry of each function the patch replaces, by the function: to
    /// where its new code is when the patch is laid out at `base`.
    fn redirects(
        &self,
        base: u64,
        placed: &FxHashMap<SectionIndex, u64>,
    ) -> anyhow::Result<Vec<(Defined, u64, Vec<u8>)>> {
        let mut found = Vec::new();
        for replaced in &self.replaced {
            let Some(old) = replaced.old.filter(|it| it.kind == SymbolKind::Text) else { continue };
            let new = base + placed[&replaced.section] + replaced.offset;
            let shown = || String::from_utf8_lossy(replaced.name);
            let jump = if Code::reaches(new, old.address) {
                Code::branch(0x1400_0000, new, old.address)?.to_le_bytes().to_vec()
            } else {
                ensure!(
                    old.size >= Code::STUB as u64,
                    "{} is too short to jump so far from",
                    shown()
                );
                Code::stub(new, old.address, false)?.to_vec()
            };
            ensure!(old.size >= jump.len() as u64, "{} is too short to jump from", shown());
            found.push((old, new, jump));
        }
        Ok(found)
    }

    /// Lays the patch out at `base` and gives everything in it its address.
    fn lay_out(&self, base: u64) -> anyhow::Result<Laid> {
        // Where everything goes, as offsets from `base`.
        let mut end = 0u64;
        let mut reserve = |len: usize, align: u64| {
            let start = end.next_multiple_of(align);
            end = start + len as u64;
            start
        };
        let mut placed = FxHashMap::default();
        if let Some(index) = self.frames {
            placed.insert(index, reserve(self.file.section_by_index(index)?.size() as usize, 8));
        }
        let (mut stubs_at, mut data_at) = (0, 0);
        for (segment, sections) in self.sections.iter().enumerate() {
            for &index in sections {
                let section = self.file.section_by_index(index)?;
                placed.insert(index, reserve(section.size() as usize, section.align().max(1)));
            }
            if segment == TEXT {
                stubs_at = reserve(self.stubs.len() * Code::STUB, 4);
                data_at = reserve(0, PAGE);
            }
        }
        // A segment of nothing is not one.
        let slots_at = reserve(self.slots.len().max(1) * Self::SLOT, 8);
        let linkedit_at = reserve(0, PAGE);
        let mut appended = vec![0u8; linkedit_at as usize];

        // A static written in place is where the executable has it: its section is added
        // as a whole, with a copy of it that nothing may use.
        let address = |target: Target, added: i64| -> anyhow::Result<u64> {
            let (section, at) = match target {
                Target::Ours(section, at) => (section, at),
                Target::Theirs(address) => return Ok(address.wrapping_add_signed(added)),
                Target::Import(_) => bail!("the address of what is in another library"),
            };
            let in_place = self.in_place.get(&section).map_or(&[][..], |it| it);
            Ok(match in_place.iter().find(|it| it.holds(at)) {
                Some(it) => it.old.address + (at - it.offset),
                None => base + placed[&section] + at,
            }
            .wrapping_add_signed(added))
        };
        // What calls a function the executable has, or takes its address, goes through the
        // entry the executable has it at, which jumps to the newest code: a program may run
        // code of this patch while a later one replaces the function, and nothing jumps
        // from where this patch has it. Where the unwinder finds code is where it is.
        let entries: FxHashMap<(SectionIndex, u64), u64> = (self.replaced.iter())
            .filter_map(|it| {
                let old = it.old.filter(|old| old.kind == SymbolKind::Text)?;
                Some(((it.section, it.offset), old.address))
            })
            .collect();
        let entry = |target: Target, added: i64| -> anyhow::Result<u64> {
            match target {
                Target::Ours(section, at) if added == 0 => match entries.get(&(section, at)) {
                    Some(&old) => Ok(old),
                    None => address(target, added),
                },
                _ => address(target, added),
            }
        };
        let slot_of: FxHashMap<SymbolIndex, u64> = (self.slots.iter().copied())
            .zip((0..).map(|at| base + slots_at + at * Self::SLOT as u64))
            .collect();
        let stub_of: FxHashMap<SymbolIndex, u64> = (self.stubs.iter().copied())
            .zip((0..).map(|at| base + stubs_at + at * Code::STUB as u64))
            .collect();
        let slot = |relocation: &Relocation| match self.targets[&relocation.symbol] {
            Target::Import(Import { slot: Some(slot), .. }) => slot,
            _ => slot_of[&relocation.symbol],
        };

        let mut statics = Vec::new();
        // The addresses stored in data, which the loader moves with the program: those of
        // the patch, and those of the statics written in place, by where.
        let mut stored = Vec::new();
        let mut restored = FxHashSet::default();
        for section in self.file.sections() {
            let Some(&start) = placed.get(&section.index()) else { continue };
            let data = section.data()?;
            appended[start as usize..][..data.len()].copy_from_slice(data);
            let in_place = self.in_place.get(&section.index()).map_or(&[][..], |it| it);
            let mut values: Vec<Vec<u8>> = in_place
                .iter()
                .map(|it| match data.get(it.offset as usize..(it.offset + it.size) as usize) {
                    Some(value) => value.to_vec(),
                    None => vec![0; it.size as usize],
                })
                .collect();
            for relocation in &self.relocations[&section.index()] {
                let target = self.targets[&relocation.symbol];
                let at = (start + relocation.at) as usize;
                let place = base + at as u64;
                let field = |len: usize| {
                    appended.get(at..at + len).context("a relocation past its section")
                };
                if let Some(which) = in_place.iter().position(|it| it.holds(relocation.at)) {
                    ensure!(relocation.fixup == Fixup::Absolute, "a static that stores a distance");
                    let within = (relocation.at - in_place[which].offset) as usize;
                    let value = entry(target, relocation.addend + u64_at(field(8)?, 0)? as i64)?;
                    let field = values[which].get_mut(within..within + 8);
                    field
                        .context("an address past its static")?
                        .copy_from_slice(&value.to_le_bytes());
                    restored.insert(in_place[which].old.address + within as u64);
                    continue;
                }
                match relocation.fixup {
                    Fixup::Absolute => {
                        let value =
                            entry(target, relocation.addend + u64_at(field(8)?, 0)? as i64)?;
                        put64(&mut appended, at, value);
                        stored.push(place);
                    }
                    Fixup::Distance(from, len) => {
                        let from = address(self.targets[&from], 0)?;
                        let written = match len {
                            4 => i64::from(u32_at(field(4)?, 0)? as i32),
                            _ => u64_at(field(8)?, 0)? as i64,
                        };
                        let to = address(target, relocation.addend + written)?;
                        let distance = to.wrapping_sub(from).to_le_bytes();
                        appended[at..at + len].copy_from_slice(&distance[..len]);
                    }
                    Fixup::SlotDistance => {
                        let written = i64::from(u32_at(field(4)?, 0)? as i32);
                        let to = slot(relocation).wrapping_add_signed(relocation.addend + written);
                        let distance = i32::try_from(to.wrapping_sub(place) as i64);
                        put32(
                            &mut appended,
                            at,
                            distance.context("the program is too large")? as u32,
                        );
                    }
                    fixup => {
                        let instruction = u32_at(field(4)?, 0)?;
                        let instruction = match fixup {
                            Fixup::Branch => {
                                let direct = match target {
                                    Target::Import(Import { stub, .. }) => stub,
                                    _ => Some(entry(target, relocation.addend)?),
                                };
                                let to = direct
                                    .filter(|to| Code::reaches(*to, place))
                                    .unwrap_or_else(|| stub_of[&relocation.symbol]);
                                Code::branch(instruction, to, place)?
                            }
                            Fixup::Page => {
                                Code::page(instruction, entry(target, relocation.addend)?, place)?
                            }
                            Fixup::Offset => {
                                Code::offset(instruction, entry(target, relocation.addend)?)?
                            }
                            Fixup::SlotPage => Code::page(instruction, slot(relocation), place)?,
                            _ => Code::offset(instruction, slot(relocation))?,
                        };
                        put32(&mut appended, at, instruction);
                    }
                }
            }
            for (new, value) in in_place.iter().zip(values) {
                statics.push((new.old.address, value));
            }
        }
        for (&index, &slot) in &slot_of {
            put64(&mut appended, (slot - base) as usize, entry(self.targets[&index], 0)?);
            stored.push(slot);
        }
        for (&index, &stub) in &stub_of {
            let code = match self.targets[&index] {
                Target::Import(Import { stub: Some(theirs), .. }) => {
                    Code::stub(theirs, stub, false)?
                }
                Target::Import(Import { slot: Some(slot), .. }) => Code::stub(slot, stub, true)?,
                target => Code::stub(entry(target, 0)?, stub, false)?,
            };
            appended[(stub - base) as usize..][..Code::STUB].copy_from_slice(&code);
        }
        Ok(Laid { appended, placed, data_at, linkedit_at, stored, restored, statics })
    }
}

/// Where the splice put what the load commands and `__LINKEDIT` have to say.
struct Layout {
    /// The address and length of each new segment.
    text: (u64, u64),
    data: (u64, u64),
    rebases: Vec<u64>,
    /// The entries of the symbol table that get another address.
    renamed: Vec<(u64, u64)>,
}

impl Layout {
    /// The header and load commands, and `__LINKEDIT` up to its signature at `offset`.
    fn linked(&self, image: &Image, offset: u64) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let old = image.linkedit();
        let address = self.data.0 + self.data.1;
        let mut commands = image.commands.clone();

        // The segments, of one section each, as long as it: `dyld_info` stops at a fixup in
        // no section. The patch's frames are not one of them, for dyld looks for the
        // unwinder's tables only in `__TEXT`.
        let mut added = Vec::new();
        let segment =
            |name: &[u8], (address, len): (u64, u64), protection: u32, section: (&[u8], u32)| {
                let mut command = vec![0; SEGMENT + SECTION];
                put32(&mut command, 0, macho::LC_SEGMENT_64);
                put32(&mut command, 4, (SEGMENT + SECTION) as u32);
                command[8..8 + name.len()].copy_from_slice(name);
                let offset = address - old.address + old.offset;
                for (at, field) in [address, len, offset, len].into_iter().enumerate() {
                    put64(&mut command, 24 + at * 8, field);
                }
                put32(&mut command, 56, protection);
                put32(&mut command, 60, protection);
                put32(&mut command, 64, 1);
                let (sectname, flags) = section;
                let header = &mut command[SEGMENT..];
                header[..sectname.len()].copy_from_slice(sectname);
                header[16..16 + name.len()].copy_from_slice(name);
                put64(header, 32, address);
                put64(header, 40, len);
                put32(header, 48, u32::try_from(offset)?);
                put32(header, 52, PAGE.trailing_zeros());
                put32(header, 64, flags);
                anyhow::Ok(command)
            };
        added.extend(segment(
            b"__RA_JIT_TEXT",
            self.text,
            macho::VM_PROT_READ | macho::VM_PROT_EXECUTE,
            (b"__text", macho::S_ATTR_PURE_INSTRUCTIONS | macho::S_ATTR_SOME_INSTRUCTIONS),
        )?);
        added.extend(segment(
            b"__RA_JIT_DATA",
            self.data,
            macho::VM_PROT_READ | macho::VM_PROT_WRITE,
            (b"__data", macho::S_REGULAR),
        )?);

        // `__LINKEDIT`: the fixups are written again, and what is behind them moves by as
        // much as they grew.
        let fixups = image.at.fixups;
        let (rebases, len) =
            (u32_at(&commands, fixups + 8)? as u64, u32_at(&commands, fixups + 12)? as u64);
        let rebases = if len == 0 { old.offset } else { rebases };
        let starts: Vec<u64> =
            (image.segments[..image.segments.len() - 1].iter().map(|it| it.address))
                .chain([self.text.0, self.data.0])
                .collect();
        let stream = Rebases::written(&self.rebases, &starts)?;
        let within = (rebases - old.offset) as usize;
        let mut linkedit = image.linkedit[..within].to_vec();
        linkedit.extend_from_slice(&stream);
        linkedit.extend_from_slice(&image.linkedit[within + len as usize..]);
        let moved = |at: u64| match at {
            0 => 0,
            at if at < rebases + len => at - old.offset + offset,
            at => at - old.offset + offset + stream.len() as u64 - len,
        };
        // The new code is in the section of the first new segment, which comes in front of
        // `__LINKEDIT`, of none.
        let text = u8::try_from(image.sectioned + 1)
            .ok()
            .filter(|_| u32_at(&image.commands, image.at.linkedit + 64).ok() == Some(0));
        for (entry, address) in &self.renamed {
            let entry = (moved(*entry) - offset) as usize;
            put64(&mut linkedit, entry + 8, *address);
            if let Some(text) = text {
                linkedit[entry + 5] = text;
            }
        }
        // The signature is looked for right behind the strings, which end the rest, at the
        // next multiple of sixteen: what filled up to the old one is not kept.
        let strings = moved(u64::from(u32_at(&commands, image.at.symbols + 16)?))
            + u64::from(u32_at(&commands, image.at.symbols + 20)?);
        let filled = linkedit.get((strings - offset) as usize..).context("strings past the end")?;
        ensure!(
            filled.len() < 16 && filled.iter().all(|&it| it == 0),
            "strings that are not the last of `__LINKEDIT`"
        );
        linkedit.truncate((strings - offset) as usize);
        linkedit.resize(linkedit.len().next_multiple_of(16), 0);
        let signed = offset + linkedit.len() as u64;
        let signature = image.signature.len(signed) as u64;

        let mut fields = vec![fixups + 16, fixups + 24, fixups + 32, fixups + 40];
        fields.extend([image.at.symbols + 8, image.at.symbols + 16]);
        fields.extend([32, 40, 48, 56, 64, 72].map(|at| image.at.indirect + at));
        fields.extend(image.at.data.iter().map(|at| at + 8));
        for field in fields {
            let at = moved(u64::from(u32_at(&commands, field)?));
            put32(&mut commands, field, u32::try_from(at)?);
        }
        put32(&mut commands, fixups + 8, u32::try_from(offset + within as u64)?);
        put32(&mut commands, fixups + 12, u32::try_from(stream.len())?);
        put32(&mut commands, image.at.signature + 8, u32::try_from(signed)?);
        put32(&mut commands, image.at.signature + 12, u32::try_from(signature)?);
        let len = signed + signature - offset;
        for (at, field) in
            [address, len.next_multiple_of(PAGE), offset, len].into_iter().enumerate()
        {
            put64(&mut commands, image.at.linkedit + 24 + at * 8, field);
        }
        let (count, size) = (u32_at(&commands, 16)?, u32_at(&commands, 20)?);
        put32(&mut commands, 16, count + 2);
        put32(&mut commands, 20, size + added.len() as u32);
        commands.splice(image.at.linkedit..image.at.linkedit, added);
        Ok((commands, linkedit))
    }
}

/// The patch laid out, as offsets from where it starts.
struct Laid {
    appended: Vec<u8>,
    placed: FxHashMap<SectionIndex, u64>,
    data_at: u64,
    linkedit_at: u64,
    /// The addresses stored in what is appended, which move with the program.
    stored: Vec<u64>,
    /// Those stored in the statics written in place.
    restored: FxHashSet<u64>,
    /// The statics written in place, by the address of the one each replaces.
    statics: Vec<(u64, Vec<u8>)>,
}

/// The patch laid out at `base` of a program that runs the executable `image` was read
/// from, for the program to apply in its memory.
pub(super) fn live(
    image: &Image,
    baked: &[Box<[u8]>],
    originals: &FxHashMap<u64, Vec<u8>>,
    patch: &[u8],
    base: u64,
    lasting: &mut Lasting,
) -> anyhow::Result<(ra_jit_live::format::Code, Vec<(u64, u64)>)> {
    let mut patch = Patch::read(patch, image)?;
    super::has_baked(baked, patch.replaced.iter().map(|it| it.name))?;
    lasting.place(&patch.lasting_own()?)?;
    patch.keep_lasting(|name| lasting.address(name))?;
    patch.place_live(image, originals, base, lasting)
}

/// The `LC_UUID` of the executable, which names it to a running program.
pub(super) fn uuid(image: &Image) -> Option<[u8; 16]> {
    uuid_of(&image.commands)
}

/// The `-headerpad` the executable that starts with `commands`, its header and load commands,
/// is to be linked with for a splice to have room in it, when the room it has is less: room
/// for the two segments a splice adds, of a section each, and for the unwind table, which
/// moves there (see [`frames_in_front`]), or for `__eh_frame` of a program without one, which
/// does when the patch's frames don't fit behind it (see [`frames_in_header`]), or for both
/// when the program has so many frames that none can go in front of them; and a quarter more
/// for the patch's frames and for the program to grow before it is short again. `None` for
/// one that has the room,
/// and for one that is not an AArch64 executable with unwind tables.
pub(super) fn headerpad_short(commands: &[u8]) -> anyhow::Result<Option<u64>> {
    Ok(headerpad_wanted(commands)?.and_then(|(pad, short)| short.then_some(pad)))
}

/// The `-headerpad` of [`headerpad_short`] whether or not the executable has the room, and
/// whether it is short of it.
pub(super) fn headerpad_wanted(commands: &[u8]) -> anyhow::Result<Option<(u64, bool)>> {
    let executable = u32_at(commands, 0)? == macho::MH_MAGIC_64
        && u32_at(commands, 4)? == macho::CPU_TYPE_ARM64
        && u32_at(commands, 12)? == macho::MH_EXECUTE;
    if !executable {
        return Ok(None);
    }
    let len = HEADER + u32_at(commands, 20)? as usize;
    let commands = commands.get(..len).context("the file ends early")?;
    let (mut room, mut unwind, mut frames) = (usize::MAX, None, None);
    let mut command = HEADER;
    for _ in 0..u32_at(commands, 16)? {
        if u32_at(commands, command)? == macho::LC_SEGMENT_64 {
            for nth in 0..u32_at(commands, command + 64)? as usize {
                let header = command + SEGMENT + nth * SECTION;
                let size = u64_at(commands, header + 40)? as usize;
                let offset = u32_at(commands, header + 48)? as usize;
                if offset != 0 && size != 0 {
                    room = room.min(offset);
                }
                match commands.get(header..header + 16) {
                    Some(b"__unwind_info\0\0\0") => unwind = Some(size),
                    Some(b"__eh_frame\0\0\0\0\0\0") => frames = Some(size),
                    _ => {}
                }
            }
        }
        command += u32_at(commands, command + 4)? as usize;
    }
    Ok(room_wanted(len, unwind, frames).map(|(pad, needed)| (pad, needed > room)))
}

/// The `-headerpad` an executable whose load commands are `len` bytes, with an `__unwind_info`
/// and an `__eh_frame` of those sizes, wants for splices ([`headerpad_short`]), and the room
/// after its header that is the least a splice fits in.
fn room_wanted(len: usize, unwind: Option<usize>, frames: Option<usize>) -> Option<(u64, usize)> {
    // Frames of the patch in front of the program's would take the table's 24-bit offsets
    // past 16 MB: both tables move.
    let both = |frames: usize| frames + frames / 4 > 0x00ff_ffff;
    let moved = match (unwind, frames) {
        (Some(unwind), Some(frames)) if both(frames) => unwind.next_multiple_of(8) + frames,
        (unwind, frames) => unwind.or(frames)?,
    };
    let needed = (len + 2 * (SEGMENT + SECTION)).next_multiple_of(16) + moved;
    Some(((needed + needed / 4 - len).next_multiple_of(PAGE as usize) as u64, needed))
}

/// Where in an AArch64 object that starts with `commands`, its header and load commands, its
/// `__eh_frame` and its `__compact_unwind` are: the offset in the file and the size of each it
/// has. `None` for a file that is not such an object.
pub(super) fn unwind_sections(commands: &[u8]) -> anyhow::Result<Option<[Option<(u64, u64)>; 2]>> {
    let object = u32_at(commands, 0)? == macho::MH_MAGIC_64
        && u32_at(commands, 4)? == macho::CPU_TYPE_ARM64
        && u32_at(commands, 12)? == macho::MH_OBJECT;
    if !object {
        return Ok(None);
    }
    let (mut frames, mut compact) = (None, None);
    let mut command = HEADER;
    for _ in 0..u32_at(commands, 16)? {
        if u32_at(commands, command)? == macho::LC_SEGMENT_64 {
            for nth in 0..u32_at(commands, command + 64)? as usize {
                let header = command + SEGMENT + nth * SECTION;
                let at = || -> anyhow::Result<_> {
                    let offset = u64::from(u32_at(commands, header + 48)?);
                    Ok((offset, u64_at(commands, header + 40)?))
                };
                match commands.get(header..header + 16) {
                    Some(b"__eh_frame\0\0\0\0\0\0") => frames = Some(at()?),
                    Some(b"__compact_unwind") => compact = Some(at()?),
                    _ => {}
                }
            }
        }
        command += u32_at(commands, command + 4)? as usize;
    }
    Ok(Some([frames, compact]))
}

/// What the objects of a link give the linker to lay out for the unwinder, added up over
/// them: `__eh_frame` as it is, and a function of `__unwind_info` for each frame there and
/// each entry of `__compact_unwind`. Read before the first link, for the executable to have
/// the room for splices at once: one whose frames are far more than the default room makes a
/// large program be linked twice (#173).
#[derive(Debug, Default)]
pub(super) struct Unwinding {
    frames: usize,
    functions: usize,
}

impl Unwinding {
    /// The `__eh_frame` of an object: its bytes, and a function for each frame (FDE) in it.
    pub(super) fn add_frames(&mut self, section: &[u8]) -> anyhow::Result<()> {
        self.frames += section.len();
        let mut at = 0;
        while at + 8 <= section.len() {
            let (len, id_at) = match u32_at(section, at)? {
                0 => break,
                0xffff_ffff => (usize::try_from(u64_at(section, at + 4)?)?, at + 12),
                len => (len as usize, at + 4),
            };
            if u32_at(section, id_at)? != 0 {
                self.functions += 1;
            }
            at = id_at + len;
        }
        Ok(())
    }

    /// The `__compact_unwind` of an object, `size` bytes: a function for each entry.
    pub(super) fn add_compact(&mut self, size: u64) {
        const ENTRY: u64 = 32;
        self.functions += (size / ENTRY) as usize;
    }

    /// The `-headerpad` the executable linked of these objects wants for splices, at most:
    /// ld64 lays a function's entry out in a regular page of `__unwind_info` (8 bytes, 511 to
    /// the page, each page in the first-level index), each may have an LSDA, and the header,
    /// its common encodings and personalities are under 1 KB; its `__eh_frame` is at most the
    /// objects'. A link with `-no_compact_unwind` writes no table, so the frames are what
    /// moves: the larger of the two. `None` when they have no unwind information.
    pub(super) fn headerpad(&self) -> Option<u64> {
        // The executable's load commands, which only a link knows: more than any program's
        // has, and a quarter of it is all the pad depends on.
        const COMMANDS: usize = 0x4000;
        const PER_PAGE: usize = (0x1000 - 8) / 8;
        let pages = self.functions.div_ceil(PER_PAGE);
        let unwind = (self.functions != 0)
            .then(|| 0x400 + (pages + 1) * 12 + pages * 0x1000 + self.functions * 8);
        let frames = (self.frames != 0).then_some(self.frames);
        let with_table = room_wanted(COMMANDS, unwind, frames);
        let without = room_wanted(COMMANDS, None, frames);
        with_table.max(without).map(|(pad, _)| pad)
    }
}

/// The `LC_UUID` among the load commands `commands` start with.
fn uuid_of(commands: &[u8]) -> Option<[u8; 16]> {
    let count = u32_at(commands, 16).ok()?;
    let mut at = HEADER;
    for _ in 0..count {
        let size = u32_at(commands, at + 4).ok()? as usize;
        if u32_at(commands, at).ok()? == macho::LC_UUID {
            return commands.get(at + 8..at + 24)?.try_into().ok();
        }
        at += size;
    }
    None
}

/// What a splice writes.
struct Addition {
    /// Where in the file `appended` goes: over `__LINKEDIT`.
    offset: u64,
    /// The new segments and `__LINKEDIT` up to its signature.
    appended: Vec<u8>,
    /// What changes of the file before that, by where it is: code and statics, which a
    /// patch applied to the running program puts back where it writes nothing.
    overwritten: Vec<(u64, Vec<u8>)>,
    /// The load commands and the unwinder's tables, which it leaves: the header in its
    /// memory says where they are.
    tables: Vec<(u64, Vec<u8>)>,
    redirected: usize,
    /// Where the statics the patch defines anew are.
    lasting: Vec<(Box<[u8]>, u64)>,
}

/// Brings the executable at `path` from what the linker wrote to that with `patch` in place
/// of the functions it defines.
pub(super) fn splice(
    image: &Image,
    path: &Path,
    baked: &[Box<[u8]>],
    undo: &mut Vec<(u64, Vec<u8>)>,
    originals: &mut FxHashMap<u64, Vec<u8>>,
    stamp: &mut Stamp,
    last: &mut Option<Appended>,
    patch: &[u8],
    give_up: &AtomicBool,
) -> anyhow::Result<(Spliced, Lasting)> {
    let start = std::time::Instant::now();
    let mut steps = Vec::new();
    let mut step = |name: &'static str, from: &mut std::time::Instant| {
        steps.push((name, from.elapsed()));
        *from = std::time::Instant::now();
    };
    let mut from = start;
    let patch = Patch::read(patch, image)?;
    super::has_baked(baked, patch.replaced.iter().map(|it| it.name))?;
    let addition = patch.place(image)?;
    step("place", &mut from);

    ensure!(Stamp::of(&File::open(path)?)? == *stamp, "the executable was written by someone else");
    let mut copy = path.as_os_str().to_owned();
    copy.push(".spliced");
    _ = std::fs::remove_file(&copy);
    super::go_on(give_up)?;
    std::fs::copy(path, &copy)?;
    step("copy", &mut from);
    // What is undone of the last splice is the copy's: the executable keeps it until the
    // rename, and a splice given up on leaves it as it was.
    let undone = undo.clone();
    let mut fresh = None;
    let written = (|| -> anyhow::Result<Stamp> {
        super::go_on(give_up)?;
        let file = OpenOptions::new().read(true).write(true).open(&copy)?;
        for (offset, bytes) in undo.drain(..).rev() {
            super::write_all_at(&file, &bytes, offset)?;
        }
        file.set_len(addition.offset)?;
        let kept = addition.overwritten.len();
        let dirty: Vec<(u64, Vec<u8>)> =
            addition.overwritten.iter().chain(&addition.tables).cloned().collect();
        for (nth, (offset, bytes)) in dirty.iter().enumerate() {
            let mut old = vec![0; bytes.len()];
            super::read_exact_at(&file, &mut old, *offset)?;
            // The last splice is undone: this is what the linker wrote.
            if nth < kept {
                let address = image.address_of(*offset)?;
                let known = originals.entry(address).or_default();
                if known.len() < old.len() {
                    known.clone_from(&old);
                }
            }
            undo.push((*offset, old));
            super::write_all_at(&file, bytes, *offset)?;
        }
        step("overwrite", &mut from);
        let (signature, hashes) = image.signature.of(
            &file,
            addition.offset,
            &dirty,
            &addition.appended,
            last.as_ref(),
            give_up,
        )?;
        fresh = Some(hashes);
        step("sign", &mut from);
        // In parts, to give up between them.
        for (index, part) in addition.appended.chunks(1 << 20).enumerate() {
            super::go_on(give_up)?;
            super::write_all_at(&file, part, addition.offset + (index << 20) as u64)?;
        }
        super::write_all_at(&file, &signature, addition.offset + addition.appended.len() as u64)?;
        step("append", &mut from);
        super::go_on(give_up)?;
        std::fs::rename(&copy, path)?;
        step("rename", &mut from);
        Stamp::of(&file)
    })();
    if std::env::var_os("RA_JIT_STATS").is_some() {
        let shown: Vec<String> =
            steps.iter().map(|(name, took)| format!("{name}={}", took.as_micros())).collect();
        eprintln!("splice-us: {} bytes={}", shown.join(" "), addition.appended.len());
    }
    if written.is_err() {
        _ = std::fs::remove_file(&copy);
        *undo = undone;
    }
    let spliced =
        Spliced { redirected: addition.redirected, appended: addition.appended.len(), copied: 0 };
    if let Some(hashes) = fresh {
        let (offset, page, bytes) = (addition.offset, image.signature.page, addition.appended);
        *last = Some(Appended { offset, page, bytes, hashes });
    }
    *stamp = written?;
    Ok((spliced, Lasting::in_executable(addition.lasting)))
}

#[cfg(test)]
mod tests {
    use object::macho;

    use super::{
        HEADER, PAGE, Pointer, SECTION, SEGMENT, Table, Unwinding, frames_behind, headerpad_short,
        put32, put64, shifted, u32_at, unwind_sections,
    };

    /// The header and load commands of an executable of one segment, whose code starts
    /// `room` bytes into the file and whose unwind table is `unwind` bytes.
    fn commands(room: u32, unwind: u64) -> Vec<u8> {
        with_frames(room, unwind, 0)
    }

    /// Those of [`commands`], and `__eh_frame` of `frames` bytes behind the table.
    fn with_frames(room: u32, unwind: u64, frames: u64) -> Vec<u8> {
        let mut sections =
            vec![(&b"__text"[..], 0x1000, room), (b"__unwind_info", unwind, room + 0x1000)];
        if frames != 0 {
            sections.push((b"__eh_frame", frames, room + 0x1000 + unwind as u32));
        }
        let len = SEGMENT + sections.len() * SECTION;
        let mut bytes = vec![0; HEADER + len];
        put32(&mut bytes, 0, macho::MH_MAGIC_64);
        put32(&mut bytes, 4, macho::CPU_TYPE_ARM64);
        put32(&mut bytes, 12, macho::MH_EXECUTE);
        put32(&mut bytes, 16, 1);
        put32(&mut bytes, 20, len as u32);
        put32(&mut bytes, HEADER, macho::LC_SEGMENT_64);
        put32(&mut bytes, HEADER + 4, len as u32);
        bytes[HEADER + 8..HEADER + 14].copy_from_slice(b"__TEXT");
        put32(&mut bytes, HEADER + 64, sections.len() as u32);
        for (nth, (name, size, offset)) in sections.into_iter().enumerate() {
            let at = HEADER + SEGMENT + nth * SECTION;
            bytes[at..at + name.len()].copy_from_slice(name);
            put64(&mut bytes, at + 40, size);
            put32(&mut bytes, at + 48, offset);
        }
        bytes
    }

    #[test]
    fn room_for_the_unwind_table() {
        let len = (HEADER + SEGMENT + 2 * SECTION) as u64;
        let added = 2 * (SEGMENT + SECTION) as u64;
        let needed = |unwind: u64| (len + added).next_multiple_of(16) + unwind;
        assert_eq!(headerpad_short(&commands(0x10_0000, 0x1000)).unwrap(), None);
        assert_eq!(headerpad_short(&commands(needed(0x1000) as u32, 0x1000)).unwrap(), None);
        // The table of a GUI app of 570 crates, in what Apple's linker leaves by default.
        let unwind = 3_231_816;
        let pad = headerpad_short(&commands(0x10_0000, unwind)).unwrap().unwrap();
        assert!(len + pad >= needed(unwind) + needed(unwind) / 4);
        assert!(pad.is_multiple_of(PAGE) && len + pad < needed(unwind) * 2);
        let pad = headerpad_short(&commands(needed(0x1000) as u32 - 1, 0x1000)).unwrap();
        assert!(pad.is_some());
        let mut library = commands(0x100, 0x1000);
        put32(&mut library, 12, macho::MH_DYLIB);
        assert_eq!(headerpad_short(&library).unwrap(), None);
    }

    #[test]
    fn room_for_both_tables_of_a_program_of_16_mb_of_frames() {
        let unwind = 0x10_0000;
        // Room for the table alone: the patch's frames go in front of the program's.
        let pad = headerpad_short(&with_frames(0x1000, unwind, 0x40_0000)).unwrap().unwrap();
        assert!(pad < unwind + 0x40_0000);
        // The table can't say the program's frames are further in: both move.
        let frames = 0xf0_0000;
        let pad = headerpad_short(&with_frames(0x1000, unwind, frames)).unwrap().unwrap();
        assert!(pad >= (unwind + frames) + (unwind + frames) / 4);
    }

    #[test]
    fn a_first_link_has_the_room_its_objects_unwind_information_wants() {
        // A common entry, three frames of 24 bytes and the terminator.
        let mut section = Vec::new();
        section.extend_from_slice(&12u32.to_le_bytes());
        section.extend_from_slice(&[0; 12]);
        for _ in 0..3 {
            section.extend_from_slice(&20u32.to_le_bytes());
            section.extend_from_slice(&(section.len() as u32).to_le_bytes());
            section.extend_from_slice(&[0; 16]);
        }
        section.extend_from_slice(&[0; 4]);
        let mut unwinding = Unwinding::default();
        unwinding.add_frames(&section).unwrap();
        unwinding.add_compact(2 * 32);
        assert_eq!((unwinding.frames, unwinding.functions), (section.len(), 5));
        assert_eq!(Unwinding::default().headerpad(), None);

        // bun's base (#173): 20 MB of frames that ld64 makes no compact unwind of, which both
        // move, are far more than the default room, and the first link has them already.
        let (frames, functions) = (20 << 20, 700_000);
        let bun = Unwinding { frames, functions };
        let pad = bun.headerpad().unwrap();
        let unwind = (functions * 8) as u64;
        let linked = with_frames(0x1000, unwind, frames as u64);
        let wanted = headerpad_short(&linked).unwrap().unwrap();
        assert!(pad >= wanted && pad < 2 * wanted, "{pad:#x} for {wanted:#x}");
        let room = (HEADER + SEGMENT + 3 * SECTION) as u32 + pad as u32;
        assert_eq!(headerpad_short(&with_frames(room, unwind, frames as u64)).unwrap(), None);
    }

    #[test]
    fn an_objects_unwind_sections_are_found_by_their_load_commands() {
        let mut object = with_frames(0x1000, 0x40, 0x80);
        put32(&mut object, 12, macho::MH_OBJECT);
        let at = HEADER + SEGMENT + SECTION;
        object[at..at + 16].copy_from_slice(b"__compact_unwind");
        let [frames, compact] = unwind_sections(&object).unwrap().unwrap();
        assert_eq!(frames, Some((0x1000 + 0x1000 + 0x40, 0x80)));
        assert_eq!(compact, Some((0x1000 + 0x1000, 0x40)));
        assert_eq!(unwind_sections(&with_frames(0x1000, 0x40, 0x80)).unwrap(), None);
    }

    /// One common entry (`zR`, addresses relative and 4 bytes) and one frame of the function
    /// at `function`, as laid out at `from`.
    fn frames_of(function: u64, from: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        // Common entry: length, id 0, version 1, "zR", code and data alignment, return
        // register, augmentation length 1, R = pcrel | sdata4, then nothing to do.
        let common = [0, 0, 0, 0, 1, b'z', b'R', 0, 1, 0x78, 30, 1, 0x1b, 0, 0, 0];
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&common);
        // Frame: length, back to the common entry, start, length of code, augmentation 0.
        let frame = bytes.len();
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&((frame + 4) as u32).to_le_bytes());
        let start = from + bytes.len() as u64;
        bytes.extend_from_slice(&(function.wrapping_sub(start) as i32).to_le_bytes());
        bytes.extend_from_slice(&64u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    /// A table of no pages whose one common encoding says "in `__eh_frame`, `offset` in".
    fn table_of(offset: u32) -> Vec<u8> {
        let mut table = vec![0; 40];
        for (at, value) in [(0, 1), (4, 28), (8, 1), (20, 32), (24, 1)] {
            put32(&mut table, at, value);
        }
        put32(&mut table, 28, 0x0300_0000 | offset);
        table
    }

    #[test]
    fn frames_in_front_overflow_the_table_of_a_large_program() {
        // The caller falls back to both tables in the header's room.
        assert!(!shifted(&mut table_of(0x00ff_ff00), 0x200).unwrap());
        assert!(shifted(&mut table_of(0x00ff_ff00), 0xff).unwrap());
    }

    #[test]
    fn frames_behind_leave_the_table_of_a_large_program_alone() {
        let table = table_of(0x00ff_ff00);
        let frames =
            Table { header: 0, nth: 0, first: 0, address: 0x1_0000_0000, bytes: vec![0; 0x100] };
        let (from, function) = (0x1_1000_0000, 0x1_1000_4000);
        let ours = frames_of(function, from);
        let (start, block, size) = frames_behind(&frames, 0x1000, &ours, from).unwrap().unwrap();
        assert_eq!(start, 0x1_0000_0100);
        assert_eq!(size, 0x100 + block.len() as u64);
        // The frame still names the function from where it now is.
        let pointer = Pointer::all(&block).unwrap().into_iter().find(|it| it.at > 20).unwrap();
        let at = start as i64 + pointer.at as i64;
        assert_eq!((at + pointer.value(&block).unwrap()) as u64, function);
        // Nothing of the table changes, so its offsets near 16 MB are fine.
        assert_eq!(u32_at(&table, 28).unwrap(), 0x0300_0000 | 0x00ff_ff00);
        // No room: the caller falls back to frames in front.
        assert!(frames_behind(&frames, 8, &ours, from).unwrap().is_none());
    }
}
