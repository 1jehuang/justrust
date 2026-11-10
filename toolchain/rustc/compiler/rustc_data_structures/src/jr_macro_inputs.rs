//! justrust: observe the external inputs of proc-macro expansions so cached
//! expansions (`-Zcache-proc-macros`) are invalidated when an environment
//! variable or file a macro read changes.
//!
//! The compiler driver dylib exports libc entry points (`getenv`, `open64`,
//! `statx`, `opendir`, ...). The dynamic linker binds proc-macro shared
//! objects to these definitions because the driver precedes libc in the
//! global lookup scope. Outside a recording they are pure pass-throughs
//! (raw syscalls, or the next definition via `dlsym(RTLD_NEXT)`). While the
//! current thread records an expansion, each call also logs its input.
//!
//! Anything the recorder cannot model soundly marks the expansion impure, and
//! impure expansions are never reused: writes, process spawning, thread
//! creation, sockets, wall-clock reads, `/proc`/`/sys`/`/dev` reads, or an
//! unsupported platform. Known gap: iterating `environ` directly
//! (`std::env::vars`) is not observable.
//!
//! The inputs of all expansions are persisted next to the incremental dep
//! graph. The next session validates every recorded input once (the
//! `jr_proc_macro_inputs` query). Any difference turns the query red, which
//! re-runs every cached expansion that depends on it.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::fx::FxIndexMap;
use crate::stable_hash::StableHasher;

pub const KIND_ENV: u8 = 0;
pub const KIND_CONTENT: u8 = 1;
pub const KIND_META: u8 = 2;
pub const KIND_DIR: u8 = 3;
pub const KIND_REAL: u8 = 4;

pub const SIDECAR: &str = "jr-macro-inputs.bin";

#[derive(Default)]
struct Recording {
    inputs: Vec<(u8, Vec<u8>)>,
    impure: Option<&'static str>,
}

thread_local! {
    static ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static REC: std::cell::RefCell<Recording> = const {
        std::cell::RefCell::new(Recording { inputs: Vec::new(), impure: None })
    };
}

/// Inputs of this session (and, after a validated load, of the previous
/// session), with the value observed for each.
static SESSION: Mutex<Option<FxIndexMap<(u8, Vec<u8>), u128>>> = Mutex::new(None);

/// The value of `jr_proc_macro_inputs` in this session (the input epoch).
/// It is persisted and reused while every recorded input is unchanged.
static EPOCH: Mutex<u128> = Mutex::new(0);

pub fn set_epoch(e: u128) {
    *EPOCH.lock().unwrap_or_else(|e| e.into_inner()) = e;
}

pub fn supported() -> bool {
    cfg!(all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"))
}

/// Start recording the current thread.
pub fn begin() {
    REC.with(|r| *r.borrow_mut() = Recording::default());
    if !supported() {
        mark_impure("unsupported platform");
    }
    ACTIVE.with(|a| a.set(true));
}

pub struct Outcome {
    pub impure: Option<&'static str>,
}

/// Stop recording, fingerprint what was read, and merge it into the session.
pub fn end() -> Outcome {
    ACTIVE.with(|a| a.set(false));
    let rec = REC.with(|r| std::mem::take(&mut *r.borrow_mut()));
    if rec.impure.is_some() {
        return Outcome { impure: rec.impure };
    }
    let mut computed = Vec::with_capacity(rec.inputs.len());
    for (kind, key) in rec.inputs {
        let value = current_value(kind, &key);
        computed.push(((kind, key), value));
    }
    let mut session = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let map = session.get_or_insert_with(Default::default);
    for (k, v) in computed {
        map.insert(k, v);
    }
    Outcome { impure: None }
}

pub fn mark_impure(why: &'static str) {
    REC.with(|r| {
        if let Ok(mut r) = r.try_borrow_mut() {
            r.impure.get_or_insert(why);
        }
    });
}

#[inline]
fn recording() -> bool {
    ACTIVE.with(|a| a.get()) && !BUSY.with(|b| b.get())
}

fn with_busy<R>(f: impl FnOnce() -> R) -> R {
    BUSY.with(|b| b.set(true));
    let r = f();
    BUSY.with(|b| b.set(false));
    r
}

fn record(kind: u8, key: Vec<u8>) {
    with_busy(|| {
        if kind != KIND_ENV {
            if key.starts_with(b"/proc/") || key.starts_with(b"/sys/") || key.starts_with(b"/dev/")
            {
                mark_impure("read of /proc, /sys or /dev");
                return;
            }
        }
        REC.with(|r| {
            if let Ok(mut r) = r.try_borrow_mut() {
                r.inputs.push((kind, key));
            }
        });
    })
}

fn hash_bytes(parts: &[&[u8]]) -> u128 {
    let mut h = StableHasher::new();
    for p in parts {
        std::hash::Hasher::write_usize(&mut h, p.len());
        std::hash::Hasher::write(&mut h, p);
    }
    h.finish::<crate::fingerprint::Fingerprint>().as_u128()
}

/// The current value of an input. Never panics; errors are part of the value.
fn current_value(kind: u8, key: &[u8]) -> u128 {
    with_busy(|| {
        let path = Path::new(OsStr::from_bytes(key));
        match kind {
            KIND_ENV => match std::env::var_os(OsStr::from_bytes(key)) {
                Some(v) => hash_bytes(&[b"some", v.as_bytes()]),
                None => hash_bytes(&[b"none"]),
            },
            KIND_CONTENT => {
                let mut buf = Vec::new();
                match std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut buf)) {
                    Ok(_) => hash_bytes(&[b"ok", &buf]),
                    Err(e) => hash_bytes(&[b"err", format!("{:?}", e.kind()).as_bytes()]),
                }
            }
            KIND_META => meta_value(path),
            KIND_DIR => match std::fs::read_dir(path) {
                Ok(rd) => {
                    let mut names: Vec<Vec<u8>> =
                        rd.filter_map(|e| e.ok()).map(|e| e.file_name().as_bytes().to_vec()).collect();
                    names.sort();
                    let mut parts: Vec<&[u8]> = vec![b"ok"];
                    parts.extend(names.iter().map(|n| n.as_slice()));
                    hash_bytes(&parts)
                }
                Err(e) => hash_bytes(&[b"err", format!("{:?}", e.kind()).as_bytes()]),
            },
            KIND_REAL => match std::fs::canonicalize(path) {
                Ok(p) => hash_bytes(&[b"ok", p.as_os_str().as_bytes(), &meta_value(path).to_le_bytes()]),
                Err(e) => hash_bytes(&[b"err", format!("{:?}", e.kind()).as_bytes()]),
            },
            _ => hash_bytes(&[b"unknown-kind"]),
        }
    })
}

fn meta_value(path: &Path) -> u128 {
    use std::os::unix::fs::MetadataExt;
    let one = |m: std::io::Result<std::fs::Metadata>| -> Vec<u8> {
        match m {
            Ok(m) => {
                let mut v = Vec::with_capacity(48);
                for x in [m.mode() as u64, m.size(), m.mtime() as u64, m.mtime_nsec() as u64, m.ino()]
                {
                    v.extend_from_slice(&x.to_le_bytes());
                }
                v
            }
            Err(e) => format!("err {:?}", e.kind()).into_bytes(),
        }
    };
    let follow = one(std::fs::metadata(path));
    let nofollow = one(std::fs::symlink_metadata(path));
    hash_bytes(&[&follow, &nofollow])
}

/// Validate the previous session's inputs. Returns a hash of the input set if
/// every input still has its recorded value; the inputs are then carried
/// forward into this session.
pub fn load_and_validate(sidecar: &Path) -> Option<u128> {
    let bytes = std::fs::read(sidecar).ok()?;
    let (epoch, entries) = decode(&bytes)?;
    for ((kind, key), value) in &entries {
        if current_value(*kind, key) != *value {
            return None;
        }
    }
    let digest = epoch;
    let mut session = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let map = session.get_or_insert_with(Default::default);
    for (k, v) in entries {
        map.entry(k).or_insert(v);
    }
    Some(digest)
}

fn decode(bytes: &[u8]) -> Option<(u128, Vec<((u8, Vec<u8>), u128)>)> {
    let mut rest = bytes.strip_prefix(b"jrmi2\n")?;
    let mut take = |n: usize| -> Option<&[u8]> {
        if rest.len() < n {
            return None;
        }
        let (a, b) = rest.split_at(n);
        rest = b;
        Some(a)
    };
    let epoch = u128::from_le_bytes(take(16)?.try_into().ok()?);
    let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let kind = take(1)?[0];
        let len = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let key = take(len)?.to_vec();
        let value = u128::from_le_bytes(take(16)?.try_into().ok()?);
        out.push(((kind, key), value));
    }
    rest.is_empty().then_some((epoch, out))
}

/// Persist this session's inputs (atomically; the session directory may hold
/// hard links into the previous session).
pub fn save(sidecar: &Path) {
    with_busy(|| {
        let session = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = b"jrmi2\n".to_vec();
        buf.extend_from_slice(&EPOCH.lock().unwrap_or_else(|e| e.into_inner()).to_le_bytes());
        let empty = FxIndexMap::default();
        let map = session.as_ref().unwrap_or(&empty);
        buf.extend_from_slice(&(map.len() as u32).to_le_bytes());
        for ((kind, key), value) in map {
            buf.push(*kind);
            buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
            buf.extend_from_slice(key);
            buf.extend_from_slice(&value.to_le_bytes());
        }
        let tmp: PathBuf = sidecar.with_extension("tmp");
        let ok = std::fs::File::create(&tmp).and_then(|mut f| f.write_all(&buf)).is_ok();
        if ok {
            let _ = std::fs::rename(&tmp, sidecar);
        } else {
            let _ = std::fs::remove_file(sidecar);
        }
    })
}

#[cfg(all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"))]
// `open` is variadic in C. On x86-64 SysV a fixed third integer argument is
// passed identically, and callers always pass `mode` when it matters.
#[allow(invalid_runtime_symbol_definitions)]
pub mod interpose {
    use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
    use std::sync::atomic::{AtomicPtr, Ordering};

    use super::*;

    unsafe extern "C" {
        static mut environ: *const *const c_char;
    }

    const AT_FDCWD: c_int = -100;
    const AT_SYMLINK_NOFOLLOW: c_int = 0x100;
    const AT_EMPTY_PATH: c_int = 0x1000;
    const O_ACCMODE: c_int = 0o3;
    const O_CREAT: c_int = 0o100;
    const O_TRUNC: c_int = 0o1000;
    const O_DIRECTORY: c_int = 0o200000;
    const O_TMPFILE: c_int = 0o20000000 | O_DIRECTORY;
    const O_PATH: c_int = 0o10000000;

    fn cpath(p: *const c_char) -> Option<Vec<u8>> {
        if p.is_null() {
            return None;
        }
        Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }

    /// Absolute form of `path` relative to `dirfd`.
    fn absolute(dirfd: c_int, path: Vec<u8>) -> Vec<u8> {
        if path.starts_with(b"/") {
            return path;
        }
        let base = if dirfd == AT_FDCWD {
            std::env::current_dir().ok().map(|p| p.into_os_string().into_vec())
        } else {
            std::fs::read_link(format!("/proc/self/fd/{dirfd}"))
                .ok()
                .map(|p| p.into_os_string().into_vec())
        };
        match base {
            Some(mut b) => {
                if !path.is_empty() {
                    b.push(b'/');
                    b.extend_from_slice(&path);
                }
                b
            }
            None => {
                mark_impure("unresolvable relative path");
                path
            }
        }
    }
    use std::os::unix::ffi::OsStringExt;

    fn note_path(kind: u8, dirfd: c_int, p: *const c_char) {
        if !recording() {
            return;
        }
        with_busy(|| match cpath(p) {
            Some(path) => {
                let abs = absolute(dirfd, path);
                BUSY.with(|b| b.set(false));
                record(kind, abs);
                BUSY.with(|b| b.set(true));
            }
            None => {}
        })
    }

    fn note_open(dirfd: c_int, p: *const c_char, flags: c_int) {
        if !recording() {
            return;
        }
        if flags & O_ACCMODE != 0 || flags & (O_CREAT | O_TRUNC) != 0 || flags & O_TMPFILE == O_TMPFILE
        {
            mark_impure("file opened for writing");
            return;
        }
        let kind = if flags & O_DIRECTORY != 0 {
            KIND_DIR
        } else if flags & O_PATH != 0 {
            KIND_META
        } else {
            KIND_CONTENT
        };
        note_path(kind, dirfd, p);
        // An open also observes existence and type.
        note_path(KIND_META, dirfd, p);
    }

    fn impure(why: &'static str) {
        if recording() {
            mark_impure(why);
        }
    }

    unsafe fn sys(n: c_long, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> c_long {
        unsafe extern "C" {
            fn syscall(n: c_long, ...) -> c_long;
        }
        unsafe { syscall(n, a, b, c, d, e, f) }
    }
    const SYS_OPENAT: c_long = 257;
    const SYS_NEWFSTATAT: c_long = 262;
    const SYS_READLINKAT: c_long = 267;
    const SYS_FACCESSAT: c_long = 269;
    const SYS_STATX: c_long = 332;
    const SYS_FACCESSAT2: c_long = 439;
    const SYS_CLOCK_GETTIME: c_long = 228;

    /// Lazily resolved next definition, with a re-entrancy guard (dlsym may
    /// allocate, and the allocator may call back into an interposed function).
    struct Next {
        name: &'static CStr,
        ptr: AtomicPtr<c_void>,
    }
    thread_local! {
        static RESOLVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    impl Next {
        const fn new(name: &'static CStr) -> Self {
            Next { name, ptr: AtomicPtr::new(std::ptr::null_mut()) }
        }
        fn get(&self) -> *mut c_void {
            let p = self.ptr.load(Ordering::Acquire);
            if !p.is_null() {
                return p;
            }
            if RESOLVING.with(|r| r.replace(true)) {
                return std::ptr::null_mut();
            }
            unsafe extern "C" {
                fn dlsym(h: *mut c_void, s: *const c_char) -> *mut c_void;
            }
            const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;
            let p = unsafe { dlsym(RTLD_NEXT, self.name.as_ptr()) };
            RESOLVING.with(|r| r.set(false));
            self.ptr.store(p, Ordering::Release);
            p
        }
    }

    unsafe fn env_lookup(name: *const c_char) -> *mut c_char {
        if name.is_null() {
            return std::ptr::null_mut();
        }
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        if name.is_empty() || name.contains(&b'=') {
            return std::ptr::null_mut();
        }
        let mut e = unsafe { environ };
        if e.is_null() {
            return std::ptr::null_mut();
        }
        unsafe {
            while !(*e).is_null() {
                let entry = CStr::from_ptr(*e).to_bytes();
                if entry.len() > name.len() && entry.starts_with(name) && entry[name.len()] == b'='
                {
                    return (*e).add(name.len() + 1) as *mut c_char;
                }
                e = e.add(1);
            }
        }
        std::ptr::null_mut()
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn getenv(name: *const c_char) -> *mut c_char {
        if recording() && let Some(n) = cpath(name) {
            record(KIND_ENV, n);
        }
        unsafe { env_lookup(name) }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn secure_getenv(name: *const c_char) -> *mut c_char {
        static NEXT: Next = Next::new(c"secure_getenv");
        if recording() && let Some(n) = cpath(name) {
            record(KIND_ENV, n);
        }
        let f = NEXT.get();
        if f.is_null() {
            return unsafe { env_lookup(name) };
        }
        let f: unsafe extern "C" fn(*const c_char) -> *mut c_char = unsafe { std::mem::transmute(f) };
        unsafe { f(name) }
    }

    unsafe fn do_openat(dirfd: c_int, p: *const c_char, flags: c_int, mode: c_uint) -> c_int {
        note_open(dirfd, p, flags);
        unsafe { sys(SYS_OPENAT, dirfd as usize, p as usize, flags as usize, mode as usize, 0, 0) as c_int }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn open(p: *const c_char, flags: c_int, mode: c_uint) -> c_int {
        unsafe { do_openat(AT_FDCWD, p, flags, mode) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn open64(p: *const c_char, flags: c_int, mode: c_uint) -> c_int {
        unsafe { do_openat(AT_FDCWD, p, flags, mode) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn openat(d: c_int, p: *const c_char, flags: c_int, mode: c_uint) -> c_int {
        unsafe { do_openat(d, p, flags, mode) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn openat64(d: c_int, p: *const c_char, flags: c_int, mode: c_uint) -> c_int {
        unsafe { do_openat(d, p, flags, mode) }
    }

    unsafe fn do_fstatat(d: c_int, p: *const c_char, buf: *mut c_void, flags: c_int) -> c_int {
        if !(flags & AT_EMPTY_PATH != 0 && !p.is_null() && unsafe { *p } == 0) {
            note_path(KIND_META, d, p);
        }
        unsafe { sys(SYS_NEWFSTATAT, d as usize, p as usize, buf as usize, flags as usize, 0, 0) as c_int }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn stat(p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, 0) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn stat64(p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, 0) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn lstat(p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, AT_SYMLINK_NOFOLLOW) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn lstat64(p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, AT_SYMLINK_NOFOLLOW) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fstatat(d: c_int, p: *const c_char, b: *mut c_void, f: c_int) -> c_int {
        unsafe { do_fstatat(d, p, b, f) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fstatat64(d: c_int, p: *const c_char, b: *mut c_void, f: c_int) -> c_int {
        unsafe { do_fstatat(d, p, b, f) }
    }
    // Pre-2.33 glibc entry points used by objects built against older glibc.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __xstat64(_v: c_int, p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, 0) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __xstat(_v: c_int, p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, 0) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __lxstat64(_v: c_int, p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, AT_SYMLINK_NOFOLLOW) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __lxstat(_v: c_int, p: *const c_char, b: *mut c_void) -> c_int {
        unsafe { do_fstatat(AT_FDCWD, p, b, AT_SYMLINK_NOFOLLOW) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __fxstatat64(
        _v: c_int,
        d: c_int,
        p: *const c_char,
        b: *mut c_void,
        f: c_int,
    ) -> c_int {
        unsafe { do_fstatat(d, p, b, f) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn statx(
        d: c_int,
        p: *const c_char,
        f: c_int,
        mask: c_uint,
        b: *mut c_void,
    ) -> c_int {
        if !(f & AT_EMPTY_PATH != 0 && !p.is_null() && unsafe { *p } == 0) {
            note_path(KIND_META, d, p);
        }
        unsafe { sys(SYS_STATX, d as usize, p as usize, f as usize, mask as usize, b as usize, 0) as c_int }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn access(p: *const c_char, m: c_int) -> c_int {
        note_path(KIND_META, AT_FDCWD, p);
        unsafe { sys(SYS_FACCESSAT, AT_FDCWD as usize, p as usize, m as usize, 0, 0, 0) as c_int }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn faccessat(d: c_int, p: *const c_char, m: c_int, f: c_int) -> c_int {
        note_path(KIND_META, d, p);
        if f == 0 {
            unsafe { sys(SYS_FACCESSAT, d as usize, p as usize, m as usize, 0, 0, 0) as c_int }
        } else {
            unsafe { sys(SYS_FACCESSAT2, d as usize, p as usize, m as usize, f as usize, 0, 0) as c_int }
        }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn readlink(p: *const c_char, b: *mut c_char, n: usize) -> isize {
        note_path(KIND_META, AT_FDCWD, p);
        unsafe { sys(SYS_READLINKAT, AT_FDCWD as usize, p as usize, b as usize, n, 0, 0) as isize }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn readlinkat(d: c_int, p: *const c_char, b: *mut c_char, n: usize) -> isize {
        note_path(KIND_META, d, p);
        unsafe { sys(SYS_READLINKAT, d as usize, p as usize, b as usize, n, 0, 0) as isize }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn opendir(p: *const c_char) -> *mut c_void {
        static NEXT: Next = Next::new(c"opendir");
        note_path(KIND_DIR, AT_FDCWD, p);
        let f: unsafe extern "C" fn(*const c_char) -> *mut c_void =
            unsafe { std::mem::transmute(NEXT.get()) };
        unsafe { f(p) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn realpath(p: *const c_char, r: *mut c_char) -> *mut c_char {
        static NEXT: Next = Next::new(c"realpath");
        note_path(KIND_REAL, AT_FDCWD, p);
        let f: unsafe extern "C" fn(*const c_char, *mut c_char) -> *mut c_char =
            unsafe { std::mem::transmute(NEXT.get()) };
        unsafe { f(p, r) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fopen(p: *const c_char, m: *const c_char) -> *mut c_void {
        static NEXT: Next = Next::new(c"fopen");
        if recording() {
            let mode = cpath(m).unwrap_or_default();
            if mode.iter().any(|c| matches!(c, b'w' | b'a' | b'+')) {
                mark_impure("file opened for writing");
            } else {
                note_path(KIND_CONTENT, AT_FDCWD, p);
                note_path(KIND_META, AT_FDCWD, p);
            }
        }
        let f: unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void =
            unsafe { std::mem::transmute(NEXT.get()) };
        unsafe { f(p, m) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn fopen64(p: *const c_char, m: *const c_char) -> *mut c_void {
        unsafe { fopen(p, m) }
    }

    macro_rules! impure_passthrough {
        ($name:ident, $why:literal, ($($a:ident: $t:ty),*) -> $r:ty) => {
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name($($a: $t),*) -> $r {
                static NEXT: Next = Next::new(
                    match CStr::from_bytes_with_nul(concat!(stringify!($name), "\0").as_bytes()) {
                        Ok(c) => c,
                        Err(_) => panic!(),
                    },
                );
                impure($why);
                let f: unsafe extern "C" fn($($t),*) -> $r = unsafe { std::mem::transmute(NEXT.get()) };
                unsafe { f($($a),*) }
            }
        };
    }
    impure_passthrough!(fork, "process spawn", () -> c_int);
    impure_passthrough!(execve, "process spawn", (a: *const c_char, b: *const c_void, c: *const c_void) -> c_int);
    impure_passthrough!(execvp, "process spawn", (a: *const c_char, b: *const c_void) -> c_int);
    impure_passthrough!(posix_spawn, "process spawn",
        (a: *mut c_void, b: *const c_char, c: *const c_void, d: *const c_void, e: *const c_void, f: *const c_void) -> c_int);
    impure_passthrough!(posix_spawnp, "process spawn",
        (a: *mut c_void, b: *const c_char, c: *const c_void, d: *const c_void, e: *const c_void, f: *const c_void) -> c_int);
    impure_passthrough!(socket, "socket", (a: c_int, b: c_int, c: c_int) -> c_int);
    impure_passthrough!(pthread_create, "thread creation",
        (a: *mut c_void, b: *const c_void, c: *const c_void, d: *mut c_void) -> c_int);
    impure_passthrough!(gettimeofday, "wall clock", (a: *mut c_void, b: *mut c_void) -> c_int);
    impure_passthrough!(time, "wall clock", (a: *mut c_long) -> c_long);

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn clock_gettime(clk: c_int, ts: *mut c_void) -> c_int {
        static NEXT: Next = Next::new(c"clock_gettime");
        // CLOCK_REALTIME, CLOCK_REALTIME_COARSE, CLOCK_REALTIME_ALARM, CLOCK_TAI
        if matches!(clk, 0 | 5 | 8 | 11) {
            impure("wall clock");
        }
        let f = NEXT.get();
        if f.is_null() {
            return unsafe { sys(SYS_CLOCK_GETTIME, clk as usize, ts as usize, 0, 0, 0, 0) as c_int };
        }
        let f: unsafe extern "C" fn(c_int, *mut c_void) -> c_int = unsafe { std::mem::transmute(f) };
        unsafe { f(clk, ts) }
    }
}
