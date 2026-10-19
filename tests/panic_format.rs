//! Formatted panic messages: `panic!("{}", x)` and friends print the stock
//! diagnostic (location, formatted message) and abort, stdout preserved.
use std::os::unix::process::ExitStatusExt;
use std::process::Command;

fn run(name: &str, source: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("jr-panic-format-{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("main.rs");
    let exe = dir.join("main");
    let _ = std::fs::remove_file(&exe);
    std::fs::write(&src, source).unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(build.status.success(), "{build:?}");
    let out = Command::new(&exe).output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.signal(),
    )
}

#[test]
fn formatted_panic_messages_match_stock_text() {
    let cases = [
        (
            "positional",
            "fn main() {\n    let x: i32 = 7;\n    println!(\"before\");\n    if x > 3 { panic!(\"bad {} of {}\", x, 9); }\n    println!(\"after\");\n}\n",
            ":4:16:\nbad 7 of 9\n",
        ),
        (
            "capture",
            "fn main() {\n    let n: u8 = 3;\n    panic!(\"n={n:>3}\");\n}\n",
            ":3:5:\nn=  3\n",
        ),
        (
            "unreachable",
            "fn main() {\n    let n: i64 = -2;\n    unreachable!(\"state {}\", n);\n}\n",
            ":3:5:\ninternal error: entered unreachable code: state -2\n",
        ),
        (
            "escaped_only",
            "fn main() {\n    panic!(\"{{x}}\");\n}\n",
            ":2:5:\n{x}\n",
        ),
    ];
    for (name, source, tail) in cases {
        let (stdout, stderr, signal) = run(name, source);
        assert!(!stdout.contains("after"), "{name}: {stdout}");
        assert!(stderr.starts_with("panicked at "), "{name}: {stderr}");
        assert!(stderr.ends_with(tail), "{name}: {stderr:?}");
        assert_eq!(signal, Some(6), "{name}");
    }
}
