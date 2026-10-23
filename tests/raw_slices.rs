//! Canonical raw-parts construction. See RAW-SLICES.md.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn inferred_generic_constructor_signatures_check_at_both_worker_counts() {
    for source in [
        "fn shared<'a,T>(p:*const T,n:usize)->&'a[T]{unsafe{std::slice::from_raw_parts(p,n)}}fn main(){}",
        "fn unique<'a,T>(p:*mut T,n:usize)->&'a mut[T]{unsafe{std::slice::from_raw_parts_mut(p,n)}}fn main(){}",
    ] {
        for workers in [1, 8] {
            let output = compiler::compile(source.into(), workers, true).unwrap();
            assert!(output.executable.is_none());
        }
    }
}

#[test]
fn pointer_cast_targets_preserve_nominal_graph_identity() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "raw-cast-targets",
        &[
            Case {
                name: "unchanged_transport",
                source: include_str!("fixtures/raw_slices/transport.rs"),
                expect: Expect::Run(include_str!("fixtures/raw_slices/transport.stdout")),
            },
            Case {
                name: "neighboring_heap_raw_read_write",
                source: include_str!("fixtures/heap/raw_read_write.rs"),
                expect: Expect::Run(include_str!("fixtures/heap/raw_read_write.stdout")),
            },
            Case {
                name: "generic_nominal_and_array_targets",
                source: r#"struct Item<T>{value:T} fn raw<T>(a:&mut[T;2])->*mut T{a as *mut[T;2] as *mut T} fn main(){let mut a=[Item{value:3i32},Item{value:7i32}];let p=raw(&mut a);unsafe{(*p).value=11;}let q=&a as *const[Item<i32>;2] as *const Item<i32>;println!("{} {}",unsafe{(*q).value},a[1].value);}"#,
                expect: Expect::Run("11 7\n"),
            },
        ],
    );
}

#[test]
fn pointer_cast_boundaries_still_reject() {
    use std::io::Write;
    for source in [
        "mod a{pub struct Item{pub value:i32}}mod b{pub struct Item{pub value:i32}}fn main(){let mut a=[a::Item{value:1}];let _=&mut a as *mut[b::Item;1];}",
        "struct Item{value:i32}fn main(){let mut a=[Item{value:1}];let _=&mut a as *mut[Item;2];}",
        "struct Item{value:i32}fn main(){let a=Item{value:1};let _=&a as *mut Item;}",
        "fn bad(a:&mut[i32]){let _=a as *mut i32;}fn main(){}",
    ] {
        let mut stock =
            Command::new(std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into()))
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--emit=metadata",
                    "--crate-name=cast_boundary",
                    "-o",
                    "/dev/null",
                    "-",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
        stock
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = stock.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("E0606"));
        for workers in [1, 8] {
            for check in [true, false] {
                let error = match compiler::compile(source.into(), workers, check) {
                    Err(error) => error,
                    Ok(_) => {
                        panic!("invalid pointer cast accepted workers={workers} check={check}")
                    }
                };
                assert!(error.to_string().contains("E0606"), "{error}");
            }
        }
    }
}

fn execute(bytes: &[u8]) -> Vec<u8> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = root.join(format!("jr-raw-slice-{}-{nonce}", std::process::id()));
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
            panic!("native raw slice timeout");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    fs::remove_file(path).unwrap();
    assert!(output.status.success(), "{output:?}");
    output.stdout
}

#[test]
fn shared_mutable_empty_noncopy_and_order_match_stock() {
    let source = include_str!("fixtures/raw_slices/transport.rs");
    let expected = include_bytes!("fixtures/raw_slices/transport.stdout");
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.into(), workers, check)
                .unwrap_or_else(|e| panic!("workers={workers},check={check}: {e}"));
            assert_eq!(result.executable.is_none(), check);
            if let Some(bytes) = result.executable {
                assert_eq!(execute(&bytes), expected);
            }
        }
    }
}
