//! static mut, unsafe fn/blocks (E0133), constant patterns, raw pointers:
//! native run output matches stock rustc.
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
        "justrust-unsafe-consts-{}-{tag}-{workers}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let binary = dir.join("program");
    fs::write(&binary, compiled.executable.unwrap()).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(&binary).output().unwrap();
    assert!(output.status.success(), "{tag}: {:?}", output.status);
    String::from_utf8(output.stdout).unwrap()
}

const CASES: &[(&str, &str, &str)] = &[
    (
        "static_mut",
        "static mut COUNT: u32 = 5; fn bump() { unsafe { COUNT += 1; } } fn main(){ bump(); bump(); let c = unsafe { COUNT }; println!(\"{}\", c); }",
        "7\n",
    ),
    (
        "unsafe_fn",
        "unsafe fn twice(x: i32) -> i32 { x * 2 } fn main(){ let v = unsafe { twice(21) }; println!(\"{}\", v); }",
        "42\n",
    ),
    (
        "const_pattern",
        "const LIMIT: i32 = 10; const NEG: i32 = -LIMIT; fn f(x: i32) -> i32 { match x { LIMIT => 1, NEG => 2, _ => 3 } } fn main(){ println!(\"{} {} {}\", f(10), f(-10), f(0)); }",
        "1 2 3\n",
    ),
    (
        "raw_ptr",
        "fn main(){ let mut x = 3i32; let p = &mut x as *mut i32; unsafe { *p += 4; } let q = &x as *const i32; let v = unsafe { *q }; println!(\"{}\", v); }",
        "7\n",
    ),
];

#[test]
fn unsafe_and_const_programs_match_stock_output() {
    for workers in [1, 8] {
        for (tag, source, expected) in CASES {
            assert_eq!(run(source, workers, tag), *expected, "{tag}");
        }
    }
}

#[test]
fn unsafe_operations_outside_unsafe_are_rejected() {
    for source in [
        "unsafe fn f() {} fn main(){ f(); }",
        "static mut X: u8 = 1; fn main(){ let y = X; }",
        "fn main(){ let x = 1; let p = &x as *const i32; let y = *p; }",
    ] {
        let error = compiler::compile_observed(source.into(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.to_string().contains("E0133"), "{source}: {error}");
    }
}
