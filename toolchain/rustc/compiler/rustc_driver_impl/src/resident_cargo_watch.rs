//! Best-effort idle eviction hints for resident external inputs (Linux only).
//!
//! An unchanged watch is never evidence that cached compiler state can be reused.
//! The normal input validation remains authoritative, including during watch setup
//! races. Watch failures should disable residency, not fail a compilation.

use std::cell::Cell;
use rustc_data_structures::fx::FxHashMap;
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const FILE_EVENTS: u32 = libc::IN_MODIFY
    | libc::IN_ATTRIB
    | libc::IN_CLOSE_WRITE
    | libc::IN_MOVE_SELF
    | libc::IN_DELETE_SELF;
const DIRECTORY_EVENTS: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_MOVE_SELF
    | libc::IN_DELETE_SELF;
const LOST_EVENTS: u32 = libc::IN_Q_OVERFLOW | libc::IN_IGNORED | libc::IN_UNMOUNT;

pub(super) struct InputWatch {
    fd: OwnedFd,
    // inotify shares a descriptor for repeated paths and inode aliases.
    watches: FxHashMap<i32, bool>, // true if this inode is also an explicit file input
    changed: Cell<bool>,
}

impl InputWatch {
    pub(super) fn new<'a>(
        files: impl Iterator<Item = &'a Path>,
        directories: impl Iterator<Item = &'a Path>,
    ) -> io::Result<Self> {
        // SAFETY: inotify_init1 returns a fresh owned descriptor on success.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut watch = Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
            watches: FxHashMap::default(),
            changed: Cell::new(false),
        };
        for path in files {
            watch.add(path, FILE_EVENTS, true)?;
        }
        for path in directories {
            watch.add(path, DIRECTORY_EVENTS | libc::IN_ONLYDIR, false)?;
        }
        Ok(watch)
    }

    fn add(&mut self, path: &Path, mask: u32, file: bool) -> io::Result<()> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in input watch path"))?;
        // IN_MASK_ADD preserves watches when aliases resolve to the same inode.
        let wd = unsafe {
            libc::inotify_add_watch(self.as_raw_fd(), path.as_ptr(), mask | libc::IN_MASK_ADD)
        };
        if wd < 0 {
            return Err(io::Error::last_os_error());
        }
        *self.watches.entry(wd).or_default() |= file;
        Ok(())
    }

    /// Drain available events without blocking. Once changed, always changed.
    /// Errors likewise mean the caller cannot keep relying on the eviction hint.
    pub(super) fn changed(&self) -> io::Result<bool> {
        if self.changed.get() {
            return Ok(true);
        }
        // Keep the kernel buffer aligned, but never dereference event pointers:
        // individual headers are copied with read_unaligned after bounds checks.
        #[repr(align(8))]
        struct Buffer([u8; 65536]);
        let mut buffer = Buffer([0; 65536]);
        // Bound work even if a busy unrelated directory continuously produces events.
        for _ in 0..64 {
            let count = unsafe {
                libc::read(self.as_raw_fd(), buffer.0.as_mut_ptr().cast(), buffer.0.len())
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                match error.kind() {
                    io::ErrorKind::WouldBlock => return Ok(false),
                    io::ErrorKind::Interrupted => continue,
                    _ => {
                        self.changed.set(true);
                        return Err(error);
                    }
                }
            }
            if count == 0 || self.events_changed(&buffer.0[..count as usize]) {
                self.changed.set(true);
                return Ok(true);
            }
        }
        // Conservatively evict instead of starving the resident command loop.
        self.changed.set(true);
        Ok(true)
    }

    fn events_changed(&self, bytes: &[u8]) -> bool {
        let header_size = std::mem::size_of::<libc::inotify_event>();
        let mut offset = 0;
        while offset < bytes.len() {
            if bytes.len() - offset < header_size {
                return true;
            }
            let event = unsafe {
                bytes.as_ptr().add(offset).cast::<libc::inotify_event>().read_unaligned()
            };
            let Some(end) =
                offset.checked_add(header_size).and_then(|n| n.checked_add(event.len as usize))
            else {
                return true;
            };
            if end > bytes.len() || event.mask & LOST_EVENTS != 0 {
                return true;
            }
            let Some(&file) = self.watches.get(&event.wd) else {
                return true;
            };
            if file && event.mask & FILE_EVENTS != 0 {
                return true;
            }
            if event.mask & (libc::IN_MOVE_SELF | libc::IN_DELETE_SELF) != 0 {
                return true;
            }
            if event.mask & DIRECTORY_EVENTS != 0 && event.mask & libc::IN_ISDIR == 0 {
                let name = &bytes[offset + header_size..end];
                let Some(nul) = name.iter().position(|&b| b == 0) else {
                    return true;
                };
                let name = &name[..nul];
                if name.ends_with(b".rlib") || name.ends_with(b".rmeta") {
                    return true;
                }
            }
            offset = end;
        }
        false
    }
}

impl AsRawFd for InputWatch {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Temp(std::path::PathBuf);

    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "jr-input-watch-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn content_attribute_move_and_delete_evict() {
        for operation in 0..4 {
            let temp = Temp::new();
            let input = temp.0.join("dependency.rlib");
            fs::write(&input, b"old").unwrap();
            let watch =
                InputWatch::new(std::iter::once(input.as_path()), std::iter::empty()).unwrap();
            assert!(!watch.changed().unwrap());
            match operation {
                0 => fs::write(&input, b"new").unwrap(),
                1 => {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&input, fs::Permissions::from_mode(0o400)).unwrap();
                }
                2 => fs::rename(&input, temp.0.join("moved")).unwrap(),
                _ => fs::remove_file(&input).unwrap(),
            }
            assert!(watch.changed().unwrap(), "operation {operation}");
            assert!(watch.changed().unwrap(), "change must remain sticky");
        }
    }

    #[test]
    fn directory_filters_names_and_tracks_artifact_membership() {
        for extension in ["rlib", "rmeta"] {
            for operation in 0..4 {
                let temp = Temp::new();
                let artifact = temp.0.join(format!("libdependency.{extension}"));
                if operation != 0 {
                    fs::write(&artifact, b"old").unwrap();
                }
                let watch =
                    InputWatch::new(std::iter::empty(), std::iter::once(temp.0.as_path())).unwrap();
                for name in ["a.d", "a.o", "a.tmp", "other", "a.rlib.tmp"] {
                    let path = temp.0.join(name);
                    fs::write(&path, b"noise").unwrap();
                    fs::remove_file(&path).unwrap();
                }
                fs::create_dir(temp.0.join("directory.rlib")).unwrap();
                assert!(!watch.changed().unwrap());
                match operation {
                    0 => fs::write(&artifact, b"new").unwrap(),
                    1 => fs::remove_file(&artifact).unwrap(),
                    2 => fs::rename(&artifact, temp.0.join("removed.tmp")).unwrap(),
                    _ => {
                        let source = temp.0.join("incoming.tmp");
                        fs::write(&source, b"replacement").unwrap();
                        fs::rename(source, &artifact).unwrap();
                    }
                }
                assert!(watch.changed().unwrap(), "{extension} operation {operation}");
            }
        }
    }

    #[test]
    fn watch_loss_and_directory_move_evict() {
        let temp = Temp::new();
        let directory = temp.0.join("deps");
        fs::create_dir(&directory).unwrap();
        let watch =
            InputWatch::new(std::iter::empty(), std::iter::once(directory.as_path())).unwrap();
        fs::rename(&directory, temp.0.join("moved")).unwrap();
        assert!(watch.changed().unwrap());
        let watch = InputWatch::new(std::iter::empty(), std::iter::once(temp.0.as_path())).unwrap();
        // This fixture installs exactly one watch, so iteration order cannot matter.
        #[allow(rustc::potential_query_instability)]
        let wd = *watch.watches.keys().next().unwrap();
        assert_eq!(unsafe { libc::inotify_rm_watch(watch.as_raw_fd(), wd) }, 0);
        assert!(watch.changed().unwrap());
    }

    #[test]
    fn descriptor_flags_and_setup_failure() {
        let temp = Temp::new();
        let watch = InputWatch::new(std::iter::empty(), std::iter::once(temp.0.as_path())).unwrap();
        assert_ne!(unsafe { libc::fcntl(watch.as_raw_fd(), libc::F_GETFL) } & libc::O_NONBLOCK, 0);
        assert_ne!(unsafe { libc::fcntl(watch.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
        assert!(
            InputWatch::new(std::iter::once(temp.0.join("missing").as_path()), std::iter::empty())
                .is_err()
        );
    }

    #[test]
    fn bounded_parser_rejects_truncation_overflow_and_unmount() {
        let watch = InputWatch::new(std::iter::empty(), std::iter::empty()).unwrap();
        assert!(!watch.events_changed(&[]));
        assert!(watch.events_changed(&[0]));
        for mask in [libc::IN_Q_OVERFLOW, libc::IN_UNMOUNT, libc::IN_IGNORED] {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&(-1i32).to_ne_bytes());
            bytes.extend_from_slice(&mask.to_ne_bytes());
            bytes.extend_from_slice(&0u32.to_ne_bytes());
            bytes.extend_from_slice(&0u32.to_ne_bytes());
            assert!(watch.events_changed(&bytes));
            bytes[12..16].copy_from_slice(&u32::MAX.to_ne_bytes());
            assert!(watch.events_changed(&bytes));
        }
    }
}
