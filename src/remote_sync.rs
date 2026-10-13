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
    let mut rels = rels;
    // Lockfiles decide which dependency versions the machine builds. Many
    // libraries gitignore theirs, and a machine without it resolves the
    // newest versions: different code than the local build, and nothing
    // shared with the local dependency cache. Mirror them regardless.
    for lock in ignored_lockfiles(root) {
        if !rels.contains(&lock) {
            rels.push(lock);
        }
    }
    for rel in rels {
        if skip_listed(root, &rel) {
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
        let (s, ns) = crate::sys::mtime(&md);
        let mode = crate::sys::mode(&md, &root.join(&rel)) & 0o777;
        out.files.insert(
            rel,
            Stamp {
                mtime_ns: s as i128 * 1_000_000_000 + ns as i128,
                size: md.len(),
                mode,
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

/// `Cargo.lock` files under `root` that git ignores.
fn ignored_lockfiles(root: &Path) -> Vec<PathBuf> {
    let Ok(o) = Command::new("git")
        .args([
            "ls-files",
            "-o",
            "-i",
            "--exclude-standard",
            "-z",
            "--",
            "Cargo.lock",
            "**/Cargo.lock",
        ])
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !o.status.success() {
        return Vec::new();
    }
    o.stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
        .filter(|p| !skip(p))
        .collect()
}

fn skip(rel: &Path) -> bool {
    rel.components().any(|c| {
        let c = c.as_os_str();
        c == "target" || c == ".git" || c == ".justrust"
    })
}

/// Skip rule for paths git listed (tracked, or untracked and not ignored).
/// A directory merely named `target` is source there (cargo's own testsuite
/// has `tests/testsuite/cargo_add/target/`); only a real cargo target dir,
/// marked by its `CACHEDIR.TAG`, is build output.
fn skip_listed(root: &Path, rel: &Path) -> bool {
    let mut dir = PathBuf::new();
    for c in rel.parent().into_iter().flat_map(|p| p.components()) {
        dir.push(c);
        let name = c.as_os_str();
        if name == ".git" || name == ".justrust" {
            return true;
        }
        if name == "target" && root.join(&dir).join("CACHEDIR.TAG").is_file() {
            return true;
        }
    }
    false
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
    // Last resort, also for a root that already exists but is not ours: an
    // earlier, narrower root created it as root (only its leaf was chowned),
    // and widening the root to it made every later sync fail. Directories
    // inside it that root created on the way are taken over too.
    format!(
        "{{ mkdir -p {q} && test -w {q} && ! find {q} -maxdepth 4 -type d ! -user \"$(id -un)\" | grep -q .; }} 2>/dev/null \
         || {{ test -x {wq} && sudo -n {wq} {q} && test -w {q} && ! find {q} -maxdepth 4 -type d ! -user \"$(id -un)\" | grep -q .; }} 2>/dev/null \
         || {{ sudo -n install -d -o \"$(id -un)\" -g \"$(id -gn)\" {q} \
               && sudo -n find {q} -maxdepth 4 -type d ! -user \"$(id -un)\" -exec chown \"$(id -un):$(id -gn)\" {{}} + \
               && test -w {q}; }} 2>/dev/null"
    )
}

/// Git identity of a root: its common git dir (shared by every worktree of
/// one repository) and its root commits (shared by clones).
#[derive(Debug, PartialEq, Eq)]
pub struct RepoId {
    pub common_dir: PathBuf,
    pub root_commits: BTreeSet<String>,
}

pub fn repo_id(root: &Path) -> Option<RepoId> {
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let common = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let roots = git(&["rev-list", "--max-parents=0", "HEAD"]).unwrap_or_default();
    Some(RepoId {
        common_dir: std::fs::canonicalize(&common).unwrap_or_else(|_| common.into()),
        root_commits: roots.lines().map(str::to_string).collect(),
    })
}

/// `(anonymous https URL, checked-out commit)` of `root`'s `origin`, for
/// [`Mirror::seed_from_origin`]. `git@host:owner/repo` becomes
/// `https://host/owner/repo` (a private repo then just fails to fetch,
/// anonymously). URLs carrying credentials, local paths and other schemes
/// give `None`.
pub fn public_origin(root: &Path) -> Option<(String, String)> {
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let url = anonymous_https(&git(&["remote", "get-url", "origin"])?)?;
    let head = git(&["rev-parse", "HEAD"])?;
    (head.len() >= 40 && head.bytes().all(|b| b.is_ascii_hexdigit())).then_some((url, head))
}

fn anonymous_https(url: &str) -> Option<String> {
    let url = if let Some(rest) = url.strip_prefix("https://") {
        let host = rest.split('/').next()?;
        if host.contains('@') {
            return None;
        }
        url.to_string()
    } else if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        format!("https://{host}/{}", path.trim_start_matches('/'))
    } else {
        let rest = url.strip_prefix("ssh://git@")?;
        let (host, path) = rest.split_once('/')?;
        let host = host.split(':').next()?;
        format!("https://{host}/{path}")
    };
    url.bytes()
        .all(|b| b.is_ascii_graphic() && b != b'\'' && b != b'\\')
        .then_some(url)
}

/// The best already-mirrored root to seed a new mirror of `root` from:
/// a worktree of the same repository first, then a clone sharing a root
/// commit. None outside git or without a relative.
pub fn pick_seed<'a>(
    root: &Path,
    mirrored: impl IntoIterator<Item = &'a PathBuf>,
) -> Option<PathBuf> {
    let me = repo_id(root)?;
    let mut best: Option<(u8, PathBuf)> = None;
    for m in mirrored {
        if m == root || m.starts_with(root) || root.starts_with(m) {
            continue;
        }
        let Some(id) = repo_id(m) else { continue };
        let score = if id.common_dir == me.common_dir {
            2
        } else if !me.root_commits.is_disjoint(&id.root_commits) {
            1
        } else {
            continue;
        };
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, m.clone()));
        }
    }
    best.map(|(_, p)| p)
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

    /// Seed the (new, empty) mirror of `root` from its public git origin:
    /// the machine fetches the checked-out commit (or, if it was never
    /// pushed, the default branch) at depth 1 straight from the forge, so
    /// the following `--checksum` rsync sends only local changes. A first
    /// sync of a repo with no mirrored relative sent every file over the
    /// home uplink: jcode-website 320 s, gear-perf 250 s (FINDINGS 67).
    /// Only anonymous https origins: the machine holds no credentials, and
    /// a URL with userinfo is never sent. Fail open.
    pub fn seed_from_origin(&self, root: &Path) -> bool {
        let Some((url, head)) = public_origin(root) else {
            return false;
        };
        self.seed_from_url(root, &url, &head)
    }

    fn seed_from_url(&self, root: &Path, url: &str, head: &str) -> bool {
        let dst = shell_quote(&self.remote(root));
        let (url, head) = (shell_quote(url), shell_quote(head));
        let script = format!(
            "test -d {dst} && test -z \"$(ls -A {dst} | grep -vx -e target -e .justrust)\" || exit 1; \
             g=$(mktemp -d); trap 'rm -rf \"$g\"' EXIT; export GIT_TERMINAL_PROMPT=0; \
             git init -q --bare \"$g\" && \
             {{ timeout 120 git --git-dir=\"$g\" fetch -q --depth 1 {url} {head} \
                || timeout 120 git --git-dir=\"$g\" fetch -q --depth 1 {url} HEAD; }} && \
             git --git-dir=\"$g\" --work-tree={dst} checkout -q -f FETCH_HEAD -- ."
        );
        let mut c = match &self.target {
            Some(t) => {
                let mut c = t.command();
                c.arg(script);
                c
            }
            None => {
                let mut c = Command::new("sh");
                c.arg("-c").arg(script);
                c
            }
        };
        c.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Seed the (new, empty) mirror of `root` from the machine's existing
    /// mirror of `from` with a machine-local copy, so the following rsync
    /// only sends what differs. Copies sources only: the machine's `target/`
    /// dirs hold path-keyed cargo fingerprints and are left alone. Returns
    /// whether a seed was copied. Fail open: any problem means no seed.
    pub fn seed(&self, root: &Path, from: &Path) -> bool {
        let (src, dst) = (
            shell_quote(&self.remote(from)),
            shell_quote(&self.remote(root)),
        );
        // Only into a mirror holding nothing but build output.
        let script = format!(
            "test -d {src} && test -d {dst} && test -z \"$(ls -A {dst} | grep -vx -e target -e .justrust)\" \
             && rsync -a --exclude=target/ --exclude=.justrust/ {src}/ {dst}/"
        );
        let mut c = match &self.target {
            Some(t) => {
                let mut c = t.command();
                c.arg(script);
                c
            }
            None => {
                let mut c = Command::new("sh");
                c.arg("-c").arg(script);
                c
            }
        };
        c.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Mirror `root` with rsync. Returns files sent. The machine's own
    /// `target/` dirs are protected from deletion so they stay warm.
    /// Oversized files are skipped here: the caller deletes them remotely.
    pub fn rsync(&self, root: &Path) -> Result<usize> {
        self.rsync_with(root, false)
    }

    /// `rsync`, but with `checksum` set, files are compared by content
    /// (size, then a full-file hash on both sides) instead of size+mtime.
    /// Used for the first sync after `seed`: the seeded copy has the seed's
    /// mtimes, so the quick check would delta-transfer every file. Content
    /// equality is exact, so a same-size edit still transfers, and matching
    /// files only get their mtime fixed, so later syncs take the quick path.
    pub fn rsync_with(&self, root: &Path, checksum: bool) -> Result<usize> {
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
            // Lockfiles travel even when gitignored (see `scan`).
            "--include=Cargo.lock",
            "--filter=:- .gitignore",
        ]);
        if checksum {
            c.arg("--checksum");
        }
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
    fn listed_target_dirs_are_source_unless_cargo_tagged() {
        let d = std::env::temp_dir().join(format!("jr-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("tests/cargo_add/target")).unwrap();
        std::fs::create_dir_all(d.join("target/debug")).unwrap();
        std::fs::write(d.join("target/CACHEDIR.TAG"), "x").unwrap();
        assert!(!skip_listed(
            &d,
            Path::new("tests/cargo_add/target/Cargo.toml")
        ));
        assert!(skip_listed(&d, Path::new("target/debug/foo")));
        assert!(skip_listed(&d, Path::new(".git/config")));
        assert!(!skip_listed(&d, Path::new("src/target.rs")));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The real case: cargo's testsuite fixtures under `.../target/`.
    #[test]
    #[ignore = "needs ~/jr-cargo"]
    fn jr_cargo_scan_keeps_target_fixtures() {
        let root = PathBuf::from(std::env::var("HOME").unwrap()).join("jr-cargo");
        let s = scan(&root).unwrap();
        let n = s
            .files
            .keys()
            .filter(|p| p.starts_with("tests/testsuite/cargo_add/target"))
            .count();
        assert!(n > 0, "cargo_add/target fixtures missing from the scan");
        assert!(!s.files.keys().any(|p| p.starts_with("target")));
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

    // Runs `sh` scripts.
    #[cfg(unix)]
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
            crate::sys::set_mode(&p, 0o755).unwrap();
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
        crate::sys::set_mode(&ro, 0o555).unwrap();
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
            crate::sys::set_mode(&ro, 0o755).unwrap();
            // With the wrapper present it is tried first. Make the parent
            // writable only for the wrapper by letting it chmod.
            crate::sys::set_mode(&ro, 0o555).unwrap();
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
        crate::sys::set_mode(&ro, 0o755).unwrap();
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

    // Runs `sh` scripts.
    #[cfg(unix)]
    #[test]
    fn gitignored_lockfiles_are_still_mirrored() {
        let d = std::env::temp_dir().join(format!("jr-scan-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&d)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        std::fs::write(d.join(".gitignore"), "Cargo.lock\n*.log\n").unwrap();
        std::fs::write(d.join("Cargo.lock"), "lock").unwrap();
        std::fs::write(d.join("sub/Cargo.lock"), "lock").unwrap();
        std::fs::write(d.join("x.log"), "noise").unwrap();
        std::fs::write(d.join("lib.rs"), "x").unwrap();
        let keys: Vec<PathBuf> = scan(&d).unwrap().files.keys().cloned().collect();
        for want in ["Cargo.lock", "sub/Cargo.lock", "lib.rs", ".gitignore"] {
            assert!(keys.contains(&PathBuf::from(want)), "{want}: {keys:?}");
        }
        assert!(!keys.contains(&PathBuf::from("x.log")));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn origin_urls_become_anonymous_https_or_nothing() {
        let a = |u: &str| anonymous_https(u);
        assert_eq!(
            a("https://github.com/o/r.git").as_deref(),
            Some("https://github.com/o/r.git")
        );
        assert_eq!(
            a("git@github.com:o/r.git").as_deref(),
            Some("https://github.com/o/r.git")
        );
        assert_eq!(
            a("ssh://git@github.com:22/o/r").as_deref(),
            Some("https://github.com/o/r")
        );
        // Credentials never leave this machine.
        assert_eq!(a("https://tok@github.com/o/r"), None);
        assert_eq!(a("https://u:p@github.com/o/r"), None);
        for u in ["/srv/r.git", "file:///r", "http://h/r", "https://h/o/r'x"] {
            assert_eq!(a(u), None, "{u}");
        }
    }

    #[cfg_attr(windows, ignore)]
    #[test]
    fn seeds_an_empty_mirror_from_git_then_never_over_sources() {
        let d = std::env::temp_dir().join(format!("jr-gitseed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let src = d.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        let git = |args: &[&str]| {
            let o = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&src)
                .output()
                .unwrap();
            assert!(o.status.success(), "{args:?}: {o:?}");
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        git(&["init", "-q"]);
        std::fs::write(src.join("sub/lib.rs"), "committed").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "x"]);
        let head = git(&["rev-parse", "HEAD"]);
        let m = Mirror {
            target: None,
            prefix: d.join("machine").display().to_string(),
        };
        let root = d.join("ws");
        let dst = PathBuf::from(m.remote(&root));
        std::fs::create_dir_all(dst.join("target")).unwrap();
        let url = src.display().to_string();
        assert!(m.seed_from_url(&root, &url, &head));
        assert_eq!(
            std::fs::read_to_string(dst.join("sub/lib.rs")).unwrap(),
            "committed"
        );
        assert!(!dst.join(".git").exists());
        // An unknown commit falls back to the default branch.
        let _ = std::fs::remove_dir_all(dst.join("sub"));
        assert!(m.seed_from_url(&root, &url, &"0".repeat(40)));
        assert!(dst.join("sub/lib.rs").is_file());
        // Never over a mirror that already holds sources.
        std::fs::write(dst.join("sub/lib.rs"), "synced").unwrap();
        assert!(!m.seed_from_url(&root, &url, &head));
        assert_eq!(
            std::fs::read_to_string(dst.join("sub/lib.rs")).unwrap(),
            "synced"
        );
        // An unreachable origin is no seed, not an error.
        let other = d.join("ws2");
        std::fs::create_dir_all(m.remote(&other)).unwrap();
        assert!(!m.seed_from_url(&other, &d.join("nope").display().to_string(), &head));
        let _ = std::fs::remove_dir_all(&d);
    }

    // Mirrors are unix paths on the machine: a Windows path (`C:\..`)
    // under a local prefix is not a valid file name there.
    #[cfg_attr(windows, ignore)]
    #[test]
    fn seeds_from_worktree_then_clone_and_copies_sources_only() {
        let d = std::env::temp_dir().join(format!("jr-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let o = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(o.status.success(), "{args:?}: {o:?}");
        };
        let (main, wt, clone, other) = (
            d.join("main"),
            d.join("wt"),
            d.join("clone"),
            d.join("other"),
        );
        for r in [&main, &other] {
            std::fs::create_dir_all(r).unwrap();
            git(r, &["init", "-q"]);
            std::fs::write(r.join("lib.rs"), r.display().to_string()).unwrap();
            git(r, &["add", "."]);
            git(r, &["commit", "-qm", "x"]);
        }
        git(
            &main,
            &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
        );
        git(
            &d,
            &[
                "clone",
                "-q",
                main.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        let all = vec![other.clone(), clone.clone(), main.clone()];
        assert_eq!(pick_seed(&wt, &all), Some(main.clone()));
        assert_eq!(
            pick_seed(&wt, &[other.clone(), clone.clone()]),
            Some(clone.clone())
        );
        assert_eq!(pick_seed(&other, &all[1..]), None);
        assert_eq!(pick_seed(&d.join("nogit"), &all), None);

        // A local "machine": mirrors under a prefix.
        let m = Mirror {
            target: None,
            prefix: d.join("machine").display().to_string(),
        };
        let (src, dst) = (PathBuf::from(m.remote(&main)), PathBuf::from(m.remote(&wt)));
        std::fs::create_dir_all(src.join("target")).unwrap();
        std::fs::write(src.join("lib.rs"), "seed").unwrap();
        std::fs::write(src.join("target/big"), "x").unwrap();
        std::fs::create_dir_all(dst.join("target")).unwrap();
        assert!(m.seed(&wt, &main));
        assert_eq!(std::fs::read_to_string(dst.join("lib.rs")).unwrap(), "seed");
        assert!(!dst.join("target/big").exists());
        // Never over an existing mirror.
        std::fs::write(src.join("lib.rs"), "newer").unwrap();
        assert!(!m.seed(&wt, &main));
        assert_eq!(std::fs::read_to_string(dst.join("lib.rs")).unwrap(), "seed");

        // Checksum sync after a seed: a same-size, same-mtime edit still
        // transfers, and matching files take the client's mtime.
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("lib.rs"), "SEED").unwrap();
        std::fs::write(wt.join("same.rs"), "same").unwrap();
        std::fs::write(dst.join("same.rs"), "same").unwrap();
        let mt = std::fs::metadata(dst.join("lib.rs"))
            .unwrap()
            .modified()
            .unwrap();
        for f in ["lib.rs", "same.rs"] {
            std::fs::File::options()
                .write(true)
                .open(wt.join(f))
                .unwrap()
                .set_modified(mt)
                .unwrap();
        }
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        std::fs::File::options()
            .write(true)
            .open(dst.join("same.rs"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(m.rsync_with(&wt, true).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(dst.join("lib.rs")).unwrap(), "SEED");
        assert_eq!(
            std::fs::metadata(dst.join("same.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            mt
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
