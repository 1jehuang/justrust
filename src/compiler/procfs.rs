//! Bounded identity of the running compiler and its executable file mappings.
//! This is on-disk identity evidence, not proof of in-memory code, process state,
//! external tools, or an atomic filesystem snapshot. It never grants cache permission.
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::sync::Arc;

const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerImage {
    bytes: u64,
    digest: [u8; 32],
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
    pub fn matches_current(&self) -> bool {
        Self::current().is_ok_and(|current| current.as_ref() == self)
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
        // These are kernel-provided executable mappings, not user-loadable files.
        // vsyscall is the fixed legacy x86-64 kernel trampoline.
        if path == "[vdso]"
            || (path == "[vsyscall]" && range == "ffffffffff600000-ffffffffff601000")
        {
            if device != "00:00" || inode != "0" || permissions.contains('w') {
                return Err(invalid("unexpected kernel executable mapping"));
            }
            continue;
        }
        if !path.starts_with('/')
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
        if inode == 0 {
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
    let mappings = read_mappings()?;
    let executable = capture(File::open("/proc/self/exe")?, MAX_IMAGE_BYTES)?;
    let mut bytes = executable.bytes;
    let mut hash = blake3::Hasher::new();
    hash.update(b"justrust-running-compiler-runtime-v2");
    hash.update(&executable.digest);
    let mut files = Vec::new();
    for mapping in &mappings {
        let file = open_mapping(mapping)?;
        let metadata = file.metadata()?;
        let image = capture(file.try_clone()?, MAX_IMAGE_BYTES)?;
        bytes = bytes
            .checked_add(image.bytes)
            .filter(|n| *n <= 1024 * 1024 * 1024)
            .ok_or_else(|| invalid("runtime image byte limit exceeded"))?;
        hash.update(&(mapping.path.len() as u64).to_le_bytes());
        hash.update(mapping.path.as_bytes());
        hash.update(&mapping.offset.to_le_bytes());
        hash.update(&mapping.length.to_le_bytes());
        hash.update(&image.digest);
        files.push((file, stamp(&metadata), image));
    }
    // Recheck every pinned file after all reads, as well as mapping membership.
    for (file, before, image) in files {
        if stamp(&file.metadata()?) != before || capture(file, MAX_IMAGE_BYTES)? != image {
            return Err(invalid("runtime file changed during capture"));
        }
    }
    if read_mappings()? != mappings
        || capture(File::open("/proc/self/exe")?, MAX_IMAGE_BYTES)? != executable
    {
        return Err(invalid("runtime mappings changed during capture"));
    }
    Ok(CompilerImage {
        bytes,
        digest: *hash.finalize().as_bytes(),
    })
}

#[cfg(target_os = "linux")]
fn open_mapping(mapping: &ExecutableMapping) -> io::Result<File> {
    use std::os::unix::fs::MetadataExt;
    let file = File::open(&mapping.path)?;
    let metadata = file.metadata()?;
    let dev = metadata.dev();
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xfffff000);
    let minor = (dev & 0xff) | ((dev >> 12) & 0xffffff00);
    if metadata.ino() != mapping.inode || major != mapping.major || minor != mapping.minor {
        return Err(invalid(
            "executable mapping path no longer identifies mapped file",
        ));
    }
    Ok(file)
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
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
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
