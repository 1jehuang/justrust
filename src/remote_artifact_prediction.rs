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

/// Version 2 of the recipe: a run of blocks equal to the basis is one
/// 9-byte record instead of 8 KiB of zero residual each.
///
/// A one-line edit of a 206 MB debug executable changes 4036 of its 25166
/// blocks. Version 1 still pushed all 206 MB through `zstd` on the machine
/// and through `zstd -d` and a rewrite here, about a second for 0.3 MB of
/// change. Here the unchanged 84% never leaves the page cache: the machine
/// compares it, and this machine copies it file to file in the kernel.
const COPY: u8 = 0;
const RESIDUAL: u8 = 1;
const LITERAL_BLOCK: u8 = 2;

fn aligned(basis: &[u8], block: &[u8], pos: usize, shift: i64) -> Option<usize> {
    let at = usize::try_from(pos as i64 + shift).ok()?;
    (basis.get(at..at.checked_add(block.len())?)? == block).then_some(at)
}

fn encode_v2(
    basis: &[u8],
    new: &[u8],
    mut out: impl Write,
    hints: Option<&unwind::Unwind>,
) -> Result<()> {
    ensure!(
        new.len() as u64 <= MAX && basis.len() as u64 <= MAX,
        "oversized prediction"
    );
    // Built on the first block that has moved: an unchanged layout never
    // pays for 16 MB of random writes.
    let mut index: Option<(u32, Vec<u32>)> = None;
    let mut previous_shift = 0i64;
    let mut referenced = 0;
    // An open run of blocks equal to the basis: (source, length).
    let mut run: Option<(usize, usize)> = None;
    let mut residual = [0u8; BLOCK];
    for (pos, block) in (0..).step_by(BLOCK).zip(new.chunks(BLOCK)) {
        let len = block.len();
        if let Some(at) = aligned(basis, block, pos, previous_shift) {
            referenced += len;
            run = match run {
                Some((from, n)) if from + n == at => Some((from, n + len)),
                open => {
                    copy_record(&mut out, open)?;
                    Some((at, len))
                }
            };
        } else {
            copy_record(&mut out, run.take())?;
            let source = match hints.and_then(|h| h.source(basis, block, pos)) {
                Some(source) => source,
                None => {
                    let (bits, table) = match &index {
                        Some(index) => index,
                        None => index.insert(basis_index(basis)?),
                    };
                    best_source(basis, block, pos, previous_shift, *bits, table)
                }
            };
            if source == LITERAL {
                out.write_all(&[LITERAL_BLOCK])?;
                out.write_all(block)?;
            } else {
                referenced += len;
                previous_shift = i64::from(source) - pos as i64;
                let residual = &mut residual[..len];
                for ((r, v), old) in residual
                    .iter_mut()
                    .zip(block)
                    .zip(&basis[source as usize..])
                {
                    *r = v.wrapping_sub(*old);
                }
                out.write_all(&[RESIDUAL])?;
                out.write_all(&source.to_le_bytes())?;
                out.write_all(residual)?;
            }
        }
        if pos + len == 8 * 1024 * 1024 && referenced < 2 * 1024 * 1024 {
            bail!("insufficient aligned basis matches");
        }
    }
    copy_record(&mut out, run)?;
    Ok(())
}

fn copy_record(out: &mut impl Write, run: Option<(usize, usize)>) -> Result<()> {
    if let Some((from, len)) = run {
        out.write_all(&[COPY])?;
        out.write_all(&(from as u32).to_le_bytes())?;
        out.write_all(&(len as u32).to_le_bytes())?;
    }
    Ok(())
}

fn basis_index(basis: &[u8]) -> Result<(u32, Vec<u32>)> {
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
    Ok((bits, table))
}

fn read_exactly(path: &Path, size: Option<u64>) -> Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let metadata = f.metadata()?;
    ensure!(metadata.is_file(), "delta input is not a regular file");
    let len = metadata.len();
    ensure!(len <= MAX, "oversized delta input");
    ensure!(size.is_none_or(|s| s == len), "delta input changed size");
    let mut data = Vec::new();
    data.try_reserve_exact(len as usize + 1)?;
    // One byte past the end shows a file that grew while being read.
    (&mut f).take(len + 1).read_to_end(&mut data)?;
    ensure!(data.len() as u64 == len, "delta input changed size");
    Ok(data)
}

/// Machine side of version 2. The digest is of the whole new file.
pub(super) fn compress_v2(old: &Path, new: &Path, size: u64, compressed: &Path) -> Result<()> {
    let (basis, new) = std::thread::scope(|scope| {
        let basis = scope.spawn(|| read_exactly(old, None));
        let new = read_exactly(new, Some(size));
        (basis.join(), new)
    });
    let basis = basis.map_err(|_| anyhow::anyhow!("basis reader failed"))??;
    let new = new?;
    let hints = unwind::Unwind::detect(&basis, &mut std::io::Cursor::new(&new[..]), new.len());
    let mut child = Command::new("zstd")
        .args(["-q", "-f", "-7", "-T0", "-", "-o"])
        .arg(compressed)
        .stdin(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().context("zstd stdin")?;
    // SHA-256 is one core's work for the whole file: run it beside the encoder.
    let res = std::thread::scope(|scope| -> Result<()> {
        let digest = scope.spawn(|| Sha256::digest(&new));
        let mut out = std::io::BufWriter::with_capacity(1 << 20, stdin);
        let encoded = encode_v2(&basis, &new, &mut out, hints.as_ref());
        let digest = digest
            .join()
            .map_err(|_| anyhow::anyhow!("digest worker failed"))?;
        encoded?;
        out.write_all(&digest)?;
        out.flush()?;
        Ok(())
    });
    if res.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    res?;
    ensure!(status.success(), "prediction compression failed");
    Ok(())
}

/// Apply a version 2 recipe to `old`, writing `tmp` (which must not exist).
/// Returns the seconds spent on the final check: the digest the machine
/// computed over its file must be the digest of `tmp` as read back.
pub(super) fn expand_v2<R: Read>(
    old: &Path,
    size: u64,
    mut frame: std::io::Take<R>,
    tmp: &Path,
) -> Result<f64> {
    ensure!(size <= MAX, "oversized prediction");
    let basis = File::open(old)?;
    ensure!(
        basis.metadata()?.is_file(),
        "delta basis is not a regular file"
    );
    let mut output = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(tmp)?;
    let mut child = Command::new("zstd")
        .args(["-q", "-d", "-c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().context("zstd stdin")?;
    let stdout = child.stdout.take().context("zstd stdout")?;
    let decoded = std::thread::scope(|scope| -> Result<[u8; 32]> {
        let join = std::thread::Builder::new().spawn_scoped(scope, || {
            decode_v2(&basis, size, BufReader::new(stdout), &output)
        })?;
        let copied = std::io::copy(&mut frame, &mut stdin);
        drop(stdin);
        let expected = join
            .join()
            .map_err(|_| anyhow::anyhow!("prediction worker failed"))??;
        copied?;
        ensure!(frame.limit() == 0, "truncated prediction frame");
        Ok(expected)
    });
    if decoded.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let expected = decoded?;
    if !status.success() {
        bail!("prediction decompression failed");
    }
    // End to end: hash what is in the file now, not what was meant to go in.
    let checking = std::time::Instant::now();
    output.rewind()?;
    ensure!(
        output.metadata()?.len() == size,
        "reconstructed artifact size mismatch"
    );
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = output.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    ensure!(
        hash.finalize()[..] == expected,
        "reconstructed artifact checksum mismatch"
    );
    Ok(checking.elapsed().as_secs_f64())
}

/// Writes the output and returns the digest the recipe ends with. Every
/// range is checked against the basis as it is now and the declared size.
fn decode_v2(basis: &File, size: u64, mut input: impl Read, mut output: &File) -> Result<[u8; 32]> {
    let mut basis = basis;
    let mut block = [0u8; BLOCK];
    let mut old = [0u8; BLOCK];
    let mut pos = 0u64;
    while pos < size {
        let mut kind = [0];
        input.read_exact(&mut kind)?;
        let len = BLOCK.min((size - pos) as usize);
        match kind[0] {
            COPY => {
                let mut words = [0; 8];
                input.read_exact(&mut words)?;
                let source = u32::from_le_bytes(words[..4].try_into().unwrap());
                let run = u64::from(u32::from_le_bytes(words[4..].try_into().unwrap()));
                ensure!(run > 0 && run <= size - pos, "delta range overflow");
                basis.seek(std::io::SeekFrom::Start(source.into()))?;
                // File to file: the kernel copies, or shares extents, without
                // the bytes passing through this process.
                let copied = std::io::copy(&mut basis.take(run), &mut output)?;
                ensure!(copied == run, "delta basis out of bounds");
                pos += run;
            }
            RESIDUAL => {
                let mut source = [0; 4];
                input.read_exact(&mut source)?;
                let (block, old) = (&mut block[..len], &mut old[..len]);
                input.read_exact(block)?;
                basis.seek(std::io::SeekFrom::Start(u32::from_le_bytes(source).into()))?;
                basis.read_exact(old).context("delta basis out of bounds")?;
                for (v, &b) in block.iter_mut().zip(&*old) {
                    *v = v.wrapping_add(b);
                }
                output.write_all(block)?;
                pos += len as u64;
            }
            LITERAL_BLOCK => {
                let block = &mut block[..len];
                input.read_exact(block)?;
                output.write_all(block)?;
                pos += len as u64;
            }
            _ => bail!("unknown prediction record"),
        }
    }
    let mut expected = [0; 32];
    input.read_exact(&mut expected)?;
    ensure!(input.read(&mut [0])? == 0, "trailing prediction data");
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_v2(old: &[u8], new: &[u8]) -> (Vec<u8>, Result<Vec<u8>>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jr-recipe2-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut recipe = Vec::new();
        encode_v2(old, new, &mut recipe, None).unwrap();
        let sent = recipe.clone();
        recipe.extend_from_slice(&Sha256::digest(new));
        std::fs::write(dir.join("old"), old).unwrap();
        let basis = File::open(dir.join("old")).unwrap();
        let output = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(dir.join("out"))
            .unwrap();
        let got = decode_v2(&basis, new.len() as u64, &recipe[..], &output)
            .map(|_| std::fs::read(dir.join("out")).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        (sent, got)
    }

    #[test]
    fn v2_copies_unchanged_runs_and_sends_only_changed_blocks() {
        let old: Vec<u8> = (0..10 * BLOCK as u64 + 77)
            .map(|i| {
                // A Weyl sequence alone nearly repeats at short distances.
                let z = i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
                let z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                ((z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb) >> 56) as u8
            })
            .collect();
        // Unchanged: one copy record for the whole file.
        let (recipe, got) = round_trip_v2(&old, &old);
        assert_eq!(got.unwrap(), old);
        assert_eq!(recipe.len(), 9);
        // One byte changed in block 3: copy, one residual block, copy.
        let mut new = old.clone();
        new[3 * BLOCK + 5] ^= 0x55;
        let (recipe, got) = round_trip_v2(&old, &new);
        assert_eq!(got.unwrap(), new);
        assert_eq!(recipe.len(), 9 + (5 + BLOCK) + 9);
        // An insertion moves everything after it: still exact.
        let mut moved = old.clone();
        moved.splice(2 * BLOCK + 9..2 * BLOCK + 9, [1, 2, 3]);
        let (recipe, got) = round_trip_v2(&old, &moved);
        assert_eq!(got.unwrap(), moved);
        assert!(recipe.len() < 4 * BLOCK, "{}", recipe.len());
        // Unrelated and empty inputs.
        for (a, b) in [
            (&old[..], &b"x"[..]),
            (b"", b"abc"),
            (b"abc", b""),
            (b"", b""),
        ] {
            assert_eq!(round_trip_v2(a, b).1.unwrap(), b);
        }
    }

    #[test]
    fn v2_recipe_never_reads_beyond_basis_or_writes_beyond_declared_size() {
        let dir = std::env::temp_dir().join(format!("jr-recipe2-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old"), [7u8; 100]).unwrap();
        let basis = File::open(dir.join("old")).unwrap();
        let run = |size: u64, recipe: &[u8]| {
            let _ = std::fs::remove_file(dir.join("out"));
            let output = File::create_new(dir.join("out")).unwrap();
            decode_v2(&basis, size, recipe, &output)
                .map(|_| std::fs::read(dir.join("out")).unwrap())
        };
        let copy = |source: u32, len: u32| {
            let mut r = vec![COPY];
            r.extend_from_slice(&source.to_le_bytes());
            r.extend_from_slice(&len.to_le_bytes());
            r
        };
        let digest = [0u8; 32];
        let ok = [copy(10, 50), digest.to_vec()].concat();
        assert_eq!(run(50, &ok).unwrap(), [7u8; 50]);
        // Past the end of the basis, past the declared size, empty, unknown,
        // truncated, and trailing data.
        assert!(run(50, &[copy(60, 50), digest.to_vec()].concat()).is_err());
        assert!(run(50, &[copy(0, 51), digest.to_vec()].concat()).is_err());
        assert!(run(50, &[copy(0, 0), digest.to_vec()].concat()).is_err());
        assert!(run(50, &[vec![9], digest.to_vec()].concat()).is_err());
        assert!(run(50, &ok[..ok.len() - 1]).is_err());
        assert!(run(50, &[ok.clone(), vec![0]].concat()).is_err());
        let mut residual = vec![RESIDUAL];
        residual.extend_from_slice(&95u32.to_le_bytes());
        residual.extend_from_slice(&[0; 10]);
        residual.extend_from_slice(&digest);
        assert!(run(10, &residual).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
