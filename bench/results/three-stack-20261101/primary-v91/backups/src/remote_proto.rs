//! Wire protocol between the local sync daemon, its clients, and the agent
//! on the remote compile machine.
//!
//! Every frame is `[u32 header len][header JSON][u32 payload len][payload]`,
//! big endian. The same framing runs over the ssh channel (daemon <-> agent)
//! and the daemon's unix socket (client <-> daemon), so a future hosted
//! backend only has to provide a byte stream.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Bumped when frames change incompatibly. The daemon requires an agent
/// that answers in this version.
pub const VERSION: u32 = 2;
/// Oldest daemon protocol the agent still serves (v1 is v2 without
/// `Synced` and without write-failure reports).
pub const MIN_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "t")]
pub enum Msg {
    /// First frame in both directions.
    Hello {
        version: u32,
    },
    /// Payload is the file content. Written atomically (temp file + rename).
    Write {
        path: String,
        mode: u32,
    },
    /// Remove a file or directory tree.
    Delete {
        path: String,
    },
    Mkdir {
        path: String,
    },
    /// Everything under `path` was just mirrored out of band (rsync):
    /// forget earlier write failures there.
    Synced {
        path: String,
    },
    /// Answered with `BarrierAck` once every earlier frame is applied.
    Barrier {
        id: u64,
    },
    /// `failed` lists every path whose last `Write`/`Delete` failed on the
    /// agent (cleared by a later successful one). Non-empty means the tree
    /// is not what the daemon sent: the daemon must not build on it.
    BarrierAck {
        id: u64,
        #[serde(default)]
        failed: Vec<String>,
    },
    /// Run `justrust <args>` in `cwd`. Client -> daemon also carries the
    /// source roots that must be synced first.
    Run {
        id: u64,
        cwd: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        #[serde(default)]
        roots: Vec<String>,
        /// Client -> daemon: may start a stopped machine (explicit
        /// `justrust remote ...`), not for automatic routing.
        #[serde(default)]
        start: bool,
        /// Run id to record under, so local and remote ids match.
        #[serde(default)]
        run_id: String,
    },
    /// Output of a run: payload bytes, stream 1 = stdout, 2 = stderr.
    Out {
        id: u64,
        stream: u8,
    },
    /// Daemon -> client: the source was flushed to the machine.
    Flushed {
        id: u64,
        files: usize,
        ms: f64,
        machine: String,
        /// Remote paths are this prefix + the local absolute path (empty
        /// when the machine mirrors the same paths).
        #[serde(default)]
        prefix: String,
        /// Something the user should know about the mirror (files too
        /// large to sync), shown before the build output.
        #[serde(default)]
        note: String,
    },
    /// The run ended. Payload: the remote run's summary.json, if recorded.
    Exit {
        id: u64,
        code: i32,
    },
    Cancel {
        id: u64,
    },
    /// Infrastructure failure (not a build failure).
    Error {
        id: u64,
        msg: String,
    },
    /// Client -> daemon: report state.
    Ping,
    Pong {
        roots: Vec<String>,
        machine: String,
        pushed: u64,
        connected: bool,
        /// Identity of the daemon's binary; clients restart a stale daemon.
        build: String,
        /// Runs relayed right now.
        #[serde(default)]
        active: u64,
        /// Files not mirrored because they exceed the size limit.
        #[serde(default)]
        skipped: u64,
    },
    /// Client -> daemon: exit now.
    Shutdown,
}

pub fn write_frame<W: Write>(w: &mut W, msg: &Msg, payload: &[u8]) -> Result<()> {
    let h = serde_json::to_vec(msg)?;
    let mut buf = Vec::with_capacity(8 + h.len() + payload.len());
    buf.extend_from_slice(&(h.len() as u32).to_be_bytes());
    buf.extend_from_slice(&h);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

/// `Ok(None)` on a clean end of stream.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Option<(Msg, Vec<u8>)>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let hl = u32::from_be_bytes(len) as usize;
    if hl > 64 << 20 {
        bail!("frame header too large ({hl} bytes): not a justrust stream");
    }
    let mut h = vec![0u8; hl];
    r.read_exact(&mut h)?;
    r.read_exact(&mut len)?;
    let pl = u32::from_be_bytes(len) as usize;
    let mut p = vec![0u8; pl];
    r.read_exact(&mut p)?;
    Ok(Some((serde_json::from_slice(&h)?, p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut buf = Vec::new();
        let m = Msg::Write {
            path: "/a/b.rs".into(),
            mode: 0o644,
        };
        write_frame(&mut buf, &m, b"fn main() {}").unwrap();
        write_frame(&mut buf, &Msg::Barrier { id: 7 }, &[]).unwrap();
        let mut r = &buf[..];
        let (a, p) = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(a, m);
        assert_eq!(p, b"fn main() {}");
        let (b, p) = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(b, Msg::Barrier { id: 7 });
        assert!(p.is_empty());
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn rejects_garbage() {
        let mut r = &b"\xff\xff\xff\xffnope"[..];
        assert!(read_frame(&mut r).is_err());
    }
}
