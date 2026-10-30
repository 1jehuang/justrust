//! Patches the memory of a PE program on Windows x86-64.

use std::{
    collections::HashMap,
    ffi::c_void,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant, SystemTime},
};

use crate::{
    Outcome,
    format::{self, Body, Code, Patch, Process},
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetCurrentProcess() -> *mut c_void;
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
    fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
    fn RtlAddFunctionTable(table: *const u8, count: u32, base: u64) -> u8;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    fn CreateEventW(
        attributes: *const c_void,
        manual: i32,
        set: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_EXECUTE_READ: u32 = 0x20;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const SYNCHRONIZE: u32 = 0x0010_0000;
const WAIT_TIMEOUT: u32 = 0x102;
const WAIT_OBJECT_0: u32 = 0;
const IMAGE_DEBUG_TYPE_CODEVIEW: u32 = 2;
/// A `RUNTIME_FUNCTION`: where a function starts, ends, and has its unwind information.
const FUNCTION: u64 = 12;
const PAGE: u64 = 4096;
/// What `VirtualAlloc` reserves at: an address it is asked for is rounded down to it.
const GRANULE: u64 = 64 << 10;
const ROOM: u64 = 256 << 20;
const REACH: u64 = 2 << 30;
const LOOK_EVERY: Duration = Duration::from_millis(30);

pub(crate) struct Live {
    dir: PathBuf,
    id: [u8; 16],
    slide: u64,
    /// Where the image is loaded, which the tables of functions are relative to.
    loaded: u64,
    process: Process,
    patches: Receiver<(Patch, Instant)>,
    originals: HashMap<u64, Vec<u8>>,
    written: HashMap<u64, usize>,
    sequence: u64,
}

impl Live {
    pub(crate) fn start() -> Result<Live, String> {
        let executable =
            std::env::current_exe().map_err(|it| format!("finding the executable: {it}"))?;
        let image = Image::read(&executable)?;
        let dir = format::dir(&image.id).ok_or("no home directory to take edits through")?;
        forget_exited();
        std::fs::create_dir_all(&dir).map_err(|it| format!("{}: {it}", dir.display()))?;
        // SAFETY: no name asks for the executable of the process, which is always loaded.
        let loaded = unsafe { GetModuleHandleW(std::ptr::null()) } as u64;
        let slide = loaded.wrapping_sub(image.base);
        let (next, end) = reserve(&image, loaded, slide)?;
        let started = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |it| it.as_millis() as u64);
        let process = Process { pid: std::process::id(), uuid: image.id, started, next, end };
        let live = Live {
            patches: watch(&dir, image.id),
            dir,
            id: image.id,
            slide,
            loaded,
            process,
            originals: HashMap::new(),
            written: HashMap::new(),
            sequence: 0,
        };
        live.publish()?;
        Ok(live)
    }

    pub(crate) fn poll(&mut self) -> Option<Outcome> {
        let mut newest = None;
        loop {
            match self.patches.try_recv() {
                Ok(patch) => newest = Some(patch),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        let (patch, arrived) = newest?;
        self.take(patch, arrived)
    }

    pub(crate) fn apply_as_they_come(mut self, mut told: impl FnMut(Outcome) + Send + 'static) {
        let applying = move || {
            while let Ok(mut newest) = self.patches.recv() {
                while let Ok(next) = self.patches.try_recv() {
                    newest = next;
                }
                if let Some(outcome) = self.take(newest.0, newest.1) {
                    told(outcome);
                }
            }
        };
        std::thread::Builder::new()
            .name("ra-jit-live-apply".to_owned())
            .spawn(applying)
            .expect("spawning the thread that applies edits");
    }

    fn take(&mut self, patch: Patch, arrived: Instant) -> Option<Outcome> {
        if patch.sequence <= self.sequence {
            return None;
        }
        self.sequence = patch.sequence;
        if patch.uuid != self.id {
            return Some(Outcome::Restart("the program was linked again".to_owned()));
        }
        Some(match patch.body {
            Body::Restart(why) => Outcome::Restart(why),
            Body::Code(code) => {
                let start = Instant::now();
                let waited = start - arrived;
                match self.apply(&code) {
                    Ok(()) => Outcome::Applied {
                        sequence: patch.sequence,
                        functions: code.redirected,
                        statics: code.statics,
                        restored: code.writes.len() as u64 - code.redirected - code.statics,
                        took: start.elapsed(),
                        waited,
                    },
                    Err(why) => Outcome::Declined(why),
                }
            }
        })
    }

    fn apply(&mut self, code: &Code) -> Result<(), String> {
        let text_len = code.text.len() as u64;
        let data_len = code.data.len() as u64;
        let len = (text_len + data_len).next_multiple_of(PAGE);
        let end = code.base.checked_add(len).ok_or("patch address overflow")?;
        if code.base < self.process.next || end > self.process.end {
            return Err(format!(
                "laid out at {:#x}, where this program has no room: build again",
                code.base
            ));
        }
        if !(code.base.is_multiple_of(PAGE) && text_len.is_multiple_of(PAGE)) {
            return Err("a patch whose code is not whole pages".to_owned());
        }
        // A patch that only puts back what earlier ones wrote has nothing to map.
        if len != 0 {
            self.map(code, len)?;
        }

        let kept: std::collections::HashSet<u64> =
            code.writes.iter().map(|it| it.address).collect();
        let undone: Vec<u64> =
            self.written.keys().copied().filter(|it| !kept.contains(it)).collect();
        for address in undone {
            let original = self.originals[&address].clone();
            self.write(address, &original)?;
            self.written.remove(&address);
        }
        for write in &code.writes {
            let mut bytes = write.bytes.clone();
            for &pointer in &write.pointers {
                slide_at(&mut bytes, pointer as usize, self.slide)?;
            }
            let address = write.address.wrapping_add(self.slide);
            let original = self.originals.entry(address).or_default();
            if original.len() < bytes.len() {
                let start = original.len();
                // SAFETY: the bytes are of the executable's image, which stays loaded.
                let more = unsafe {
                    std::slice::from_raw_parts(
                        (address + start as u64) as *const u8,
                        bytes.len() - start,
                    )
                };
                original.extend_from_slice(more);
            }
            let last = self.written.get(&address).copied().unwrap_or(0);
            if last > bytes.len() {
                bytes.extend_from_slice(&original[bytes.len()..last]);
            }
            self.write(address, &bytes)?;
            self.written.insert(address, write.bytes.len());
        }
        self.process.next = end;
        self.publish()
    }

    /// Maps the code and data of the patch, `len` bytes, where it was laid out.
    fn map(&self, code: &Code, len: u64) -> Result<(), String> {
        let text_len = code.text.len() as u64;
        let mapped = code.base.wrapping_add(self.slide);
        // SAFETY: the pages are in the room `reserve` set aside, which nothing else uses.
        let result = unsafe {
            VirtualAlloc(mapped as *mut c_void, len as usize, MEM_COMMIT, PAGE_READWRITE)
        };
        if result as u64 != mapped {
            return Err(format!(
                "mapping the patch at {mapped:#x}: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: the pages were just committed, readable and writable, and are this one's.
        let memory = unsafe { std::slice::from_raw_parts_mut(mapped as *mut u8, len as usize) };
        memory[..code.text.len()].copy_from_slice(&code.text);
        memory[code.text.len()..code.text.len() + code.data.len()].copy_from_slice(&code.data);
        for &pointer in &code.pointers {
            slide_at(memory, pointer as usize, self.slide)?;
        }
        if text_len != 0 {
            let mut old = 0;
            // SAFETY: the code's pages are committed above.
            let protected = unsafe {
                VirtualProtect(
                    mapped as *mut c_void,
                    text_len as usize,
                    PAGE_EXECUTE_READ,
                    &mut old,
                )
            };
            if protected == 0 {
                return Err(format!(
                    "making the patch executable: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: the range is the code just written.
            unsafe {
                FlushInstructionCache(
                    GetCurrentProcess(),
                    mapped as *const c_void,
                    text_len as usize,
                )
            };
        }
        self.register_functions(mapped, code.frames)
    }

    /// Has the unwinder find the functions of the patch, whose table at `start` of the code is
    /// sorted and relative to where the image is loaded: a Rust panic unwinds through them.
    fn register_functions(&self, mapped: u64, (start, len): (u64, u64)) -> Result<(), String> {
        if len == 0 {
            return Ok(());
        }
        if !len.is_multiple_of(FUNCTION) || !start.is_multiple_of(4) {
            return Err("a table of functions that is not whole entries".to_owned());
        }
        let table = (mapped + start) as *const u8;
        // SAFETY: the table is in the code just mapped, which is never unmapped.
        if unsafe { RtlAddFunctionTable(table, (len / FUNCTION) as u32, self.loaded) } == 0 {
            return Err("the unwinder refused the patch's table of functions".to_owned());
        }
        Ok(())
    }

    fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
        let mut old = 0;
        // SAFETY: the bytes are of the executable's image, which stays loaded.
        let writable = unsafe {
            VirtualProtect(address as *mut c_void, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old)
        };
        if writable == 0 {
            return Err(format!(
                "making {address:#x} writable: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: the bytes were made writable above.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len()) };
        let mut was = 0;
        // SAFETY: as above.
        if unsafe { VirtualProtect(address as *mut c_void, bytes.len(), old, &mut was) } == 0 {
            return Err(format!(
                "restoring protection at {address:#x}: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: the range was just written.
        unsafe {
            FlushInstructionCache(GetCurrentProcess(), address as *const c_void, bytes.len())
        };
        Ok(())
    }

    fn publish(&self) -> Result<(), String> {
        let path = self.dir.join("process");
        let partial = self.dir.join(format!("process.{}", self.process.pid));
        std::fs::write(&partial, self.process.encode())
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|it| format!("{}: {it}", path.display()))
    }
}

/// What a program has to know of its executable, every address as the linker has it.
struct Image {
    /// The GUID of the PDB the linker wrote with it.
    id: [u8; 16],
    /// Where the image is loaded when the loader does not move it, and how long it is.
    base: u64,
    len: u64,
}

impl Image {
    fn read(path: &Path) -> Result<Image, String> {
        let bytes =
            std::fs::read(path).map_err(|it| format!("reading {}: {it}", path.display()))?;
        let u16_at = |at: usize| -> Result<u16, String> {
            bytes
                .get(at..at + 2)
                .and_then(|it| it.try_into().ok())
                .map(u16::from_le_bytes)
                .ok_or("PE header ends early".to_owned())
        };
        let u32_at = |at: usize| -> Result<u32, String> {
            bytes
                .get(at..at + 4)
                .and_then(|it| it.try_into().ok())
                .map(u32::from_le_bytes)
                .ok_or("PE header ends early".to_owned())
        };
        let u64_at = |at: usize| -> Result<u64, String> {
            bytes
                .get(at..at + 8)
                .and_then(|it| it.try_into().ok())
                .map(u64::from_le_bytes)
                .ok_or("PE header ends early".to_owned())
        };
        let nt = u32_at(0x3c)? as usize;
        if bytes.get(..2) != Some(b"MZ") || bytes.get(nt..nt + 4) != Some(b"PE\0\0") {
            return Err("the executable is not a PE image".to_owned());
        }
        let header = nt + 4;
        let optional = header + 20;
        if u16_at(header)? != 0x8664 || u16_at(optional)? != 0x20b {
            return Err("the executable is not a PE32+ image for x86-64".to_owned());
        }
        let sections = optional + usize::from(u16_at(header + 16)?);
        let section_count = usize::from(u16_at(header + 2)?);
        let base = u64_at(optional + 24)?;
        let len = u64::from(u32_at(optional + 56)?);
        // Where the file has what is loaded at `address`.
        let offset_of = |address: u32| -> Result<usize, String> {
            for nth in 0..section_count {
                let at = sections + nth * 40;
                let start = u32_at(at + 12)?;
                let raw = u32_at(at + 16)?;
                if (start..start.saturating_add(raw)).contains(&address) {
                    return Ok((u32_at(at + 20)? + (address - start)) as usize);
                }
            }
            Err(format!("nothing in the file is loaded at {address:#x}"))
        };
        if u32_at(optional + 108)? <= 6 {
            return Err("the executable has no debug directory".to_owned());
        }
        let (debug, debug_len) = (u32_at(optional + 112 + 6 * 8)?, u32_at(optional + 116 + 6 * 8)?);
        let debug = if debug_len == 0 { None } else { Some(offset_of(debug)?) };
        let id = (0..debug_len as usize / 28)
            .filter_map(|nth| debug.map(|it| it + nth * 28))
            .filter(|&entry| u32_at(entry + 12) == Ok(IMAGE_DEBUG_TYPE_CODEVIEW))
            .find_map(|entry| {
                let at = u32_at(entry + 24).ok()? as usize;
                let record = bytes.get(at..at + 20)?;
                (record[..4] == *b"RSDS").then(|| record[4..20].try_into().ok()).flatten()
            })
            .ok_or("the executable names no PDB")?;
        Ok(Image { id, base, len })
    }
}

/// Sets aside room for patches behind the image, near enough for a jump of 32 bits from any
/// of its functions to any of the patch's and back. Says where, as the linker has addresses.
fn reserve(image: &Image, loaded: u64, slide: u64) -> Result<(u64, u64), String> {
    let mut at = (loaded + image.len).next_multiple_of(GRANULE);
    for _ in 0..64 {
        if at + ROOM - loaded > REACH {
            break;
        }
        // SAFETY: a reservation takes nothing that is in use, and makes nothing accessible.
        let room =
            unsafe { VirtualAlloc(at as *mut c_void, ROOM as usize, MEM_RESERVE, PAGE_NOACCESS) };
        if room as u64 == at {
            return Ok((at.wrapping_sub(slide), at.wrapping_sub(slide) + ROOM));
        }
        at += GRANULE;
    }
    Err("nothing near the program is free to put edits in".to_owned())
}

fn forget_exited() {
    let Some(root) = format::root() else { return };
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let process = std::fs::read_to_string(entry.path().join("process")).ok();
        let pid = process.as_deref().and_then(Process::decode).map(|it| it.pid);
        if !pid.is_some_and(running) {
            _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn running(pid: u32) -> bool {
    // SAFETY: it takes no pointer; the handle is closed below.
    let process = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return false;
    }
    // SAFETY: the handle is open; a wait of no time only asks whether the process ended.
    let running = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
    // SAFETY: the handle is open and nothing else has it.
    unsafe { CloseHandle(process) };
    running
}

/// Watches for patches: the daemon sets an event once it wrote one ([`format::wake_event`]),
/// and a daemon that does not is looked for every [`LOOK_EVERY`]. Whether the executable was
/// linked again is looked at on a thread of its own: the first read of an executable just
/// written waits for the scan of it, half a second, which the patch that came with it is not
/// to wait for.
fn watch(dir: &Path, id: [u8; 16]) -> Receiver<(Patch, Instant)> {
    let (send, receive) = std::sync::mpsc::channel();
    let path = dir.join("patch");
    let mut seen = stamp(&path);
    let name: Vec<u16> = format::wake_event(dir).encode_utf16().chain([0]).collect();
    // SAFETY: the name ends with a zero. The event is never closed: the thread below waits on
    // it as long as the program runs. A handle is not `Send`, its value is.
    let wake = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) } as usize;
    if let Some(executable) = std::env::current_exe().ok() {
        let send = send.clone();
        let mut linked = stamp(&executable);
        std::thread::Builder::new()
            .name("ra-jit-live-linked".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(LOOK_EVERY);
                    let now = stamp(&executable);
                    if now.is_none() || now == linked {
                        continue;
                    }
                    linked = now;
                    let other =
                        Image::read(&executable).ok().map(|it| it.id).filter(|it| *it != id);
                    if let Some(other) = other {
                        let sequence = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map_or(0, |it| it.as_millis() as u64);
                        let patch = Patch {
                            uuid: other,
                            sequence,
                            body: Body::Restart("the program was linked again".to_owned()),
                        };
                        if send.send((patch, Instant::now())).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("spawning the thread that watches the executable");
    }
    std::thread::Builder::new()
        .name("ra-jit-live".to_owned())
        .spawn(move || {
            loop {
                let woke = if wake == 0 {
                    std::thread::sleep(LOOK_EVERY);
                    false
                } else {
                    let wait = LOOK_EVERY.as_millis() as u32;
                    // SAFETY: the event is open for as long as the program runs.
                    unsafe { WaitForSingleObject(wake as *mut c_void, wait) == WAIT_OBJECT_0 }
                };
                let now = stamp(&path);
                if now.is_none() || (now == seen && !woke) {
                    continue;
                }
                seen = now;
                let Ok(bytes) = std::fs::read(&path) else { continue };
                if let Ok(patch) = Patch::decode(&bytes) {
                    if send.send((patch, Instant::now())).is_err() {
                        return;
                    }
                }
            }
        })
        .expect("spawning the thread that watches for edits");
    receive
}

/// When the file at `path` was last written, and how long it is.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn slide_at(bytes: &mut [u8], at: usize, slide: u64) -> Result<(), String> {
    let field = bytes.get_mut(at..at + 8).ok_or("an address past the end of the patch")?;
    let value =
        u64::from_le_bytes(field.try_into().map_err(|_| "an address shorter than 8 bytes")?);
    field.copy_from_slice(&value.wrapping_add(slide).to_le_bytes());
    Ok(())
}

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn socket(family: i32, kind: i32, protocol: i32) -> usize;
    fn connect(socket: usize, address: *const u8, len: i32) -> i32;
    fn send(socket: usize, bytes: *const u8, len: i32, flags: i32) -> i32;
    fn recv(socket: usize, bytes: *mut u8, len: i32, flags: i32) -> i32;
    fn closesocket(socket: usize) -> i32;
    fn WSAGetLastError() -> i32;
}

/// A connection to the daemon's AF_UNIX socket, which the standard library has no type for on
/// Windows.
pub(crate) struct Socket(usize);

impl Socket {
    pub(crate) fn connect(path: &Path) -> std::io::Result<Socket> {
        const AF_UNIX: u16 = 1;
        const SOCK_STREAM: i32 = 1;
        const INVALID_SOCKET: usize = !0;
        let path = path.to_str().ok_or(std::io::ErrorKind::InvalidInput)?.as_bytes();
        // `sockaddr_un`: the family, then the path ending with a zero in 108 bytes.
        let mut address = [0u8; 2 + 108];
        if path.len() >= 108 {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        address[..2].copy_from_slice(&AF_UNIX.to_le_bytes());
        address[2..2 + path.len()].copy_from_slice(path);
        // SAFETY: `WSADATA` is under 512 bytes; a second start only counts one more user.
        if unsafe { WSAStartup(0x202, [0u8; 512].as_mut_ptr()) } != 0 {
            return Err(Socket::error());
        }
        // SAFETY: it takes no pointer.
        let socket = unsafe { socket(i32::from(AF_UNIX), SOCK_STREAM, 0) };
        if socket == INVALID_SOCKET {
            return Err(Socket::error());
        }
        let socket = Socket(socket);
        // SAFETY: the address is a whole `sockaddr_un`.
        if unsafe { connect(socket.0, address.as_ptr(), address.len() as i32) } != 0 {
            return Err(Socket::error());
        }
        Ok(socket)
    }

    fn error() -> std::io::Error {
        // SAFETY: it takes nothing.
        std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
    }
}

impl std::io::Read for Socket {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let len = bytes.len().min(i32::MAX as usize) as i32;
        // SAFETY: `bytes` has room for `len`.
        let read = unsafe { recv(self.0, bytes.as_mut_ptr(), len, 0) };
        if read < 0 { Err(Socket::error()) } else { Ok(read as usize) }
    }
}

impl std::io::Write for Socket {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let len = bytes.len().min(i32::MAX as usize) as i32;
        // SAFETY: `bytes` has `len` of them.
        let sent = unsafe { send(self.0, bytes.as_ptr(), len, 0) };
        if sent < 0 { Err(Socket::error()) } else { Ok(sent as usize) }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        // SAFETY: the socket is this one's alone.
        unsafe { closesocket(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test's own executable, which rustc linked with a PDB, is read as the loader has it.
    #[test]
    fn the_executable_names_its_pdb_and_where_it_goes() {
        let image = Image::read(&std::env::current_exe().unwrap()).unwrap();
        assert_ne!(image.id, [0; 16]);
        // SAFETY: as in `Live::start`.
        let loaded = unsafe { GetModuleHandleW(std::ptr::null()) } as u64;
        assert!(image.len != 0 && image.len.is_multiple_of(PAGE), "{:#x}", image.len);
        let (next, end) = reserve(&image, loaded, loaded.wrapping_sub(image.base)).unwrap();
        assert!(next >= image.base + image.len && end - next == ROOM, "{next:#x} {end:#x}");
    }
}
