//! Const generics (`const N: usize` on structs, impls and functions, `[T; N]`,
//! inferred and explicit const arguments, `N` in expressions): native run
//! output matches stock rustc. Unsupported forms are explicit diagnostics.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn run(source: &str, workers: usize, tag: &str) -> String {
    compiler::compile_observed(source.into(), workers, true)
        .unwrap_or_else(|e| panic!("check {tag}: {e}"));
    let compiled = compiler::compile_observed(source.into(), workers, false)
        .unwrap_or_else(|e| panic!("build {tag}: {e}"));
    let dir = std::env::temp_dir().join(format!(
        "justrust-const-generics-{}-{tag}-{workers}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let binary = dir.join("program");
    fs::write(&binary, compiled.executable.unwrap()).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(&binary).output().unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "{tag}: {:?}", output.status);
    String::from_utf8(output.stdout).unwrap()
}

const BUF: &str = r#"
struct Buf<const N: usize> {
    data: [u32; N],
}
impl<const N: usize> Buf<N> {
    fn len(&self) -> usize {
        N
    }
    fn sum(self) -> u32 {
        let mut s = 0;
        let mut i = 0;
        while i < N {
            s += self.data[i];
            i += 1;
        }
        s
    }
}
fn first<T: Copy, const N: usize>(a: [T; N]) -> T {
    a[0]
}
fn count<const N: usize>(_a: &[i32; N]) -> usize {
    N * 2
}
fn n<const N: usize>() -> usize {
    N + 1
}
fn main() {
    let b = Buf { data: [1, 2, 3] };
    let c: Buf<5> = Buf { data: [1; 5] };
    let bl = b.len();
    println!("{} {}", bl, b.sum());
    let cl = c.len();
    println!("{} {}", cl, c.sum());
    println!("{}", first([7u64, 8]));
    println!("{}", count::<4>(&[1, 2, 3, 4]));
    println!("{}", count(&[1, 2]));
    println!("{} {}", n::<3>(), n::<{ 2 * 5 }>());
}
"#;

#[test]
fn const_generics_match_stock_output() {
    for workers in [1, 8] {
        assert_eq!(run(BUF, workers, "buf"), "3 6\n5 5\n7\n8\n4\n4 11\n");
    }
}

#[test]
fn unsupported_and_invalid_const_generics_are_rejected() {
    for (tag, source) in [
        (
            "bool-param",
            "fn f<const B: bool>() {} fn main() { f::<true>(); }",
        ),
        ("default", "struct S<const N: usize = 3>; fn main() {}"),
        (
            "mismatch",
            "fn f<const N: usize>(_a: [u8; N], _b: [u8; N]) {} fn main() { f([1u8], [1u8, 2]); }",
        ),
    ] {
        assert!(
            compiler::compile_observed(source.into(), 1, true).is_err(),
            "{tag} must be rejected"
        );
    }
}
