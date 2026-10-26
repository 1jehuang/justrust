//! Bounded identity of the running compiler's on-disk executable image.
//! This does not identify dynamically loaded libraries, process state, external
//! tools, or an atomic filesystem snapshot. It never grants cache permission.
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
        capture(File::open("/proc/self/exe")?, MAX_IMAGE_BYTES).map(Arc::new)
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
fn capture(file: File, limit: u64) -> io::Result<CompilerImage> {
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
            "compiler image changed while being read",
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
