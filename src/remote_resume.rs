//! Resume source mirrors across sync daemon restarts.
//!
//! A new daemon used to mirror every requested root again with a
//! `--checksum` rsync, which reads every source file on both sides: 43-51 s
//! for this repository (95k files, 1.4 GB) after each client upgrade or idle
//! exit, mostly cold page cache reads on the client.
//!
//! On exit a daemon that knows exactly what the machine has (a successful
//! barrier with nothing pending, no failed paths) saves each root's
//! manifest and writes a fresh random token into `<root>/.justrust/` on the
//! machine. The next daemon resumes a root only when the machine still has
//! that token and the sizes of the most recently changed files match. Then
//! it pushes just the difference between the saved manifest and the local
//! tree, as the live daemon would have. Anything else (no file, a new or
//! reprovisioned machine, another daemon's token, a size mismatch, an
//! unreadable file) falls back to the full mirror.
//!
//! A saved manifest is consumed when read, and the probe reads and deletes
//! the machine's token in one command, so at most one daemon can resume it.
//! The token is also deleted by every change to the machine's tree that
//! the next daemon would not know about: the agent drops it before applying
//! any source write or delete under the root (from any client), and full or
//! bulk rsyncs drop it before they start. A save is written only after a
//! clean barrier on exit.
//! `JUSTRUST_REMOTE_RESUME=0` (in the daemon's environment) turns this off.

use crate::remote_sync::{Manifest, Mirror, Stamp};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;

const VERSION: u32 = 1;
/// Files whose size the machine must confirm before a resume.
const SPOT_CHECK: usize = 24;
const TOKEN_FILE: &str = ".justrust/mirror-token";

#[derive(Serialize, Deserialize)]
struct Saved {
    v: u32,
    backend: String,
    root: String,
    remote: String,
    token: String,
    /// (relative path, mtime ns, size, mode)
    files: Vec<(String, i64, u64, u32)>,
    oversized: Vec<String>,
}

pub fn enabled() -> bool {
    std::env::var("JUSTRUST_REMOTE_RESUME").as_deref() != Ok("0")
}

fn dir() -> Option<PathBuf> {
    crate::remote::dir().ok().map(|d| d.join("mirrors"))
}

fn file_in(dir: &Path, backend: &str, root: &Path) -> PathBuf {
    let mut h = crate::depcache::H128::new();
    h.field(backend.as_bytes());
    h.field(root.as_os_str().as_encoded_bytes());
    dir.join(format!("{}.json", &h.hex()[..24]))
}

fn new_token() -> String {
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

/// The most recently modified files: the ones most likely to differ if the
/// machine's copy diverged.
fn spot_files(m: &Manifest) -> Vec<(&PathBuf, &Stamp)> {
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by_key(|(_, s)| std::cmp::Reverse(s.mtime_ns));
    v.truncate(SPOT_CHECK);
    v
}

/// Shell script printing the token, then the size of each file (-1 if absent).
fn probe_script(remote: &str, files: &[&PathBuf]) -> String {
    let q = crate::remote_build::shell_quote;
    let mut s = format!(
        "cat {t} 2>/dev/null; rm -f {t}; echo; cd {} || exit 1",
        q(remote),
        t = q(&format!("{remote}/{TOKEN_FILE}")),
    );
    for f in files {
        s.push_str(&format!(
            "; stat -c %s -- {} 2>/dev/null || echo -1",
            q(&f.to_string_lossy())
        ));
    }
    s
}

/// Whether the probe output confirms `token` and every expected size.
fn confirms(out: &str, token: &str, sizes: &[u64]) -> bool {
    let mut lines = out.lines();
    if lines.next().map(str::trim) != Some(token) || token.is_empty() {
        return false;
    }
    let got: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    got.len() == sizes.len()
        && got
            .iter()
            .zip(sizes)
            .all(|(g, want)| g.parse::<u64>().ok() == Some(*want))
}

fn run_remote(mirror: &Mirror, script: &str) -> Option<String> {
    let mut c = match &mirror.target {
        Some(t) => {
            let mut c = t.command();
            c.arg(script);
            c
        }
        None => {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg(script);
            c
        }
    };
    let out = c.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Save `roots` (root, manifest, oversized), which the machine is known to
/// hold exactly. Tokens go to the machine first: a save without its token
/// is never trusted.
pub fn save(backend: &str, mirror: &Mirror, roots: &[(&Path, &Manifest, &[PathBuf])]) -> usize {
    dir().map_or(0, |d| save_in(&d, backend, mirror, roots))
}

fn save_in(
    d: &Path,
    backend: &str,
    mirror: &Mirror,
    roots: &[(&Path, &Manifest, &[PathBuf])],
) -> usize {
    let _ = std::fs::create_dir_all(d);
    let mut planned = Vec::new();
    let mut script = String::from("set -e");
    let q = crate::remote_build::shell_quote;
    for (root, manifest, oversized) in roots {
        let Some(files) = manifest
            .iter()
            .map(|(p, s)| {
                Some((
                    p.to_str()?.to_owned(),
                    i64::try_from(s.mtime_ns).ok()?,
                    s.size,
                    s.mode,
                ))
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue; // non-UTF-8 paths: not worth the risk
        };
        let Some(oversized) = oversized
            .iter()
            .map(|p| p.to_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let remote = mirror.remote(root);
        let token = new_token();
        script.push_str(&format!(
            "; mkdir -p {d} && printf %s {t} > {f}",
            d = q(&format!("{remote}/.justrust")),
            t = q(&token),
            f = q(&format!("{remote}/{TOKEN_FILE}"))
        ));
        planned.push(Saved {
            v: VERSION,
            backend: backend.to_owned(),
            root: root.display().to_string(),
            remote,
            token,
            files,
            oversized,
        });
    }
    if planned.is_empty() || run_remote(mirror, &script).is_none() {
        return 0;
    }
    let mut n = 0;
    for s in planned {
        let path = file_in(d, backend, Path::new(&s.root));
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if let Ok(b) = serde_json::to_vec(&s)
            && std::fs::write(&tmp, b).is_ok()
            && std::fs::rename(&tmp, &path).is_ok()
        {
            n += 1;
        }
    }
    n
}

/// The saved manifest of `root`, if the machine confirms it still holds
/// exactly that tree. Consumes the saved file either way.
pub fn take(backend: &str, mirror: &Mirror, root: &Path) -> Option<(Manifest, Vec<PathBuf>)> {
    if !enabled() {
        return None;
    }
    take_in(&dir()?, backend, mirror, root)
}

/// Before a full mirror: no saved state for `root` survives it.
pub fn invalidate(mirror: &Mirror, root: &Path) {
    let q = crate::remote_build::shell_quote;
    let _ = run_remote(
        mirror,
        &format!(
            "rm -f {}",
            q(&format!("{}/{TOKEN_FILE}", mirror.remote(root)))
        ),
    );
}

/// Machine side, before applying a source write or delete at `path`: any
/// saved mirror state of a root above it no longer describes the tree.
/// One `stat` per ancestor.
pub fn invalidate_above(path: &Path) {
    for dir in path.ancestors().skip(1) {
        let token = dir.join(TOKEN_FILE);
        if token.is_file() {
            let _ = std::fs::remove_file(token);
        }
    }
}

fn take_in(
    d: &Path,
    backend: &str,
    mirror: &Mirror,
    root: &Path,
) -> Option<(Manifest, Vec<PathBuf>)> {
    let path = file_in(d, backend, root);
    let raw = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let s: Saved = serde_json::from_slice(&raw).ok()?;
    if s.v != VERSION
        || s.backend != backend
        || s.root != root.display().to_string()
        || s.remote != mirror.remote(root)
    {
        return None;
    }
    let manifest: Manifest = s
        .files
        .into_iter()
        .map(|(p, mtime_ns, size, mode)| {
            (
                PathBuf::from(p),
                Stamp {
                    mtime_ns: mtime_ns as i128,
                    size,
                    mode,
                },
            )
        })
        .collect();
    let spot = spot_files(&manifest);
    let names: Vec<&PathBuf> = spot.iter().map(|(p, _)| *p).collect();
    let sizes: Vec<u64> = spot.iter().map(|(_, s)| s.size).collect();
    let out = run_remote(mirror, &probe_script(&s.remote, &names))?;
    confirms(&out, &s.token, &sizes).then(|| {
        (
            manifest,
            s.oversized.into_iter().map(PathBuf::from).collect(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(t: i128, size: u64) -> Stamp {
        Stamp {
            mtime_ns: t,
            size,
            mode: 0o644,
        }
    }

    #[test]
    fn probe_output_must_match_token_and_every_size() {
        assert!(confirms("tok\n12\n7\n", "tok", &[12, 7]));
        assert!(!confirms("other\n12\n7\n", "tok", &[12, 7]));
        assert!(!confirms("tok\n12\n-1\n", "tok", &[12, 7]));
        assert!(!confirms("tok\n12\n", "tok", &[12, 7]));
        assert!(!confirms("\n", "", &[]));
        assert!(confirms("tok\n", "tok", &[]));
    }

    #[test]
    fn spot_check_takes_the_newest_files() {
        let m: Manifest = (0..40)
            .map(|i| (PathBuf::from(format!("f{i}")), stamp(i, i as u64)))
            .collect();
        let s = spot_files(&m);
        assert_eq!(s.len(), SPOT_CHECK);
        assert_eq!(s[0].0, &PathBuf::from("f39"));
        assert!(
            s.iter()
                .all(|(_, st)| st.mtime_ns >= 40 - SPOT_CHECK as i128)
        );
    }

    /// Save on a local "machine", then resume: the manifest comes back only
    /// while the machine's copy still matches, and only once.
    #[cfg(unix)]
    #[test]
    fn save_then_take_round_trip_and_divergence() {
        let base = std::env::temp_dir().join(format!("jr-resume-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let root = base.join("src");
        let machine = base.join("machine");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let mirror = Mirror {
            target: None,
            prefix: machine.display().to_string(),
        };
        let remote = PathBuf::from(mirror.remote(&root));
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("a.rs"), "fn a() {}\n").unwrap();
        let manifest: Manifest = [(PathBuf::from("a.rs"), stamp(5, 10))].into();
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        let (got, over) = take_in(&home, "hosted", &mirror, &root).expect("resumes");
        assert_eq!(got, manifest);
        assert!(over.is_empty());
        // The probe consumed the machine's token too.
        assert!(!remote.join(TOKEN_FILE).exists());
        // A full mirror invalidates a save before it starts.
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        invalidate(&mirror, &root);
        assert!(take_in(&home, "hosted", &mirror, &root).is_none());
        // Consumed: a second daemon cannot reuse it.
        assert!(take_in(&home, "hosted", &mirror, &root).is_none());
        // The machine's copy changed size: full mirror instead.
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        std::fs::write(remote.join("a.rs"), "fn a() { changed }\n").unwrap();
        assert!(take_in(&home, "hosted", &mirror, &root).is_none());
        // A different machine (no token): full mirror.
        std::fs::write(remote.join("a.rs"), "fn a() {}\n").unwrap();
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        std::fs::remove_file(remote.join(TOKEN_FILE)).unwrap();
        assert!(take_in(&home, "hosted", &mirror, &root).is_none());
        // Any source write under the root on the machine drops the token.
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        invalidate_above(&remote.join("src/new.rs"));
        assert!(take_in(&home, "hosted", &mirror, &root).is_none());
        // Another saver's token replaced ours: only the last save matches.
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        let first = std::fs::read(file_in(&home, "hosted", &root)).unwrap();
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        std::fs::write(file_in(&home, "hosted", &root), first).unwrap();
        assert!(take_in(&home, "hosted", &mirror, &root).is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
