//! Negotiated File.len sentinel, followed by little-endian u32 chunk sizes.
//! Zero plus the exact u64 payload count commits the file. Bare EOF never does.
use super::*;
use std::io::{self, ErrorKind};

pub(super) const TAG: u64 = u64::MAX;
pub(super) const MIN_SIZE: u64 = 32 << 20;
const CHUNK: usize = 64 << 10;

#[derive(Debug)]
pub(super) struct TransferFailure;
impl std::fmt::Display for TransferFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("full artifact stream failed")
    }
}

#[derive(Debug)]
pub(super) struct BeforeHeader;
impl std::fmt::Display for BeforeHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stream encoder unavailable before header")
    }
}

fn bound(size: u64) -> io::Result<u64> {
    // Greater than ZSTD_compressBound for our single ordinary full frame.
    size.checked_add(size / 128)
        .and_then(|n| n.checked_add(CHUNK as u64))
        .filter(|n| *n < TAG)
        .ok_or_else(|| io::Error::other("artifact size overflow"))
}

pub(super) fn send(
    out: &mut impl Write,
    header: Frame,
    snapshot: std::fs::File,
    size: u64,
) -> Result<()> {
    let mut encoder = Command::new("zstd");
    encoder
        .args(["-q", "-9", "-T0", "--check", "-c"])
        .stdin(snapshot);
    send_encoder(out, header, size, &mut encoder)
}

fn send_encoder(
    out: &mut impl Write,
    mut header: Frame,
    size: u64,
    encoder: &mut Command,
) -> Result<()> {
    let max = bound(size)?;
    let started = Instant::now();
    let mut child = encoder
        .stdout(Stdio::piped())
        .spawn()
        .context("zstd on the build machine")
        .context(BeforeHeader)?;
    let mut pipe = child.stdout.take().expect("piped stdout");
    let sent = (|| -> Result<u64> {
        let mut total = 0u64;
        let mut buf = [0; CHUNK];
        loop {
            let n = match pipe.read(&mut buf) {
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                other => other?,
            };
            if n == 0 {
                break;
            }
            if total == 0 {
                if let Frame::File {
                    pack_us: Some(us), ..
                } = &mut header
                {
                    *us = us.saturating_add(started.elapsed().as_micros() as u64);
                }
                write_frame(out, &header, &[])?;
            }
            total = total
                .checked_add(n as u64)
                .filter(|n| *n <= max)
                .context("oversize compressed artifact")?;
            out.write_all(&(n as u32).to_le_bytes())?;
            out.write_all(&buf[..n])?;
            if total == n as u64 {
                out.flush()?;
            }
        }
        Ok(total)
    })();
    drop(pipe);
    if sent.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let total = sent?;
    if total == 0 {
        return Err(
            anyhow::anyhow!("artifact encoder returned no data ({status})").context(BeforeHeader),
        );
    }
    if !status.success() {
        bail!("artifact zstd encoder failed ({status})");
    }
    out.write_all(&0u32.to_le_bytes())?;
    out.write_all(&total.to_le_bytes())?;
    out.flush()?;
    Ok(())
}

pub(super) struct Chunks<R> {
    inner: R,
    remaining: usize,
    max: u64,
    pub(super) total: u64,
    done: bool,
}
impl<R> Chunks<R> {
    pub(super) fn new(inner: R, size: u64) -> io::Result<Self> {
        Ok(Self {
            inner,
            remaining: 0,
            max: bound(size)?,
            total: 0,
            done: false,
        })
    }
}
impl<R: Read> Read for Chunks<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut len = [0; 4];
            self.inner.read_exact(&mut len)?;
            self.remaining = u32::from_le_bytes(len) as usize;
            if self.remaining == 0 {
                let mut end = [0; 8];
                self.inner.read_exact(&mut end)?;
                if self.total == 0 || u64::from_le_bytes(end) != self.total {
                    return Err(io::Error::other("invalid artifact success trailer"));
                }
                self.done = true;
                return Ok(0);
            }
            if self.remaining > CHUNK {
                return Err(io::Error::other("oversize artifact chunk"));
            }
            self.total = self
                .total
                .checked_add(self.remaining as u64)
                .filter(|n| *n <= self.max)
                .ok_or_else(|| io::Error::other("oversize compressed artifact"))?;
        }
        let want = self.remaining.min(buf.len());
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_chunk_limits() {
        assert!(Chunks::new(&b""[..], u64::MAX).is_err());
        for wire in [u32::MAX.to_le_bytes().to_vec(), vec![0; 12], {
            let mut bytes = (CHUNK as u32).to_le_bytes().to_vec();
            bytes.resize(4 + CHUNK, 0);
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes
        }] {
            assert!(
                io::copy(
                    &mut Chunks::new(wire.as_slice(), 0).unwrap(),
                    &mut io::sink()
                )
                .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn encoder_failure_is_retryable_only_before_output() {
        for (script, preheader) in [("exit 1", true), ("printf some-payload; exit 1", false)] {
            let mut wire = Vec::new();
            let error = send_encoder(
                &mut wire,
                Frame::End,
                40,
                Command::new("sh").args(["-c", script]),
            )
            .unwrap_err();
            assert_eq!(error.is::<BeforeHeader>(), preheader);
            assert_eq!(wire.is_empty(), preheader);
            if !preheader {
                let body = wire.splitn(2, |c| *c == b'\n').nth(1).unwrap();
                assert!(io::copy(&mut Chunks::new(body, 40).unwrap(), &mut io::sink()).is_err());
                assert!(!is_delta_failure(
                    &error.context("(delta)").context(TransferFailure)
                ));
            }
        }
    }
}
