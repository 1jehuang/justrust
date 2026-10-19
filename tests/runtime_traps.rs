//! Source-backed arithmetic diagnostics survive both stack and SSA lowering.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

fn diagnostic(exe: &std::path::Path) -> Vec<u8> {
    let child = Command::new("sh")
        .args(["-c", "ulimit -c 0; exec \"$1\"", "runtime-trap-test"])
        .arg(exe)
        .env("RUST_BACKTRACE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.signal(), Some(6), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let identity = format!("thread 'main' ({pid})");
    assert!(stderr.contains(&identity), "{stderr:?}");
    stderr
        .replace(&identity, "thread 'main' (<PID>)")
        .into_bytes()
}

#[test]
fn arithmetic_panics_keep_stock_messages_locations_and_abort_after_optimization() {
    let dir = std::env::temp_dir().join(format!("jr-runtime-traps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, ty, expression, left, right) in [
        ("add", "i32", "left + right", "2147483647", "1"),
        ("sub", "u32", "left - right", "0", "1"),
        ("mul", "i32", "left * right", "2147483647", "2"),
        ("divzero", "i32", "left / right", "7", "0"),
        ("remzero", "i32", "left % right", "7", "0"),
        ("divoverflow", "i32", "left / right", "-2147483648", "-1"),
        ("removerflow", "i32", "left % right", "-2147483648", "-1"),
        ("shl", "u32", "left << right", "1", "32"),
        ("shr", "i32", "left >> right", "1", "-1"),
        ("neg", "i32", "-left", "-2147483648", "0"),
        (
            "wideadd",
            "u128",
            "left + right",
            "340282366920938463463374607431768211455",
            "1",
        ),
        ("widediv", "i128", "left / right", "7", "0"),
        (
            "wideneg",
            "i128",
            "-left",
            "-170141183460469231731687303715884105728",
            "0",
        ),
    ] {
        if std::env::var("JR_RUNTIME_TRAP_CASE").is_ok_and(|case| case != name) {
            continue;
        }
        let source = dir.join(format!("{name}.rs"));
        std::fs::write(&source, format!(
            "fn operation(left:{ty},right:{ty})->{ty}{{\n    {expression}\n}}\nfn main(){{let _value=operation({left},{right});}}\n"
        )).unwrap();
        let stock = dir.join(format!("stock-{name}"));
        let build = Command::new("rustc")
            .args([
                "--edition=2024",
                "-Cpanic=abort",
                "-Coverflow-checks=yes",
                "-Copt-level=0",
            ])
            .arg(&source)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        let expected = diagnostic(&stock);
        for mode in ["off", "adaptive", "final"] {
            for workers in [1, 8] {
                let exe = dir.join(format!("native-{name}-{mode}-{workers}"));
                let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                    .arg(&source)
                    .args(["--opt", mode, "--workers"])
                    .arg(workers.to_string())
                    .arg("-o")
                    .arg(&exe)
                    .output()
                    .unwrap();
                assert!(build.status.success(), "{name}/{mode}/{workers}: {build:?}");
                assert_eq!(diagnostic(&exe), expected, "{name}/{mode}/{workers}");
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compound_updates_do_not_borrow_their_rhs_or_index_panic_site() {
    let dir = std::env::temp_dir().join(format!("jr-compound-trap-sites-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in [
        (
            "rhs",
            "fn f(a:i32,b:i32){let mut x=2147483647i32;x+=a+b;}fn main(){f(1,0);}",
        ),
        (
            "index",
            "fn f(a:&mut [i32;1],i:usize){a[i+0]+=1;}fn main(){let mut a=[2147483647i32];f(&mut a,0);}",
        ),
    ] {
        let source = dir.join(format!("{name}.rs"));
        std::fs::write(&source, text).unwrap();
        for mode in ["off", "adaptive", "final"] {
            for workers in [1, 8] {
                let exe = dir.join(format!("{name}-{mode}-{workers}"));
                let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                    .arg(&source)
                    .args(["--opt", mode, "--workers"])
                    .arg(workers.to_string())
                    .arg("-o")
                    .arg(&exe)
                    .output()
                    .unwrap();
                assert!(build.status.success(), "{name}/{mode}/{workers}: {build:?}");
                let output = Command::new(&exe).output().unwrap();
                // Compound-operation diagnostics remain a separate open feature.
                // Its synthetic update must not impersonate successful arithmetic
                // on the RHS/index merely because their diagnostic origins match.
                assert_eq!(
                    output.status.code(),
                    Some(101),
                    "{name}/{mode}/{workers}: {output:?}"
                );
                assert!(
                    output.stderr.is_empty(),
                    "{name}/{mode}/{workers}: {output:?}"
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
