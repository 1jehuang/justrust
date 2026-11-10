//! Builtin `vec!`, `format!`, `write!` and `writeln!`: native check, build and
//! run must match stock rustc output (expected text recorded from stock).
use std::process::Command;

#[path = "support/closure_oracle.rs"]
mod radix_oracle;

#[test]
fn radix_formatting_matches_stock() {
    use radix_oracle::{Case, Expect};
    radix_oracle::check(
        "radix-formatting",
        &[
            Case {
                name: "radix_integer_widths_and_padding",
                source: r#"fn main(){println!("{:x}|{:X}|{:b}|{:o}",255u8,255u16,5u32,64u64);println!("{:#010x}|{:+#010X}|{:*>10x}|{:#010o}",42u16,42u16,42u16,42u16);println!("{:x}|{:x}|{:x}|{:x}|{:x}|{:x}",-1i8,-1i16,-1i32,-1i64,-1i128,-1isize);println!("{:b}|{:o}|{:#x}|{:#X}",-1i8,-1i16,0u128,u128::MAX);println!("{:x}|{:x}|{:x}|{:x}|{:x}|{:x}",u8::MAX,u16::MAX,u32::MAX,u64::MAX,u128::MAX,usize::MAX);}"#,
                expect: Expect::Run(
                    "ff|FF|101|100\n0x0000002a|+0x000002A|********2a|0o00000052\nff|ffff|ffffffff|ffffffffffffffff|ffffffffffffffffffffffffffffffff|ffffffffffffffff\n11111111|177777|0x0|0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\nff|ffff|ffffffff|ffffffffffffffff|ffffffffffffffffffffffffffffffff|ffffffffffffffff\n",
                ),
            },
            Case {
                name: "radix_custom_traits_format_write_and_references",
                source: r#"use std::fmt;use std::fmt::Write;struct N;impl fmt::LowerHex for N{fn fmt(&self,f:&mut fmt::Formatter)->fmt::Result{f.pad_integral(true,"0x","2a")}}fn emit<T:fmt::LowerHex>(x:&T)->String{format!("{:#08x}",x)}fn main(){println!("{}",emit(&N));println!("{:x}|{:X}|{:b}|{:o}",&42u8,&42u8,&42u8,&42u8);let mut s=String::new();write!(s,"[{:#06x}|{:^8X}|{:08b}|{:#o}]",42u8,42u8,5u8,64u8).unwrap();println!("{}",s);println!("{}",format!("{:x}/{:X}/{:b}/{:o}",u128::MAX,255u8,5u8,64u8));}"#,
                expect: Expect::Run(
                    "0x00002a\n2a|2A|101010|52\n[0x002a|   2A   |00000101|0o100]\nffffffffffffffffffffffffffffffff/FF/101/100\n",
                ),
            },
            Case {
                name: "unused_radix_float",
                source: r#"fn unused(){let _=format!("{:x}",1.0f64);}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_radix_str",
                source: r#"fn unused(){println!("{:b}","text");}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_radix_missing_custom_impl",
                source: r#"struct N;fn unused(){let _=format!("{:o}",N);}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

fn run(name: &str, source: &str) -> Result<(String, String), String> {
    run_with_workers(name, source, 1)
}

fn run_with_workers(name: &str, source: &str, workers: usize) -> Result<(String, String), String> {
    let dir = std::env::temp_dir().join(format!("jr-format-macros-{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("main.rs");
    let exe = dir.join("main");
    let _ = std::fs::remove_file(&exe);
    std::fs::write(&src, source).unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg(&src)
        .arg("--workers")
        .arg(workers.to_string())
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    if !build.status.success() {
        return Err(String::from_utf8_lossy(&build.stderr).into_owned());
    }
    let out = Command::new(&exe).output().unwrap();
    assert!(out.status.success(), "{name}: {out:?}");
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

#[test]
fn independent_format_call_conversions_match_stock() {
    let source = include_str!("fixtures/format_macros_basic.rs");
    let dir = std::env::temp_dir().join(format!("jr-format-stock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("main.rs");
    let executable = dir.join("stock");
    std::fs::write(&src, source).unwrap();
    let build = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&src)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(build.status.success(), "{build:?}");
    let expected = Command::new(&executable).output().unwrap();
    assert!(expected.status.success(), "{expected:?}");
    let expected = (
        String::from_utf8(expected.stdout).unwrap(),
        String::from_utf8(expected.stderr).unwrap(),
    );
    for workers in [1, 8] {
        let checked = justrust_engine::compiler::compile(source.into(), workers, true)
            .unwrap_or_else(|error| panic!("workers={workers}, CHECK: {error}"));
        assert!(checked.executable.is_none());
        let actual = run_with_workers(&format!("independent-{workers}"), source, workers)
            .unwrap_or_else(|error| panic!("workers={workers}, BUILD: {error}"));
        assert_eq!(actual, expected, "workers={workers}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn format_and_write_match_stock() {
    let (stdout, stderr) = run("basic", include_str!("fixtures/format_macros_basic.rs")).unwrap();
    assert_eq!(
        stdout,
        "x=5 p=P { x: 1, y: 2 } n=bob sum=6\nlit only\n[     5|ab  | 1.5 ]\na1 b5\n\n[1, 2, 3] 3\n"
    );
    assert_eq!(stderr, "err 34\n");
    let (stdout, _) = run("display", include_str!("fixtures/format_macros_display.rs")).unwrap();
    assert_eq!(stdout, "(1, -2)\n[(1, -2)]\nlen=2\nend\n(1, -2)\n");
    // Run-time width and precision operands: `width$`, `N$`, `.name$`, `.*`.
    for workers in [1, 8] {
        let (stdout, stderr) = run_with_workers(
            &format!("counts-{workers}"),
            include_str!("fixtures/format_macros_counts.rs"),
            workers,
        )
        .unwrap();
        assert_eq!(
            stdout,
            "    r|1.23|(\n    1,\n    \"a\",\n)\n[     42] [42     ] [  42   ]\n[   3.14] [   3.14] [3.1416]\n[0000005] [+3.142] [**ab***]\n   a|4      |x\n0009-2.5|   true\n"
        );
        assert_eq!(stderr, "[      e]\n");
    }
}

/// f32/f64 shortest, fixed, Debug and `{:e}` output (text recorded from
/// stock rustc into the `.stdout` file).
#[test]
fn float_formatting_matches_stock() {
    for workers in [1, 8] {
        let (stdout, _) = run_with_workers(
            &format!("floats-{workers}"),
            include_str!("fixtures/format_macros_floats.rs"),
            workers,
        )
        .unwrap();
        assert_eq!(
            stdout,
            include_str!("fixtures/format_macros_floats.stdout"),
            "workers={workers}"
        );
    }
}

#[test]
fn vec_macro_forms_match_stock() {
    let (stdout, _) = run(
        "vec",
        "#[derive(Clone)]\nstruct S(i32);\nfn main() {\n    let v = vec![1, 2, 3];\n    let w = vec![0u8; 4];\n    let s = vec![S(1); 2];\n    let o = vec![7];\n    let e: Vec<i32> = vec![];\n    let mut m = vec![1i64, 2,];\n    m.push(3);\n    println!(\"{} {} {} {} {} {}\", v.len(), w.len(), s.len(), o.len(), e.len(), m.len());\n    println!(\"{:?}\", v);\n}\n",
    )
    .unwrap();
    assert_eq!(stdout, "3 4 2 1 0 3\n[1, 2, 3]\n");
}

#[test]
fn invalid_format_macros_are_rejected() {
    for (name, source) in [
        (
            "missing_display",
            "struct N;\nfn main() { let _s = format!(\"{}\", N); }\n",
        ),
        ("unused_arg", "fn main() { let _s = format!(\"x\", 1); }\n"),
        (
            "missing_arg",
            "fn main() { let _s = format!(\"{} {}\", 1); }\n",
        ),
        ("no_string", "fn main() { let _s = format!(); }\n"),
        (
            "missing_lower_exp",
            "fn unused() { println!(\"{:e}\", \"text\"); }\nfn main() {}\n",
        ),
        (
            "missing_upper_exp",
            "struct N;\nfn main() { let _s = format!(\"{:E}\", N); }\n",
        ),
        (
            "width_not_usize",
            "fn main() { let w = 3i32; let _s = format!(\"{:w$}\", 1); }\n",
        ),
        (
            "star_precision_missing_value",
            "fn main() { let _s = format!(\"{:.*}\", 2); }\n",
        ),
        (
            "width_out_of_range",
            "fn main() { let _s = format!(\"{:3$}\", 1, 2); }\n",
        ),
        (
            "write_no_comma",
            "use std::fmt::Write;\nfn main() { let mut s = String::new(); let _ = write!(s); }\n",
        ),
    ] {
        assert!(run(name, source).is_err(), "{name} must be rejected");
    }
}

#[test]
fn option_result_combinators_and_lines_match_stock() {
    for workers in [1, 8] {
        let (stdout, _) = run_with_workers(
            &format!("std-option-result-lines-{workers}"),
            include_str!("fixtures/std_option_result_lines.rs"),
            workers,
        )
        .unwrap();
        assert_eq!(
            stdout,
            "0:[alpha] 5\n1:[beta] 4\n2:[] 0\n3:[gamma] 5\n[\"one\", \"two\"] 0\n[\"a\", \"b\\r\"]\n8 -1\n5 7\n4 0 Some(\"y\") None\n[]\n9 0\n3 3\n3 0\nOk(9) Err(\"bad\")\n"
        );
    }
    for (name, source) in [
        (
            "map-or-arity",
            "fn main() {\n    let a: Option<i32> = Some(1);\n    let _ = a.map_or(0);\n}\n",
        ),
        (
            "lines-arity",
            "fn main() {\n    let _ = \"a\\nb\".lines(1);\n}\n",
        ),
        (
            "result-clone-bound",
            "struct N;\nfn main() {\n    let r: Result<N, N> = Ok(N);\n    let _ = r.clone();\n}\n",
        ),
    ] {
        assert!(run(name, source).is_err(), "{name} must be rejected");
    }
}
