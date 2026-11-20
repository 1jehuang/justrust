//! Sender-only alignment hints for relocated ELF64 unwind tables. A hint is
//! just a checked byte offset into the same old file; receivers need no parser.
use object::read::elf::{FileHeader, ProgramHeader};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::ops::Range;

const ROW: usize = 8;

fn rows(reader: &mut (impl Read + Seek), file_size: usize) -> Option<Range<usize>> {
    // Only inspect a bounded program-header prefix and one fixed-size header,
    // never the (often huge) ELF string or symbol tables. Decline exotic ELF.
    let mut prefix = [0u8; 8192];
    let n = prefix.len().min(file_size);
    reader.seek(SeekFrom::Start(0)).ok()?;
    reader.read_exact(&mut prefix[..n]).ok()?;
    let prefix = &prefix[..n];
    let header = object::elf::FileHeader64::<object::LittleEndian>::parse(prefix).ok()?;
    let e = header.endian().ok()?;
    let p = header
        .program_headers(e, prefix)
        .ok()?
        .iter()
        .find(|p| p.p_type(e) == object::elf::PT_GNU_EH_FRAME)?;
    let (offset, size) = p.file_range(e);
    let a = usize::try_from(offset).ok()?;
    let b = a.checked_add(usize::try_from(size).ok()?)?;
    if b > file_size || size < 12 {
        return None;
    }
    reader.seek(SeekFrom::Start(offset)).ok()?;
    let mut h = [0; 12];
    reader.read_exact(&mut h).ok()?;
    // Version 1, pcrel sdata4 pointer, udata4 count, datarel sdata4 pairs.
    let count = u32::from_le_bytes(h[8..].try_into().ok()?) as usize;
    if h[..4] != [1, 0x1b, 3, 0x3b]
        || !(4096..=2_097_152).contains(&count)
        || count * ROW != b - a - 12
    {
        return None;
    }
    Some(a + 12..b)
}

// Hash the two successive changes in each column of three adjacent rows.
// Absolute function PCs and FDE pointers can all move on an incremental link,
// while the spacings among neighboring functions and records stay the same.
fn key(bytes: &[u8]) -> u64 {
    let v = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let a = u64::from(v(8).wrapping_sub(v(0))) | (u64::from(v(12).wrapping_sub(v(4))) << 32);
    let b = u64::from(v(16).wrapping_sub(v(8))) | (u64::from(v(20).wrapping_sub(v(12))) << 32);
    (a.wrapping_mul(0x9e3779b185ebca87) ^ b.rotate_left(19)).wrapping_mul(0xd6e8feb86659fd93)
}

pub(super) struct Unwind {
    old: Range<usize>,
    new: Range<usize>,
    table: Vec<u32>,
    bits: u32,
}

impl Unwind {
    pub(super) fn detect(basis: &[u8], new: &mut (impl Read + Seek), size: usize) -> Option<Self> {
        let old = rows(&mut Cursor::new(basis), basis.len())?;
        let new = rows(new, size)?;
        Self::index(basis, old, new)
    }

    fn index(basis: &[u8], old: Range<usize>, new: Range<usize>) -> Option<Self> {
        let count = old.len() / ROW;
        let bits = count.next_power_of_two().ilog2().min(20);
        let mut table = Vec::new();
        table.try_reserve_exact(1 << bits).ok()?;
        table.resize(1 << bits, u32::MAX);
        for pos in (old.start..old.end - 2 * ROW).step_by(ROW) {
            let hash = key(&basis[pos..]);
            table[(hash >> (64 - bits)) as usize] = pos as u32;
        }
        Some(Self {
            old,
            new,
            table,
            bits,
        })
    }

    pub(super) fn source(&self, basis: &[u8], block: &[u8], pos: usize) -> Option<u32> {
        if pos < self.new.start || pos.checked_add(block.len())? > self.new.end {
            return None;
        }
        let skip = self.new.start.wrapping_sub(pos) % ROW;
        let mut votes = [0u32; super::BLOCK.div_ceil(3 * ROW)];
        let mut count = 0;
        for k in (skip..block.len().saturating_sub(3 * ROW - 1)).step_by(3 * ROW) {
            let h = key(&block[k..]);
            let at = self.table[(h >> (64 - self.bits)) as usize];
            if at == u32::MAX || key(&basis[at as usize..]) != h {
                continue;
            }
            let Some(start) = (at as usize).checked_sub(k) else {
                continue;
            };
            if start >= self.old.start && start + block.len() <= self.old.end {
                votes[count] = start as u32;
                count += 1;
            }
        }
        let votes = &mut votes[..count];
        votes.sort_unstable();
        votes
            .chunk_by(|a, b| a == b)
            .filter(|v| v.len() >= 3)
            .max_by_key(|v| v.len())
            .map(|v| v[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(offset: usize, count: u32, pc_bias: u32, fde_bias: u32) -> Vec<u8> {
        let size = 12 + count as usize * ROW;
        let mut b = vec![0; offset + size];
        b[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        b[32..40].copy_from_slice(&64u64.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&object::elf::PT_GNU_EH_FRAME.to_le_bytes());
        b[72..80].copy_from_slice(&(offset as u64).to_le_bytes());
        b[96..104].copy_from_slice(&(size as u64).to_le_bytes());
        b[offset..offset + 4].copy_from_slice(&[1, 0x1b, 3, 0x3b]);
        b[offset + 8..offset + 12].copy_from_slice(&count.to_le_bytes());
        let (mut pc, mut fde, mut seed) = (pc_bias, fde_bias, 3u32);
        for row in b[offset + 12..].as_chunks_mut::<ROW>().0 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            pc = pc.wrapping_add((seed >> 16) & 1023);
            fde = fde.wrapping_add(seed >> 24);
            row[..4].copy_from_slice(&pc.to_le_bytes());
            row[4..].copy_from_slice(&fde.to_le_bytes());
        }
        b
    }

    #[test]
    fn relocated_rows_propose_existing_exactly_bounded_basis_offsets() {
        let old = fixture(512, 8192, 0, 0);
        let new = fixture(1036, 8192, 0x3456_7890, u32::MAX - 981);
        let h = Unwind::detect(&old, &mut Cursor::new(&new), new.len()).unwrap();
        let pos = super::super::BLOCK;
        let block = &new[pos..pos * 2];
        assert_eq!(h.source(&old, block, pos), Some((pos - 1036 + 512) as u32));
        assert_eq!(h.source(&old, block, 0), None);
        assert_eq!(h.source(&old, block, new.len() - 1), None);
    }

    #[test]
    fn unsupported_encodings_or_ranges_decline_without_affecting_transfer() {
        let data = fixture(512, 4096, 0, 0);
        for (at, value) in [(5, 2), (32, 255), (64, 0), (512 + 3, 0xff), (512 + 8, 1)] {
            let mut b = data.clone();
            b[at] = value;
            assert!(rows(&mut Cursor::new(&b), b.len()).is_none());
        }
        assert!(rows(&mut Cursor::new(&data), data.len() - 1).is_none());
    }
}
