//! Unsafe fn/blocks (E0133), constant patterns, and raw pointers retain their
//! native controls. Mutable statics are an explicit unsupported boundary:
//! the old fixed-address accessor was an unsound approximation, not support.
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
            if *tag == "static_mut" {
                // This unchanged source has pinned-stock CHECK/BUILD/run coverage
                // in const_source; native admission must not invent storage.
                for check in [true, false] {
                    let error = compiler::compile_observed((*source).into(), workers, check)
                        .err()
                        .expect("mutable statics must not publish a successful compilation");
                    assert!(error.to_string().contains("unsupported"), "{error}");
                    assert!(
                        error.to_string().contains("mutable static storage"),
                        "{error}"
                    );
                }
                continue;
            }
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
        if source.starts_with("static mut") {
            assert!(
                error.to_string().contains("unsupported"),
                "{source}: {error}"
            );
            assert!(
                error.to_string().contains("mutable static storage"),
                "{source}: {error}"
            );
        } else {
            assert!(error.to_string().contains("E0133"), "{source}: {error}");
        }
    }
}

#[test]
fn denied_unsafe_op_in_unsafe_fn_matches_stock() {
    let source = r#"#![deny(unsafe_op_in_unsafe_fn)]
unsafe fn read(p: *const i32) -> i32 {
    unsafe { *p }
}
unsafe fn twice(p: *const i32) -> i32 {
    unsafe { read(p) * 2 }
}
unsafe fn nothing() -> i32 { 7 }
fn main() {
    let x = 21;
    let v = unsafe { twice(&x) + nothing() };
    println!("{v}");
}
"#;
    for workers in [1, 8] {
        assert_eq!(run(source, workers, "unsafe_op"), "49\n");
    }
    for source in [
        "#![deny(unsafe_op_in_unsafe_fn)] unsafe fn f() {} unsafe fn g() { f() } fn main() { unsafe { g() } }",
        "#![forbid(unsafe_op_in_unsafe_fn)] unsafe fn r(p: *const u8) -> u8 { *p } fn main() { let x = 1u8; unsafe { r(&x); } }",
    ] {
        let error = compiler::compile_observed(source.into(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.to_string().contains("E0133"), "{source}: {error}");
    }
    for source in [
        "#![deny(unsafe_op_in_unsafe_fn)] #[allow(unsafe_op_in_unsafe_fn)] unsafe fn f() {} fn main() {}",
        "#![deny(rust_2024_compatibility)] fn main() {}",
        "mod m { #![deny(unsafe_op_in_unsafe_fn)] } fn main() {}",
    ] {
        let error = compiler::compile_observed(source.into(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(
            error.to_string().contains("unsupported"),
            "{source}: {error}"
        );
    }
}

#[test]
fn denied_trivial_casts_match_stock() {
    let source = r#"#![deny(trivial_casts, trivial_numeric_casts)]
fn widen(x: u8) -> u32 {
    x as u32
}
fn main() {
    let bytes = [3u8, 200];
    let total: u32 = bytes.iter().map(|b| widen(*b)).sum();
    let x = 7;
    let wide = x as u64;
    let c = 65u8 as char;
    let back = c as u32;
    let f = 2.5f32 as f64;
    println!("{total} {wide} {c} {back} {f}");
}
"#;
    for workers in [1, 8] {
        assert_eq!(run(source, workers, "trivial_casts"), "203 7 A 65 2.5\n");
    }
    for source in [
        "#![deny(trivial_numeric_casts)] fn main() { let x = 1u8; let _ = x as u8; }",
        "#![deny(trivial_numeric_casts)] fn main() { let v = [1u32]; let _ = v.len() as usize; }",
        "#![deny(trivial_numeric_casts)] fn main() { let x = 1; let y: u8 = x; let _ = x as u8; let _ = y; }",
        "#![forbid(trivial_casts)] fn main() { let x = 5u8; let _ = &x as *const u8; }",
        "#![deny(trivial_casts)] fn main() { let _ = 'a' as char; }",
    ] {
        let error = compiler::compile_observed(source.into(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.to_string().contains("trivial"), "{source}: {error}");
    }
    for source in [
        "#![deny(trivial_casts)] #[allow(trivial_casts)] fn f() {} fn main() {}",
        "mod m { #![deny(trivial_numeric_casts)] } fn main() {}",
    ] {
        let error = compiler::compile_observed(source.into(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(
            error.to_string().contains("unsupported"),
            "{source}: {error}"
        );
    }
    // Without the lint, `char as char` is an ordinary (trivial) cast.
    assert_eq!(
        run(
            "fn main() { let c = 'z' as char; println!(\"{c}\"); }",
            1,
            "char_cast"
        ),
        "z\n"
    );
}

#[path = "support/oracle.rs"]
mod target_feature_oracle;

#[test]
fn baseline_target_features_preserve_unsafe_scalar_semantics() {
    use target_feature_oracle::{Case, Expect};
    let cases = [
        Case {
            name: "baseline_feature_scalar",
            source: "#[target_feature(enable=\"sse2\")] unsafe fn calculate(x:u32)->u32{x*3+2} fn main(){println!(\"{}\",unsafe{calculate(7)});}",
            expect: Expect::Run("23\n"),
        },
        Case {
            name: "baseline_feature_module",
            source: "mod arithmetic { #[target_feature(enable=\"sse2\")] pub unsafe fn compute(x:u32)->u32{x*3+2} } fn main(){println!(\"{}\",unsafe{arithmetic::compute(7)});}",
            expect: Expect::Run("23\n"),
        },
        Case {
            name: "baseline_feature_set",
            source: "#[target_feature(enable=\"fxsr,sse,sse2,sse2\")] unsafe fn calculate(x:u32)->u32{x*3+2} fn main(){println!(\"{}\",unsafe{calculate(7)});}",
            expect: Expect::Run("23\n"),
        },
        Case {
            name: "target_feature_call_requires_unsafe",
            source: "#[target_feature(enable=\"sse2\")] unsafe fn calculate(x:u32)->u32{x+1} fn main(){println!(\"{}\",calculate(7));}",
            expect: Expect::Invalid("E0133"),
        },
        Case {
            name: "unused_invalid_target_feature",
            source: "#[target_feature(enable=\"not_a_real_x86_extension\")] unsafe fn unused(){} fn main(){}",
            expect: Expect::Invalid("not valid for this target"),
        },
        Case {
            name: "target_feature_whitespace_name_invalid",
            source: "#[target_feature(enable=\"sse2, sse\")] unsafe fn unused(){} fn main(){}",
            expect: Expect::Invalid("not valid for this target"),
        },
        Case {
            name: "target_feature_configured_inline_invalid",
            source: "#[target_feature(enable=\"sse2\")] #[cfg_attr(all(),inline(always))] unsafe fn unused(){} fn main(){}",
            expect: Expect::Invalid("inline(always)"),
        },
        Case {
            name: "target_feature_inline_always_invalid",
            source: "#[target_feature(enable=\"sse2\")] #[inline(always)] unsafe fn unused(){} fn main(){}",
            expect: Expect::Invalid("inline(always)"),
        },
    ];
    target_feature_oracle::check("baseline_target_features", &cases);
    let stock = target_feature_oracle::pinned_stock::stock_rustc();
    let directory = std::env::temp_dir().join(format!(
        "justrust-feature-boundaries-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.rs");
    for source in [
        "#[target_feature(enable=\"sse2\")] const unsafe fn unused(){} fn main(){}",
        "#[target_feature(enable=\"avx2\")] unsafe fn unused(){} fn main(){}",
        "#[target_feature(enable=\"sse2\")] fn unused(){} fn main(){}",
    ] {
        fs::write(&input, source).unwrap();
        let output = Command::new(&stock)
            .env_remove("RUSTC_BOOTSTRAP")
            .args([
                "--edition=2024",
                "--emit=metadata",
                "--crate-name=feature_boundary",
                "-Awarnings",
            ])
            .arg(&input)
            .arg("-o")
            .arg(directory.join("output.rmeta"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "valid unsupported feature control: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for workers in [1, 8] {
        for case in &cases {
            for check in [true, false] {
                match case.expect {
                    Expect::Run(expected) if !check => {
                        assert_eq!(run(case.source, workers, case.name), expected)
                    }
                    Expect::Run(_) => {
                        let compiled =
                            compiler::compile_observed(case.source.into(), workers, true).unwrap();
                        assert!(compiled.executable.is_none());
                        assert!(
                            compiled
                                .report
                                .node_evaluations
                                .iter()
                                .all(|(key, _)| ![103, 105].contains(&key.kind))
                        );
                    }
                    Expect::Invalid(_) => {
                        let error = compiler::compile_observed(case.source.into(), workers, check)
                            .err()
                            .expect("invalid target feature accepted");
                        assert!(!error.to_string().contains("unsupported"), "{error}");
                    }
                }
            }
        }
        for source in [
            "#[target_feature(enable=\"sse2\",enable=\"sse\")] unsafe fn unused(){} fn main(){}",
            "#[target_feature(enable=concat!(\"sse\",\"2\"))] unsafe fn unused(){} fn main(){}",
            "#[target_feature(enable=\"sse2\")] const unsafe fn unused(){} fn main(){}",
            "#[target_feature(enable=\"avx2\")] unsafe fn unused(){} fn main(){}",
            "#[target_feature(enable=\"amx-tile\")] unsafe fn unused(){} fn main(){}",
            "#[target_feature(enable=\"sse2\")] fn unused(){} fn main(){}",
            "#[target_feature(enable=\"sse2\")] unsafe fn f(){let x=(|x:u32|->u32{x+1})(1);} fn main(){}",
            "struct S{} impl S{#[target_feature(enable=\"sse2\")] unsafe fn f(){}} fn main(){}",
        ] {
            for check in [true, false] {
                let error = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .expect("unsupported target feature route accepted");
                assert!(
                    error.to_string().contains("unsupported"),
                    "{source}: {error}"
                );
            }
        }
    }
}
