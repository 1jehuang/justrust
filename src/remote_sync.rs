//! Keep source roots mirrored on the remote machine.
//!
//! A root is first mirrored with rsync (fast for many files, deletes stale
//! ones). After that only changes travel: a scan (`git ls-files` plus a
//! `stat` per file, about 8 ms for the 2,100 files of Jcode) is compared
//! with the manifest of what was last pushed, and each changed file goes out
//! as one `Write` frame. The scan, not inotify, decides what is pushed, so a
//! missed or overflowed watch can delay a push but never lose one.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::remote_backend::Target;
use crate::remote_build::shell_quote;

/// Files larger than this are not mirrored (demo videos, datasets).
pub const MAX_FILE: u64 = 8 << 20;
/// Above this many changed files, rsync is cheaper than single frames.
pub const RSYNC_ABOVE: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub mtime_ns: i128,
    pub size: u64,
    pub mode: u32,
}

/// Relative path -> stamp of every mirrored regular file in a root.
pub type Manifest = BTreeMap<PathBuf, Stamp>;

/// Every regular file the build may read under `root`: tracked and untracked
/// files that are not gitignored (or a plain walk outside git), at most
/// `MAX_FILE` bytes.
pub fn scan(root: &Path) -> Result<Manifest> {
    let mut m = Manifest::new();
    let listed = Command::new("git")
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(root)
        .stderr(Stdio::null())
        .output();
    let rels: Vec<PathBuf> = match listed {
        Ok(o) if o.status.success() => o
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
            .collect(),
        _ => walk(root)?,
    };
    for rel in rels {
        if skip(&rel) {
            continue;
        }
        // Tracked files deleted from the worktree fail here and count as gone.
        let Ok(md) = std::fs::symlink_metadata(root.join(&rel)) else {
            continue;
        };
        if !md.file_type().is_file() || md.len() > MAX_FILE {
            continue;
        }
        m.insert(
            rel,
            Stamp {
                mtime_ns: md.mtime() as i128 * 1_000_000_000 + md.mtime_nsec() as i128,
                size: md.len(),
                mode: md.permissions().mode() & 0o777,
            },
        );
    }
    Ok(m)
}

fn skip(rel: &Path) -> bool {
    rel.components().any(|c| {
        let c = c.as_os_str();
        c == "target" || c == ".git" || c == ".justrust"
    })
}

fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for e in rd.flatten() {
            let r = rel.join(e.file_name());
            if skip(&r) {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(r),
                Ok(t) if t.is_file() => out.push(r),
                _ => {}
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub changed: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.deleted.is_empty()
    }
}

pub fn diff(old: &Manifest, new: &Manifest) -> Diff {
    let mut d = Diff::default();
    for (p, s) in new {
        if old.get(p) != Some(s) {
            d.changed.push(p.clone());
        }
    }
    for p in old.keys() {
        if !new.contains_key(p) {
            d.deleted.push(p.clone());
        }
    }
    d
}

/// Directories to watch for a root: the root and every directory that holds
/// a mirrored file.
pub fn dirs(root: &Path, m: &Manifest) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    out.insert(root.to_path_buf());
    for p in m.keys() {
        let mut d = p.parent();
        while let Some(x) = d {
            if x.as_os_str().is_empty() || !out.insert(root.join(x)) {
                break;
            }
            d = x.parent();
        }
    }
    out
}

/// Create `remote` (the remote path of a root) owned by the ssh user.
pub fn prepare_dir(t: &Target, remote: &str) -> Result<bool> {
    let q = shell_quote(remote);
    let script = format!(
        "mkdir -p {q} 2>/dev/null && test -w {q} || sudo -n install -d -o \"$(id -un)\" -g \"$(id -gn)\" {q} 2>/dev/null"
    );
    Ok(t.command()
        .arg(script)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success())
}

/// Mirror `root` to `remote` with rsync. The machine's own `target/` dirs
/// are protected from deletion so they stay warm.
pub fn rsync(t: &Target, root: &Path, remote: &str) -> Result<usize> {
    let src = format!("{}/", root.display());
    let dst = format!("{}:{}/", t.dest, remote);
    let out = Command::new("rsync")
        .args([
            "-a",
            "--delete",
            "-z",
            "--out-format=%i",
            &format!("--max-size={MAX_FILE}"),
            "--filter=P target/",
            "--filter=P .justrust/",
            "--exclude=.git",
            "--exclude=target/",
            "--exclude=.justrust/",
            "--filter=:- .gitignore",
            "-e",
            &t.rsync_shell(),
            &src,
            &dst,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("rsync (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "rsync {} failed: {}",
            root.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("<f"))
        .count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(t: i128) -> Stamp {
        Stamp {
            mtime_ns: t,
            size: 1,
            mode: 0o644,
        }
    }

    #[test]
    fn diff_finds_changes_and_deletions() {
        let old: Manifest = [("a".into(), st(1)), ("b".into(), st(1))].into();
        let new: Manifest = [("a".into(), st(2)), ("c".into(), st(1))].into();
        let d = diff(&old, &new);
        assert_eq!(d.changed, vec![PathBuf::from("a"), PathBuf::from("c")]);
        assert_eq!(d.deleted, vec![PathBuf::from("b")]);
        assert!(diff(&new, &new).is_empty());
    }

    #[test]
    fn scan_skips_target_and_big_files() {
        let d = std::env::temp_dir().join(format!("jr-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("target/debug")).unwrap();
        std::fs::write(d.join("src/lib.rs"), "x").unwrap();
        std::fs::write(d.join("target/debug/out"), "x").unwrap();
        std::fs::write(d.join("big.bin"), vec![0u8; (MAX_FILE + 1) as usize]).unwrap();
        let m = scan(&d).unwrap();
        let keys: Vec<_> = m.keys().cloned().collect();
        assert_eq!(keys, vec![PathBuf::from("src/lib.rs")]);
        let ds = dirs(&d, &m);
        assert!(ds.contains(&d) && ds.contains(&d.join("src")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
