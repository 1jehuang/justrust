//! Original-source scalar const M1 workflow gates. Not full Rust const support.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn mutable_static_source_rejects_before_accessor_or_codegen() {
    let directory = std::env::temp_dir().join(format!(
        "justrust-mutable-static-boundary-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    for (index, (source, stock_valid)) in [
        ("static mut X:u8=1;fn main(){}", true),
        ("pub static mut X:u8=1;fn main(){}", true),
        ("mod inner{pub static mut X:u8=1;}fn main(){}", true),
        ("static mut X:u8=1;fn main(){unsafe{X=2;}}", true),
        ("static mut X:u8=1;fn main(){assert!(unsafe{X}==1);}", true),
        ("static mut COUNT: u32 = 5; fn bump() { unsafe { COUNT += 1; } } fn main(){ bump(); bump(); let c = unsafe { COUNT }; println!(\"{}\", c); }", true),
        ("fn runtime()->u8{1}static mut X:u8=runtime();fn main(){}", false),
    ]
    .iter()
    .enumerate()
    {
        let input = directory.join(format!("case{index}.rs"));
        fs::write(&input, source).unwrap();
        for metadata in [true, false] {
            let output_path = directory.join(format!("stock-{index}-{metadata}"));
            let mut command = Command::new(stock_rustc::stock_rustc());
            command.env_remove("RUSTC_BOOTSTRAP").arg("--edition=2024");
            if metadata {
                command.arg("--emit=metadata");
            }
            let result = command.arg(&input).arg("-o").arg(&output_path).output().unwrap();
            assert_eq!(result.status.success(), *stock_valid, "{source}: {}", String::from_utf8_lossy(&result.stderr));
            if !stock_valid {
                assert!(String::from_utf8_lossy(&result.stderr).contains("E0015"));
            }
            if !metadata && *stock_valid {
                let run = Command::new(output_path).output().unwrap();
                assert!(run.status.success());
                let expected: &[u8] = if index == 5 { b"7\n" } else { b"" };
                assert_eq!(run.stdout, expected, "stock runtime {source}");
                assert!(run.stderr.is_empty());
            }
        }
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed((*source).into(), workers, check)
                    .err()
                    .unwrap_or_else(|| panic!("accepted {source}, workers{workers}, check{check}"));
                let diagnostic = failure.to_string();
                assert!(diagnostic.contains("unsupported") && diagnostic.contains("mutable static storage"), "{diagnostic}");
                let report = failure.report.as_ref().expect("captured rejection graph");
                assert!(report.capture_complete);
                assert!(report.node_evaluations.iter().all(|(key, _)| key.kind != 103), "demanded object code for rejected source");
                assert!(!report.published_kind(101), "published resolved synthetic accessor");
                assert!(!report.published_kind(105), "published successful executable");
                assert!(failure.analyses.is_empty());
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

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
        // Stock reports E0308 and E0015 here; either rejection code is accepted.
        ("fn f(x:u8)->u8{x}const BAD:u8=f(true);fn main(){}", "E0"),
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
        let source = "static mut X:u8=1;fn main(){}";
        assert!(
            compiler::compile_observed(source.into(), workers, true).is_err(),
            "accepted unsupported {source}"
        );
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
    let directory =
        std::env::temp_dir().join(format!("justrust-constant-patterns-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    // Scalar binding sites still have an explicit unsupported boundary. The
    // existing tuple/loop/match lowering must reject refutable patterns instead
    // of capturing the constant name as a new local binding.
    for (index, (source, stock_code, native_diagnostic)) in [
        (
            "const X:u8=42;fn main(){let X:u8=1;assert!(X==1);}",
            "E0005",
            "unsupported: constant patterns",
        ),
        (
            "mod m{pub const A:u8=42;}use m::A as X;fn main(){let X:u8=1;}",
            "E0005",
            "unsupported: constant patterns",
        ),
        (
            "const X:u8=42;fn f(X:u8)->u8{X}fn main(){}",
            "E0005",
            "unsupported: constant patterns",
        ),
        (
            "const X:u8=42;fn main(){let (X,y)=(1u8,2u8);}",
            "E0005",
            "refutable literal pattern",
        ),
        (
            "const X:u8=42;fn main(){for X in 0u8..3u8{}}",
            "E0005",
            "non-exhaustive match",
        ),
        (
            "const X:u8=42;fn main(){let _=(|X:u8|->u8{X})(1u8);}",
            "E0005",
            "unsupported: constant patterns",
        ),
        (
            "const X:u8=42;fn main(){match 42u8{X=>{}};}",
            "E0004",
            "non-exhaustive match",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let input = directory.join(format!("invalid-{index}.rs"));
        fs::write(&input, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&input)
            .arg("-o")
            .arg(directory.join("invalid.rmeta"))
            .output()
            .unwrap();
        assert!(!stock.status.success(), "stock accepted {source}");
        assert!(
            String::from_utf8_lossy(&stock.stderr).contains(stock_code),
            "{}",
            String::from_utf8_lossy(&stock.stderr)
        );
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap_or_else(|| panic!("constant pattern accepted as binding: {source}"));
                assert!(
                    failure.to_string().contains(native_diagnostic),
                    "{source}: {failure}"
                );
                let report = failure.report.as_ref().expect("complete rejection graph");
                assert!(report.capture_complete);
                assert!(!report.published_kind(105));
                if check {
                    assert!(
                        !report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 103)
                    );
                }
            }
        }
    }
    for (index, source) in [
        "mod m{pub const X:u8=42;}fn main(){match 42u8{m::X=>{},_=>{}};}",
        "mod m{pub const X:u8=42;}fn classify(n:u8)->u8{match n{m::X=>1,_=>2}}fn main(){assert!(classify(42)==1);assert!(classify(41)==2);}",
    ].into_iter().enumerate() {
        let input = directory.join(format!("valid-{index}.rs"));
        let stock_binary = directory.join(format!("stock-{index}"));
        fs::write(&input, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc()).arg("--edition=2024")
            .arg(&input).arg("-o").arg(&stock_binary).output().unwrap();
        assert!(stock.status.success(), "{}", String::from_utf8_lossy(&stock.stderr));
        let expected = Command::new(stock_binary).output().unwrap();
        assert!(expected.status.success());
        for workers in [1, 8] {
            for check in [true, false] {
                let compiled = compiler::compile_observed(source.into(), workers, check)
                    .unwrap_or_else(|error| panic!("{source}: {error}"));
                assert!(compiled.report.capture_complete);
                if check {
                    assert!(compiled.executable.is_none());
                    assert!(!compiled.report.node_evaluations.iter().any(|(key, _)| key.kind == 103));
                } else {
                    let binary = directory.join(format!("native-{index}-{workers}"));
                    fs::write(&binary, compiled.executable.unwrap()).unwrap();
                    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
                    let actual = Command::new(binary).output().unwrap();
                    assert_eq!(actual.status.code(), expected.status.code());
                    assert_eq!(actual.stdout, expected.stdout);
                    assert_eq!(actual.stderr, expected.stderr);
                }
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
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
