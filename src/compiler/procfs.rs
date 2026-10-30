//! Bounded identity of the running compiler, executable files and mapped code.
//! Includes bounded process-visible configuration, not proof of arbitrary
//! mutable process state, external tools, or an atomic filesystem snapshot.
//! It never grants cache permission.
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::sync::Arc;

#[cfg(target_os = "linux")]
mod floating;

const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

/// Launcher evidence does not establish the floating environment of a worker.
/// Check at the operation that consumes native floating arithmetic as well.
pub(super) fn validate_floating_environment() -> io::Result<()> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    floating::current()?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerImage {
    bytes: u64,
    digest: [u8; 32],
    implementation_digest: [u8; 32],
}
impl crate::engine::Fact for CompilerImage {}
impl CompilerImage {
    /// Open the kernel's running image, not a path that an updater can retarget.
    /// Failure or the resource ceiling means no compiler-image evidence exists.
    #[cfg(target_os = "linux")]
    pub fn current() -> io::Result<Arc<Self>> {
        // A launcher can still be completing a chmod/link while its child starts.
        // Discard the entire attempt on failure. Never combine partial evidence
        // across attempts, and keep the total work bounded.
        let mut result = capture_runtime();
        for _ in 0..2 {
            if result.is_ok() {
                break;
            }
            result = capture_runtime();
        }
        result.map(Arc::new)
    }
    #[cfg(not(target_os = "linux"))]
    pub fn current() -> io::Result<Arc<Self>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "compiler image identity requires Linux",
        ))
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    /// Version identity for pure operations with explicit semantic inputs.
    /// Includes the executable, mapped libraries and mapped code, but not
    /// invocation arguments, cwd, environment or resource limits. This is not
    /// permission to reuse operations that actually consume those inputs.
    pub fn implementation_digest(&self) -> &[u8; 32] {
        &self.implementation_digest
    }
    pub fn matches_current(&self) -> bool {
        self.validate_current().is_ok()
    }
    pub fn validate_current(&self) -> io::Result<()> {
        let current = Self::current()?;
        if current.as_ref() == self {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "compiler runtime identity changed ({} bytes to {} bytes)",
                    self.bytes, current.bytes
                ),
            ))
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, PartialEq, Eq)]
struct ExecutableMapping {
    range: String,
    permissions: String,
    offset: u64,
    length: u64,
    major: u64,
    minor: u64,
    inode: u64,
    path: String,
}

#[cfg(target_os = "linux")]
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(target_os = "linux")]
fn executable_mappings(text: &str) -> io::Result<Vec<ExecutableMapping>> {
    let mut result = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let range = fields
            .next()
            .ok_or_else(|| invalid("missing mapping range"))?;
        let permissions = fields
            .next()
            .ok_or_else(|| invalid("missing permissions"))?;
        if permissions.as_bytes().get(2) != Some(&b'x') {
            continue;
        }
        let offset = fields
            .next()
            .ok_or_else(|| invalid("missing mapping offset"))?;
        let device = fields
            .next()
            .ok_or_else(|| invalid("missing mapping device"))?;
        let inode = fields
            .next()
            .ok_or_else(|| invalid("missing mapping inode"))?;
        let path = fields.collect::<Vec<_>>().join(" ");
        // The fixed legacy vsyscall trampoline is not readable via proc mem.
        // vDSO, in contrast, is ordinary readable executable ELF code and can
        // differ between kernels or private process mappings. Capture its bytes.
        if path == "[vsyscall]" && range == "ffffffffff600000-ffffffffff601000" {
            if device != "00:00" || inode != "0" || permissions.contains('w') {
                return Err(invalid("unexpected kernel executable mapping"));
            }
            continue;
        }
        let vdso = path == "[vdso]";
        if (!path.starts_with('/') && !vdso)
            || path.ends_with(" (deleted)")
            || path.contains('\\')
            || permissions != "r-xp"
            || result.len() >= 128
        {
            return Err(invalid("unsupported executable mapping"));
        }
        let (major, minor) = device
            .split_once(':')
            .ok_or_else(|| invalid("bad mapping device"))?;
        let hex = |s| u64::from_str_radix(s, 16).map_err(|_| invalid("bad mapping integer"));
        let (start, end) = range
            .split_once('-')
            .ok_or_else(|| invalid("bad mapping range"))?;
        let length = hex(end)?
            .checked_sub(hex(start)?)
            .filter(|length| *length > 0)
            .ok_or_else(|| invalid("empty or reversed mapping"))?;
        let inode = inode.parse().map_err(|_| invalid("bad mapping inode"))?;
        if vdso && (major != "00" || minor != "00" || inode != 0 || hex(offset)? != 0) {
            return Err(invalid("unexpected vDSO mapping identity"));
        }
        if vdso
            && (length > 2 * 1024 * 1024
                || result
                    .iter()
                    .any(|m: &ExecutableMapping| m.path == "[vdso]"))
        {
            return Err(invalid("duplicate or oversized vDSO mapping"));
        }
        if inode == 0 && !vdso {
            return Err(invalid("anonymous executable mapping"));
        }
        result.push(ExecutableMapping {
            range: range.into(),
            permissions: permissions.into(),
            offset: hex(offset)?,
            length,
            major: hex(major)?,
            minor: hex(minor)?,
            inode,
            path,
        });
    }
    if result.is_empty() {
        return Err(invalid("no executable file mappings"));
    }
    Ok(result)
}

#[cfg(target_os = "linux")]
fn read_mappings() -> io::Result<Vec<ExecutableMapping>> {
    let mut text = String::new();
    File::open("/proc/self/maps")?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 1024 * 1024 {
        return Err(invalid("mapping list exceeds limit"));
    }
    executable_mappings(&text)
}

#[cfg(target_os = "linux")]
fn capture_runtime() -> io::Result<CompilerImage> {
    let floating = floating::current()?;
    let process = current_process_identity()?;
    let mappings = read_mappings()?;
    let mut pinned = RuntimeFiles::default();
    let executable = pinned.capture(File::open("/proc/self/exe")?)?;
    let mut bytes = executable.bytes;
    let mut hash = blake3::Hasher::new();
    hash.update(b"justrust-running-compiler-runtime-v7");
    hash.update(&executable.digest);
    let memory = File::open("/proc/self/mem")?;
    let mut remaining_code = 128 * 1024 * 1024u64;
    let mut files = Vec::new();
    let mut kernel_code = Vec::new();
    for mapping in &mappings {
        if mapping.path == "[vdso]" {
            remaining_code = remaining_code
                .checked_sub(mapping.length)
                .ok_or_else(|| invalid("executable memory byte limit exceeded"))?;
            let code = mapped_code_identity(&memory, mapping)?;
            hash.update(b"kernel-vdso");
            hash.update(&mapping.length.to_le_bytes());
            hash.update(&code);
            bytes = bytes
                .checked_add(mapping.length)
                .filter(|n| *n <= 1024 * 1024 * 1024)
                .ok_or_else(|| invalid("runtime image byte limit exceeded"))?;
            kernel_code.push((mapping, code));
            continue;
        }
        let file = open_mapping(mapping)?;
        let image = pinned.capture(file)?;
        bytes = bytes
            .checked_add(image.bytes)
            .filter(|n| *n <= 1024 * 1024 * 1024)
            .ok_or_else(|| invalid("runtime image byte limit exceeded"))?;
        hash.update(&(mapping.path.len() as u64).to_le_bytes());
        hash.update(mapping.path.as_bytes());
        hash.update(&mapping.offset.to_le_bytes());
        hash.update(&mapping.length.to_le_bytes());
        hash.update(&image.digest);
        remaining_code = remaining_code
            .checked_sub(mapping.length)
            .ok_or_else(|| invalid("executable memory byte limit exceeded"))?;
        let code = mapped_code_identity(&memory, mapping)?;
        hash.update(&code);
        files.push((mapping, code));
    }
    // Recheck every pinned file after all reads, as well as mapping membership.
    for (mapping, code) in files {
        if mapped_code_identity(&memory, mapping)? != code {
            return Err(invalid(
                "runtime file or mapped code changed during capture",
            ));
        }
    }
    for (mapping, code) in kernel_code {
        if mapped_code_identity(&memory, mapping)? != code {
            return Err(invalid("kernel executable mapping changed during capture"));
        }
    }
    // Hash each pinned backing file again, including the running executable.
    // Reopening exe still verifies that the kernel alias names that same file.
    pinned.recheck()?;
    if read_mappings()? != mappings
        || pinned.capture(File::open("/proc/self/exe")?)? != executable
        || current_process_identity()? != process
        || floating::current()? != floating
    {
        return Err(invalid("runtime mappings changed during capture"));
    }
    let implementation_digest = *hash.finalize().as_bytes();
    hash.update(&process);
    hash.update(b"native-floating-controls-v1");
    hash.update(&floating);
    Ok(CompilerImage {
        bytes,
        digest: *hash.finalize().as_bytes(),
        implementation_digest,
    })
}

/// Scratch local to one capture, never a cache across observations. Keeping an
/// open descriptor prevents inode reuse. Equal device/inode is sufficient to
/// share bytes only while the complete mutation stamp still agrees.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct RuntimeFiles {
    files: Vec<(File, Metadata, CompilerImage)>,
}

#[cfg(target_os = "linux")]
impl RuntimeFiles {
    fn capture(&mut self, file: File) -> io::Result<CompilerImage> {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        for (_, before, image) in &self.files {
            if metadata.dev() == before.dev() && metadata.ino() == before.ino() {
                if stamp(&metadata) != stamp(before) {
                    return Err(invalid("runtime file changed during capture"));
                }
                return Ok(image.clone());
            }
        }
        let image = capture(file.try_clone()?, MAX_IMAGE_BYTES)?;
        if stamp(&file.metadata()?) != stamp(&metadata) {
            return Err(invalid("runtime file changed during capture"));
        }
        self.files.push((file, metadata, image.clone()));
        Ok(image)
    }

    fn recheck(&self) -> io::Result<()> {
        for (file, before, image) in &self.files {
            if stamp(&file.metadata()?) != stamp(before)
                || capture(file.try_clone()?, MAX_IMAGE_BYTES)? != *image
            {
                return Err(invalid("runtime file changed during capture"));
            }
        }
        Ok(())
    }
}

/// MAP_PRIVATE code can differ from its backing file after a writable mapping
/// is changed and made executable again. File identity alone misses that input.
/// Read through the kernel rather than dereferencing addresses that can vanish
/// concurrently. Address ranges select bytes but do not enter the digest: ASLR
/// is not itself a code change. This still does not seal arbitrary mutable data.
#[cfg(target_os = "linux")]
fn mapped_code_identity(memory: &File, mapping: &ExecutableMapping) -> io::Result<[u8; 32]> {
    use std::os::unix::fs::FileExt;
    let (start, _) = mapping
        .range
        .split_once('-')
        .ok_or_else(|| invalid("bad executable memory range"))?;
    let mut address =
        u64::from_str_radix(start, 16).map_err(|_| invalid("bad executable memory address"))?;
    let mut remaining = mapping.length;
    if remaining > 128 * 1024 * 1024 {
        return Err(invalid("executable memory byte limit exceeded"));
    }
    let mut hash = blake3::Hasher::new();
    hash.update(b"justrust-executable-memory-v1");
    hash.update(&remaining.to_le_bytes());
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let length = remaining.min(buffer.len() as u64) as usize;
        memory.read_exact_at(&mut buffer[..length], address)?;
        hash.update(&buffer[..length]);
        address = address
            .checked_add(length as u64)
            .ok_or_else(|| invalid("executable memory range overflow"))?;
        remaining -= length as u64;
    }
    Ok(*hash.finalize().as_bytes())
}

#[cfg(target_os = "linux")]
fn current_process_identity() -> io::Result<[u8; 32]> {
    let mut limits = Vec::new();
    File::open("/proc/self/limits")?
        .take(64 * 1024 + 1)
        .read_to_end(&mut limits)?;
    if limits.len() > 64 * 1024 {
        return Err(invalid("process resource limits exceed identity limit"));
    }
    process_identity(
        &std::env::current_dir()?,
        std::env::args_os(),
        std::env::vars_os(),
        &limits,
    )
}

#[cfg(target_os = "linux")]
fn process_identity(
    cwd: &std::path::Path,
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
    environment: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    limits: &[u8],
) -> io::Result<[u8; 32]> {
    const MAX_PROCESS_BYTES: usize = 1024 * 1024;
    const MAX_PROCESS_ENTRIES: usize = 4096;
    let mut remaining = MAX_PROCESS_BYTES;
    let mut hash = blake3::Hasher::new();
    hash.update(b"justrust-process-configuration-v1");
    let mut frame = |bytes: &[u8]| -> io::Result<()> {
        remaining = remaining
            .checked_sub(bytes.len().saturating_add(8))
            .ok_or_else(|| invalid("process configuration byte limit exceeded"))?;
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
        Ok(())
    };
    frame(cwd.as_os_str().as_encoded_bytes())?;
    frame(limits)?;
    // Argument order matters. Environment entry order does not. Retain raw
    // non-UTF-8 values, reject ambiguous duplicates, and never publish plaintext credentials.
    let mut count = 0usize;
    for argument in arguments {
        count += 1;
        if count > MAX_PROCESS_ENTRIES {
            return Err(invalid("argument identity limit exceeded"));
        }
        frame(&[0])?;
        frame(argument.as_encoded_bytes())?;
    }
    frame(&[1])?;
    let mut entries = Vec::new();
    let mut environment_bytes = 0usize;
    for (name, value) in environment {
        count += 1;
        environment_bytes = environment_bytes
            .saturating_add(name.len())
            .saturating_add(value.len());
        if count > MAX_PROCESS_ENTRIES || environment_bytes > MAX_PROCESS_BYTES {
            return Err(invalid("environment identity limit exceeded"));
        }
        entries.push((name, value));
    }
    entries.sort();
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("duplicate process environment key"));
    }
    for (name, value) in entries {
        frame(name.as_encoded_bytes())?;
        frame(value.as_encoded_bytes())?;
    }
    Ok(*hash.finalize().as_bytes())
}

#[cfg(target_os = "linux")]
fn open_mapping(mapping: &ExecutableMapping) -> io::Result<File> {
    use std::os::unix::fs::MetadataExt;
    let file = File::open(&mapping.path)?;
    let metadata = file.metadata()?;
    let dev = metadata.dev();
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xfffff000);
    let minor = (dev & 0xff) | ((dev >> 12) & 0xffffff00);
    // btrfs subvolumes report a per-subvolume st_dev, while /proc/self/maps
    // reports the superblock device recorded in mountinfo. Accept either, but
    // always require the exact inode; contents are hashed independently.
    let device_matches = (major == mapping.major && minor == mapping.minor)
        || mount_device(&mapping.path)? == Some((mapping.major, mapping.minor));
    if metadata.ino() != mapping.inode || !device_matches {
        return Err(invalid(
            "executable mapping path no longer identifies mapped file",
        ));
    }
    Ok(file)
}

/// Superblock device of the mount containing `path` (longest mount point
/// prefix in /proc/self/mountinfo).
#[cfg(target_os = "linux")]
fn mount_device(path: &str) -> io::Result<Option<(u64, u64)>> {
    let mut text = String::new();
    File::open("/proc/self/mountinfo")?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 1024 * 1024 {
        return Err(invalid("mount list exceeds limit"));
    }
    Ok(mount_device_in(&text, path))
}

#[cfg(target_os = "linux")]
fn mount_device_in(mountinfo: &str, path: &str) -> Option<(u64, u64)> {
    let mut best: Option<(usize, (u64, u64))> = None;
    for line in mountinfo.lines() {
        let mut fields = line.split(' ');
        let (Some(_), Some(_), Some(device), Some(_), Some(point)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        // Mount points with escaped characters never prefix-match plain paths.
        let inside = point == "/"
            || path == point
            || path
                .strip_prefix(point)
                .is_some_and(|rest| rest.starts_with('/'));
        if !inside || best.is_some_and(|(len, _)| len >= point.len()) {
            continue;
        }
        let Some((major, minor)) = device.split_once(':') else {
            continue;
        };
        let (Ok(major), Ok(minor)) = (major.parse(), minor.parse()) else {
            continue;
        };
        best = Some((point.len(), (major, minor)));
    }
    best.map(|(_, device)| device)
}

#[cfg(target_os = "linux")]
fn stamp(metadata: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

#[cfg(target_os = "linux")]
fn capture(mut file: File, limit: u64) -> io::Result<CompilerImage> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() == 0 || before.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "compiler image is empty, nonregular, or exceeds its byte limit",
        ));
    }
    let mut reader = (&file).take(limit.saturating_add(1));
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    let mut hash = blake3::Hasher::new();
    hash.update(b"justrust-running-compiler-image-v1");
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compiler image byte limit exceeded",
            ));
        }
        hash.update(&buffer[..count]);
    }
    let after = file.metadata()?;
    if bytes != before.len() || stamp(&before) != stamp(&after) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "compiler image changed while being read: read {bytes}, expected {}, before {:?}, after {:?}",
                before.len(),
                stamp(&before),
                stamp(&after)
            ),
        ));
    }
    hash.update(&bytes.to_le_bytes());
    Ok(CompilerImage {
        bytes,
        digest: *hash.finalize().as_bytes(),
        implementation_digest: *hash.finalize().as_bytes(),
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn runtime_files_deduplicate_pinned_aliases_and_reject_mutation() {
        use std::io::{Seek, SeekFrom};
        let directory =
            std::env::temp_dir().join(format!("jr-runtime-dedup-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("image");
        let alias = directory.join("alias");
        std::fs::write(&path, b"first").unwrap();
        std::fs::hard_link(&path, &alias).unwrap();
        let mut pinned = RuntimeFiles::default();
        let first = pinned.capture(File::open(&path).unwrap()).unwrap();
        let mut file = File::open(&alias).unwrap();
        file.seek(SeekFrom::Start(2)).unwrap();
        // capture() hashes via a cloned descriptor with a shared cursor. A
        // duplicate must not seek or read that cursor at all.
        assert_eq!(pinned.capture(file.try_clone().unwrap()).unwrap(), first);
        assert_eq!(file.stream_position().unwrap(), 2);
        assert_eq!(pinned.files.len(), 1);
        pinned.recheck().unwrap();
        std::fs::write(&path, b"other").unwrap();
        assert!(pinned.capture(File::open(&alias).unwrap()).is_err());
        assert!(pinned.recheck().is_err());
        // Even if metadata alone appeared unchanged, final content hashing
        // must still reject changed bytes. Simulate that condition locally.
        pinned.files[0].1 = std::fs::metadata(&path).unwrap();
        assert!(pinned.recheck().is_err());
        // A fresh observation cannot reuse the earlier capture's identity.
        let mut fresh = RuntimeFiles::default();
        assert_ne!(fresh.capture(File::open(&path).unwrap()).unwrap(), first);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mount_device_uses_longest_mount_prefix() {
        let info = "31 2 0:28 /@ / rw shared:1 - btrfs /dev/nvme0n1p2 rw\n\
                    56 31 0:29 /@home /home rw shared:141 - btrfs /dev/nvme0n1p2 rw\n\
                    43 31 0:39 / /tmp rw shared:18 - tmpfs tmpfs rw\n";
        assert_eq!(mount_device_in(info, "/usr/lib/libc.so.6"), Some((0, 28)));
        assert_eq!(mount_device_in(info, "/home/u/jr-rust"), Some((0, 29)));
        assert_eq!(mount_device_in(info, "/homework/x"), Some((0, 28)));
        assert_eq!(mount_device_in(info, "/tmp"), Some((0, 39)));
        assert_eq!(mount_device_in("garbage", "/x"), None);
    }
    #[test]
    fn frozen_graph_rejects_forged_runtime_before_success_publication() {
        use crate::compiler::{
            self,
            source::{SourceOptions, SourceSnapshot},
        };
        let frozen = SourceSnapshot::new(
            "/frozen-runtime-graph".into(),
            [("main.rs".into(), b"fn main() {}".to_vec())],
        )
        .unwrap();
        let mut forged = (*CompilerImage::current().unwrap()).clone();
        forged.digest[0] ^= 1;
        for workers in [1, 8] {
            for image in [None, Some(Arc::new(forged.clone()))] {
                let snapshot = Arc::new(compiler::Snapshot {
                    input: compiler::Input::File("/frozen-runtime-graph/main.rs".into()),
                    source_options: SourceOptions {
                        source_snapshot: Some(frozen.clone()),
                        compiler_image: image,
                        ..Default::default()
                    },
                    budget: compiler::ir::PlanBudget::default(),
                });
                let engine = crate::engine::Engine::new(compiler::CompilerProvider(snapshot));
                let check = engine.request(compiler::keys::CheckRoot.key());
                let build = engine.request(compiler::keys::BuildRoot.key());
                engine.run(workers).unwrap();
                assert!(check.completion().is_err());
                assert!(build.completion().is_err());
                assert!(build.fact::<Vec<u8>>(0).is_err());
            }
        }
    }
    #[test]
    fn runtime_mapping_child() {
        use std::os::fd::AsRawFd;
        if std::env::var_os("JR_MAPPING_IDENTITY_CHILD").is_none() {
            return;
        }
        unsafe extern "C" {
            fn mmap(
                address: *mut std::ffi::c_void,
                length: usize,
                protection: i32,
                flags: i32,
                fd: i32,
                offset: i64,
            ) -> *mut std::ffi::c_void;
            fn munmap(address: *mut std::ffi::c_void, length: usize) -> i32;
            fn mprotect(address: *mut std::ffi::c_void, length: usize, protection: i32) -> i32;
        }
        let path = std::env::temp_dir().join(format!("jr-runtime-map-{}", std::process::id()));
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let file = File::open(&path).unwrap();
        let before = CompilerImage::current().unwrap();
        // Linux PROT_READ | PROT_EXEC, MAP_PRIVATE. The mapping is never
        // dereferenced or executed; only the kernel's mapping table is read.
        let mapped = unsafe { mmap(std::ptr::null_mut(), 4096, 1 | 4, 2, file.as_raw_fd(), 0) };
        assert_ne!(mapped as usize, usize::MAX, "executable mmap failed");
        let added = CompilerImage::current().unwrap();
        assert_ne!(before.digest(), added.digest());
        assert_ne!(
            before.implementation_digest(),
            added.implementation_digest()
        );
        assert!(!before.matches_current());
        assert!(added.matches_current());
        // The same file, inode, mapping range and permissions can hide private
        // code changes. Restore RX before capture so the mapping table itself
        // cannot distinguish this from the original input.
        assert_eq!(unsafe { mprotect(mapped, 4096, 1 | 2) }, 0);
        unsafe { mapped.cast::<u8>().write_volatile(7) };
        assert_eq!(unsafe { mprotect(mapped, 4096, 1 | 4) }, 0);
        assert_eq!(std::fs::read(&path).unwrap(), vec![0u8; 4096]);
        assert!(!added.matches_current());
        let private_code = CompilerImage::current().unwrap();
        assert_ne!(added.digest(), private_code.digest());
        assert_ne!(
            added.implementation_digest(),
            private_code.implementation_digest()
        );
        assert!(private_code.matches_current());
        assert_eq!(unsafe { mprotect(mapped, 4096, 1 | 2) }, 0);
        unsafe { mapped.cast::<u8>().write_volatile(0) };
        assert_eq!(unsafe { mprotect(mapped, 4096, 1 | 4) }, 0);
        assert!(added.matches_current());
        std::fs::write(&path, vec![1u8; 4096]).unwrap();
        assert!(!added.matches_current());
        let changed = CompilerImage::current().unwrap();
        assert_ne!(added.digest(), changed.digest());
        assert_ne!(
            added.implementation_digest(),
            changed.implementation_digest()
        );
        std::fs::remove_file(&path).unwrap();
        assert!(CompilerImage::current().is_err());
        // Replacing the pathname cannot legitimize the still-mapped old inode.
        std::fs::write(&path, vec![1u8; 4096]).unwrap();
        assert!(CompilerImage::current().is_err());
        assert_eq!(unsafe { munmap(mapped, 4096) }, 0);
        assert!(before.matches_current());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn public_runtime_identity_tracks_mapped_file_mutations() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "compiler::procfs::tests::runtime_mapping_child",
                "--nocapture",
            ])
            .env("JR_MAPPING_IDENTITY_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn runtime_process_child() {
        if std::env::var_os("JR_PROCESS_IDENTITY_CHILD").is_none() {
            return;
        }
        let image = CompilerImage::current().unwrap();
        println!("PROCESS_DIGEST:{:?}", image.digest());
        println!("IMPLEMENTATION_DIGEST:{:?}", image.implementation_digest());
        assert!(image.matches_current());
    }

    #[test]
    fn public_runtime_identity_tracks_process_configuration() {
        let root = std::env::temp_dir().join(format!("jr-process-identity-{}", std::process::id()));
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let run = |cwd: &std::path::Path, value: &str| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "compiler::procfs::tests::runtime_process_child",
                    "--nocapture",
                ])
                .env("JR_PROCESS_IDENTITY_CHILD", "1")
                .env("JR_PROCESS_IDENTITY_VALUE", value)
                .current_dir(cwd)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).unwrap();
            let process = text
                .lines()
                .find(|line| line.starts_with("PROCESS_DIGEST:"))
                .expect("child did not emit identity")
                .to_owned();
            let implementation = text
                .lines()
                .find(|line| line.starts_with("IMPLEMENTATION_DIGEST:"))
                .expect("child did not emit implementation identity")
                .to_owned();
            (process, implementation)
        };
        let first = run(&root, "first");
        assert_eq!(first, run(&root, "first"));
        for changed in [run(&root, "changed"), run(&other, "first")] {
            assert_ne!(first.0, changed.0);
            assert_eq!(first.1, changed.1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn process_configuration_identity_is_framed_bounded_and_sensitive() {
        use std::ffi::OsString;
        use std::path::Path;
        let env = vec![
            (OsString::from("A"), OsString::from("one")),
            ("B".into(), "two".into()),
        ];
        let identity =
            |cwd: &str, args: Vec<OsString>, env: Vec<(OsString, OsString)>, limits: &[u8]| {
                process_identity(Path::new(cwd), args, env, limits)
            };
        let first = identity("/root", vec!["compiler".into()], env.clone(), b"limits").unwrap();
        let mut reversed = env.clone();
        reversed.reverse();
        assert_eq!(
            first,
            identity("/root", vec!["compiler".into()], reversed, b"limits").unwrap()
        );
        for other in [
            identity("/other", vec!["compiler".into()], env.clone(), b"limits"),
            identity(
                "/root",
                vec!["compiler".into(), "--check".into()],
                env.clone(),
                b"limits",
            ),
            identity(
                "/root",
                vec!["compiler".into()],
                vec![("A".into(), "changed".into())],
                b"limits",
            ),
            identity(
                "/root",
                vec!["compiler".into()],
                env.clone(),
                b"different limit",
            ),
        ] {
            assert_ne!(first, other.unwrap());
        }
        assert!(
            identity(
                "/root",
                vec![],
                vec![("A".into(), "one".into()), ("A".into(), "two".into())],
                b""
            )
            .is_err()
        );
        assert!(identity("/root", vec!["argument".into(); 4097], vec![], b"").is_err());
        assert!(
            identity(
                "/root",
                vec![],
                vec![("A".into(), "x".repeat(1024 * 1024).into())],
                b""
            )
            .is_err()
        );
        use std::os::unix::ffi::OsStringExt;
        assert_ne!(
            identity(
                "/root",
                vec![],
                vec![("A".into(), OsString::from_vec(vec![0xff]))],
                b""
            )
            .unwrap(),
            identity(
                "/root",
                vec![],
                vec![("A".into(), OsString::from_vec(vec![0xfe]))],
                b""
            )
            .unwrap()
        );
    }
    #[test]
    fn mapped_file_mutation_replacement_and_removal_are_detected() {
        use std::os::unix::fs::MetadataExt;
        let path = std::env::temp_dir().join(format!("jr-mapped-image-{}", std::process::id()));
        std::fs::write(&path, b"first").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let dev = metadata.dev();
        let mapping = ExecutableMapping {
            range: "1000-2000".into(),
            permissions: "r-xp".into(),
            offset: 0,
            length: 4096,
            major: ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xfffff000),
            minor: (dev & 0xff) | ((dev >> 12) & 0xffffff00),
            inode: metadata.ino(),
            path: path.to_str().unwrap().into(),
        };
        let pinned = open_mapping(&mapping).unwrap();
        let first = capture(pinned.try_clone().unwrap(), 128).unwrap();
        std::fs::write(&path, b"other").unwrap();
        assert_ne!(
            first,
            capture(open_mapping(&mapping).unwrap(), 128).unwrap()
        );
        std::fs::remove_file(&path).unwrap();
        assert!(open_mapping(&mapping).is_err());
        std::fs::write(&path, b"first").unwrap();
        assert!(open_mapping(&mapping).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn executable_mapping_policy_fails_closed() {
        let supported = "1000-2000 r-xp 00001000 08:01 17 /lib/runtime.so\n";
        let mappings = executable_mappings(supported).unwrap();
        assert_eq!(mappings[0].offset, 4096);
        for unsupported in [
            "1000-2000 rwxp 00001000 08:01 17 /lib/runtime.so\n",
            "1000-2000 r-xp 00001000 08:01 17 /lib/runtime.so (deleted)\n",
            "1000-2000 r-xp 00000000 00:00 0\n",
            "1000-2000 r-xp 00000000 00:00 0 [jit]\n",
            "1000-2000 r-xp 00000000 08:01 17 /bad\\040name\n",
            "1000-2000 r-xp 00000000 08:01 17 [vdso]\n",
            "1000-2000 rwxp 00000000 00:00 0 [vdso]\n",
            "1000-2000 r-xp 00001000 00:00 0 [vdso]\n",
            "1000-401000 r-xp 00000000 00:00 0 [vdso]\n",
        ] {
            assert!(executable_mappings(&format!("{supported}{unsupported}")).is_err());
        }
        assert!(executable_mappings(&supported.repeat(129)).is_err());
        assert!(executable_mappings("").is_err());
        assert!(
            executable_mappings(&format!(
                "{supported}1000-2000 r-xp 00000000 00:00 0 [vdso]\n"
            ))
            .is_ok()
        );
        let vdso = "3000-4000 r-xp 00000000 00:00 0 [vdso]\n";
        let mappings = executable_mappings(&format!("{supported}{vdso}")).unwrap();
        assert_eq!(mappings.len(), 2);
        assert_eq!(mappings[1].path, "[vdso]");
        assert!(executable_mappings(&format!("{supported}{vdso}{vdso}")).is_err());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn runtime_vdso_child() {
        if std::env::var_os("JR_VDSO_IDENTITY_CHILD").is_none() {
            return;
        }
        use std::os::unix::fs::FileExt;
        unsafe extern "C" {
            fn mprotect(address: *mut std::ffi::c_void, length: usize, protection: i32) -> i32;
        }
        let mapping = read_mappings()
            .unwrap()
            .into_iter()
            .find(|m| m.path == "[vdso]")
            .expect("x86-64 kernel vDSO absent");
        let start = u64::from_str_radix(mapping.range.split_once('-').unwrap().0, 16).unwrap();
        let memory = File::open("/proc/self/mem").unwrap();
        let mut magic = [0; 4];
        memory.read_exact_at(&mut magic, start).unwrap();
        assert_eq!(&magic, b"\x7fELF");
        let before = CompilerImage::current().unwrap();
        // EI_PAD is unused ELF identification padding, not instructions or
        // dynamic symbols. Mutate only a private child mapping and restore RX
        // before capture so permissions cannot explain the digest difference.
        let byte = (start + 15) as *mut u8;
        assert_eq!(
            unsafe { mprotect(start as *mut _, mapping.length as usize, 1 | 2 | 4) },
            0
        );
        let saved = unsafe { byte.read_volatile() };
        unsafe { byte.write_volatile(saved ^ 1) };
        assert_eq!(
            unsafe { mprotect(start as *mut _, mapping.length as usize, 1 | 4) },
            0
        );
        let changed = CompilerImage::current().unwrap();
        assert_ne!(before.digest(), changed.digest());
        assert!(!before.matches_current());
        assert!(changed.matches_current());
        assert_eq!(
            unsafe { mprotect(start as *mut _, mapping.length as usize, 1 | 2 | 4) },
            0
        );
        unsafe { byte.write_volatile(saved) };
        assert_eq!(
            unsafe { mprotect(start as *mut _, mapping.length as usize, 1 | 4) },
            0
        );
        assert!(before.matches_current());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn public_runtime_identity_tracks_private_vdso_mutations() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "compiler::procfs::tests::runtime_vdso_child",
                "--nocapture",
            ])
            .env("JR_VDSO_IDENTITY_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    #[test]
    fn running_image_is_revalidated_without_path_aliases() {
        let image = CompilerImage::current().unwrap();
        assert!(image.bytes() > 0);
        assert!(image.matches_current());
        let mut forged = (*image).clone();
        forged.digest[0] ^= 1;
        assert!(!forged.matches_current());
    }
    #[test]
    fn public_compilation_replaces_forged_stamp_and_returns_graph_evidence() {
        use crate::compiler::{self, source::SourceOptions};
        let directory =
            std::env::temp_dir().join(format!("jr-image-compile-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let fake_path = directory.join("fake-image");
        std::fs::write(&fake_path, b"not the running compiler").unwrap();
        let fake = Arc::new(capture(File::open(&fake_path).unwrap(), 128).unwrap());
        let source = directory.join("main.rs");
        std::fs::write(&source, "fn main() {}").unwrap();
        let options = SourceOptions {
            compiler_image: Some(fake.clone()),
            prelude: false,
            ..Default::default()
        };
        let unit = compiler::source::prepare_root_unit(
            &source,
            &options,
            &compiler::expansion_budget::ExpansionBudget::default(),
        )
        .unwrap();
        assert!(!unit.inputs.matches_current_source(&options));
        let actual = CompilerImage::current().unwrap();
        for workers in [1, 8] {
            for check_only in [true, false] {
                let compiled = compiler::compile_file_observed_with_options(
                    source.clone(),
                    workers,
                    check_only,
                    options.clone(),
                )
                .unwrap();
                let recorded = compiled
                    .compiler_image
                    .as_ref()
                    .expect("missing prepared-source image identity");
                assert_eq!(recorded.digest(), actual.digest());
                assert_ne!(recorded.digest(), fake.digest());
                if let Some(bytes) = compiled.executable {
                    let executable = directory.join(format!("run-{workers}"));
                    std::fs::write(&executable, bytes).unwrap();
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    assert!(
                        std::process::Command::new(executable)
                            .status()
                            .unwrap()
                            .success()
                    );
                }
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn image_content_and_limits_are_fail_closed() {
        let path = std::env::temp_dir().join(format!("jr-image-{}", std::process::id()));
        std::fs::write(&path, b"first").unwrap();
        let first = capture(File::open(&path).unwrap(), 5).unwrap();
        assert_eq!(first.bytes(), 5);
        assert!(capture(File::open(&path).unwrap(), 4).is_err());
        std::fs::write(&path, b"other").unwrap();
        let other = capture(File::open(&path).unwrap(), 5).unwrap();
        assert_ne!(first.digest(), other.digest());
        std::fs::write(&path, b"").unwrap();
        assert!(capture(File::open(&path).unwrap(), 5).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(File::open(&path).is_err());
        assert!(capture(File::open(std::env::temp_dir()).unwrap(), 5).is_err());
    }
}
