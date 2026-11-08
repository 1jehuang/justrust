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
//! that token and the size of every file in the manifest matches (0.6 s for
//! 94k files over ssh; a sample of the newest files once missed a
//! truncated file the previous daemon had saved as current). Then
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

/// Shell script printing, gzipped, the token, then `size path` for each
/// NUL-separated gzipped path on stdin that exists.
fn probe_script(remote: &str) -> String {
    let q = crate::remote_build::shell_quote;
    format!(
        "{{ cat {t} 2>/dev/null; rm -f {t}; echo; cd {} || exit 1; \
         gzip -dc | xargs -0 -r stat -c '%s %n' -- 2>/dev/null; true; }} | gzip -1",
        q(remote),
        t = q(&format!("{remote}/{TOKEN_FILE}")),
    )
}

fn probe_input(files: &[&PathBuf]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    for f in files {
        let b = f.as_os_str().as_encoded_bytes();
        if b.contains(&0) {
            return None;
        }
        gz.write_all(b).ok()?;
        gz.write_all(b"\0").ok()?;
    }
    gz.finish().ok()
}

/// Whether the probe output confirms `token` and the size of every file,
/// else the first file that differs.
fn confirms(out: &str, token: &str, files: &Manifest) -> Result<(), String> {
    let mut lines = out.lines();
    if lines.next().map(str::trim) != Some(token) || token.is_empty() {
        return Err("machine token missing or replaced".into());
    }
    let got: std::collections::HashMap<&str, &str> = lines
        .filter_map(|l| l.split_once(' ').map(|(size, name)| (name, size)))
        .collect();
    for (p, st) in files {
        match p.to_str().and_then(|n| got.get(n)) {
            None => return Err(format!("{} missing on the machine", p.display())),
            Some(size) if size.parse::<u64>().ok() != Some(st.size) => {
                return Err(format!(
                    "{} is {size} bytes on the machine, {} saved",
                    p.display(),
                    st.size
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn run_remote(mirror: &Mirror, script: &str) -> Option<String> {
    run_remote_with(mirror, script, None)
}

fn run_remote_with(mirror: &Mirror, script: &str, input: Option<Vec<u8>>) -> Option<String> {
    let gunzip = input.is_some();
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
    c.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    let mut child = c.spawn().ok()?;
    let feeder = input.and_then(|b| {
        let mut w = child.stdin.take()?;
        Some(std::thread::spawn(move || {
            use std::io::Write;
            let _ = w.write_all(&b);
        }))
    });
    let out = child.wait_with_output().ok()?;
    if let Some(f) = feeder {
        let _ = f.join();
    }
    let out = std::process::Output {
        stdout: if gunzip {
            use std::io::Read;
            let mut v = Vec::new();
            flate2::read::GzDecoder::new(&out.stdout[..])
                .read_to_end(&mut v)
                .ok()?;
            v
        } else {
            out.stdout
        },
        ..out
    };
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
/// exactly that tree, else why not. Consumes the saved file either way.
pub fn take(
    backend: &str,
    mirror: &Mirror,
    root: &Path,
) -> Result<(Manifest, Vec<PathBuf>), String> {
    if !enabled() {
        return Err("disabled".into());
    }
    take_in(&dir().ok_or("no state dir")?, backend, mirror, root)
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
#[cfg_attr(not(unix), allow(dead_code))]
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
) -> Result<(Manifest, Vec<PathBuf>), String> {
    let path = file_in(d, backend, root);
    let raw = std::fs::read(&path).map_err(|_| "nothing saved".to_string())?;
    let _ = std::fs::remove_file(&path);
    let s: Saved = serde_json::from_slice(&raw).map_err(|_| "unreadable save".to_string())?;
    if s.v != VERSION
        || s.backend != backend
        || s.root != root.display().to_string()
        || s.remote != mirror.remote(root)
    {
        return Err("saved for another backend or version".into());
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
    let names: Vec<&PathBuf> = manifest.keys().collect();
    let input = probe_input(&names).ok_or("path with NUL")?;
    let out =
        run_remote_with(mirror, &probe_script(&s.remote), Some(input)).ok_or("probe failed")?;
    confirms(&out, &s.token, &manifest)?;
    Ok((
        manifest,
        s.oversized.into_iter().map(PathBuf::from).collect(),
    ))
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
        let m: Manifest = [
            (PathBuf::from("a b.rs"), stamp(1, 12)),
            (PathBuf::from("c.rs"), stamp(2, 7)),
        ]
        .into();
        assert!(confirms("tok\n7 c.rs\n12 a b.rs\n", "tok", &m).is_ok());
        assert!(confirms("other\n12 a b.rs\n7 c.rs\n", "tok", &m).is_err());
        assert_eq!(
            confirms("tok\n12 a b.rs\n0 c.rs\n", "tok", &m),
            Err("c.rs is 0 bytes on the machine, 7 saved".into())
        );
        assert_eq!(
            confirms("tok\n12 a b.rs\n", "tok", &m),
            Err("c.rs missing on the machine".into())
        );
        assert!(confirms("\n", "", &Manifest::new()).is_err());
        assert!(confirms("tok\n", "tok", &Manifest::new()).is_ok());
    }

    /// Save on a local "machine", then resume: the manifest comes back only
    /// while the machine's copy still matches, and only once.
    /// The probe uses GNU `stat -c` like the Linux build machines; BSD stat
    /// on macOS does not support it, so the local-machine test is Linux-only.
    #[cfg(target_os = "linux")]
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
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
        // Consumed: a second daemon cannot reuse it.
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
        // The machine's copy changed size: full mirror instead.
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        std::fs::write(remote.join("a.rs"), "fn a() { changed }\n").unwrap();
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
        // Every file is checked, not just the newest: an old file the
        // previous daemon wrongly believed current blocks the resume.
        std::fs::write(remote.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(remote.join("old.rs"), "").unwrap();
        let two: Manifest = [
            (PathBuf::from("a.rs"), stamp(5, 10)),
            (PathBuf::from("old.rs"), stamp(1, 27)),
        ]
        .into();
        assert_eq!(save_in(&home, "hosted", &mirror, &[(&root, &two, &[])]), 1);
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
        // A file missing on the machine blocks it too.
        std::fs::remove_file(remote.join("old.rs")).unwrap();
        assert_eq!(save_in(&home, "hosted", &mirror, &[(&root, &two, &[])]), 1);
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
        // A different machine (no token): full mirror.
        std::fs::write(remote.join("a.rs"), "fn a() {}\n").unwrap();
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        std::fs::remove_file(remote.join(TOKEN_FILE)).unwrap();
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
        // Any source write under the root on the machine drops the token.
        assert_eq!(
            save_in(&home, "hosted", &mirror, &[(&root, &manifest, &[])]),
            1
        );
        invalidate_above(&remote.join("src/new.rs"));
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
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
        assert!(take_in(&home, "hosted", &mirror, &root).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
