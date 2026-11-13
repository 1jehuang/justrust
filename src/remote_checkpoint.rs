//! Keep a source mirror across a reconnect of a running sync daemon.
//!
//! A new agent connection used to distrust every manifest the daemon held
//! and mirror each root again with a `--checksum` rsync, which reads every
//! source file on both sides: 10.8-22.3 s for this repository to send zero
//! or one file (`docs/native-transport.md`). The daemon never guesses that
//! the machine is in sync, and this keeps that rule with evidence that is
//! cheaper to collect.
//!
//! After a root is in sync the daemon sends `Checkpoint` with a fresh
//! random token. The agent applies it in frame order, so writing the token
//! into `<root>/.justrust/` proves every earlier frame was applied, and it
//! refuses while any write or delete under the root is still failed. From
//! then on the daemon records every path it sends (`Mark::dirty`): those are
//! the only paths whose state on the machine it cannot know when the
//! connection dies.
//!
//! On the next connection the daemon sends `Verify` with the token and the
//! size of every file outside the dirty set. The agent confirms the token
//! is still the one on disk and that every size matches, and replaces it
//! with a new token in the same step. The daemon then sends the dirty paths
//! again together with whatever changed locally. Anything else (no token,
//! another connection's token, a size mismatch, an agent without the
//! capability) falls back to the full mirror.
//!
//! The token is removed by every change the daemon would not know about:
//! an agent drops it before applying a source write or delete under the
//! root unless the token on disk is the one this very connection wrote or
//! verified, and bulk and full rsyncs drop it before they start. Because a
//! verify replaces the token, a frame the previous connection applies late
//! removes the new one, and the new connection reports that root as `lost`
//! in its next barrier acknowledgement: the daemon mirrors it in full
//! before anything builds on it.
//!
//! Sizes and a token are the same evidence `remote_resume` accepts across a
//! daemon restart, and both use the same token file. Agents and clients
//! from before this module therefore still remove a checkpoint whenever
//! they write or rsync under the root, and a restart save simply replaces
//! it.
//! Opt-in: `JUSTRUST_REMOTE_CHECKPOINT=1` in the daemon's environment.

use std::collections::{BTreeSet, HashMap};
#[cfg(unix)]
use std::path::Component;
use std::path::{Path, PathBuf};

/// `Hello` capability of an agent that serves `Checkpoint` and `Verify`.
pub const CAP: &str = "checkpoint";
pub const TOKEN_FILE: &str = ".justrust/mirror-token";

/// Unix clients only: relative paths travel as raw bytes.
pub fn enabled() -> bool {
    cfg!(unix) && std::env::var("JUSTRUST_REMOTE_CHECKPOINT").as_deref() == Ok("1")
}

/// Checkpoint again once this many paths would have to be sent twice.
const MAX_DIRTY: usize = 256;

/// What the daemon knows about one root's checkpoint on the machine.
#[derive(Default, Debug)]
pub struct Mark {
    /// Token of the last checkpoint sent, whether or not the agent took it.
    pub token: Option<String>,
    /// Relative paths sent since that checkpoint.
    pub dirty: BTreeSet<PathBuf>,
}

impl Mark {
    pub fn wanted(&self) -> bool {
        self.token.is_none() || self.dirty.len() > MAX_DIRTY
    }
}

/// A checkpoint this agent connection wrote or verified.
pub struct Held {
    pub token: String,
    /// The root as the daemon names it.
    pub name: String,
}

pub fn new_token() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok()
    {
        return b.iter().map(|x| format!("{x:02x}")).collect();
    }
    let mut h = crate::depcache::H128::new();
    h.field(format!("{:?}{}", std::time::SystemTime::now(), std::process::id()).as_bytes());
    h.hex()
}

/// `size NUL path NUL` for each file, gzipped.
pub fn encode<'a>(files: impl Iterator<Item = (&'a Path, u64)>) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    for (path, size) in files {
        let b = path.as_os_str().as_encoded_bytes();
        if b.contains(&0) {
            return None;
        }
        gz.write_all(size.to_string().as_bytes()).ok()?;
        gz.write_all(b"\0").ok()?;
        gz.write_all(b).ok()?;
        gz.write_all(b"\0").ok()?;
    }
    gz.finish().ok()
}

fn token_on_disk(dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;
    Some(s.trim().to_owned())
}

pub fn forget(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(TOKEN_FILE));
}

/// Machine side of `Checkpoint`.
pub fn write(dir: &Path, token: &str) -> std::io::Result<()> {
    let path = dir.join(TOKEN_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, token).and_then(|()| std::fs::rename(&tmp, &path))
}

/// Machine side of `Verify`: the token under `dir` is `token` and every
/// listed file is a regular file of the listed size, else why not.
#[cfg(unix)]
pub fn check(dir: &Path, token: &str, list: &[u8]) -> Result<(), String> {
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    if token.is_empty() || token_on_disk(dir).as_deref() != Some(token) {
        return Err("checkpoint token missing or replaced".into());
    }
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(list)
        .read_to_end(&mut raw)
        .map_err(|_| "unreadable file list".to_string())?;
    let mut fields = raw.split(|b| *b == 0);
    loop {
        let size = match fields.next() {
            Some([]) | None => return Ok(()),
            Some(size) => size,
        };
        let name = fields.next().ok_or("truncated file list")?;
        let size: u64 = std::str::from_utf8(size)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or("malformed file list")?;
        let rel = Path::new(std::ffi::OsStr::from_bytes(name));
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(format!("{} is not inside the root", rel.display()));
        }
        match std::fs::symlink_metadata(dir.join(rel)) {
            Ok(md) if md.is_file() && md.len() == size => {}
            Ok(md) if md.is_file() => {
                return Err(format!(
                    "{} is {} bytes on the machine, {size} expected",
                    rel.display(),
                    md.len()
                ));
            }
            _ => return Err(format!("{} missing on the machine", rel.display())),
        }
    }
}

/// Machine side, before applying a source write or delete at `path`: a
/// checkpoint of a root above it survives only if this connection holds it
/// (`owned`) and it is still the token on disk. A displaced holding stays
/// in `owned` for `take_lost`.
pub fn invalidate_above(path: &Path, owned: &HashMap<PathBuf, Held>) {
    for dir in path.ancestors().skip(1) {
        match owned.get(dir) {
            Some(held) if token_on_disk(dir).as_deref() == Some(&held.token) => {}
            Some(_) => forget(dir),
            None if dir.join(TOKEN_FILE).is_file() => forget(dir),
            None => {}
        }
    }
}

/// Roots whose checkpoint is no longer the one this connection holds:
/// something else wrote there. Reported once.
pub fn take_lost(owned: &mut HashMap<PathBuf, Held>) -> Vec<String> {
    let mut lost = Vec::new();
    owned.retain(|dir, held| {
        let kept = token_on_disk(dir).as_deref() == Some(&held.token);
        if !kept {
            lost.push(held.name.clone());
        }
        kept
    });
    lost.sort();
    lost
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn list(files: &[(&str, u64)]) -> Vec<u8> {
        encode(files.iter().map(|(p, s)| (Path::new(*p), *s))).unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jr-checkpoint-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn held(token: &str) -> Held {
        Held {
            token: token.into(),
            name: "/client/root".into(),
        }
    }

    #[test]
    fn verify_needs_the_token_and_every_size() {
        let tmp = scratch("verify");
        let root = tmp.as_path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a b.rs"), b"twelve bytes").unwrap();
        std::fs::write(root.join("c.rs"), b"seven..").unwrap();
        let ok = list(&[("src/a b.rs", 12), ("c.rs", 7)]);
        assert_eq!(
            check(root, "tok", &ok),
            Err("checkpoint token missing or replaced".into())
        );
        write(root, "tok").unwrap();
        assert_eq!(check(root, "tok", &ok), Ok(()));
        assert_eq!(check(root, "tok", &list(&[])), Ok(()));
        assert!(check(root, "other", &ok).is_err());
        assert!(check(root, "", &ok).is_err());
        assert_eq!(
            check(root, "tok", &list(&[("src/a b.rs", 12), ("c.rs", 8)])),
            Err("c.rs is 7 bytes on the machine, 8 expected".into())
        );
        assert_eq!(
            check(root, "tok", &list(&[("gone.rs", 1)])),
            Err("gone.rs missing on the machine".into())
        );
        // A directory or a symlink where a file was sent is not that file.
        assert!(check(root, "tok", &list(&[("src", 0)])).is_err());
        std::os::unix::fs::symlink(root.join("c.rs"), root.join("link.rs")).unwrap();
        assert!(check(root, "tok", &list(&[("link.rs", 7)])).is_err());
        assert!(check(root, "tok", &list(&[("../c.rs", 7)])).is_err());
        assert!(check(root, "tok", b"not gzip").is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn only_the_holding_connection_may_write_under_a_checkpoint() {
        let root = scratch("holder");
        let file = root.join("src/lib.rs");
        let token = root.join(TOKEN_FILE);
        write(&root, "mine").unwrap();
        let mut mine = HashMap::from([(root.clone(), held("mine"))]);
        invalidate_above(&file, &mine);
        assert!(token.is_file(), "the holder's own writes keep it");
        assert!(take_lost(&mut mine).is_empty());

        // Another connection (a second client, or this daemon's previous
        // connection applying a late frame) does not hold it.
        invalidate_above(&file, &HashMap::new());
        assert!(!token.is_file());
        // The holder does not recreate it, and reports the loss once.
        invalidate_above(&file, &mine);
        assert!(!token.is_file());
        assert_eq!(take_lost(&mut mine), ["/client/root"]);
        assert!(take_lost(&mut mine).is_empty());

        // A verify on a new connection replaced the token: a late write of
        // the previous connection drops the new one, and the new connection
        // notices.
        write(&root, "newer").unwrap();
        let stale = HashMap::from([(root.clone(), held("mine"))]);
        let mut newer = HashMap::from([(root.clone(), held("newer"))]);
        assert!(take_lost(&mut newer).is_empty());
        invalidate_above(&file, &stale);
        assert!(!token.is_file());
        assert_eq!(take_lost(&mut newer), ["/client/root"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
