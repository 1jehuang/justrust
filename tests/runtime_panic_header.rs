//! Panic-abort diagnostic bytes agree with stock. Only the actual process ID
//! is normalized, since native and stock are necessarily different processes.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};

fn failed_diagnostic_still_aborts(exe: &std::path::Path) {
    unsafe extern "C" {
        fn syscall(number: i64, ...) -> i64;
    }
    for closed in [true, false] {
        let (peer, writer) = std::os::unix::net::UnixStream::pair().unwrap();
        drop(peer);
        let writer: std::os::fd::OwnedFd = writer.into();
        let mut command = Command::new(exe);
        command.stdout(Stdio::piped()).stderr(Stdio::from(writer));
        // SAFETY: fixed, stack-backed Linux sigaction and close syscalls in
        // the forked child only. The parent retains its own signal policy.
        unsafe {
            command.pre_exec(move || {
                let action = [0u64; 4];
                let core_limit = [0u64; 2];
                if syscall(160, 4usize, core_limit.as_ptr()) != 0
                    || syscall(13, 13usize, action.as_ptr(), 0usize, 8usize) != 0
                    || (closed && syscall(3, 2usize) != 0)
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.signal(),
            Some(6),
            "closed={closed}: {output:?}"
        );
        assert_eq!(output.stdout, b"before\n");
    }
}

fn diagnostic(exe: &std::path::Path) -> Vec<u8> {
    let child = Command::new("sh")
        .args(["-c", "ulimit -c 0; exec \"$1\"", "panic-test"])
        .arg(exe)
        .env("RUST_BACKTRACE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.signal(), Some(6), "{output:?}");
    assert_eq!(output.stdout, b"before\n");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let identity = format!("thread 'main' ({pid})");
    assert!(stderr.contains(&identity), "{stderr:?}");
    stderr
        .replace(&identity, "thread 'main' (<PID>)")
        .into_bytes()
}

#[test]
fn literal_and_formatted_panic_diagnostics_match_stock() {
    let dir = std::env::temp_dir().join(format!("jr-panic-header-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, panic) in [
        ("literal", "panic!(\"literal failure\")"),
        ("formatted", "panic!(\"bad {} of {}\", 7, 9)"),
        ("escaped", "panic!(\"{{escaped}}\")"),
    ] {
        let source = dir.join(format!("{name}.rs"));
        std::fs::write(
            &source,
            format!("fn main() {{\n println!(\"before\");\n {panic};\n}}\n"),
        )
        .unwrap();
        let stock = dir.join(format!("stock-{name}"));
        let build = Command::new("rustc")
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(&source)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        let expected = diagnostic(&stock);
        failed_diagnostic_still_aborts(&stock);
        for workers in [1, 8] {
            let exe = dir.join(format!("native-{name}-{workers}"));
            let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&source)
                .arg("--workers")
                .arg(workers.to_string())
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            assert!(build.status.success(), "{build:?}");
            assert_eq!(diagnostic(&exe), expected, "{name}, workers={workers}");
            failed_diagnostic_still_aborts(&exe);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn generated_panic_pid_does_not_authorize_unsafe_user_arguments() {
    let dir = std::env::temp_dir().join(format!("jr-panic-pid-unsafe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.rs");
    std::fs::write(&source, "fn main(){panic!(\"{}\", __justrust_linux_syscall6!(39usize,0usize,0usize,0usize,0usize,0usize,0usize));}").unwrap();
    let checked = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg("--check")
        .arg(&source)
        .output()
        .unwrap();
    assert!(!checked.status.success(), "{checked:?}");
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("E0133"),
        "{checked:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// `--panic unwind` matches stock's default strategy: pending drops run in
/// every unwound frame (direct, recursive, virtual, partially moved) and the
/// process exits 101 with the same stdout and diagnostic.
#[test]
fn unwind_runs_pending_drops_and_exits_101_like_stock() {
    let dir = std::env::temp_dir().join(format!("jr-panic-unwind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let template = r#"struct D(u8);
impl Drop for D{fn drop(&mut self){println!("drop {}",self.0);}}
trait T{fn go(&self,n:usize)->usize;}
struct S;
impl T for S{fn go(&self,n:usize)->usize{let _g=D(7);let v=[1usize,2,3];v[n]}}
fn deep(k:u32,t:&dyn T)->usize{let _x=D(k as u8+10);if k==0{t.go(9)}else{deep(k-1,t)+1}}
fn lit(n:u32)->u32{let _d=D(30);if n>2{panic!("boom {}",n)}n}
struct Pair{a:D,b:D}
fn main(){
  let _a=D(0);
  let p=Pair{a:D(1),b:D(2)};
  let moved=p.a;
  drop(moved);
  println!("{}",lit(1));
  if LIT{lit(5);}
  println!("{}",deep(2,&S));
}
"#;
    let run = |exe: &std::path::Path| {
        let output = Command::new(exe)
            .env("RUST_BACKTRACE", "0")
            .output()
            .unwrap();
        let pid_free = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .filter(|line| !line.starts_with("thread 'main'"))
            .collect::<Vec<_>>()
            .join("\n");
        (output.status.code(), output.stdout, pid_free)
    };
    for literal in [false, true] {
        let source = dir.join(format!("main-{literal}.rs"));
        std::fs::write(&source, format!("const LIT: bool = {literal};\n{template}")).unwrap();
        let stock = dir.join(format!("stock-{literal}"));
        let build = Command::new("rustc")
            .args(["--edition=2024", "--remap-path-prefix"])
            .arg(format!("{}=", dir.display()))
            .arg(&source)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        for workers in [1, 8] {
            let exe = dir.join(format!("native-{literal}-{workers}"));
            let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&source)
                .args(["--panic", "unwind", "--workers"])
                .arg(workers.to_string())
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            assert!(build.status.success(), "{build:?}");
            let native = run(&exe);
            let expected = run(&stock);
            assert_eq!(expected.0, Some(101));
            assert_eq!(native, expected, "literal={literal} workers={workers}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
