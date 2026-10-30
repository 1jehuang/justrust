//! Minimal inotify watcher (libc only, no extra dependency). Sends the
//! directory of every changed file to a channel. Overflow sends every
//! watched directory so a full rescan follows: the scan, not these events,
//! decides what is pushed.
//!
//! A directory created (or moved in) under a watched one is watched at once,
//! together with any subdirectories it already has, so files written into a
//! brand-new module directory are pushed without waiting for a build. The
//! daemon also refreshes the watch set from the manifest after every push.

use anyhow::{Result, bail};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Watches {
    by_wd: HashMap<i32, PathBuf>,
    by_path: HashMap<PathBuf, i32>,
}

pub struct Watcher {
    fd: i32,
    w: Arc<Mutex<Watches>>,
}

const MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_ATTRIB;

/// Directories never worth watching: build output and VCS internals churn
/// constantly and are never mirrored.
fn ignored(name: &std::ffi::OsStr) -> bool {
    name == "target" || name == ".git" || name == ".justrust"
}

fn add(fd: i32, w: &Mutex<Watches>, d: &Path) -> bool {
    if w.lock().unwrap().by_path.contains_key(d) {
        return false;
    }
    let Ok(c) = CString::new(d.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: valid fd and NUL-terminated path.
    let wd = unsafe { libc::inotify_add_watch(fd, c.as_ptr(), MASK) };
    if wd < 0 {
        return false;
    }
    let mut g = w.lock().unwrap();
    // The kernel returns the existing wd for an inode already watched under
    // another path (a directory that was renamed): forget the old path.
    if let Some(old) = g.by_wd.insert(wd, d.to_path_buf()) {
        g.by_path.remove(&old);
    }
    g.by_path.insert(d.to_path_buf(), wd);
    true
}

/// A watched directory moved away: drop the watches of it and everything
/// below it, so its old paths can be watched again if re-created and its
/// new location is watched under the right name.
fn remove_tree(fd: i32, w: &Mutex<Watches>, d: &Path) {
    let mut g = w.lock().unwrap();
    let gone: Vec<(PathBuf, i32)> = g
        .by_path
        .iter()
        .filter(|(p, _)| p.starts_with(d))
        .map(|(p, wd)| (p.clone(), *wd))
        .collect();
    for (p, wd) in gone {
        g.by_path.remove(&p);
        g.by_wd.remove(&wd);
        // SAFETY: valid fd; a stale wd only makes this fail.
        unsafe {
            libc::inotify_rm_watch(fd, wd);
        }
    }
}

/// Watch `d` and every directory below it (except ignored ones).
fn add_tree(fd: i32, w: &Mutex<Watches>, d: &Path) {
    let mut stack = vec![d.to_path_buf()];
    while let Some(d) = stack.pop() {
        add(fd, w, &d);
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) && !ignored(&e.file_name()) {
                stack.push(e.path());
            }
        }
    }
}

impl Watcher {
    pub fn new(tx: Sender<PathBuf>) -> Result<Self> {
        // SAFETY: plain syscall.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            bail!("inotify_init1 failed");
        }
        let w: Arc<Mutex<Watches>> = Arc::default();
        let w2 = w.clone();
        std::thread::spawn(move || read_loop(fd, w2, tx));
        Ok(Watcher { fd, w })
    }

    /// Watch every directory in `dirs` not watched yet. Returns how many
    /// were added.
    pub fn watch_all<'a>(&self, dirs: impl IntoIterator<Item = &'a PathBuf>) -> usize {
        dirs.into_iter()
            .filter(|d| add(self.fd, &self.w, d))
            .count()
    }

    #[cfg(test)]
    pub fn is_watched(&self, d: &Path) -> bool {
        self.w.lock().unwrap().by_path.contains_key(d)
    }
}

fn read_loop(fd: i32, w: Arc<Mutex<Watches>>, tx: Sender<PathBuf>) {
    let mut buf = vec![0u8; 64 << 10];
    loop {
        // SAFETY: reading into a buffer we own.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            return;
        }
        let mut off = 0usize;
        let hdr = std::mem::size_of::<libc::inotify_event>();
        while off + hdr <= n as usize {
            // SAFETY: the kernel wrote a whole event at `off`.
            let ev: libc::inotify_event =
                unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off).cast()) };
            let name_bytes = &buf[off + hdr..off + hdr + ev.len as usize];
            let name = std::ffi::OsStr::from_bytes(
                name_bytes.split(|b| *b == 0).next().unwrap_or_default(),
            );
            off += hdr + ev.len as usize;
            if ev.mask & libc::IN_Q_OVERFLOW != 0 {
                // Lost events: every root gets rescanned.
                let all: Vec<PathBuf> = w.lock().unwrap().by_wd.values().cloned().collect();
                for d in all {
                    let _ = tx.send(d);
                }
                continue;
            }
            if ev.mask & libc::IN_IGNORED != 0 {
                // The directory is gone (or unmounted): forget it, so a
                // directory re-created at the same path is watched again.
                let mut g = w.lock().unwrap();
                if let Some(p) = g.by_wd.remove(&ev.wd) {
                    g.by_path.remove(&p);
                }
                continue;
            }
            let dir = w.lock().unwrap().by_wd.get(&ev.wd).cloned();
            let Some(d) = dir else { continue };
            if ev.mask & libc::IN_ISDIR != 0
                && ev.mask & libc::IN_MOVED_FROM != 0
                && !name.is_empty()
            {
                remove_tree(fd, &w, &d.join(name));
            }
            if ev.mask & libc::IN_ISDIR != 0
                && ev.mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0
                && !name.is_empty()
                && !ignored(name)
            {
                add_tree(fd, &w, &d.join(name));
            }
            // Our own editors' temp files still trigger a scan; cheap.
            if tx.send(d).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn new_directories_are_watched() {
        let d = std::env::temp_dir().join(format!("jr-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let w = Watcher::new(tx).unwrap();
        assert_eq!(w.watch_all([&d]), 1);
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::create_dir_all(d.join("target/x")).unwrap();
        // The create event arrives and the new dir gets a watch.
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), d);
        let t = std::time::Instant::now();
        while !w.is_watched(&d.join("a/b")) && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(w.is_watched(&d.join("a")) && w.is_watched(&d.join("a/b")));
        assert!(!w.is_watched(&d.join("target")));
        std::fs::write(d.join("a/b/new.rs"), "x").unwrap();
        recv_until(&rx, &d.join("a/b"));
        // Removed and re-created: watched again.
        std::fs::remove_dir_all(d.join("a")).unwrap();
        let t = std::time::Instant::now();
        while w.is_watched(&d.join("a")) && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!w.is_watched(&d.join("a")));

        // Renamed: watched under the new name, the old name is free.
        std::fs::create_dir_all(d.join("m/n")).unwrap();
        wait(|| w.is_watched(&d.join("m/n")));
        std::fs::rename(d.join("m"), d.join("r")).unwrap();
        wait(|| w.is_watched(&d.join("r/n")));
        assert!(w.is_watched(&d.join("r")));
        assert!(!w.is_watched(&d.join("m")) && !w.is_watched(&d.join("m/n")));
        std::fs::create_dir_all(d.join("m")).unwrap();
        wait(|| w.is_watched(&d.join("m")));
        std::fs::write(d.join("r/n/x.rs"), "x").unwrap();
        recv_until(&rx, &d.join("r/n"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Events for earlier steps may still be in flight: skip them.
    fn recv_until(rx: &std::sync::mpsc::Receiver<PathBuf>, want: &Path) {
        loop {
            let got = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if got == want {
                return;
            }
        }
    }

    fn wait(f: impl Fn() -> bool) {
        let t = std::time::Instant::now();
        while !f() && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(f());
    }
}
