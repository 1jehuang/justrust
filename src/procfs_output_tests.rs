use super::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn scratch(tag: &str) -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = base.join(format!(
        "jr-output-consumer-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn maps_parser_handles_spaces_escapes_deleted_and_component_boundaries() {
    let row = |p: &[u8]| {
        let mut row = b"1000-2000 r--p 00000000 00:01 123 ".to_vec();
        row.extend_from_slice(p);
        row.push(b'\n');
        row
    };
    for p in [
        b"/build/out/file".as_slice(),
        b"/build/out/has space",
        b"/build/out/file (deleted)",
    ] {
        assert_eq!(maps_in_use(&row(p), Path::new("/build/out")), Some(true));
    }
    assert_eq!(
        maps_in_use(&row(b"/build/outside/file"), Path::new("/build/out")),
        Some(false)
    );
    assert_eq!(
        maps_in_use(
            &row(b"/build/has\\012newline/file"),
            Path::new("/build/has\nnewline")
        ),
        Some(true)
    );
    assert_eq!(
        maps_in_use(
            &row(b"/build/literal\\012/file"),
            Path::new("/build/literal\\012")
        ),
        Some(true)
    );
    assert_eq!(
        maps_in_use(
            b"1000-2000 rw-p 00000000 00:00 0 [heap]\n",
            Path::new("/build")
        ),
        Some(false)
    );
    assert_eq!(maps_in_use(b"malformed", Path::new("/build")), None);
    let non_utf8 = row(b"/build/\xff/out");
    assert_eq!(maps_in_use(&non_utf8, Path::new("/build")), Some(true));
}

#[test]
fn unreadable_same_principal_maps_and_malformed_maps_fail_closed() {
    let base = scratch("unreadable");
    // A directory at maps produces a deterministic read error even as root.
    std::fs::create_dir(base.join("maps")).unwrap();
    assert!(executable_or_mapping_in_use(
        &base,
        Path::new("/output"),
        true
    ));
    assert!(!executable_or_mapping_in_use(
        &base,
        Path::new("/output"),
        false
    ));
    std::fs::remove_dir(base.join("maps")).unwrap();
    std::fs::write(base.join("maps"), "malformed").unwrap();
    assert!(executable_or_mapping_in_use(
        &base,
        Path::new("/output"),
        true
    ));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn running_generated_executable_without_output_cwd_or_fd_is_live_use() {
    let root = scratch("exe");
    let output = root.join("output");
    std::fs::create_dir(&output).unwrap();
    let executable = output.join("generated-sleeper");
    std::fs::copy("/bin/sleep", &executable).unwrap();
    let child = Child(
        std::process::Command::new(&executable)
            .arg("30")
            .current_dir("/")
            .spawn()
            .unwrap(),
    );
    let base = PathBuf::from(format!("/proc/{}", child.0.id()));
    assert_eq!(std::fs::read_link(base.join("exe")).unwrap(), executable);
    assert!(
        !std::fs::read_link(base.join("cwd"))
            .unwrap()
            .starts_with(&output)
    );
    assert!(
        std::fs::read_dir(base.join("fd"))
            .unwrap()
            .flatten()
            .all(|e| !std::fs::read_link(e.path()).is_ok_and(|p| p.starts_with(&output)))
    );
    assert!(executable_or_mapping_in_use(&base, &output, true));
    assert!(path_in_use_by_other(&output));
    drop(child);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mapped_output_without_open_fd_is_live_use() {
    const CHILD_PATH: &str = "JUSTRUST_MMAP_OUTPUT_TEST";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        use std::os::fd::AsRawFd;
        let path = PathBuf::from(path);
        let file = std::fs::File::open(&path).unwrap();
        // SAFETY: valid file descriptor and one page backed by the fixture.
        // The mapping remains owned here until the child exits or unmaps below.
        let mapping = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        assert_ne!(mapping, libc::MAP_FAILED);
        drop(file);
        std::fs::write(
            path.parent().unwrap().parent().unwrap().join("ready"),
            "mapped",
        )
        .unwrap();
        std::thread::sleep(Duration::from_secs(30));
        // SAFETY: this child owns exactly this mapping and has no aliases.
        assert_eq!(unsafe { libc::munmap(mapping, 4096) }, 0);
        return;
    }
    let root = scratch("mmap");
    let output = root.join("output");
    std::fs::create_dir(&output).unwrap();
    let mapped = output.join("mapped data");
    std::fs::write(&mapped, [0u8; 4096]).unwrap();
    let child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "procfs::output_consumer_tests::mapped_output_without_open_fd_is_live_use",
                "--test-threads=1",
            ])
            .env(CHILD_PATH, &mapped)
            .current_dir("/")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert!(
        (0..300).any(|_| {
            if root.join("ready").exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
            false
        }),
        "mapped child did not become ready"
    );
    let base = PathBuf::from(format!("/proc/{}", child.0.id()));
    assert!(
        !std::fs::read_link(base.join("exe"))
            .unwrap()
            .starts_with(&output)
    );
    assert!(
        !std::fs::read_link(base.join("cwd"))
            .unwrap()
            .starts_with(&output)
    );
    assert!(
        std::fs::read_dir(base.join("fd"))
            .unwrap()
            .flatten()
            .all(|e| !std::fs::read_link(e.path()).is_ok_and(|p| p.starts_with(&output)))
    );
    assert_eq!(
        maps_in_use(&std::fs::read(base.join("maps")).unwrap(), &output),
        Some(true)
    );
    assert!(executable_or_mapping_in_use(&base, &output, true));
    assert!(path_in_use_by_other(&output));
    drop(child);
    std::fs::remove_dir_all(root).unwrap();
}
