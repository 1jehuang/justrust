//! Patches the memory of an ELF program on Linux x86-64.

use std::{
    collections::HashMap,
    ffi::c_void,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant, SystemTime},
};

use crate::{
    Outcome,
    format::{self, Body, Code, Patch, Process},
};

unsafe extern "C" {
    fn __register_frame(frame: *const u8);
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut c_void;
    fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
}

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const MAP_PRIVATE: i32 = 2;
const MAP_ANONYMOUS: i32 = 0x20;
const MAP_FIXED: i32 = 0x10;
const MAP_FIXED_NOREPLACE: i32 = 0x100000;
const MAP_FAILED: *mut c_void = -1isize as *mut c_void;
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const NT_GNU_BUILD_ID: u32 = 3;
const PAGE: u64 = 4096;
const ROOM: u64 = 256 << 20;
const REACH: u64 = 2 << 30;
const LOOK_EVERY: Duration = Duration::from_millis(30);

pub(crate) struct Live {
    dir: PathBuf,
    id: [u8; 16],
    slide: u64,
    process: Process,
    patches: Receiver<(Patch, Instant)>,
    originals: HashMap<u64, Vec<u8>>,
    written: HashMap<u64, usize>,
    sequence: u64,
    /// The statics new since the executable that a patch gave a first value, by address, and
    /// the pages of the room they are in.
    lasting: std::collections::HashSet<u64>,
    lasting_pages: std::collections::HashSet<u64>,
    /// Where the room ends, above those statics: the `end` published is below them.
    room_end: u64,
}

impl Live {
    pub(crate) fn start() -> Result<Live, String> {
        let executable =
            std::env::current_exe().map_err(|it| format!("finding the executable: {it}"))?;
        let image = Image::read(&executable)?;
        let dir = format::dir(&image.id).ok_or("no home directory to take edits through")?;
        forget_exited();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|it| format!("{}: {it}", dir.display()))?;
        let (next, end) = reserve(&image)?;
        let started = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |it| it.as_millis() as u64);
        let process = Process { pid: std::process::id(), uuid: image.id, started, next, end };
        let live = Live {
            patches: watch(&dir, image.id),
            dir,
            id: image.id,
            slide: image.slide,
            process,
            originals: HashMap::new(),
            written: HashMap::new(),
            sequence: 0,
            lasting: std::collections::HashSet::new(),
            lasting_pages: std::collections::HashSet::new(),
            room_end: end,
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
        let mapped = code.base.checked_add(self.slide).ok_or("patch address overflow")?;
        let result = unsafe {
            mmap(
                mapped as *mut c_void,
                len as usize,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
                -1,
                0,
            )
        };
        if result == MAP_FAILED || result as u64 != mapped {
            return Err(format!(
                "mapping the patch at {mapped:#x}: {}",
                std::io::Error::last_os_error()
            ));
        }
        let memory = unsafe { std::slice::from_raw_parts_mut(mapped as *mut u8, len as usize) };
        memory[..code.text.len()].copy_from_slice(&code.text);
        memory[code.text.len()..code.text.len() + code.data.len()].copy_from_slice(&code.data);
        for &pointer in &code.pointers {
            slide_at(memory, pointer as usize, self.slide)?;
        }
        if text_len != 0
            && unsafe { mprotect(mapped as *mut c_void, text_len as usize, PROT_READ | PROT_EXEC) }
                != 0
        {
            return Err(format!(
                "making the patch executable: {}",
                std::io::Error::last_os_error()
            ));
        }
        if data_len != 0
            && unsafe {
                mprotect(
                    (mapped + text_len) as *mut c_void,
                    data_len as usize,
                    PROT_READ | PROT_WRITE,
                )
            } != 0
        {
            return Err(format!("making patch data writable: {}", std::io::Error::last_os_error()));
        }
        register_frames(memory, code.frames)?;
        self.first_values(code, end)?;

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
            let address = write.address + self.slide;
            let original = self.originals.entry(address).or_default();
            if original.len() < bytes.len() {
                let start = original.len();
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

    /// Writes the first value of each static new since the executable that the patch refers
    /// to and no patch before gave one, in the room above the patch, which ends at `end`.
    fn first_values(&mut self, code: &Code, end: u64) -> Result<(), String> {
        for value in &code.lasting {
            // Above every patch, in the room: nothing else of the program is mapped there.
            let len = value.bytes.len() as u64;
            if value.address < end || value.address.saturating_add(len) > self.room_end {
                return Err(format!("a static at {:#x}, outside the room", value.address));
            }
            let address = value.address + self.slide;
            if self.lasting.contains(&address) {
                continue;
            }
            let last = address + len;
            let mut page = address & !(PAGE - 1);
            while page < last {
                if !self.lasting_pages.contains(&page) {
                    let result = unsafe {
                        mmap(
                            page as *mut c_void,
                            PAGE as usize,
                            PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
                            -1,
                            0,
                        )
                    };
                    if result == MAP_FAILED || result as u64 != page {
                        return Err(format!(
                            "mapping a static's page at {page:#x}: {}",
                            std::io::Error::last_os_error()
                        ));
                    }
                    self.lasting_pages.insert(page);
                }
                page += PAGE;
            }
            let mut bytes = value.bytes.clone();
            for &pointer in &value.pointers {
                slide_at(&mut bytes, pointer as usize, self.slide)?;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len())
            };
            self.lasting.insert(address);
        }
        // A daemon started again knows none of them, and places what it has below.
        if let Some(&low) = self.lasting_pages.iter().min() {
            self.process.end = self.process.end.min(low - self.slide);
        }
        Ok(())
    }

    fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
        let page = address & !(PAGE - 1);
        let end = (address + bytes.len() as u64).next_multiple_of(PAGE);
        let len = end - page;
        let old = protection_at(address).unwrap_or(PROT_READ | PROT_EXEC);
        if unsafe {
            mprotect(page as *mut c_void, len as usize, PROT_READ | PROT_WRITE | PROT_EXEC)
        } != 0
        {
            return Err(format!(
                "making {address:#x} writable: {}",
                std::io::Error::last_os_error()
            ));
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len()) };
        if unsafe { mprotect(page as *mut c_void, len as usize, old) } != 0 {
            return Err(format!(
                "restoring protection at {address:#x}: {}",
                std::io::Error::last_os_error()
            ));
        }
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

struct Image {
    id: [u8; 16],
    slide: u64,
    start: u64,
    end: u64,
}

impl Image {
    fn read(path: &Path) -> Result<Image, String> {
        let bytes =
            std::fs::read(path).map_err(|it| format!("reading {}: {it}", path.display()))?;
        if bytes.get(..4) != Some(b"\x7fELF")
            || bytes.get(4) != Some(&2)
            || bytes.get(5) != Some(&1)
        {
            return Err("the executable is not a little-endian ELF64 image".to_owned());
        }
        let u16_at = |at: usize| -> Result<u16, String> {
            bytes
                .get(at..at + 2)
                .and_then(|it| it.try_into().ok())
                .map(u16::from_le_bytes)
                .ok_or("ELF header ends early".to_owned())
        };
        let u32_at = |at: usize| -> Result<u32, String> {
            bytes
                .get(at..at + 4)
                .and_then(|it| it.try_into().ok())
                .map(u32::from_le_bytes)
                .ok_or("ELF header ends early".to_owned())
        };
        let u64_at = |at: usize| -> Result<u64, String> {
            bytes
                .get(at..at + 8)
                .and_then(|it| it.try_into().ok())
                .map(u64::from_le_bytes)
                .ok_or("ELF header ends early".to_owned())
        };
        let kind = u16_at(16)?;
        let phoff = u64_at(32)? as usize;
        let phentsize = u16_at(54)? as usize;
        let phnum = u16_at(56)? as usize;
        let mut start = u64::MAX;
        let mut end = 0;
        let mut first = None;
        let mut notes = Vec::new();
        for nth in 0..phnum {
            let at = phoff + nth * phentsize;
            let kind_at = u32_at(at)?;
            let offset = u64_at(at + 8)? as usize;
            let address = u64_at(at + 16)?;
            let file_len = u64_at(at + 32)?;
            let mem_len = u64_at(at + 40)?;
            if kind_at == PT_LOAD {
                start = start.min(address);
                end = end.max(address + mem_len);
                if first.is_none() {
                    first = Some((offset as u64, address));
                }
            } else if kind_at == PT_NOTE {
                notes.push((offset, file_len as usize));
            }
        }
        let id = notes
            .into_iter()
            .find_map(|(offset, len)| build_id(bytes.get(offset..offset.saturating_add(len))?))
            .ok_or("the executable has no GNU build ID")?;
        let first = first.ok_or("the executable has no load segment")?;
        let slide = if kind == 3 { load_bias(path, first.0, first.1)? } else { 0 };
        Ok(Image { id, slide, start, end })
    }
}

fn build_id(mut bytes: &[u8]) -> Option<[u8; 16]> {
    while bytes.len() >= 12 {
        let namesz = u32::from_le_bytes(bytes[..4].try_into().ok()?) as usize;
        let descsz = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
        let kind = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
        let name_end = 12 + namesz.next_multiple_of(4);
        let desc_end = name_end + descsz.next_multiple_of(4);
        if desc_end > bytes.len() {
            return None;
        }
        if kind == NT_GNU_BUILD_ID
            && bytes.get(12..12 + namesz).is_some_and(|it| it.starts_with(b"GNU\0"))
        {
            let mut id = [0; 16];
            let desc = &bytes[name_end..name_end + descsz];
            id[..desc.len().min(16)].copy_from_slice(&desc[..desc.len().min(16)]);
            return Some(id);
        }
        bytes = &bytes[desc_end..];
    }
    None
}

fn load_bias(path: &Path, first_offset: u64, first_address: u64) -> Result<u64, String> {
    let wanted =
        std::fs::canonicalize(path).map_err(|it| format!("canonicalizing executable: {it}"))?;
    let maps = std::fs::read_to_string("/proc/self/maps")
        .map_err(|it| format!("reading process maps: {it}"))?;
    for line in maps.lines() {
        let mut fields = line.split_whitespace();
        let Some(range) = fields.next() else { continue };
        let Some(_permissions) = fields.next() else { continue };
        let Some(offset) = fields.next() else { continue };
        let Some(pathname) = fields.nth(2) else { continue };
        let Some((start, _)) = range.split_once('-') else { continue };
        if offset != "00000000" || !pathname.starts_with('/') {
            continue;
        }
        if std::fs::canonicalize(pathname).ok().as_deref() != Some(&wanted) {
            continue;
        }
        let address =
            u64::from_str_radix(start, 16).map_err(|_| "invalid process map address".to_owned())?;
        return address
            .checked_sub(first_address & !(PAGE - 1))
            .and_then(|it| it.checked_sub(first_offset))
            .ok_or("invalid ELF load bias".to_owned());
    }
    Err("the executable is not mapped at a known address".to_owned())
}

fn reserve(image: &Image) -> Result<(u64, u64), String> {
    let desired = image.end.next_multiple_of(PAGE);
    let actual = desired + image.slide;
    let mut at = actual;
    for _ in 0..64 {
        if at.abs_diff(image.start + image.slide) <= REACH {
            let room = unsafe {
                mmap(
                    at as *mut c_void,
                    ROOM as usize,
                    0,
                    MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE,
                    -1,
                    0,
                )
            };
            if room != MAP_FAILED {
                return Ok((at - image.slide, at - image.slide + ROOM));
            }
        }
        at = at.saturating_add(ROOM);
    }
    Err("nothing near the program is free to put edits in".to_owned())
}

fn forget_exited() {
    let Some(root) = format::root() else { return };
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let process = std::fs::read_to_string(entry.path().join("process")).ok();
        let pid = process.as_deref().and_then(Process::decode).map(|it| it.pid);
        let running = pid.is_some_and(|pid| unsafe { kill(pid as i32, 0) } == 0);
        if !running {
            _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn watch(dir: &Path, id: [u8; 16]) -> Receiver<(Patch, Instant)> {
    let (send, receive) = std::sync::mpsc::channel();
    let path = dir.join("patch");
    let wake_path = dir.join(format::WAKE);
    _ = std::fs::remove_file(&wake_path);
    let wake = std::os::unix::net::UnixDatagram::bind(&wake_path)
        .ok()
        .filter(|it| it.set_read_timeout(Some(LOOK_EVERY)).is_ok());
    let stamp = |path: &Path| {
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    };
    let mut seen = stamp(&path);
    let executable = std::env::current_exe().ok();
    let mut linked = executable.as_ref().and_then(|path| stamp(path));
    std::thread::Builder::new()
        .name("ra-jit-live".to_owned())
        .spawn(move || {
            loop {
                let woke = match &wake {
                    Some(wake) => wake.recv(&mut [0; 16]).is_ok(),
                    None => {
                        std::thread::sleep(LOOK_EVERY);
                        false
                    }
                };
                if let Some(executable) = &executable {
                    let now = stamp(executable);
                    if now.is_some() && now != linked {
                        linked = now;
                        let other =
                            Image::read(executable).ok().map(|it| it.id).filter(|it| *it != id);
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
                }
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

fn slide_at(bytes: &mut [u8], at: usize, slide: u64) -> Result<(), String> {
    let field = bytes.get_mut(at..at + 8).ok_or("an address past the end of the patch")?;
    let value =
        u64::from_le_bytes(field.try_into().map_err(|_| "an address shorter than 8 bytes")?);
    field.copy_from_slice(&value.wrapping_add(slide).to_le_bytes());
    Ok(())
}

fn protection_at(address: u64) -> Result<i32, String> {
    let maps = std::fs::read_to_string("/proc/self/maps")
        .map_err(|it| format!("reading process maps: {it}"))?;
    for line in maps.lines() {
        let mut fields = line.split_whitespace();
        let Some(range) = fields.next() else { continue };
        let Some((start, end)) = range.split_once('-') else { continue };
        let start =
            u64::from_str_radix(start, 16).map_err(|_| "invalid process map start".to_owned())?;
        let end = u64::from_str_radix(end, 16).map_err(|_| "invalid process map end".to_owned())?;
        if (start..end).contains(&address) {
            let perms = fields.next().unwrap_or("").as_bytes();
            return Ok((if perms.first() == Some(&b'r') { PROT_READ } else { 0 })
                | (if perms.get(1) == Some(&b'w') { PROT_WRITE } else { 0 })
                | (if perms.get(2) == Some(&b'x') { PROT_EXEC } else { 0 }));
        }
    }
    Err(format!("nothing is mapped at {address:#x}"))
}

fn register_frames(text: &[u8], (start, len): (u64, u64)) -> Result<(), String> {
    if len == 0 {
        return Ok(());
    }
    let section = text
        .get(start as usize..(start + len) as usize)
        .ok_or("unwind tables past the end of the patch")?;
    let mut at = 0;
    while at + 8 <= section.len() {
        let length =
            u32::from_le_bytes(section[at..at + 4].try_into().map_err(|_| "short unwind length")?)
                as usize;
        if length == 0 || length == u32::MAX as usize {
            break;
        }
        let end = at.checked_add(4 + length).ok_or("unwind entry overflows")?;
        if end > section.len() {
            return Err("unwind entry ends past the patch".to_owned());
        }
        let common = u32::from_le_bytes(
            section[at + 4..at + 8].try_into().map_err(|_| "short unwind entry")?,
        );
        if common != 0 {
            unsafe { __register_frame(section[at..].as_ptr()) };
        }
        at = end;
    }
    Ok(())
}
