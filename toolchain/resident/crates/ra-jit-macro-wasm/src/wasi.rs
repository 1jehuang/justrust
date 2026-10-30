//! The part of WASI's first snapshot (`wasi_snapshot_preview1`) a proc macro's module is
//! given: its package's directory to read, at [`PACKAGE`] and nothing else, its arguments and
//! environment, a clock that always says the same and random bytes that are always the same,
//! and stdout and stderr kept for its errors. Every read of the package is recorded with what
//! it found ([`Files::reads`]), for the second key of what the expansion made
//! ([`crate::keyed`]). A function this does not have answers `ENOSYS`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use wasmtime::{Caller, Extern, FuncType, Linker, Memory, Module, Val, ValType};

use crate::{State, keyed};

/// Where the module sees its package: what `CARGO_MANIFEST_DIR` says to it. Not where the
/// package is, so that what it makes of a path is the same on every machine.
pub(crate) const PACKAGE: &str = "/blaze/package";

/// Where the module is told `OUT_DIR` is: nothing is there for it.
pub(crate) const OUT_DIR: &str = "/blaze/out";

const MODULE: &str = "wasi_snapshot_preview1";

const SUCCESS: i32 = 0;
const EBADF: i32 = 8;
const EINVAL: i32 = 28;
const EISDIR: i32 = 31;
const ENOENT: i32 = 44;
const ENOSYS: i32 = 52;
const ENOTDIR: i32 = 54;
const EROFS: i32 = 69;
const ENOTCAPABLE: i32 = 76;

const FILETYPE_UNKNOWN: u8 = 0;
const FILETYPE_CHARACTER_DEVICE: u8 = 2;
const FILETYPE_DIRECTORY: u8 = 3;
const FILETYPE_REGULAR_FILE: u8 = 4;

const OFLAGS_CREAT: i32 = 1;
const OFLAGS_DIRECTORY: i32 = 2;
const OFLAGS_TRUNC: i32 = 8;
const RIGHTS_FD_WRITE: i64 = 1 << 6;

/// What one expansion may keep of what the module writes to stdout and stderr.
const KEPT_OUTPUT: usize = 64 << 10;

enum Open {
    Stdio,
    Dir { rel: String },
    File { bytes: Vec<u8>, at: u64 },
}

/// The module's files: the package, read-only, and what of it was read.
pub(crate) struct Files {
    root: PathBuf,
    fds: Vec<Option<Open>>,
    /// Each read of the package, as a read set has it (`keyed`), and what it found.
    pub(crate) reads: BTreeMap<String, String>,
    pub(crate) output: Vec<u8>,
    random: u64,
}

impl Files {
    /// The files of a module whose package is at `root`, or of one with no package to read
    /// when `None`.
    pub(crate) fn new(root: Option<&Path>) -> Files {
        let mut fds = vec![Some(Open::Stdio), Some(Open::Stdio), Some(Open::Stdio)];
        if root.is_some() {
            fds.push(Some(Open::Dir { rel: String::new() }));
        }
        Files {
            root: root.map(Path::to_path_buf).unwrap_or_default(),
            fds,
            reads: BTreeMap::new(),
            output: Vec::new(),
            random: 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn insert(&mut self, open: Open) -> u32 {
        match self.fds.iter().position(Option::is_none) {
            Some(at) => {
                self.fds[at] = Some(open);
                at as u32
            }
            None => {
                self.fds.push(Some(open));
                (self.fds.len() - 1) as u32
            }
        }
    }

    fn get(&mut self, fd: i32) -> Option<&mut Open> {
        self.fds.get_mut(usize::try_from(fd).ok()?)?.as_mut()
    }

    fn dir(&mut self, fd: i32) -> Result<String, i32> {
        match self.get(fd) {
            Some(Open::Dir { rel }) => Ok(rel.clone()),
            Some(_) => Err(ENOTDIR),
            None => Err(EBADF),
        }
    }

    /// The package's path of `path` from the directory `rel`, or why the module may not
    /// have it: it leaves the package, by `..` or by a link.
    fn resolve(&self, rel: &str, path: &str) -> Result<(String, PathBuf), i32> {
        let mut parts: Vec<&str> = rel.split('/').filter(|it| !it.is_empty()).collect();
        if path.starts_with('/') {
            return Err(ENOTCAPABLE);
        }
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => _ = parts.pop().ok_or(ENOTCAPABLE)?,
                part => parts.push(part),
            }
        }
        let rel = parts.join("/");
        let real = self.root.join(&rel);
        // A link may point anywhere: what it leads to must be in the package.
        if let Ok(found) = real.canonicalize() {
            let root = self.root.canonicalize().map_err(|_| ENOENT)?;
            if !found.starts_with(&root) {
                return Err(ENOTCAPABLE);
            }
        }
        Ok((rel, real))
    }

    #[cfg(test)]
    pub(crate) fn resolve_for_test(&self, rel: &str, path: &str) -> Result<String, i32> {
        self.resolve(rel, path).map(|(rel, _)| rel)
    }

    fn record(&mut self, read: String, found: String) {
        self.reads.entry(read).or_insert(found);
    }

    fn next_random(&mut self) -> u64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        self.random
    }
}

/// What a read set calls the read of the package's `rel`, the whole of what is there.
fn read_of(rel: &str) -> String {
    if rel.is_empty() { ".".to_owned() } else { rel.to_owned() }
}

fn memory(caller: &mut Caller<'_, State>) -> wasmtime::Result<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => Ok(memory),
        _ => Err(wasmtime::Error::msg("the module exports no memory")),
    }
}

fn bytes(mem: &[u8], ptr: i32, len: i32) -> Result<&[u8], i32> {
    let (ptr, len) = (ptr as u32 as usize, len as u32 as usize);
    mem.get(ptr..ptr.checked_add(len).ok_or(EINVAL)?).ok_or(EINVAL)
}

fn put(mem: &mut [u8], ptr: i32, value: &[u8]) -> Result<(), i32> {
    let ptr = ptr as u32 as usize;
    let room = mem.get_mut(ptr..ptr.checked_add(value.len()).ok_or(EINVAL)?).ok_or(EINVAL)?;
    room.copy_from_slice(value);
    Ok(())
}

fn text(mem: &[u8], ptr: i32, len: i32) -> Result<String, i32> {
    String::from_utf8(bytes(mem, ptr, len)?.to_vec()).map_err(|_| EINVAL)
}

/// The (pointer, length) pairs of an iovec array.
fn iovecs(mem: &[u8], iovs: i32, count: i32) -> Result<Vec<(i32, i32)>, i32> {
    let all = bytes(mem, iovs, count.checked_mul(8).ok_or(EINVAL)?)?;
    Ok(all
        .as_chunks::<8>()
        .0
        .iter()
        .map(|it| {
            let ptr = i32::from_le_bytes(it[..4].try_into().unwrap());
            let len = i32::from_le_bytes(it[4..].try_into().unwrap());
            (ptr, len)
        })
        .collect())
}

fn errno(result: Result<(), i32>) -> i32 {
    result.err().unwrap_or(SUCCESS)
}

/// A WASI filestat: device, inode, type, links, size and three times, all but the type and
/// size the same for every file.
fn filestat(ino: u64, filetype: u8, size: u64) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[8..16].copy_from_slice(&ino.to_le_bytes());
    out[16] = filetype;
    out[24..32].copy_from_slice(&1u64.to_le_bytes());
    out[32..40].copy_from_slice(&size.to_le_bytes());
    out
}

fn ino(rel: &str) -> u64 {
    let digest = keyed::sha256(rel.as_bytes());
    u64::from_le_bytes(digest[..8].try_into().unwrap()) | 1
}

/// What the package has at `real`, as the module is told and as its read is recorded: a
/// file's type and size are the file's, so a look at one is a read of all of it.
fn stat(files: &mut Files, rel: &str, real: &Path) -> Result<[u8; 64], i32> {
    match std::fs::metadata(real) {
        Ok(meta) if meta.is_dir() => {
            files.record(keyed::kind(&read_of(rel)), "d".to_owned());
            Ok(filestat(ino(rel), FILETYPE_DIRECTORY, 0))
        }
        Ok(meta) => {
            files.record(read_of(rel), keyed::digest_of(real));
            let filetype = if meta.is_file() { FILETYPE_REGULAR_FILE } else { FILETYPE_UNKNOWN };
            Ok(filestat(ino(rel), filetype, meta.len()))
        }
        Err(_) => {
            files.record(keyed::kind(&read_of(rel)), "-".to_owned());
            Err(ENOENT)
        }
    }
}

/// Adds to `linker` the functions of WASI's first snapshot that a proc macro has.
pub(crate) fn add(linker: &mut Linker<State>) -> wasmtime::Result<()> {
    linker.func_wrap(
        MODULE,
        "args_sizes_get",
        |mut caller: Caller<'_, State>, argc: i32, size: i32| {
            let memory = memory(&mut caller)?;
            let mem = memory.data_mut(&mut caller);
            let arg = b"proc-macro\0";
            Ok(errno(put(mem, argc, &1u32.to_le_bytes()).and(put(
                mem,
                size,
                &(arg.len() as u32).to_le_bytes(),
            ))))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "args_get",
        |mut caller: Caller<'_, State>, argv: i32, buf: i32| {
            let memory = memory(&mut caller)?;
            let mem = memory.data_mut(&mut caller);
            Ok(errno(put(mem, argv, &buf.to_le_bytes()).and(put(mem, buf, b"proc-macro\0"))))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "environ_sizes_get",
        |mut caller: Caller<'_, State>, count: i32, size: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let total: usize = state.env.iter().map(|(k, v)| k.len() + v.len() + 2).sum();
            let counted = put(mem, count, &(state.env.len() as u32).to_le_bytes());
            Ok(errno(counted.and(put(mem, size, &(total as u32).to_le_bytes()))))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "environ_get",
        |mut caller: Caller<'_, State>, environ: i32, buf: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let mut at = buf;
            for (index, (key, value)) in state.env.iter().enumerate() {
                let entry = format!("{key}={value}\0");
                if let Err(e) = put(mem, environ + 4 * index as i32, &at.to_le_bytes()).and(put(
                    mem,
                    at,
                    entry.as_bytes(),
                )) {
                    return Ok(e);
                }
                at += entry.len() as i32;
            }
            Ok(SUCCESS)
        },
    )?;
    linker.func_wrap(
        MODULE,
        "clock_res_get",
        |mut caller: Caller<'_, State>, _id: i32, out: i32| {
            let memory = memory(&mut caller)?;
            Ok(errno(put(memory.data_mut(&mut caller), out, &1u64.to_le_bytes())))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "clock_time_get",
        |mut caller: Caller<'_, State>, _id: i32, _precision: i64, out: i32| {
            let memory = memory(&mut caller)?;
            Ok(errno(put(memory.data_mut(&mut caller), out, &0u64.to_le_bytes())))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "random_get",
        |mut caller: Caller<'_, State>, buf: i32, len: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let mut random = Vec::with_capacity(len as u32 as usize);
            while random.len() < len as u32 as usize {
                random.extend_from_slice(&state.files.next_random().to_le_bytes());
            }
            random.truncate(len as u32 as usize);
            Ok(errno(put(mem, buf, &random)))
        },
    )?;
    linker.func_wrap(MODULE, "sched_yield", || SUCCESS)?;
    linker.func_wrap(MODULE, "proc_exit", |code: i32| -> wasmtime::Result<()> {
        Err(wasmtime::Error::msg(format!("the proc macro exited the process, with status {code}")))
    })?;
    linker.func_wrap(
        MODULE,
        "fd_write",
        |mut caller: Caller<'_, State>, fd: i32, iovs: i32, count: i32, written: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            if !matches!(state.files.get(fd), Some(Open::Stdio)) {
                return Ok(EBADF);
            }
            let vecs = match iovecs(mem, iovs, count) {
                Ok(it) => it,
                Err(e) => return Ok(e),
            };
            let mut total = 0u32;
            for (ptr, len) in vecs {
                let Ok(chunk) = bytes(mem, ptr, len) else { return Ok(EINVAL) };
                let room = KEPT_OUTPUT.saturating_sub(state.files.output.len());
                state.files.output.extend_from_slice(&chunk[..chunk.len().min(room)]);
                total += len as u32;
            }
            Ok(errno(put(mem, written, &total.to_le_bytes())))
        },
    )?;
    fn read(
        state: &mut State,
        mem: &mut [u8],
        fd: i32,
        iovs: i32,
        count: i32,
        at: Option<u64>,
    ) -> Result<u32, i32> {
        let vecs = iovecs(mem, iovs, count)?;
        let (bytes, pos) = match state.files.get(fd) {
            Some(Open::File { bytes, at: pos }) => (bytes, pos),
            Some(Open::Stdio) => return Ok(0),
            Some(Open::Dir { .. }) => return Err(EISDIR),
            None => return Err(EBADF),
        };
        let mut from = at.unwrap_or(*pos) as usize;
        let mut total = 0u32;
        for (ptr, len) in vecs {
            let chunk =
                &bytes[from.min(bytes.len())..(from + len as u32 as usize).min(bytes.len())];
            put(mem, ptr, chunk)?;
            from += chunk.len();
            total += chunk.len() as u32;
            if chunk.len() < len as u32 as usize {
                break;
            }
        }
        if at.is_none() {
            *pos = from as u64;
        }
        Ok(total)
    }
    linker.func_wrap(
        MODULE,
        "fd_read",
        |mut caller: Caller<'_, State>, fd: i32, iovs: i32, count: i32, out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            Ok(match read(state, mem, fd, iovs, count, None) {
                Ok(n) => errno(put(mem, out, &n.to_le_bytes())),
                Err(e) => e,
            })
        },
    )?;
    linker.func_wrap(
        MODULE,
        "fd_pread",
        |mut caller: Caller<'_, State>, fd: i32, iovs: i32, count: i32, at: i64, out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            Ok(match read(state, mem, fd, iovs, count, Some(at as u64)) {
                Ok(n) => errno(put(mem, out, &n.to_le_bytes())),
                Err(e) => e,
            })
        },
    )?;
    linker.func_wrap(MODULE, "fd_close", |mut caller: Caller<'_, State>, fd: i32| {
        let files = &mut caller.data_mut().files;
        // The package's directory stays: closing it would leave the module nothing to open.
        if fd <= 3 || files.get(fd).is_none() {
            return if files.get(fd).is_some() { SUCCESS } else { EBADF };
        }
        files.fds[fd as usize] = None;
        SUCCESS
    })?;
    linker.func_wrap(
        MODULE,
        "fd_seek",
        |mut caller: Caller<'_, State>, fd: i32, offset: i64, whence: i32, out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let Some(Open::File { bytes, at }) = state.files.get(fd) else { return Ok(EBADF) };
            let base = match whence {
                0 => 0,
                1 => *at as i64,
                2 => bytes.len() as i64,
                _ => return Ok(EINVAL),
            };
            let Some(new) = base.checked_add(offset).filter(|it| *it >= 0) else {
                return Ok(EINVAL);
            };
            *at = new as u64;
            Ok(errno(put(mem, out, &(new as u64).to_le_bytes())))
        },
    )?;
    linker.func_wrap(MODULE, "fd_tell", |mut caller: Caller<'_, State>, fd: i32, out: i32| {
        let memory = memory(&mut caller)?;
        let (mem, state) = memory.data_and_store_mut(&mut caller);
        let Some(Open::File { at, .. }) = state.files.get(fd) else { return Ok(EBADF) };
        let at = *at;
        Ok(errno(put(mem, out, &at.to_le_bytes())))
    })?;
    linker.func_wrap(
        MODULE,
        "fd_fdstat_get",
        |mut caller: Caller<'_, State>, fd: i32, out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let filetype = match state.files.get(fd) {
                Some(Open::Stdio) => FILETYPE_CHARACTER_DEVICE,
                Some(Open::Dir { .. }) => FILETYPE_DIRECTORY,
                Some(Open::File { .. }) => FILETYPE_REGULAR_FILE,
                None => return Ok(EBADF),
            };
            let mut stat = [0u8; 24];
            stat[0] = filetype;
            stat[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
            stat[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
            Ok(errno(put(mem, out, &stat)))
        },
    )?;
    linker.func_wrap(MODULE, "fd_fdstat_set_flags", |_: i32, _: i32| ENOSYS)?;
    linker.func_wrap(
        MODULE,
        "fd_filestat_get",
        |mut caller: Caller<'_, State>, fd: i32, out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let stat = match state.files.get(fd) {
                Some(Open::Stdio) => filestat(0, FILETYPE_CHARACTER_DEVICE, 0),
                Some(Open::File { bytes, .. }) => {
                    filestat(1, FILETYPE_REGULAR_FILE, bytes.len() as u64)
                }
                Some(Open::Dir { rel }) => filestat(ino(rel), FILETYPE_DIRECTORY, 0),
                None => return Ok(EBADF),
            };
            Ok(errno(put(mem, out, &stat)))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "fd_prestat_get",
        |mut caller: Caller<'_, State>, fd: i32, out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            if fd != 3 || state.files.fds.len() < 4 {
                return Ok(EBADF);
            }
            let mut prestat = [0u8; 8];
            prestat[4..].copy_from_slice(&(PACKAGE.len() as u32).to_le_bytes());
            Ok(errno(put(mem, out, &prestat)))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "fd_prestat_dir_name",
        |mut caller: Caller<'_, State>, fd: i32, ptr: i32, len: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            if fd != 3 || state.files.fds.len() < 4 {
                return Ok(EBADF);
            }
            let name = &PACKAGE.as_bytes()[..PACKAGE.len().min(len as u32 as usize)];
            Ok(errno(put(mem, ptr, name)))
        },
    )?;
    linker.func_wrap(
        MODULE,
        "path_open",
        |mut caller: Caller<'_, State>,
         dirfd: i32,
         _dirflags: i32,
         ptr: i32,
         len: i32,
         oflags: i32,
         rights: i64,
         _inheriting: i64,
         _fdflags: i32,
         out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let files = &mut state.files;
            let opened = (|| {
                let path = text(mem, ptr, len)?;
                let dir = files.dir(dirfd)?;
                let (rel, real) = files.resolve(&dir, &path)?;
                if oflags & (OFLAGS_CREAT | OFLAGS_TRUNC) != 0 || rights & RIGHTS_FD_WRITE != 0 {
                    return Err(EROFS);
                }
                let Ok(meta) = std::fs::metadata(&real) else {
                    files.record(read_of(&rel), "-".to_owned());
                    return Err(ENOENT);
                };
                if meta.is_dir() {
                    files.record(keyed::kind(&read_of(&rel)), "d".to_owned());
                    return Ok(files.insert(Open::Dir { rel }));
                }
                if oflags & OFLAGS_DIRECTORY != 0 {
                    files.record(keyed::kind(&read_of(&rel)), "o".to_owned());
                    return Err(ENOTDIR);
                }
                let bytes = std::fs::read(&real).map_err(|_| ENOENT)?;
                files.record(read_of(&rel), keyed::digest_bytes(&bytes));
                Ok(files.insert(Open::File { bytes, at: 0 }))
            })();
            Ok(match opened {
                Ok(fd) => errno(put(mem, out, &fd.to_le_bytes())),
                Err(e) => e,
            })
        },
    )?;
    linker.func_wrap(
        MODULE,
        "path_filestat_get",
        |mut caller: Caller<'_, State>, dirfd: i32, _flags: i32, ptr: i32, len: i32, out: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let files = &mut state.files;
            let found = (|| {
                let path = text(mem, ptr, len)?;
                let dir = files.dir(dirfd)?;
                let (rel, real) = files.resolve(&dir, &path)?;
                stat(files, &rel, &real)
            })();
            Ok(match found {
                Ok(stat) => errno(put(mem, out, &stat)),
                Err(e) => e,
            })
        },
    )?;
    linker.func_wrap(
        MODULE,
        "path_readlink",
        |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32| EINVAL,
    )?;
    linker.func_wrap(
        MODULE,
        "fd_readdir",
        |mut caller: Caller<'_, State>, fd: i32, buf: i32, len: i32, cookie: i64, used: i32| {
            let memory = memory(&mut caller)?;
            let (mem, state) = memory.data_and_store_mut(&mut caller);
            let files = &mut state.files;
            let rel = match files.dir(fd) {
                Ok(it) => it,
                Err(e) => return Ok(e),
            };
            let real = files.root.join(&rel);
            files.record(read_of(&rel), keyed::digest_of(&real));
            let mut names: Vec<_> = std::fs::read_dir(&real)
                .into_iter()
                .flatten()
                .filter_map(|it| it.ok().and_then(|it| it.file_name().into_string().ok()))
                .collect();
            names.sort();
            let mut out = Vec::new();
            for (index, name) in names.iter().enumerate().skip(cookie as u64 as usize) {
                let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                let filetype = match std::fs::metadata(files.root.join(&child)) {
                    Ok(meta) if meta.is_dir() => FILETYPE_DIRECTORY,
                    Ok(meta) if meta.is_file() => FILETYPE_REGULAR_FILE,
                    _ => FILETYPE_UNKNOWN,
                };
                out.extend_from_slice(&(index as u64 + 1).to_le_bytes());
                out.extend_from_slice(&ino(&child).to_le_bytes());
                out.extend_from_slice(&(name.len() as u32).to_le_bytes());
                out.extend_from_slice(&[filetype, 0, 0, 0]);
                out.extend_from_slice(name.as_bytes());
                if out.len() >= len as u32 as usize {
                    break;
                }
            }
            out.truncate(len as u32 as usize);
            Ok(errno(put(mem, buf, &out).and(put(mem, used, &(out.len() as u32).to_le_bytes()))))
        },
    )?;
    Ok(())
}

/// Adds to `linker`, for each function of WASI's that `module` imports and [`add`] does not
/// have, one that answers `ENOSYS`.
pub(crate) fn rest(linker: &mut Linker<State>, module: &Module) -> wasmtime::Result<()> {
    let mut store = wasmtime::Store::new(module.engine(), State::empty());
    for import in module.imports() {
        if import.module() != MODULE {
            continue;
        }
        let Some(ty) = import.ty().func().cloned() else { continue };
        if linker.get_by_import(&mut store, &import).is_some() {
            continue;
        }
        let results: Vec<ValType> = ty.results().collect();
        let ty = FuncType::new(module.engine(), ty.params(), results.clone());
        linker.func_new(MODULE, import.name(), ty, move |_, _, out| {
            for (slot, ty) in out.iter_mut().zip(&results) {
                *slot = match ty {
                    ValType::I64 => Val::I64(ENOSYS as i64),
                    _ => Val::I32(ENOSYS),
                };
            }
            Ok(())
        })?;
    }
    Ok(())
}
