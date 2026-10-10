//! Minimal inotify watcher (libc only, no extra dependency). Sends the
//! directory of every changed file to a channel. Overflow or an unknown
//! watch sends the root-level paths so a full rescan follows: the scan, not
//! these events, decides what is pushed.

use anyhow::{Result, bail};
use std::collections::{BTreeSet, HashMap};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub struct Watcher {
    fd: i32,
    wds: Arc<Mutex<HashMap<i32, PathBuf>>>,
    watched: BTreeSet<PathBuf>,
}

const MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_ATTRIB;

impl Watcher {
    pub fn new(tx: Sender<PathBuf>) -> Result<Self> {
        // SAFETY: plain syscall.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            bail!("inotify_init1 failed");
        }
        let wds: Arc<Mutex<HashMap<i32, PathBuf>>> = Arc::default();
        let w2 = wds.clone();
        std::thread::spawn(move || read_loop(fd, w2, tx));
        Ok(Watcher {
            fd,
            wds,
            watched: BTreeSet::new(),
        })
    }

    pub fn watch_all(&mut self, dirs: &BTreeSet<PathBuf>) {
        for d in dirs {
            if self.watched.contains(d) {
                continue;
            }
            let Ok(c) = CString::new(d.as_os_str().as_bytes()) else {
                continue;
            };
            // SAFETY: valid fd and NUL-terminated path.
            let wd = unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), MASK) };
            if wd >= 0 {
                self.wds.lock().unwrap().insert(wd, d.clone());
                self.watched.insert(d.clone());
            }
        }
    }
}

fn read_loop(fd: i32, wds: Arc<Mutex<HashMap<i32, PathBuf>>>, tx: Sender<PathBuf>) {
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
            let dir = wds.lock().unwrap().get(&ev.wd).cloned();
            if ev.mask & libc::IN_Q_OVERFLOW != 0 {
                // Lost events: every root gets rescanned.
                for d in wds.lock().unwrap().values() {
                    let _ = tx.send(d.clone());
                }
            } else if let Some(d) = dir {
                // Our own editors' temp files still trigger a scan; cheap.
                if tx.send(d).is_err() {
                    return;
                }
            }
            off += hdr + ev.len as usize;
        }
    }
}
