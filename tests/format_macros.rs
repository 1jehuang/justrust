//! Builtin `vec!`, `format!`, `write!` and `writeln!`: native check, build and
//! run must match stock rustc output (expected text recorded from stock).
use std::process::Command;

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
            "write_no_comma",
            "use std::fmt::Write;\nfn main() { let mut s = String::new(); let _ = write!(s); }\n",
        ),
    ] {
        assert!(run(name, source).is_err(), "{name} must be rejected");
    }
}
