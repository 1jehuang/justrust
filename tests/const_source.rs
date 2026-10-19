//! Original-source scalar const M1 workflow gates. Not full Rust const support.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn original_scalar_const_check_and_native_execution_workers1_8() {
    let source = "const A:u8=128u8<<1;const B:i8=64i8<<1;const MIN:i8=-128;const X:u8=40+2;const OK:bool=false&&(1u8/0==0);fn identity(x:u8)->u8{x}fn main(){assert!(A==0);assert!(B==MIN);assert!(identity(X)==42);assert!(!OK);}";
    for workers in [1, 8] {
        let compiled = compiler::compile_observed(source.into(), workers, false)
            .unwrap_or_else(|e| panic!("workers{workers}: {e}"));
        assert!(compiled.report.capture_complete);
        for kind in [118, 119, 120] {
            assert!(compiled.report.published_kind(kind), "missing kind{kind}");
        }
        let dir = std::env::temp_dir().join(format!(
            "justrust-source-const-{}-{workers}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("program");
        fs::write(&binary, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&binary).output().unwrap();
        assert!(
            output.status.success(),
            "workers{workers}: status{:?} stderr{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
#[test]
fn unused_initializer_faults_and_call_categories_are_not_runtime_aliases() {
    let cases = [
        ("const BAD:u8=1/0;fn main(){}", "E0080"),
        ("const BAD:i8=-128/-1;fn main(){}", "E0080"),
        ("const BAD:i8=-128%-1;fn main(){}", "E0080"),
        ("const BAD:u8=1u8<<8;fn main(){}", "E0080"),
        ("const BAD:u8=255u8+1;fn main(){}", "E0080"),
        ("fn f(x:u8)->u8{x}const BAD:u8=f(1);fn main(){}", "E0015"),
        ("fn f(x:u8)->u8{x}const BAD:u8=f(true);fn main(){}", "E0308"),
        (
            "const fn f(x:u8)->u8{x}const BAD:u8=f(1);fn main(){}",
            "unsupported",
        ),
        ("const BAD:bool=1;fn main(){}", "E0308"),
        ("const BAD:u8=MISSING;fn main(){}", "MISSING"),
    ];
    for workers in [1, 8] {
        for (source, category) in cases {
            let error = match compiler::compile_observed(source.into(), workers, true) {
                Ok(_) => panic!("accepted {source}"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains(category),
                "{source}: expected{category}, got{error}"
            );
            if source.starts_with("const fn") {
                assert!(!error.to_string().contains("E0015"));
            }
        }
    }
}
#[test]
fn names_imports_visibility_and_local_shadowing_use_value_namespace() {
    for workers in [1, 8] {
        let source = "mod m{pub const X:u8=42;}use m::X as Z;const Y:u8=Z;fn main(){let X:u8=1;assert!(X==1);assert!(Y==42);assert!(m::X==42);}";
        compiler::compile_observed(source.into(), workers, true)
            .unwrap_or_else(|error| panic!("{error}"));
        for source in [
            "static X:u8=1;fn main(){}",
            "static mut X:u8=1;fn main(){}",
            "const X:isize=1;fn main(){}",
            "const X:[u8;1]=[1];fn main(){}",
        ] {
            assert!(
                compiler::compile_observed(source.into(), workers, true).is_err(),
                "accepted unsupported {source}"
            );
        }
    }
}
#[test]
fn qualification_cycles_reject_even_skipped_references_and_preserve_partial_body_gate() {
    for workers in [1, 8] {
        for source in [
            "const A:bool=false&&A;fn main(){}",
            "const A:bool=true||A;fn main(){}",
            "const A:bool=false&&B;const B:bool=true||A;fn main(){}",
            "const A:bool=A;fn main(){}",
        ] {
            let failure = match compiler::compile_observed(source.into(), workers, true) {
                Ok(_) => panic!("accepted {source}"),
                Err(error) => error,
            };
            let report = failure.report.as_ref().expect("captured graph");
            assert!(
                report.published_kind(119),
                "typed body0 should publish before qualification wait"
            );
            assert!(
                !report.published_kind(120),
                "body0 must not launder qualification failure"
            );
            assert!(failure.to_string().contains("E0391"), "{failure}");
        }
    }
}

#[test]
fn constant_patterns_never_silently_become_local_bindings() {
    // Stock rustc1.98.1 48a229cea, edition2024, metadata: E0005 at
    // parameter/let/tuple/for/closure sites and E0004 at bare match.
    // Frozen combined source SHA2567e45fe38cd990ae4cea2abf266b94071ad313e8bbee040a8f0b5ecc48fb8f8f6.
    for workers in [1, 8] {
        for source in [
            "const X:u8=42;fn main(){let X:u8=1;assert!(X==1);}",
            "mod m{pub const A:u8=42;}use m::A as X;fn main(){let X:u8=1;}",
            "const X:u8=42;fn f(X:u8)->u8{X}fn main(){}",
            "const X:u8=42;fn main(){let (X,y)=(1u8,2u8);}",
            "const X:u8=42;fn main(){for X in 0u8..3u8{}}",
            "const X:u8=42;fn main(){let _=(|X:u8|->u8{X})(1u8);}",
            "const X:u8=42;fn main(){match 42u8{X=>{}};}",
            "mod m{pub const X:u8=42;}fn main(){match 42u8{m::X=>{},_=>{}};}",
        ] {
            let error = match compiler::compile_observed(source.into(), workers, false) {
                Ok(_) => panic!("constant pattern accepted as binding: {source}"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("unsupported: constant patterns"),
                "{source}: {error}"
            );
            assert!(
                !error.report.as_ref().unwrap().published_kind(103),
                "unsupported constant pattern emitted object"
            );
        }
    }
}
#[test]
fn usize_constants_match_frozen_stock_on_pinned_64bit_target() {
    // tests/fixtures/const_usize: rustc 1.98.1 (48a229cea) oracle stdout.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/const_usize");
    let source = fs::read_to_string(dir.join("operators.rs")).unwrap();
    let expected = fs::read(dir.join("operators.stdout")).unwrap();
    for workers in [1, 8] {
        compiler::compile_observed(source.clone(), workers, true)
            .unwrap_or_else(|e| panic!("check workers{workers}: {e}"));
        let compiled = compiler::compile_observed(source.clone(), workers, false)
            .unwrap_or_else(|e| panic!("build workers{workers}: {e}"));
        let scratch = std::env::temp_dir().join(format!(
            "justrust-const-usize-{}-{workers}",
            std::process::id()
        ));
        fs::create_dir_all(&scratch).unwrap();
        let binary = scratch.join("program");
        fs::write(&binary, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&binary).output().unwrap();
        fs::remove_dir_all(&scratch).unwrap();
        assert!(
            output.status.success(),
            "workers{workers}: {:?}",
            output.status
        );
        assert_eq!(output.stdout, expected, "workers{workers}");
    }
}
#[test]
fn usize_constant_faults_match_stock_categories_even_when_unused() {
    let cases = [
        ("const X:usize=18446744073709551615+1;fn main(){}", "E0080"),
        ("const X:usize=0-1;fn main(){}", "E0080"),
        ("const X:usize=1<<64;fn main(){}", "E0080"),
        ("const X:usize=5/0;fn main(){}", "E0080"),
        ("const X:usize=5%0;fn main(){}", "E0080"),
        ("const X:usize=2*9223372036854775808;fn main(){}", "E0080"),
        ("const X:usize=3;const Y:u32=X;fn main(){}", "E0308"),
        ("const X:u64=3;const Y:usize=X;fn main(){}", "E0308"),
        ("const X:usize=3u32;fn main(){}", "E0308"),
        ("const X:usize=-1;fn main(){}", "E0600"),
        (
            "const X:usize=18446744073709551616;fn main(){}",
            "out of range",
        ),
        ("const X:usize=Y;const Y:usize=X;fn main(){}", "E0391"),
        ("const X:isize=3;fn main(){}", "unsupported"),
    ];
    for workers in [1, 8] {
        for (source, category) in cases {
            for check in [true, false] {
                let error = match compiler::compile_observed(source.into(), workers, check) {
                    Ok(_) => panic!("accepted {source}"),
                    Err(error) => error,
                };
                assert!(
                    error.to_string().contains(category),
                    "{source}: expected {category}, got {error}"
                );
            }
        }
    }
}
