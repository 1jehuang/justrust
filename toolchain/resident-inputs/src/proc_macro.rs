//! Bounded structural inspection only. Success does not authorize admission:
//! callers must independently verify the complete runtime closure and its identity.
//! This module never loads or executes the inspected object.

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_HEADERS: usize = 1024;
const MAX_DYNAMIC: usize = 4096;
const MAX_STRINGS: usize = 1024 * 1024;
const MAX_NEEDED: usize = 32;
const MAX_NAME: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

fn range(bytes: &[u8], offset: u64, size: u64) -> Result<&[u8]> {
    let end = offset.checked_add(size).ok_or(Error("offset overflow"))?;
    let start = usize::try_from(offset).map_err(|_| Error("offset overflow"))?;
    let end = usize::try_from(end).map_err(|_| Error("offset overflow"))?;
    bytes.get(start..end).ok_or(Error("truncated range"))
}
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

#[derive(Clone, Copy)]
struct Load {
    address: u64,
    offset: u64,
    size: u64,
}
fn mapped<'a>(bytes: &'a [u8], loads: &[Load], address: u64, size: u64) -> Result<&'a [u8]> {
    let end = address.checked_add(size).ok_or(Error("address overflow"))?;
    let mut result = None;
    for load in loads {
        let load_end = load
            .address
            .checked_add(load.size)
            .ok_or(Error("address overflow"))?;
        if address >= load.address && end <= load_end {
            if result.is_some() {
                return Err(Error("ambiguous virtual address mapping"));
            }
            let offset = load
                .offset
                .checked_add(address - load.address)
                .ok_or(Error("offset overflow"))?;
            result = Some(range(bytes, offset, size)?);
        }
    }
    result.ok_or(Error("virtual range is not file backed"))
}

/// Inspect an ELF64 little-endian x86-64 shared object and return its DT_NEEDED
/// names in table order. This allowlist is structural screening, not a trust
/// decision about the libraries resolved at runtime.
pub fn validate_proc_macro_elf(bytes: &[u8]) -> Result<Vec<String>> {
    if bytes.len() > MAX_BYTES {
        return Err(Error("object byte budget exceeded"));
    }
    let header = range(bytes, 0, 64)?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 || header[6] != 1 {
        return Err(Error("expected ELF64 little endian version 1"));
    }
    if u16_at(header, 16) != 3 || u16_at(header, 18) != 62 || u32_at(header, 20) != 1 {
        return Err(Error("expected x86-64 ET_DYN"));
    }
    if u16_at(header, 52) != 64 || u16_at(header, 54) != 56 {
        return Err(Error("invalid ELF or program header size"));
    }
    let count = usize::from(u16_at(header, 56));
    if count == 0 || count > MAX_HEADERS {
        return Err(Error("program header budget exceeded"));
    }
    let headers = range(bytes, u64_at(header, 32), (count as u64) * 56)?;
    let mut loads = Vec::new();
    let mut dynamic = None;
    for ph in headers.as_chunks::<56>().0 {
        let kind = u32_at(ph, 0);
        let offset = u64_at(ph, 8);
        let address = u64_at(ph, 16);
        let size = u64_at(ph, 32);
        let memory = u64_at(ph, 40);
        range(bytes, offset, size)?;
        if kind == 1 || kind == 2 {
            if size > memory || address.checked_add(memory).is_none() {
                return Err(Error("invalid segment extent"));
            }
            if kind == 1 {
                loads.push(Load {
                    address,
                    offset,
                    size,
                });
            } else if dynamic.replace((offset, address, size)).is_some() {
                return Err(Error("multiple dynamic segments"));
            }
        }
    }
    let (offset, address, size) = dynamic.ok_or(Error("missing dynamic segment"))?;
    if size == 0 || size % 16 != 0 || size / 16 > MAX_DYNAMIC as u64 {
        return Err(Error("invalid dynamic table size or budget"));
    }
    let table = range(bytes, offset, size)?;
    if mapped(bytes, &loads, address, size)?.as_ptr() != table.as_ptr() {
        return Err(Error("dynamic mapping disagrees with file offset"));
    }
    let mut strings = None;
    let mut string_size = None;
    let mut needed = Vec::new();
    let mut terminated = false;
    for entry in table.as_chunks::<16>().0 {
        let tag = u64_at(entry, 0);
        let value = u64_at(entry, 8);
        if terminated {
            if tag != 0 || value != 0 {
                return Err(Error("nonzero data after dynamic terminator"));
            }
            continue;
        }
        match tag {
            0 => terminated = true,
            1 => {
                if needed.len() == MAX_NEEDED {
                    return Err(Error("dependency budget exceeded"));
                }
                needed.push(value);
            }
            5 => {
                if strings.replace(value).is_some() {
                    return Err(Error("duplicate DT_STRTAB"));
                }
            }
            10 => {
                if string_size.replace(value).is_some() {
                    return Err(Error("duplicate DT_STRSZ"));
                }
            }
            15 | 29 | 0x6fff_fefb | 0x6fff_fefc | 0x7fff_fffd | 0x7fff_ffff => {
                return Err(Error("forbidden dynamic loader directive"));
            }
            // DT_FLAGS: DF_ORIGIN requests $ORIGIN processing.
            30 if value & 1 != 0 => return Err(Error("forbidden dynamic loader flags")),
            // DT_FLAGS_1: DF_1_ORIGIN and DF_1_NODEFLIB alter loader resolution.
            0x6fff_fffb if value & (0x80 | 0x800) != 0 => {
                return Err(Error("forbidden dynamic loader flags"));
            }
            _ => {}
        }
    }
    if !terminated {
        return Err(Error("unterminated dynamic table"));
    }
    let size = string_size.ok_or(Error("missing DT_STRSZ"))?;
    if size == 0 || size > MAX_STRINGS as u64 {
        return Err(Error("string table budget exceeded"));
    }
    let strings = mapped(
        bytes,
        &loads,
        strings.ok_or(Error("missing DT_STRTAB"))?,
        size,
    )?;
    if strings[0] != 0 || strings[strings.len() - 1] != 0 {
        return Err(Error("malformed string table"));
    }
    needed
        .into_iter()
        .map(|offset| {
            let offset = usize::try_from(offset).map_err(|_| Error("string offset overflow"))?;
            let tail = strings
                .get(offset..)
                .ok_or(Error("string offset out of bounds"))?;
            let length = tail
                .iter()
                .take(MAX_NAME + 1)
                .position(|&b| b == 0)
                .ok_or(Error("unbounded dependency name"))?;
            let name =
                std::str::from_utf8(&tail[..length]).map_err(|_| Error("non UTF-8 dependency"))?;
            match name {
                "libgcc_s.so.1"
                | "libc.so.6"
                | "libm.so.6"
                | "libpthread.so.0"
                | "libdl.so.2"
                | "librt.so.1"
                | "ld-linux-x86-64.so.2" => Ok(name.to_owned()),
                _ => Err(Error("unknown runtime dependency")),
            }
        })
        .collect()
}
