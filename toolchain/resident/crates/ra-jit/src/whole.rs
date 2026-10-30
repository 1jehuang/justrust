//! The object with all of the program a daemon compiled (`ra-jit.o`). On Linux only a link
//! reads it, the executable having the debug information, so between links it is kept
//! zstd-compressed beside where it was (`ra-jit.o.zst`, a sixth of it) and inflated there again
//! for the next link. Where there is the object it is what counts; the packed one is of the
//! same object when both are there, as only [`read`] writes the object beside it.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard},
};

use crate::lock;

/// Held to pack an object, to inflate one, and to write one anew: each changes which of the two
/// files there is.
static CHANGING: Mutex<()> = Mutex::new(());
/// Held by a link for as long as it reads an object, which is not removed meanwhile.
static READING: RwLock<()> = RwLock::new(());

/// What is compressed at a time on each thread: what one thread does to a part does not wait
/// for the others, and the parts in memory are a few of these.
const PART: usize = 4 << 20;

fn packed(path: &Path) -> PathBuf {
    let mut packed = path.as_os_str().to_owned();
    packed.push(".zst");
    packed.into()
}

fn beside(path: &Path) -> PathBuf {
    let mut new = path.as_os_str().to_owned();
    new.push(format!(".{}.new", std::process::id()));
    new.into()
}

/// Whether the object at `path` is there, or what [`pack`] left of it.
pub(crate) fn is_there(path: &Path) -> bool {
    let _changing = lock(&CHANGING);
    path.exists() || packed(path).exists()
}

/// Has the object at `path` be there for as long as the guard is held, inflated from what
/// [`pack`] left of it when it is not.
pub(crate) fn read(path: &Path) -> std::io::Result<RwLockReadGuard<'static, ()>> {
    let _changing = lock(&CHANGING);
    let reading = READING.read().unwrap_or_else(PoisonError::into_inner);
    let zst = packed(path);
    if !path.exists() && zst.exists() {
        let new = beside(path);
        let mut out = File::create(&new)?;
        zstd::stream::copy_decode(File::open(&zst)?, &mut out)?;
        out.flush()?;
        std::fs::rename(&new, path)?;
    }
    Ok(reading)
}

/// To be held while an object is written at `path`: what [`pack`] left of the last one is
/// gone first.
pub(crate) fn writing(path: &Path) -> MutexGuard<'static, ()> {
    let changing = lock(&CHANGING);
    _ = std::fs::remove_file(packed(path));
    changing
}

/// Compresses the object at `path` beside it and removes it, unless a link reads it now; `true`
/// when it was removed. Only an ELF object: a debugger on macOS reads the debug information
/// from the object itself.
pub(crate) fn pack(path: &Path) -> std::io::Result<bool> {
    let _changing = lock(&CHANGING);
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut head = [0; 4];
    if file.read_exact(&mut head).is_err() || head != *b"\x7fELF" {
        return Ok(false);
    }
    let zst = packed(path);
    if !zst.exists() {
        let new = beside(&zst);
        compress(path, file.metadata()?.len(), &mut File::create(&new)?)?;
        std::fs::rename(&new, &zst)?;
    }
    match READING.try_write() {
        Ok(_none_reads) => std::fs::remove_file(path).map(|()| true),
        // The next link packs it.
        Err(_) => Ok(false),
    }
}

/// The `len` bytes of the file at `path` as zstd frames of a [`PART`] each, which read back as
/// one: as many parts at once as there are cores, up to 16.
fn compress(path: &Path, len: u64, out: &mut File) -> std::io::Result<()> {
    let threads = std::thread::available_parallelism().map_or(1, |it| it.get()).min(16) as u64;
    let parts = len.div_ceil(PART as u64);
    let mut first = 0;
    while first < parts {
        let wave = first..(first + threads).min(parts);
        let packed = std::thread::scope(|scope| {
            let each = wave.map(|part| {
                scope.spawn(move || {
                    let at = part * PART as u64;
                    let mut bytes = vec![0; (len - at).min(PART as u64) as usize];
                    let mut file = File::open(path)?;
                    file.seek(SeekFrom::Start(at))?;
                    file.read_exact(&mut bytes)?;
                    zstd::bulk::compress(&bytes, 1)
                })
            });
            let each: Vec<_> = each.collect();
            each.into_iter()
                .map(|it| it.join().unwrap_or_else(|_| Err(std::io::Error::other("panicked"))))
                .collect::<std::io::Result<Vec<_>>>()
        })?;
        for frame in packed {
            out.write_all(&frame)?;
        }
        first += threads;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests share the locks: one holding [`READING`] would keep another's object.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    /// A directory of the test's own, and the place of an object in it.
    struct Scratch {
        dir: PathBuf,
        _one: MutexGuard<'static, ()>,
    }

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let one = lock(&ONE_AT_A_TIME);
            let dir =
                std::env::temp_dir().join(format!("ra-jit-whole-{name}-{}", std::process::id()));
            _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch { dir, _one: one }
        }

        fn object(&self) -> PathBuf {
            self.dir.join("ra-jit.o")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn object(len: usize) -> Vec<u8> {
        let mut bytes = b"\x7fELF".to_vec();
        bytes.extend((0..len).map(|it| (it % 251) as u8 ^ (it / 4093) as u8));
        bytes
    }

    #[test]
    fn packed_reads_back_as_it_was() {
        let dir = Scratch::new("packed");
        let path = dir.object();
        // More than a wave of parts, and a last part that is not whole.
        let bytes = object(PART * 17 + 12345);
        std::fs::write(&path, &bytes).unwrap();
        assert!(pack(&path).unwrap());
        assert!(!path.exists());
        let zst = packed(&path);
        assert!(std::fs::metadata(&zst).unwrap().len() < bytes.len() as u64 / 4);
        drop(read(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // Both there are the same object: packing again only removes it.
        assert!(pack(&path).unwrap());
        assert!(!path.exists() && zst.exists());
    }

    #[test]
    fn written_anew_leaves_nothing_of_the_last() {
        let dir = Scratch::new("written");
        let path = dir.object();
        std::fs::write(&path, object(1000)).unwrap();
        assert!(pack(&path).unwrap());
        let again = object(2000);
        {
            let _writing = writing(&path);
            std::fs::write(&path, &again).unwrap();
        }
        assert!(!packed(&path).exists());
        drop(read(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), again);
    }

    #[test]
    fn only_elf_is_packed() {
        let dir = Scratch::new("elf");
        let path = dir.object();
        std::fs::write(&path, b"\xcf\xfa\xed\xfe and the rest of a Mach-O object").unwrap();
        assert!(!pack(&path).unwrap());
        assert!(path.exists() && !packed(&path).exists());
    }

    #[test]
    fn read_by_a_link_is_not_removed() {
        let dir = Scratch::new("read");
        let path = dir.object();
        std::fs::write(&path, object(1000)).unwrap();
        let reading = read(&path).unwrap();
        assert!(!pack(&path).unwrap());
        assert!(path.exists() && packed(&path).exists());
        drop(reading);
        assert!(pack(&path).unwrap());
        assert!(!path.exists());
    }
}
