//! Controlled target-lock owner, not a compiler performance workload.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
    fn statfs(path: *const i8, buffer: *mut u8) -> i32;
}

// A deterministic byte-level ledger, not a security hash. Check every existing
// shared fingerprint and dependency file while the exclusive lock is held.
fn ledger(root: &Path) -> Vec<(String, u64, u64)> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<(String, u64, u64)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(Result::unwrap).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, out);
            } else if entry.file_type().unwrap().is_file() {
                let mut file = File::open(&path).unwrap();
                let mut buffer = [0_u8; 65536];
                let mut checksum = 0xcbf29ce484222325_u64;
                let mut size = 0;
                loop {
                    let count = file.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    size += count as u64;
                    for byte in &buffer[..count] {
                        checksum = (checksum ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
                    }
                }
                out.push((
                    path.strip_prefix(root).unwrap().display().to_string(),
                    size,
                    checksum,
                ));
            }
        }
    }
    let mut out = Vec::new();
    for sub in ["deps", ".fingerprint"] {
        visit(root, &root.join(sub), &mut out);
    }
    out
}

#[test]
fn hold_shared_target() {
    let seconds = 20;
    assert_eq!(justrust_contention_fixture::answer(), 42);
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().unwrap().parent().unwrap();
    assert_eq!(profile.file_name().unwrap(), "debug");
    assert!(!profile.to_string_lossy().contains("justrust-slots"));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(profile.join(".cargo-build-lock"))
        .unwrap();
    assert_eq!(
        unsafe { flock(lock.as_raw_fd(), 2 | 4) },
        0,
        "Cargo must release its build lock before executing the holder test"
    );
    let before = ledger(profile);
    assert!(!before.is_empty());
    // Linux statfs's leading field is f_type. Allocate more than any supported
    // Linux struct statfs, aligned to a native long, without a libc dependency.
    let path = std::ffi::CString::new(profile.as_os_str().as_encoded_bytes()).unwrap();
    let mut fs_info = [0_i64; 32];
    assert_eq!(
        unsafe { statfs(path.as_ptr(), fs_info.as_mut_ptr().cast()) },
        0
    );
    println!(
        "BENCH_LOCK_READY filesystem_magic={:x} files={}",
        fs_info[0],
        before.len()
    );
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(seconds));
    assert_eq!(
        before,
        ledger(profile),
        "shared artifacts changed while exclusively locked"
    );
    println!("BENCH_SHARED_ARTIFACTS_UNCHANGED files={}", before.len());
    std::io::stdout().flush().unwrap();
    assert_eq!(unsafe { flock(lock.as_raw_fd(), 8) }, 0);
}
