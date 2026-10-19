//! General const/static items, const fn, inline const blocks and const-expression
//! array lengths: native run output matches stock rustc semantics.
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
        "justrust-const-items-{}-{tag}-{workers}",
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

#[test]
fn general_constants_match_stock_output() {
    let cases = [
        (
            "static",
            "static S: i32 = 7; fn main(){ println!(\"{}\", S); }",
            "7\n",
        ),
        (
            "constfn",
            "const fn sq(x: i32) -> i32 { x * x } const Q: i32 = sq(4); fn main(){ println!(\"{} {}\", Q, sq(3)); }",
            "16 9\n",
        ),
        (
            "aggregate",
            "const A: [i32; 3] = [1, 2, 3]; struct P { x: i32 } const O: P = P { x: 3 }; fn main(){ let v = A[1]; let o = O.x; println!(\"{} {}\", v, o); }",
            "2 3\n",
        ),
        (
            "str",
            "const S: &str = \"hi\"; fn main(){ println!(\"{}\", S); }",
            "hi\n",
        ),
        (
            "inline",
            "fn main(){ let x = const { 3 + 4 }; println!(\"{}\", x); }",
            "7\n",
        ),
        (
            "arraylen",
            "const N: usize = 3; const M: usize = N * 2 + 1; fn main(){ let a = [5u8; M]; let b: [i32; N * 2] = [1, 2, 3, 4, 5, 6]; println!(\"{} {}\", a[6], b[5]); }",
            "5 6\n",
        ),
    ];
    for workers in [1, 8] {
        for (tag, source, expected) in cases {
            assert_eq!(run(source, workers, tag), expected, "{tag}");
        }
    }
}

#[test]
fn invalid_constants_reject_even_when_unused() {
    for (source, category) in [
        (
            "fn f()->[i32;1]{[1]} const A:[i32;1]=f(); fn main(){}",
            "E0015",
        ),
        ("const A:[i32;2]=[1,true]; fn main(){}", ""),
        ("static mut X:u8=1;fn main(){}", "unsupported"),
    ] {
        let error = compiler::compile_observed(source.into(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.to_string().contains(category), "{source}: {error}");
    }
}
