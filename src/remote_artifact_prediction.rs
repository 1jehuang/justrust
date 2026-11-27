//! Negotiated exact delta for large, closely related build outputs. Offsets are
//! local to the single previously returned file, never paths or external IDs.
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read, Seek, Write};
use std::path::Path;
use std::process::{Command, Stdio};

#[path = "remote_artifact_unwind.rs"]
mod unwind;

const BLOCK: usize = 8192;
const LITERAL: u32 = u32::MAX;
pub(super) const MAX: u64 = 512 << 20;

pub(super) fn medium_elf(path: &Path, size: u64) -> bool {
    if !((16 << 20)..(256 << 20)).contains(&size) {
        return false;
    }
    let mut magic = [0; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"\x7fELF"
}

fn read_basis(path: &Path) -> Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let metadata = f.metadata()?;
    ensure!(metadata.is_file(), "delta basis is not a regular file");
    let size = metadata.len();
    ensure!(size <= MAX, "oversized delta basis");
    let mut data = Vec::new();
    data.try_reserve_exact(size as usize)?;
    data.resize(size as usize, 0);
    f.read_exact(&mut data)?;
    ensure!(f.read(&mut [0])? == 0, "delta basis changed size");
    Ok(data)
}

fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn bucket(w: u64, bits: u32) -> usize {
    ((w ^ (w >> 29)).wrapping_mul(0x9e3779b185ebca87) >> (64 - bits)) as usize
}

fn encode(
    basis: &[u8],
    size: u64,
    mut input: impl Read,
    mut out: impl Write,
    hints: Option<&unwind::Unwind>,
) -> Result<()> {
    ensure!(
        size <= MAX && basis.len() as u64 <= MAX,
        "oversized prediction"
    );
    let bits = basis
        .len()
        .div_ceil(16)
        .max(2)
        .next_power_of_two()
        .ilog2()
        .min(24);
    let mut table = Vec::new();
    table.try_reserve_exact(1 << bits)?;
    table.resize(1 << bits, LITERAL);
    for i in (0..basis.len().saturating_sub(7)).step_by(16) {
        table[bucket(word(basis, i), bits)] = i as u32;
    }
    let mut hash = Sha256::new();
    let mut previous_shift = 0i64;
    let mut referenced = 0;
    let mut block = [0u8; BLOCK];
    for pos in (0..size as usize).step_by(BLOCK) {
        let len = BLOCK.min(size as usize - pos);
        let block = &mut block[..len];
        input.read_exact(block)?;
        hash.update(&*block);
        let source = hints
            .and_then(|h| h.source(basis, block, pos))
            .unwrap_or_else(|| best_source(basis, block, pos, previous_shift, bits, &table));
        if source != LITERAL {
            referenced += len;
            previous_shift = i64::from(source) - pos as i64;
            for (v, old) in block.iter_mut().zip(&basis[source as usize..]) {
                *v = v.wrapping_sub(*old);
            }
        }
        // Reject unrelated or substantially reordered binaries early. No wire
        // frame has been emitted, so the caller can still send its usual patch.
        if pos + len == 8 * 1024 * 1024 && referenced < 2 * 1024 * 1024 {
            bail!("insufficient aligned basis matches");
        }
        out.write_all(&source.to_le_bytes())?;
        out.write_all(block)?;
    }
    ensure!(input.read(&mut [0])? == 0, "artifact grew");
    out.write_all(&hash.finalize())?;
    Ok(())
}

fn best_source(
    basis: &[u8],
    block: &[u8],
    pos: usize,
    previous_shift: i64,
    bits: u32,
    table: &[u32],
) -> u32 {
    let len = block.len();
    // Most neighboring blocks have moved together. Probe their old alignment
    // first, only touching the random-access index when the match is weak.
    if let Ok(at) = usize::try_from(pos as i64 + previous_shift)
        && let Some(old) = basis.get(at..at + len)
        && (0..len).step_by(31).filter(|&i| old[i] == block[i]).count() * 8 >= len.div_ceil(31) * 7
    {
        return at as u32;
    }
    // Hash matches only propose alignments. A collision can affect ratio,
    // never reconstruction: every chosen reference is corrected by residuals.
    let mut candidates = [0i64; 130];
    candidates[0] = previous_shift;
    let mut count = 2;
    for j in (0..len.saturating_sub(23)).step_by(BLOCK / 8) {
        for k in j..j + 16 {
            let w = word(block, k);
            let s = table[bucket(w, bits)];
            if s != LITERAL && word(basis, s as usize) == w {
                candidates[count] = i64::from(s) - (pos + k) as i64;
                count += 1;
            }
        }
    }
    let candidates = &mut candidates[..count];
    candidates.sort_unstable();
    let mut last = None;
    let mut best = 0;
    let mut source = LITERAL;
    for &shift in candidates.iter() {
        if last == Some(shift) {
            continue;
        }
        last = Some(shift);
        let Ok(at) = usize::try_from(pos as i64 + shift) else {
            continue;
        };
        let Some(old) = basis.get(at..at + len) else {
            continue;
        };
        let score = (0..len).step_by(17).filter(|&i| old[i] == block[i]).count();
        if score > best && score * 17 > len / 2 {
            best = score;
            source = at as u32;
        }
    }
    source
}

fn decode(basis: &[u8], size: u64, mut input: impl Read, mut out: impl Write) -> Result<()> {
    ensure!(size <= MAX, "oversized prediction");
    let mut hash = Sha256::new();
    let mut block = [0u8; BLOCK];
    for pos in (0..size as usize).step_by(BLOCK) {
        let len = BLOCK.min(size as usize - pos);
        let block = &mut block[..len];
        let mut addr = [0; 4];
        input.read_exact(&mut addr)?;
        input.read_exact(block)?;
        let source = u32::from_le_bytes(addr);
        if source != LITERAL {
            let at = source as usize;
            let old = basis
                .get(at..at.checked_add(len).context("delta range overflow")?)
                .context("delta basis out of bounds")?;
            for (v, &b) in block.iter_mut().zip(old) {
                *v = v.wrapping_add(b);
            }
        }
        hash.update(&*block);
        out.write_all(block)?;
    }
    let mut expected = [0; 32];
    input.read_exact(&mut expected)?;
    ensure!(
        hash.finalize()[..] == expected,
        "reconstructed artifact checksum mismatch"
    );
    ensure!(input.read(&mut [0])? == 0, "trailing prediction data");
    Ok(())
}

pub(super) fn compress(old: &Path, new: &Path, size: u64, compressed: &Path) -> Result<()> {
    let basis = read_basis(old)?;
    let mut input = File::open(new)?;
    let hints = unwind::Unwind::detect(&basis, &mut input, size as usize);
    input.rewind()?;
    // Every worker: the residual is mostly zeros, and two workers took 0.81s
    // of a 0.93s return of a 205 MB executable where all of them take 0.19s
    // for the same bytes. Level 7 jobs are small, so memory stays bounded.
    let mut child = Command::new("zstd")
        .args(["-q", "-f", "-7", "-T0", "-", "-o"])
        .arg(compressed)
        .stdin(Stdio::piped())
        .spawn()?;
    let res = encode(
        &basis,
        size,
        BufReader::new(input),
        child.stdin.take().context("zstd stdin")?,
        hints.as_ref(),
    );
    let status = child.wait()?;
    res?;
    ensure!(status.success(), "prediction compression failed");
    Ok(())
}

pub(super) fn expand<R: Read>(
    old: &Path,
    size: u64,
    mut frame: std::io::Take<R>,
    output: File,
) -> Result<()> {
    ensure!(size <= MAX, "oversized prediction");
    let basis = read_basis(old)?;
    let mut child = Command::new("zstd")
        .args(["-q", "-d", "-c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().context("zstd stdin")?;
    let stdout = child.stdout.take().context("zstd stdout")?;
    // Concurrently drain stdout. On a malformed recipe decode returns and drops
    // its pipe, so the decoder exits rather than blocking on more output.
    let decoded = std::thread::scope(|scope| -> Result<()> {
        let join = std::thread::Builder::new().spawn_scoped(scope, move || {
            decode(&basis, size, BufReader::new(stdout), output)
        })?;
        let copied = std::io::copy(&mut frame, &mut stdin);
        drop(stdin);
        join.join()
            .map_err(|_| anyhow::anyhow!("prediction worker failed"))??;
        copied?;
        ensure!(frame.limit() == 0, "truncated prediction frame");
        Ok(())
    });
    if decoded.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    decoded?;
    if !status.success() {
        bail!("prediction decompression failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_recipe_never_reads_beyond_basis_or_declared_output() {
        let mut input = Vec::new();
        encode(b"", 1, &b"x"[..], &mut input, None).unwrap();
        let mut result = Vec::new();
        decode(b"", 1, &input[..], &mut result).unwrap();
        assert_eq!(result, b"x");
        for len in [0, 3, 4, 5, input.len() - 1] {
            assert!(decode(b"", 1, &input[..len], std::io::sink()).is_err());
        }
        input.push(0);
        assert!(decode(b"", 1, &input[..], std::io::sink()).is_err());
        input.pop();
        input[..4].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
        assert!(decode(b"", 1, &input[..], std::io::sink()).is_err());
        assert!(decode(b"", MAX + 1, &b""[..], std::io::sink()).is_err());
    }

    #[test]
    fn short_and_partial_blocks_allow_any_literal_bytes() {
        for len in [0, 1, 7, 8, 23, 24, BLOCK - 1, BLOCK, BLOCK + 1] {
            let new: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let mut recipe = Vec::new();
            encode(&[], len as u64, &new[..], &mut recipe, None).unwrap();
            let mut got = Vec::new();
            decode(&[], len as u64, &recipe[..], &mut got).unwrap();
            assert_eq!(got, new);
        }
    }
}
