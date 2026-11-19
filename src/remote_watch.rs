//! Minimal directory watcher (libc only, no extra dependency): inotify on
//! Linux, kqueue on macOS. Sends the directory of every changed file to a
//! channel. Overflow sends every watched directory so a full rescan
//! follows: the scan, not these events, decides what is pushed.
//!
//! A directory created (or moved in) under a watched one is watched at once,
//! together with any subdirectories it already has, so files written into a
//! brand-new module directory are pushed without waiting for a build. The
//! daemon also refreshes the watch set from the manifest after every push.
//!
//! kqueue watches a directory's entries, not its files' contents: a file
//! rewritten in place (no rename) is pushed by the run's rescan instead of
//! early. Editors that save atomically (write and rename) are seen at once.
//!
//! Windows has no backend yet: nothing is watched, and every change is
//! pushed by the run's rescan.

use anyhow::{Result, bail};
use std::collections::HashMap;
#[cfg(unix)]
use std::collections::HashSet;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use inotify as sys;
#[cfg(target_os = "macos")]
use kqueue as sys;
#[cfg(windows)]
use unwatched as sys;

#[derive(Default)]
struct Watches {
    by_wd: HashMap<i32, PathBuf>,
    by_path: HashMap<PathBuf, i32>,
}

/// Only notifications are coalesced. Every event must still update the watch
/// set, and each kernel read starts fresh so a later edit invalidates again.
#[cfg(unix)]
#[derive(Default)]
struct ChangedBatch {
    sent: HashSet<PathBuf>,
}

#[cfg(unix)]
impl ChangedBatch {
    fn send(&mut self, tx: &Sender<PathBuf>, dir: PathBuf) -> bool {
        if self.sent.contains(&dir) {
            return true;
        }
        self.sent.insert(dir.clone());
        tx.send(dir).is_ok()
    }
}

pub struct Watcher {
    fd: i32,
    w: Arc<Mutex<Watches>>,
}

/// Directories never worth watching: build output and VCS internals churn
/// constantly and are never mirrored.
fn ignored(name: &std::ffi::OsStr) -> bool {
    name == "target" || name == ".git" || name == ".justrust"
}

fn add(fd: i32, w: &Mutex<Watches>, d: &Path) -> bool {
    if w.lock().unwrap().by_path.contains_key(d) {
        return false;
    }
    let Ok(c) = CString::new(d.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    let Some(wd) = sys::watch(fd, &c) else {
        return false;
    };
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
        sys::unwatch(fd, wd);
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
        let Some(fd) = sys::init() else {
            bail!("cannot create a file watcher");
        };
        let w: Arc<Mutex<Watches>> = Arc::default();
        let w2 = w.clone();
        std::thread::spawn(move || sys::read_loop(fd, w2, tx));
        Ok(Watcher { fd, w })
    }

    /// Watch every directory in `dirs` not watched yet. Returns how many
    /// were added.
    pub fn watch_all<'a>(&self, dirs: impl IntoIterator<Item = &'a PathBuf>) -> usize {
        // Build each chunk outside the lock: the caller's iterator can do
        // arbitrary work. Only bounded map lookups run under the reader's lock.
        const BATCH: usize = 16;
        let mut dirs = dirs.into_iter();
        // A new watcher (and the Windows fallback) has nothing to skip.
        if self.w.lock().unwrap().by_path.is_empty() {
            return dirs.filter(|d| add(self.fd, &self.w, d)).count();
        }
        let mut batch = Vec::with_capacity(BATCH);
        let mut added = 0;
        loop {
            batch.extend(dirs.by_ref().take(BATCH));
            if batch.is_empty() {
                return added;
            }
            {
                let g = self.w.lock().unwrap();
                batch.retain(|d| !g.by_path.contains_key(*d));
            }
            // Recheck misses via add: an event may have registered the path
            // in the meantime. A failed registration stays eligible next time.
            added += batch.drain(..).filter(|d| add(self.fd, &self.w, d)).count();
        }
    }

    #[cfg(test)]
    pub fn is_watched(&self, d: &Path) -> bool {
        self.w.lock().unwrap().by_path.contains_key(d)
    }
}

/// Windows: no watch is ever added, so `add` reports nothing watched and
/// the daemon falls back to the rescan every run does.
#[cfg(windows)]
mod unwatched {
    use super::*;

    pub fn init() -> Option<i32> {
        Some(0)
    }

    pub fn watch(_fd: i32, _path: &CString) -> Option<i32> {
        None
    }

    pub fn unwatch(_fd: i32, _wd: i32) {}

    pub fn read_loop(_fd: i32, _w: Arc<Mutex<Watches>>, _tx: Sender<PathBuf>) {}
}

#[cfg(target_os = "linux")]
mod inotify {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    const MASK: u32 = libc::IN_CLOSE_WRITE
        | libc::IN_MOVED_TO
        | libc::IN_MOVED_FROM
        | libc::IN_CREATE
        | libc::IN_DELETE
        | libc::IN_ATTRIB;

    pub fn init() -> Option<i32> {
        // SAFETY: plain syscall.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        (fd >= 0).then_some(fd)
    }

    pub fn watch(fd: i32, path: &CString) -> Option<i32> {
        // SAFETY: valid fd and NUL-terminated path.
        let wd = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), MASK) };
        (wd >= 0).then_some(wd)
    }

    pub fn unwatch(fd: i32, wd: i32) {
        // SAFETY: valid fd; a stale wd only makes this fail.
        unsafe {
            libc::inotify_rm_watch(fd, wd);
        }
    }

    pub fn read_loop(fd: i32, w: Arc<Mutex<Watches>>, tx: Sender<PathBuf>) {
        let mut buf = vec![0u8; 64 << 10];
        loop {
            // SAFETY: reading into a buffer we own.
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                return;
            }
            if !process_batch(fd, &w, &tx, &buf[..n as usize]) {
                return;
            }
        }
    }

    fn process_batch(fd: i32, w: &Mutex<Watches>, tx: &Sender<PathBuf>, buf: &[u8]) -> bool {
        let mut changed = ChangedBatch::default();
        let mut off = 0usize;
        let hdr = std::mem::size_of::<libc::inotify_event>();
        while off + hdr <= buf.len() {
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
                    if !changed.send(tx, d) {
                        return false;
                    }
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
                remove_tree(fd, w, &d.join(name));
            }
            if ev.mask & libc::IN_ISDIR != 0
                && ev.mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0
                && !name.is_empty()
                && !ignored(name)
            {
                add_tree(fd, w, &d.join(name));
            }
            // Our own editors' temp files still trigger a scan; cheap.
            if !changed.send(tx, d) {
                return false;
            }
        }
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

        fn event(buf: &mut Vec<u8>, wd: i32, mask: u32, name: &str) {
            let len = if name.is_empty() { 0 } else { name.len() + 1 };
            buf.extend_from_slice(&wd.to_ne_bytes());
            buf.extend_from_slice(&mask.to_ne_bytes());
            buf.extend_from_slice(&0u32.to_ne_bytes()); // cookie
            buf.extend_from_slice(&(len as u32).to_ne_bytes());
            if len != 0 {
                buf.extend_from_slice(name.as_bytes());
                buf.push(0);
            }
        }

        #[test]
        fn kernel_batch_coalesces_without_skipping_watch_updates() {
            let root = std::env::temp_dir().join(format!("jr-watch-batch-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("new/child")).unwrap();
            std::fs::create_dir_all(root.join("other")).unwrap();
            std::fs::create_dir_all(root.join("target/ignored")).unwrap();
            // SAFETY: init returns a fresh descriptor owned by this test.
            let owned_fd = unsafe { OwnedFd::from_raw_fd(init().unwrap()) };
            let fd = owned_fd.as_raw_fd();
            let w = Mutex::new(Watches::default());
            assert!(add(fd, &w, &root));
            let wd = w.lock().unwrap().by_path[&root];
            let (tx, rx) = std::sync::mpsc::channel();
            let mut batch = Vec::new();
            event(&mut batch, wd, libc::IN_CLOSE_WRITE, "file.rs");
            event(&mut batch, wd, libc::IN_ISDIR | libc::IN_CREATE, "new");
            event(&mut batch, wd, libc::IN_ISDIR | libc::IN_CREATE, "other");
            event(&mut batch, wd, libc::IN_ISDIR | libc::IN_CREATE, "target");
            event(&mut batch, wd, libc::IN_ISDIR | libc::IN_MOVED_FROM, "new");
            assert!(process_batch(fd, &w, &tx, &batch));
            assert_eq!(
                rx.try_iter().collect::<Vec<_>>().as_slice(),
                std::slice::from_ref(&root)
            );
            {
                let g = w.lock().unwrap();
                assert!(!g.by_path.contains_key(&root.join("new")));
                assert!(!g.by_path.contains_key(&root.join("new/child")));
                assert!(!g.by_path.contains_key(&root.join("target")));
                assert!(g.by_path.contains_key(&root.join("other")));
            }

            let other_wd = w.lock().unwrap().by_path[&root.join("other")];
            batch.clear();
            event(&mut batch, wd, libc::IN_ATTRIB, "file.rs");
            event(&mut batch, other_wd, libc::IN_CLOSE_WRITE, "nested.rs");
            event(&mut batch, -1, libc::IN_Q_OVERFLOW, "");
            event(&mut batch, -1, libc::IN_Q_OVERFLOW, "");
            event(&mut batch, other_wd, libc::IN_IGNORED, "");
            assert!(process_batch(fd, &w, &tx, &batch));
            // A new read invalidates root again. Overflow preserves every
            // watched path but does not repeat earlier notifications.
            assert_eq!(
                rx.try_iter().collect::<Vec<_>>(),
                [root.clone(), root.join("other")]
            );
            assert!(!w.lock().unwrap().by_path.contains_key(&root.join("other")));
            std::fs::remove_dir_all(&root).unwrap();
        }
    }
}

/// kqueue: one `O_EVTONLY` descriptor per watched directory (the "wd"),
/// registered for vnode events. A directory's entries changed: watch new
/// subdirectories, drop vanished ones, report it.
#[cfg(target_os = "macos")]
mod kqueue {
    use super::*;

    const FFLAGS: u32 = libc::NOTE_WRITE
        | libc::NOTE_EXTEND
        | libc::NOTE_ATTRIB
        | libc::NOTE_DELETE
        | libc::NOTE_RENAME
        | libc::NOTE_REVOKE;

    pub fn init() -> Option<i32> {
        // SAFETY: plain syscalls.
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return None;
        }
        unsafe { libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC) };
        Some(kq)
    }

    fn change(kq: i32, wd: i32, flags: u16) -> bool {
        let ev = libc::kevent {
            ident: wd as libc::uintptr_t,
            filter: libc::EVFILT_VNODE,
            flags,
            fflags: FFLAGS,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: one change, no events out, no wait.
        unsafe { libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) == 0 }
    }

    pub fn watch(kq: i32, path: &CString) -> Option<i32> {
        // SAFETY: NUL-terminated path; O_EVTONLY does not keep the volume busy.
        let wd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_EVTONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if wd < 0 {
            return None;
        }
        if !change(kq, wd, libc::EV_ADD | libc::EV_CLEAR | libc::EV_ENABLE) {
            unsafe { libc::close(wd) };
            return None;
        }
        Some(wd)
    }

    pub fn unwatch(_kq: i32, wd: i32) {
        // SAFETY: we own wd; closing it removes its kevent.
        unsafe { libc::close(wd) };
    }

    pub fn read_loop(kq: i32, w: Arc<Mutex<Watches>>, tx: Sender<PathBuf>) {
        let mut evs: Vec<libc::kevent> = (0..256)
            .map(|_| unsafe { std::mem::zeroed::<libc::kevent>() })
            .collect();
        loop {
            // SAFETY: the event buffer is writable for its length.
            let n = unsafe {
                libc::kevent(
                    kq,
                    std::ptr::null(),
                    0,
                    evs.as_mut_ptr(),
                    evs.len() as i32,
                    std::ptr::null(),
                )
            };
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            // Resolve every event's directory before acting on any: acting
            // closes descriptors, and a new watch may reuse a closed one's
            // number, so a later event in this batch would name the wrong
            // directory.
            let batch: Vec<(i32, u32, PathBuf)> = {
                let g = w.lock().unwrap();
                evs[..n as usize]
                    .iter()
                    .filter_map(|ev| {
                        let wd = ev.ident as i32;
                        Some((wd, ev.fflags, g.by_wd.get(&wd)?.clone()))
                    })
                    .collect()
            };
            let mut changed = ChangedBatch::default();
            for (wd, fflags, d) in batch {
                if fflags & (libc::NOTE_DELETE | libc::NOTE_RENAME | libc::NOTE_REVOKE) != 0 {
                    // Gone or moved: forget it and everything below, so a
                    // directory re-created at the path is watched again and
                    // the new location is picked up from its parent. Skip
                    // it if the parent's rescan already did (the path may
                    // now be a new directory with its own watch).
                    if w.lock().unwrap().by_path.get(&d) == Some(&wd) {
                        remove_tree(kq, &w, &d);
                    }
                    if let Some(p) = d.parent()
                        && !changed.send(&tx, p.to_path_buf())
                    {
                        return;
                    }
                    continue;
                }
                if fflags & libc::NOTE_WRITE != 0 {
                    sync_children(kq, &w, &d);
                }
                if !changed.send(&tx, d) {
                    return;
                }
            }
        }
    }

    /// `d`'s entries changed: watch new subdirectories, drop the watches of
    /// subdirectories no longer there.
    fn sync_children(kq: i32, w: &Mutex<Watches>, d: &Path) {
        let gone: Vec<PathBuf> = w
            .lock()
            .unwrap()
            .by_path
            .keys()
            .filter(|p| p.parent() == Some(d) && !p.is_dir())
            .cloned()
            .collect();
        for p in gone {
            remove_tree(kq, w, &p);
        }
        let Ok(rd) = std::fs::read_dir(d) else {
            return;
        };
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir())
                && !ignored(&e.file_name())
                && !w.lock().unwrap().by_path.contains_key(&e.path())
            {
                add_tree(kq, w, &e.path());
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn changed_batch_sends_each_exact_path_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut changed = ChangedBatch::default();
        for path in ["root", "root/a", "root", "root/a/b", "root/a"] {
            assert!(changed.send(&tx, PathBuf::from(path)));
        }
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            ["root", "root/a", "root/a/b"].map(PathBuf::from)
        );
    }

    #[test]
    fn changed_batch_does_not_suppress_later_invalidations() {
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..2 {
            let mut changed = ChangedBatch::default();
            assert!(changed.send(&tx, PathBuf::from("root")));
            assert!(changed.send(&tx, PathBuf::from("root")));
        }
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            ["root", "root"].map(PathBuf::from)
        );
    }

    #[test]
    fn changed_batch_overflow_keeps_all_watched_paths() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut changed = ChangedBatch::default();
        assert!(changed.send(&tx, PathBuf::from("root/a")));
        // Overflow resends the whole watch set, including paths already seen.
        for path in ["root", "root/a", "root/b", "root"] {
            assert!(changed.send(&tx, PathBuf::from(path)));
        }
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            ["root/a", "root", "root/b"].map(PathBuf::from)
        );
        drop(rx);
        assert!(!changed.send(&tx, PathBuf::from("root/c")));
    }

    #[test]
    fn refresh_retries_failed_watches_across_chunks() {
        let d = std::env::temp_dir().join(format!("jr-watch-refresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dirs: Vec<_> = (0..39).map(|i| d.join(i.to_string())).collect();
        for p in &dirs[..34] {
            std::fs::create_dir_all(p).unwrap();
        }
        let (tx, _rx) = std::sync::mpsc::channel();
        let w = Watcher::new(tx).unwrap();
        assert_eq!(w.watch_all(dirs.iter().chain(&dirs)), 34);
        assert_eq!(w.watch_all(&dirs), 0);
        for p in &dirs[34..] {
            std::fs::create_dir(p).unwrap();
        }
        assert_eq!(w.watch_all(&dirs), 5);
        assert_eq!(w.watch_all(&dirs), 0);
        std::fs::remove_dir_all(d).unwrap();
    }

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
