//! Root slice return transport. See SLICE-RETURNS.md.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn execute(bytes: &[u8]) -> Vec<u8> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = root.join(format!("jr-slice-return-{}-{nonce}", std::process::id()));
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            let _ = fs::remove_file(&path);
            panic!("native slice return timeout");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    fs::remove_file(path).unwrap();
    assert!(output.status.success(), "{output:?}");
    output.stdout
}

#[test]
fn generic_shared_mutable_noncopy_and_subslice_returns_match_stock() {
    let source = include_str!("fixtures/slice_returns/transport.rs");
    let expected = include_bytes!("fixtures/slice_returns/transport.stdout");
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.into(), workers, check)
                .unwrap_or_else(|e| panic!("workers={workers}, check={check}: {e}"));
            assert_eq!(result.executable.is_none(), check);
            if let Some(bytes) = result.executable {
                assert_eq!(execute(&bytes), expected);
            }
        }
    }
}
