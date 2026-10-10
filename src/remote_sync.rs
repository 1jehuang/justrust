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

/// What a scan found: the files to mirror, and those too large to mirror.
#[derive(Debug, Default)]
pub struct Scan {
    pub files: Manifest,
    /// Files over `MAX_FILE`. Never mirrored, and removed from the machine
    /// if an older, smaller version was, so the machine never builds a
    /// stale copy.
    pub oversized: Vec<PathBuf>,
}

/// Every regular file the build may read under `root`: tracked and untracked
/// files that are not gitignored (or a plain walk outside git).
pub fn scan(root: &Path) -> Result<Scan> {
    let mut out = Scan::default();
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
        if !md.file_type().is_file() {
            continue;
        }
        if md.len() > MAX_FILE {
            out.oversized.push(rel);
            continue;
        }
        out.files.insert(
            rel,
            Stamp {
                mtime_ns: md.mtime() as i128 * 1_000_000_000 + md.mtime_nsec() as i128,
                size: md.len(),
                mode: md.permissions().mode() & 0o777,
            },
        );
    }
    Ok(out)
}

/// Whether a file not reaching the machine certainly breaks the build
/// there (as opposed to assets that only matter if `include_bytes!`ed).
pub fn is_build_source(rel: &Path) -> bool {
    matches!(
        rel.extension().and_then(|e| e.to_str()),
        Some("rs" | "toml" | "lock")
    )
}

/// A stamp that never equals a real one: marks a file whose last push
/// failed, so the next diff sends it again (or deletes it).
pub const UNKNOWN: Stamp = Stamp {
    mtime_ns: -1,
    size: u64::MAX,
    mode: 0,
};

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

/// The jcode-mkdir wrapper on hosted build machines: the only sudo command
/// allowed there. Creates missing components owned by the ssh user and
/// refuses system trees.
pub const JCODE_MKDIR: &str = "/usr/local/sbin/jcode-mkdir";

/// Shell script that creates `remote` writable by the ssh user: a plain
/// `mkdir -p` (paths under the user's reach), then the hosted machines'
/// `jcode-mkdir` wrapper if present, then general sudo (aws and ssh
/// machines, where the user may sudo).
pub fn prepare_script(remote: &str) -> String {
    prepare_script_with(remote, JCODE_MKDIR)
}

fn prepare_script_with(remote: &str, wrapper: &str) -> String {
    let q = shell_quote(remote);
    let wq = shell_quote(wrapper);
    format!(
        "{{ mkdir -p {q} && test -w {q}; }} 2>/dev/null \
         || {{ test -x {wq} && sudo -n {wq} {q} && test -w {q}; }} 2>/dev/null \
         || sudo -n install -d -o \"$(id -un)\" -g \"$(id -gn)\" {q} 2>/dev/null"
    )
}

/// Where roots are mirrored: an ssh target (or, in tests, this machine)
/// plus the prefix every remote path gets.
#[derive(Clone, Debug)]
pub struct Mirror {
    pub target: Option<Target>,
    pub prefix: String,
}

impl Mirror {
    pub fn remote(&self, p: &Path) -> String {
        if self.prefix.is_empty() {
            p.display().to_string()
        } else {
            format!("{}{}", self.prefix, p.display())
        }
    }

    /// Create the remote directory of `root`, owned by the ssh user.
    pub fn prepare(&self, root: &Path) -> Result<()> {
        let remote = self.remote(root);
        let Some(t) = &self.target else {
            return Ok(std::fs::create_dir_all(&remote)?);
        };
        if !t
            .command()
            .arg(prepare_script(&remote))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success()
        {
            bail!("cannot create {remote} on the remote machine");
        }
        Ok(())
    }

    /// Mirror `root` with rsync. Returns files sent. The machine's own
    /// `target/` dirs are protected from deletion so they stay warm.
    /// Oversized files are skipped here: the caller deletes them remotely.
    pub fn rsync(&self, root: &Path) -> Result<usize> {
        let src = format!("{}/", root.display());
        let remote = self.remote(root);
        let mut c = Command::new("rsync");
        c.args([
            "-a",
            "--delete",
            "--out-format=%i",
            &format!("--max-size={MAX_FILE}"),
            "--filter=P target/",
            "--filter=P .justrust/",
            "--exclude=.git",
            "--exclude=target/",
            "--exclude=.justrust/",
            "--filter=:- .gitignore",
        ]);
        let dst = match &self.target {
            Some(t) => {
                c.args(["-z", "-e", &t.rsync_shell()]);
                format!("{}:{remote}/", t.dest)
            }
            None => format!("{remote}/"),
        };
        let out = c
            .arg(&src)
            .arg(&dst)
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
            .filter(|l| l.starts_with(">f") || l.starts_with("<f"))
            .count())
    }
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
    fn prepare_script_fallback_chain() {
        let d = std::env::temp_dir().join(format!("jr-prep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let bin = d.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = d.join("log");
        // A fake sudo that records what it was asked and runs it, and a
        // fake jcode-mkdir that creates the directory.
        let script = |name: &str, body: &str| {
            let p = bin.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        script(
            "sudo",
            &format!(
                "[ \"$1\" = -n ] && shift; echo \"sudo $*\" >> {}; exec \"$@\"",
                log.display()
            ),
        );
        let wrapper = script("jcode-mkdir", "mkdir -p \"$1\"");
        let run = |target: &Path, wrapper: &Path| {
            let _ = std::fs::remove_file(&log);
            let ok = Command::new("sh")
                .arg("-c")
                .arg(prepare_script_with(
                    &target.display().to_string(),
                    &wrapper.display().to_string(),
                ))
                .env(
                    "PATH",
                    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
                )
                .status()
                .unwrap()
                .success();
            (ok, std::fs::read_to_string(&log).unwrap_or_default())
        };
        // Writable parent: plain mkdir, no sudo. Spaces and quotes survive.
        let plain = d.join("it's a dir/x");
        assert_eq!(run(&plain, &wrapper), (true, String::new()));
        assert!(plain.is_dir());
        // Read-only parent: mkdir fails, the wrapper runs under sudo.
        let ro = d.join("ro");
        std::fs::create_dir_all(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let target = ro.join("proj");
        // As root (CI containers) mkdir succeeds anyway: nothing to check.
        if Command::new("mkdir")
            .arg(&target)
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
        {
            std::fs::remove_dir(&target).unwrap();
        } else {
            let (ok, log1) = run(&target, &bin.join("missing"));
            // No wrapper: the general sudo install fallback runs (and fails
            // here since the fake sudo is not root).
            assert!(!ok);
            assert!(log1.contains("sudo install -d"), "{log1}");
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
            // With the wrapper present it is tried first. Make the parent
            // writable only for the wrapper by letting it chmod.
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
            let wrapper2 = script(
                "jcode-mkdir2",
                "chmod 755 \"$(dirname \"$1\")\" && mkdir -p \"$1\"",
            );
            let (ok, log2) = run(&target, &wrapper2);
            assert!(ok, "{log2}");
            assert!(
                log2.starts_with(&format!("sudo {}", wrapper2.display())),
                "{log2}"
            );
            assert!(!log2.contains("install -d"), "{log2}");
            assert!(target.is_dir());
        }
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
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
        let s = scan(&d).unwrap();
        let m = s.files;
        let keys: Vec<_> = m.keys().cloned().collect();
        assert_eq!(keys, vec![PathBuf::from("src/lib.rs")]);
        assert_eq!(s.oversized, vec![PathBuf::from("big.bin")]);
        assert!(is_build_source(Path::new("src/lib.rs")));
        assert!(!is_build_source(Path::new("big.bin")));
        let ds = dirs(&d, &m);
        assert!(ds.contains(&d) && ds.contains(&d.join("src")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
