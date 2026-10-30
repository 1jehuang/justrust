//! Patches in the memory of a Mach-O program on AArch64.
//!
//! Code is written without a moment where it cannot run: a page of code that is made
//! writable is no longer executable, and anything else on it that runs then faults. So a
//! copy of the page is written, made executable, and mapped over the original in one call.

use std::{
    collections::HashMap,
    ffi::c_void,
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant, SystemTime},
};

use crate::{
    Outcome,
    format::{self, Body, Code, Patch, Process},
    sample,
};

unsafe extern "C" {
    fn mach_task_self() -> u32;
    fn mach_vm_allocate(task: u32, address: *mut u64, size: u64, flags: i32) -> i32;
    fn mach_vm_deallocate(task: u32, address: u64, size: u64) -> i32;
    fn mach_vm_protect(task: u32, address: u64, size: u64, set_max: i32, protection: i32) -> i32;
    fn mach_vm_remap(
        target: u32,
        address: *mut u64,
        size: u64,
        mask: u64,
        flags: i32,
        source_task: u32,
        source: u64,
        copy: i32,
        protection: *mut i32,
        max_protection: *mut i32,
        inheritance: u32,
    ) -> i32;
    fn mach_vm_region(
        task: u32,
        address: *mut u64,
        size: *mut u64,
        flavor: i32,
        info: *mut i32,
        count: *mut u32,
        object: *mut u32,
    ) -> i32;
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
    fn _dyld_get_image_header(index: u32) -> *const u8;
    fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
    fn __register_frame(frame: *const u8);
    fn kill(pid: i32, signal: i32) -> i32;
}

const READ: i32 = 1;
const WRITE: i32 = 2;
const EXECUTE: i32 = 4;
const PAGE: u64 = 0x4000;
const ANYWHERE: i32 = 1;
const FIXED: i32 = 0;
const OVERWRITE: i32 = 0x4000;
const INHERIT_NONE: u32 = 2;
const BASIC_INFO_64: i32 = 9;

const LC_SEGMENT_64: u32 = 0x19;
const LC_UUID: u32 = 0x1b;

/// How much room is set aside for patches. Each is laid out behind the last, since code of
/// an earlier one may still be running on some thread. A large program leaves only what is
/// between its image and the next library within reach, a few hundred megabytes for one of
/// half a gigabyte.
const ROOM: u64 = 256 << 20;
/// What `adrp` reaches, which is how the code of a patch reaches the program's.
const REACH: u64 = 4 << 30;
/// How often the patch and the executable are looked at without a word from the daemon.
const LOOK_EVERY: Duration = Duration::from_millis(30);

pub(crate) struct Live {
    dir: PathBuf,
    uuid: [u8; 16],
    slide: u64,
    process: Process,
    /// Each patch with when the thread that watches for them read it.
    patches: Receiver<(Patch, Instant)>,
    /// What the executable had where a patch wrote, by where, as long as the longest
    /// write there.
    originals: HashMap<u64, Vec<u8>>,
    /// How long the last patch's write at each address was.
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
        // SAFETY: the first image is the executable, which dyld loaded before anything ran.
        let (header, slide) =
            unsafe { (_dyld_get_image_header(0), _dyld_get_image_vmaddr_slide(0) as u64) };
        let image = Image::read(header)?;
        let dir = format::dir(&image.uuid).ok_or("no home directory to take edits through")?;
        forget_exited();
        // For the user alone: the daemon refuses a directory of its own that others can enter,
        // and `~/.cache/ra-jit` is its parent.
        std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700)
            .create(&dir)
            .map_err(|it| format!("{}: {it}", dir.display()))?;
        let (next, end) = reserve(&image, slide)?;
        _ = crate::DIR.set(dir.clone());
        let code = sample::Code { slide, image: image.start..image.end, patches: next..end };
        sample::start(dir.clone(), code);
        let started = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);
        let started = started.map_or(0, |it| it.as_millis() as u64);
        let process = Process { pid: std::process::id(), uuid: image.uuid, started, next, end };
        let live = Live {
            patches: watch(&dir, image.uuid),
            dir,
            uuid: image.uuid,
            slide,
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

    /// Applies each patch on a thread of its own as soon as it comes, and tells `told`.
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
        if patch.uuid != self.uuid {
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
        let len = code.text.len() as u64 + code.data.len() as u64;
        let end = code.base + len.next_multiple_of(PAGE);
        if code.base < self.process.next || end > self.process.end {
            return Err(format!(
                "laid out at {:#x}, where this program has no room: build again",
                code.base
            ));
        }
        if !(code.base.is_multiple_of(PAGE) && (code.text.len() as u64).is_multiple_of(PAGE)) {
            return Err("a patch whose code is not whole pages".to_owned());
        }
        let task = unsafe { mach_task_self() };
        let mut at = code.base + self.slide;
        // SAFETY: the room is ours, set aside by `reserve` and not used by an earlier patch.
        unsafe {
            check(mach_vm_allocate(task, &mut at, len.next_multiple_of(PAGE), FIXED | OVERWRITE))
                .map_err(|it| format!("mapping the patch: {it}"))?;
            let memory = std::slice::from_raw_parts_mut(at as *mut u8, len as usize);
            memory[..code.text.len()].copy_from_slice(&code.text);
            memory[code.text.len()..].copy_from_slice(&code.data);
            for &pointer in &code.pointers {
                slide_at(memory, pointer as usize, self.slide)?;
            }
            if !code.text.is_empty() {
                check(mach_vm_protect(task, at, code.text.len() as u64, 0, READ | EXECUTE))
                    .map_err(|it| format!("making the patch executable: {it}"))?;
                sys_icache_invalidate(at as *mut c_void, code.text.len());
            }
            register_frames(&memory[..code.text.len()], code.frames)?;
        }
        self.first_values(code, end)?;

        // A function the last patch replaced and this one does not is as it first was.
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
                // SAFETY: what a patch writes over is the executable's, which is mapped.
                let more = unsafe {
                    std::slice::from_raw_parts(
                        (address + start as u64) as *const u8,
                        bytes.len() - start,
                    )
                };
                original.extend_from_slice(more);
            }
            // What a longer write left behind is put back.
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
        let task = unsafe { mach_task_self() };
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
            let mut page = address & !(PAGE - 1);
            while page < address + len {
                if !self.lasting_pages.contains(&page) {
                    let mut at = page;
                    // SAFETY: the room is ours, and no patch goes above where its statics are.
                    unsafe { check(mach_vm_allocate(task, &mut at, PAGE, FIXED | OVERWRITE)) }
                        .map_err(|it| format!("mapping a static's page at {page:#x}: {it}"))?;
                    self.lasting_pages.insert(page);
                }
                page += PAGE;
            }
            let mut bytes = value.bytes.clone();
            for &pointer in &value.pointers {
                slide_at(&mut bytes, pointer as usize, self.slide)?;
            }
            // SAFETY: mapped above, readable and writable, and no code refers to it yet.
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

    /// Writes `bytes` at `address` of the program, however it is mapped.
    fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
        let protection = protection_at(address)?;
        // SAFETY: the address is the executable's, which a patch names and is mapped.
        unsafe {
            if protection & WRITE != 0 {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
                return Ok(());
            }
            swap_pages(address, bytes, protection)
        }
    }

    fn publish(&self) -> Result<(), String> {
        let path = self.dir.join("process");
        let partial = self.dir.join(format!("process.{}", self.process.pid));
        std::fs::write(&partial, self.process.encode())
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|it| format!("{}: {it}", path.display()))
    }
}

/// What the program's own load commands say.
struct Image {
    uuid: [u8; 16],
    /// Where the segment with the header starts and where the last segment ends.
    start: u64,
    end: u64,
}

impl Image {
    fn read(header: *const u8) -> Result<Image, String> {
        if header.is_null() {
            return Err("the program has no image".to_owned());
        }
        // SAFETY: dyld mapped the header and its load commands, which it read itself.
        let u32_at = |at: usize| unsafe { (header.add(at) as *const u32).read_unaligned() };
        let u64_at = |at: usize| unsafe { (header.add(at) as *const u64).read_unaligned() };
        let (count, mut at) = (u32_at(16), 32);
        let mut image = Image { uuid: [0; 16], start: u64::MAX, end: 0 };
        for _ in 0..count {
            let (command, size) = (u32_at(at), u32_at(at + 4) as usize);
            match command {
                LC_SEGMENT_64 => {
                    let (address, len) = (u64_at(at + 24), u64_at(at + 32));
                    let (offset, file_size, protection) =
                        (u64_at(at + 40), u64_at(at + 48), u32_at(at + 60));
                    if protection != 0 {
                        image.end = image.end.max(address + len);
                        if offset == 0 && file_size != 0 {
                            image.start = image.start.min(address);
                        }
                    }
                }
                LC_UUID => {
                    for (nth, byte) in image.uuid.iter_mut().enumerate() {
                        // SAFETY: the command is 24 bytes, the last 16 of them the UUID.
                        *byte = unsafe { *header.add(at + 8 + nth) };
                    }
                }
                _ => {}
            }
            at += size;
        }
        if image.uuid == [0; 16] || image.start == u64::MAX {
            return Err("the executable has no UUID or no header segment".to_owned());
        }
        Ok(image)
    }
}

/// Sets aside the room for patches as near behind the program as is free, for the code
/// in it to reach all of the program, and says where it is before the slide.
fn reserve(image: &Image, slide: u64) -> Result<(u64, u64), String> {
    const STEP: u64 = 2 << 20;
    let task = unsafe { mach_task_self() };
    let mut at = (image.end + slide).next_multiple_of(STEP);
    while at + ROOM - (image.start + slide) <= REACH {
        let mut address = at;
        // SAFETY: a fixed allocation without OVERWRITE fails where anything is mapped.
        if unsafe { mach_vm_allocate(task, &mut address, ROOM, FIXED) } == 0 {
            unsafe { check(mach_vm_protect(task, address, ROOM, 0, 0)) }
                .map_err(|it| format!("setting aside room for edits: {it}"))?;
            return Ok((address - slide, address - slide + ROOM));
        }
        at += STEP;
    }
    Err("nothing near the program is free to put edits in".to_owned())
}

/// Removes the directories of programs that are no longer running.
fn forget_exited() {
    let Some(root) = format::root() else { return };
    let Ok(entries) = std::fs::read_dir(&root) else { return };
    for entry in entries.flatten() {
        let process = std::fs::read_to_string(entry.path().join("process")).ok();
        let pid = process.as_deref().and_then(Process::decode).map(|it| it.pid);
        // SAFETY: a signal of 0 is only a question.
        let running = pid.is_some_and(|pid| unsafe { kill(pid as i32, 0) } == 0);
        if !running {
            _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Reads the patch in `dir` on a thread of its own each time it is written: at once when the
/// daemon says so on the `wake` socket, else when its time changes. A link that writes
/// another executable where the program's was, which no patch says of when nothing that
/// links it knew the program (`cargo build`, or a daemon started since), is told as a patch
/// of that executable.
fn watch(dir: &std::path::Path, uuid: [u8; 16]) -> Receiver<(Patch, Instant)> {
    let (send, receive) = std::sync::mpsc::channel();
    let path = dir.join("patch");
    let wake = dir.join(format::WAKE);
    _ = std::fs::remove_file(&wake);
    let wake = std::os::unix::net::UnixDatagram::bind(&wake).ok();
    let wake = wake.filter(|it| it.set_read_timeout(Some(LOOK_EVERY)).is_ok());
    let stamp = |path: &PathBuf| {
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    };
    // A patch from before the program started is for another run of it.
    let mut seen: Option<(SystemTime, u64)> = stamp(&path);
    let executable = std::env::current_exe().ok();
    let mut linked = executable.as_ref().and_then(stamp);
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
                        let other = uuid_of(executable).filter(|it| *it != uuid);
                        if let Some(other) = other {
                            let why = "the program was linked again".to_owned();
                            let sequence = SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .map_or(0, |it| it.as_millis() as u64);
                            let body = Body::Restart(why);
                            let patch = Patch { uuid: other, sequence, body };
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
                match Patch::decode(&bytes) {
                    Ok(patch) => {
                        if send.send((patch, Instant::now())).is_err() {
                            return;
                        }
                    }
                    Err(why) => eprintln!("blaze: {}: {why}", path.display()),
                }
            }
        })
        .expect("spawning the thread that watches for edits");
    receive
}

/// The UUID of the executable at `path`, which its load commands near the start say.
fn uuid_of(path: &PathBuf) -> Option<[u8; 16]> {
    use std::io::Read as _;
    const MAGIC: u32 = 0xfeed_facf;
    let mut head = Vec::with_capacity(1 << 16);
    std::fs::File::open(path).ok()?.take(1 << 16).read_to_end(&mut head).ok()?;
    let u32_at = |at: usize| Some(u32::from_le_bytes(head.get(at..at + 4)?.try_into().ok()?));
    if u32_at(0)? != MAGIC {
        return None;
    }
    let mut at = 32;
    for _ in 0..u32_at(16)? {
        if u32_at(at)? == LC_UUID {
            return head.get(at + 8..at + 24)?.try_into().ok();
        }
        at += u32_at(at + 4)? as usize;
    }
    None
}

fn check(code: i32) -> Result<(), String> {
    if code == 0 { Ok(()) } else { Err(format!("kern_return {code}")) }
}

fn slide_at(bytes: &mut [u8], at: usize, slide: u64) -> Result<(), String> {
    let field = bytes.get_mut(at..at + 8).ok_or("an address past the end of the patch")?;
    let value = u64::from_le_bytes(field.try_into().expect("8 bytes"));
    field.copy_from_slice(&value.wrapping_add(slide).to_le_bytes());
    Ok(())
}

fn protection_at(address: u64) -> Result<i32, String> {
    let task = unsafe { mach_task_self() };
    let (mut start, mut size, mut object) = (address, 0u64, 0u32);
    let mut info = [0i32; 9];
    let mut count = info.len() as u32;
    // SAFETY: `info` is as large as the basic information of a region, in `int`s.
    unsafe {
        check(mach_vm_region(
            task,
            &mut start,
            &mut size,
            BASIC_INFO_64,
            info.as_mut_ptr(),
            &mut count,
            &mut object,
        ))
    }
    .map_err(|it| format!("reading the protection at {address:#x}: {it}"))?;
    if start > address {
        return Err(format!("nothing is mapped at {address:#x}"));
    }
    Ok(info[0])
}

/// Writes `bytes` at `address` of pages that are not writable, by mapping a written copy
/// of them over them.
unsafe fn swap_pages(address: u64, bytes: &[u8], protection: i32) -> Result<(), String> {
    let page = address & !(PAGE - 1);
    let len = (address + bytes.len() as u64).next_multiple_of(PAGE) - page;
    let task = unsafe { mach_task_self() };
    let mut scratch = 0u64;
    unsafe {
        check(mach_vm_allocate(task, &mut scratch, len, ANYWHERE))?;
        std::ptr::copy_nonoverlapping(page as *const u8, scratch as *mut u8, len as usize);
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            (scratch + (address - page)) as *mut u8,
            bytes.len(),
        );
        let swapped = (|| {
            check(mach_vm_protect(task, scratch, len, 0, protection))?;
            let (mut current, mut max, mut target) = (0, 0, page);
            check(mach_vm_remap(
                task,
                &mut target,
                len,
                0,
                FIXED | OVERWRITE,
                task,
                scratch,
                0,
                &mut current,
                &mut max,
                INHERIT_NONE,
            ))
        })();
        mach_vm_deallocate(task, scratch, len);
        swapped.map_err(|it| format!("writing at {address:#x}: {it}"))?;
        if protection & EXECUTE != 0 {
            sys_icache_invalidate(address as *mut c_void, bytes.len());
        }
    }
    Ok(())
}

/// Tells the unwinder of each frame in `text` at `frames`, so that a panic unwinds through
/// the new code. Apple's unwinder takes one frame description at a time.
unsafe fn register_frames(text: &[u8], (start, len): (u64, u64)) -> Result<(), String> {
    let section = text
        .get(start as usize..(start + len) as usize)
        .ok_or("unwind tables past the end of the patch")?;
    let mut at = 0;
    while at + 8 <= section.len() {
        let length = u32::from_le_bytes(section[at..at + 4].try_into().expect("4 bytes")) as usize;
        if length == 0 || length == u32::MAX as usize {
            break;
        }
        let common = u32::from_le_bytes(section[at + 4..at + 8].try_into().expect("4 bytes"));
        if common != 0 {
            // SAFETY: the description is in the patch's code, which stays mapped.
            unsafe { __register_frame(section[at..].as_ptr()) };
        }
        at += 4 + length;
    }
    Ok(())
}
