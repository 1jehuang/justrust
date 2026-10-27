//! Bounded, read-only decoding of owned registry archive bytes. No extraction,
//! subprocess, ambient path lookup or archive-provided link is executed.
use super::manifest::ManifestError;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_ARCHIVE: usize = 16 * 1024 * 1024;
const MAX_DECODED: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

fn error(message: &str) -> ManifestError {
    ManifestError(format!("registry archive {message}"))
}

pub(super) fn files(bytes: &[u8], stem: &str) -> Result<BTreeMap<PathBuf, Vec<u8>>, ManifestError> {
    if bytes.len() > MAX_ARCHIVE {
        return Err(error("compressed byte limit exceeded"));
    }
    let mut decoded = Vec::new();
    flate2::bufread::MultiGzDecoder::new(bytes)
        .take((MAX_DECODED + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|e| ManifestError(format!("registry archive gzip decoding failed: {e}")))?;
    if decoded.len() > MAX_DECODED {
        return Err(error("decoded byte limit exceeded"));
    }
    let mut offset = 0usize;
    let mut seen = BTreeSet::new();
    let mut files = BTreeMap::new();
    let mut remaining = MAX_ARCHIVE;
    loop {
        let header = decoded
            .get(offset..offset + 512)
            .ok_or_else(|| error("is truncated"))?;
        if header.iter().all(|b| *b == 0) {
            let tail = decoded
                .get(offset + 512..)
                .ok_or_else(|| error("is truncated"))?;
            if tail.len() < 512 || tail.iter().any(|b| *b != 0) {
                return Err(error("has missing end markers or trailing data"));
            }
            break;
        }
        if seen.len() >= MAX_ENTRIES {
            return Err(error("entry limit exceeded"));
        }
        let sum: usize = header
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    *b as usize
                }
            })
            .sum();
        if octal(&header[148..156])? != sum {
            return Err(error("header checksum mismatch"));
        }
        let mut name = text(&header[..100])?.to_owned();
        if &header[257..263] == b"ustar\0" {
            let prefix = text(&header[345..500])?;
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        } else if &header[257..263] != b"ustar " && header[257..263].iter().any(|b| *b != 0) {
            return Err(error("uses an unsupported tar header"));
        }
        let kind = header[156];
        if !matches!(kind, 0 | b'0' | b'5') {
            return Err(error(
                "uses unsupported tar links, sparse data or extended headers",
            ));
        }
        let name = if kind == b'5' {
            name.trim_end_matches('/')
        } else {
            &name
        };
        if name.is_empty()
            || name
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || name.chars().any(char::is_control)
        {
            return Err(error("has unsafe paths"));
        }
        let relative = Path::new(name)
            .strip_prefix(stem)
            .map_err(|_| error("package identity mismatch"))?
            .to_owned();
        if !seen.insert(relative.clone()) {
            return Err(error("has duplicate file paths"));
        }
        let size = octal(&header[124..136])?;
        if size > remaining || (kind == b'5' && size != 0) {
            return Err(error("file byte limit exceeded or nonempty directory"));
        }
        offset += 512;
        let data = decoded
            .get(
                offset
                    ..offset
                        .checked_add(size)
                        .ok_or_else(|| error("size overflow"))?,
            )
            .ok_or_else(|| error("is truncated"))?;
        if kind != b'5' {
            if relative.as_os_str().is_empty() {
                return Err(error("package root is not a directory"));
            }
            remaining = remaining
                .checked_sub(size.saturating_add(name.len()))
                .ok_or_else(|| error("file byte limit exceeded"))?;
            files.insert(relative, data.to_vec());
        }
        offset = offset
            .checked_add(size.div_ceil(512) * 512)
            .ok_or_else(|| error("size overflow"))?;
    }
    if !files.contains_key(Path::new("Cargo.toml")) {
        return Err(error("lacks Cargo.toml"));
    }
    Ok(files)
}

fn text(field: &[u8]) -> Result<&str, ManifestError> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    if field[end..].iter().any(|b| *b != 0) {
        return Err(error("has invalid path padding"));
    }
    std::str::from_utf8(&field[..end]).map_err(|_| error("has non-UTF-8 paths"))
}

fn octal(field: &[u8]) -> Result<usize, ManifestError> {
    let text = std::str::from_utf8(field)
        .map_err(|_| error("uses unsupported numeric fields"))?
        .trim_matches([' ', '\0']);
    if text.is_empty() {
        return Ok(0);
    }
    if !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return Err(error("uses unsupported numeric fields"));
    }
    usize::from_str_radix(text, 8).map_err(|_| error("numeric field overflow"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    fn checksum(header: &mut [u8]) {
        header[148..156].fill(b' ');
        let sum: usize = header.iter().map(|b| *b as usize).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    }
    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }
    fn tar(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (name, data, kind) in entries {
            let mut header = [0u8; 512];
            assert!(name.len() <= 100);
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
            header[156] = *kind;
            header[257..263].copy_from_slice(b"ustar\0");
            checksum(&mut header);
            bytes.extend_from_slice(&header);
            bytes.extend_from_slice(data);
            bytes.resize(bytes.len().div_ceil(512) * 512, 0);
        }
        bytes.extend_from_slice(&[0; 1024]);
        bytes
    }
    pub(crate) fn archive(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        gzip(&tar(entries))
    }

    #[test]
    fn unchanged_registry_archives_decode_without_external_tools() {
        for (stem, bytes) in [
            (
                "utf8-width-0.1.8",
                include_bytes!("../../tests/fixtures/registry-cache/utf8-width-0.1.8.crate")
                    .as_slice(),
            ),
            (
                "nix-base32-0.2.0",
                include_bytes!("../../tests/fixtures/registry-cache/nix-base32-0.2.0.crate")
                    .as_slice(),
            ),
            (
                "time-core-0.1.2",
                include_bytes!("../../tests/fixtures/registry-cache/time-core-0.1.2.crate")
                    .as_slice(),
            ),
        ] {
            let files = files(bytes, stem).unwrap();
            assert!(files.contains_key(Path::new("Cargo.toml")));
            assert!(files.contains_key(Path::new("src/lib.rs")));
        }
    }

    #[test]
    fn decoder_rejects_corruption_duplicates_escapes_links_and_hidden_members() {
        let valid = [("dep-1/Cargo.toml", b"manifest".as_slice(), b'0')];
        let bytes = archive(&valid);
        assert!(files(&bytes, "dep-1").is_ok());
        for length in [0, bytes.len() / 2, bytes.len() - 1] {
            assert!(files(&bytes[..length], "dep-1").is_err());
        }
        let mut corrupt = bytes.clone();
        let end = corrupt.len();
        corrupt[end - 8] ^= 1;
        assert!(files(&corrupt, "dep-1").is_err());
        let mut header = tar(&valid);
        header[0] ^= 1;
        assert!(files(&gzip(&header), "dep-1").is_err());
        assert!(files(&archive(&[valid[0], valid[0]]), "dep-1").is_err());
        for name in [
            "/dep-1/Cargo.toml",
            "dep-1/../Cargo.toml",
            "other/Cargo.toml",
            "dep-1/./Cargo.toml",
            "dep-1//Cargo.toml",
        ] {
            assert!(
                files(&archive(&[(name, b"", b'0')]), "dep-1").is_err(),
                "{name}"
            );
        }
        for kind in [b'1', b'2', b'3', b'4', b'6', b'L', b'x', b'g', b'S'] {
            assert!(files(&archive(&[("dep-1/Cargo.toml", b"", kind)]), "dep-1").is_err());
        }
        let mut joined = bytes.clone();
        joined.extend_from_slice(&bytes);
        assert!(files(&joined, "dep-1").is_err());
    }

    #[test]
    fn decoder_limits_compressed_decoded_and_entry_inputs() {
        assert!(
            files(&vec![0; MAX_ARCHIVE + 1], "dep-1")
                .unwrap_err()
                .0
                .contains("limit")
        );
        assert!(
            files(&gzip(&vec![0; MAX_DECODED + 1]), "dep-1")
                .unwrap_err()
                .0
                .contains("limit")
        );
        let names: Vec<_> = (0..MAX_ENTRIES + 1)
            .map(|i| format!("dep-1/file-{i}"))
            .collect();
        let entries: Vec<_> = names
            .iter()
            .map(|name| (name.as_str(), b"".as_slice(), b'0'))
            .collect();
        assert!(
            files(&archive(&entries), "dep-1")
                .unwrap_err()
                .0
                .contains("entry limit")
        );
    }
}
