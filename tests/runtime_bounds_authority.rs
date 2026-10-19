//! End-to-end nonmoving bounds controls and compile-time array rejection.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::process::Command;

#[test]
fn bounds_preserve_noncopy_storage_and_normal_drop() {
    let dir = std::env::temp_dir().join(format!("jr-bounds-storage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.rs");
    let source = r#"
struct S { x:i32 }
struct D { x:i32 }
impl Drop for D { fn drop(&mut self) { println!("drop {}",self.x); } }
fn inspect(a:&[S], i:usize) { let _r=&a[i]; let _c=&a[0]; }
fn mutate(a:&mut [S], i:usize) { let r=&mut a[i]; r.x=9; let r=&mut a[0]; r.x=7; }
fn main(){let _d=D{x:42};let mut a=[S{x:1},S{x:2}];inspect(&a,1);mutate(&mut a,1);let i=0usize;let _=a[i];println!("live {} {}",a[0].x,a[1].x);}
"#;
    std::fs::write(&input, source).unwrap();
    let stock = dir.join("stock");
    let built = Command::new("rustc")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(&input)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    let expected = Command::new(&stock).output().unwrap();
    assert!(expected.status.success(), "{expected:?}");
    for mode in ["off", "adaptive", "final"] {
        for workers in ["1", "8"] {
            let native = dir.join(format!("native-{mode}-{workers}"));
            for check in [true, false] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .arg(&input)
                    .args(["--opt", mode, "--workers", workers]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&native);
                }
                let built = command.output().unwrap();
                assert!(
                    built.status.success(),
                    "{mode}/{workers}/check={check}: {built:?}"
                );
                if check {
                    assert!(!native.exists());
                }
            }
            let result = Command::new(&native).output().unwrap();
            assert_eq!(result.status.code(), expected.status.code(), "{result:?}");
            assert_eq!(result.stdout, expected.stdout, "{mode}/{workers}");
            assert_eq!(result.stderr, expected.stderr, "{mode}/{workers}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fixed_array_oob_rejects_even_in_unused_borrow_or_write() {
    let dir = std::env::temp_dir().join(format!("jr-bounds-reject-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, source) in [
        ("borrow", "fn bad(){let a=[1,2];let _r=&a[2];}fn main(){}"),
        ("write", "fn bad(){let mut a=[1,2];a[2]=9;}fn main(){}"),
        (
            "nested",
            "fn bad(i:usize){let a=[[1,2],[3,4]];let _r=&a[i][2];}fn main(){}",
        ),
    ] {
        let input = dir.join(format!("{name}.rs"));
        std::fs::write(&input, source).unwrap();
        let stock = Command::new("rustc")
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(&input)
            .arg("-o")
            .arg(dir.join("stock"))
            .output()
            .unwrap();
        assert!(!stock.status.success(), "{name}: {stock:?}");
        for workers in ["1", "8"] {
            for check in [true, false] {
                let image = dir.join(format!("{name}-{workers}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.arg(&input).args(["--workers", workers]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = command.output().unwrap();
                assert!(
                    !result.status.success(),
                    "{name}/{workers}/check={check}: {result:?}"
                );
                assert!(
                    result.status.code().is_some(),
                    "compiler crashed: {result:?}"
                );
                assert!(!image.exists());
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
