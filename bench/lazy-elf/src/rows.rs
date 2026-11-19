#[cfg(test)]
#[path = "tests.rs"]
mod tests;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::ops::Range;
const BLOCK: usize = 8192;
const NONE: u32 = u32::MAX;
const NAMES: [&[u8]; 3] = [b".rela.dyn", b".eh_frame_hdr", b".symtab"];
type Sections = [Option<Range<usize>>; 3];
fn u16at(b: &[u8], i: usize) -> usize {
    u16::from_le_bytes(b[i..i + 2].try_into().unwrap()) as usize
}
fn u32at(b: &[u8], i: usize) -> u64 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap()).into()
}
fn u64at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}
fn read(r: &mut (impl Read + Seek), size: u64, at: u64, b: &mut [u8]) -> Option<()> {
    if at.checked_add(b.len() as u64)? > size {
        return None;
    }
    r.seek(SeekFrom::Start(at)).ok()?;
    r.read_exact(b).ok()
}
fn sections(r: &mut (impl Read + Seek), size: u64) -> Option<Sections> {
    let mut h = [0; 64];
    read(r, size, 0, &mut h)?;
    let count = u16at(&h, 60);
    let names = u16at(&h, 62);
    if h[..7] != *b"\x7fELF\x02\x01\x01" || u16at(&h, 58) != 64 || count > 1024 || names >= count {
        return None;
    }
    let mut table = vec![0; count * 64];
    read(r, size, u64at(&h, 40), &mut table)?;
    let strtab = &table[names * 64..];
    let n = u64at(strtab, 32);
    if n > 65536 || u32at(strtab, 4) != 3 {
        return None;
    }
    let mut strings = vec![0; n as usize];
    read(r, size, u64at(strtab, 24), &mut strings)?;
    let mut result: Sections = Default::default();
    for s in table.chunks_exact(64) {
        let Some(name) = strings.get(u32at(s, 0) as usize..) else {
            continue;
        };
        let Some(k) = NAMES
            .iter()
            .position(|n| name.starts_with(n) && name.get(n.len()) == Some(&0))
        else {
            continue;
        };
        let at = u64at(s, 24);
        let len = u64at(s, 32);
        let end = at.checked_add(len)?;
        let row = row(k);
        let skip = if k == 1 { 12 } else { 0 };
        if result[k].is_some()
            || end > size
            || len < skip + 3 * row as u64
            || !(len - skip).is_multiple_of(row as u64)
            || u32at(s, 4) != [4, 1, 2][k]
        {
            return None;
        }
        if k == 1 {
            let mut hdr = [0; 12];
            read(r, size, at, &mut hdr)?;
            if hdr[..4] != [1, 0x1b, 3, 0x3b] || u32at(&hdr, 8) * 8 != len - 12 {
                return None;
            }
        } else if u64at(s, 56) != 24 {
            return None;
        }
        result[k] = Some(usize::try_from(at + skip).ok()?..usize::try_from(end).ok()?);
    }
    Some(result)
}
fn row(k: usize) -> usize {
    if k == 1 { 8 } else { 24 }
}
fn key(b: &[u8], k: usize) -> u64 {
    let mut h = 1u64;
    let mut mix = |v| h = (h.rotate_left(19) ^ v).wrapping_mul(0xd6e8feb86659fd93);
    let d64 = |a, c| u64at(b, a).wrapping_sub(u64at(b, c));
    let d32 = |a, c| u32at(b, a).wrapping_sub(u32at(b, c));
    match k {
        0 => {
            for v in [
                d64(24, 0),
                d64(40, 16),
                u64at(b, 8),
                d64(48, 24),
                d64(64, 40),
            ] {
                mix(v)
            }
        }
        1 => {
            for v in [d32(8, 0), d32(12, 4), d32(16, 8), d32(20, 12)] {
                mix(v)
            }
        }
        _ => {
            for v in [
                u32at(b, 4),
                u64at(b, 16),
                u64at(b, 40),
                u64at(b, 64),
                d32(24, 0) & 0xffffffff,
                d32(48, 24) & 0xffffffff,
            ] {
                mix(v)
            }
        }
    }
    h
}
pub struct Rows {
    old: Sections,
    new: Sections,
    active: Option<usize>,
    table: Vec<u32>,
}
impl Rows {
    pub fn detect(b: &[u8], r: &mut (impl Read + Seek), size: u64) -> Self {
        Self {
            old: sections(&mut Cursor::new(b), b.len() as u64).unwrap_or_default(),
            new: sections(r, size).unwrap_or_default(),
            active: None,
            table: vec![],
        }
    }
    pub fn choose(&mut self, b: &[u8], pos: usize, data: &[u8], ordinary: u32) -> u32 {
        let n = data.len();
        let k = self.new.iter().position(|r| {
            r.as_ref()
                .is_some_and(|r| pos >= r.start && pos + n <= r.end)
        });
        if k != self.active {
            self.table = Vec::new();
            self.active = k;
        }
        let Some(k) = k else { return ordinary };
        let Some(old) = &self.old[k] else {
            return ordinary;
        };
        let prior = if ordinary == NONE {
            None
        } else {
            b.get(ordinary as usize..ordinary as usize + n)
        };
        if prior.is_some_and(|o| {
            (0..n).step_by(31).filter(|&i| data[i] == o[i]).count() * 8 >= n.div_ceil(31) * 7
        }) {
            return ordinary;
        }
        let row = row(k);
        let entries = (old.len() / row).next_power_of_two().min(1 << 20);
        let bucket = |h: u64| (h >> (64 - entries.ilog2())) as usize;
        if self.table.is_empty() {
            if self.table.try_reserve_exact(entries).is_err() {
                return ordinary;
            }
            self.table.resize(entries, NONE);
            for i in (old.start..=old.end - 3 * row).step_by(row) {
                self.table[bucket(key(&b[i..], k))] = i as u32;
            }
        }
        let start = self.new[k].as_ref().unwrap().start;
        let skip = (row - (pos - start) % row) % row;
        let mut votes = [0u32; BLOCK.div_ceil(24)];
        let mut count = 0;
        for i in (skip..n.saturating_sub(3 * row - 1)).step_by(if k == 1 { 24 } else { 48 }) {
            let h = key(&data[i..], k);
            let hit = self.table[bucket(h)];
            if hit == NONE || key(&b[hit as usize..], k) != h {
                continue;
            }
            if let Some(at) = (hit as usize).checked_sub(i)
                && at >= old.start
                && at + n <= old.end
            {
                votes[count] = at as u32;
                count += 1;
            }
        }
        let votes = &mut votes[..count];
        votes.sort_unstable();
        let Some(v) = votes
            .chunk_by(|a, b| a == b)
            .filter(|v| v.len() >= 3)
            .max_by_key(|v| v.len())
        else {
            return ordinary;
        };
        let at = v[0] as usize;
        let same = |o: &[u8]| data.iter().zip(o).filter(|(x, y)| x == y).count();
        if same(&b[at..at + n])
            > prior.map_or_else(|| data.iter().filter(|&&x| x == 0).count(), same)
        {
            at as u32
        } else {
            ordinary
        }
    }
}
