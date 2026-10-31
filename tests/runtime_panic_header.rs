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
fn aligned(value:&u128){assert_eq!(value as *const u128 as usize%16usize,0usize);}
impl Drop for D{fn drop(&mut self){let local=17u128;aligned(&local);println!("drop {}",self.0);}}
trait T{fn go(&self,n:usize)->usize;}
struct S;
impl T for S{fn go(&self,n:usize)->usize{let _g=D(7);let wide=[19u128,23u128];aligned(&wide[0]);aligned(&wide[1]);let v=[1usize,2,3];v[n]}}
fn deep(k:u32,t:&dyn T)->usize{let _x=D(k as u8+10);let local=k as u128;aligned(&local);if k==0{t.go(9)}else{deep(k-1,t)+1}}
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

#[test]
fn exported_c_unwind_requires_initialized_runtime_even_without_query() {
    let dir = std::env::temp_dir().join(format!("jr-external-unwind-{}", std::process::id()));
    let sources = dir.join("source");
    std::fs::create_dir_all(&sources).unwrap();
    let source = sources.join("main.rs");
    std::fs::write(
        &source,
        r#"
#[unsafe(no_mangle)] pub extern "C" fn exported(fail:bool)->u32 {
    if fail { panic!("first panic without query"); }
    7
}
fn main(){println!("{}",exported(false));}
"#,
    )
    .unwrap();
    let stock = dir.join("stock");
    let compiled =
        Command::new(std::env::var_os("JR_STOCK_RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "-Cpanic=unwind"])
            .arg(&source)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    let expected = Command::new(&stock).output().unwrap();
    assert!(expected.status.success(), "{expected:?}");
    assert_eq!(expected.stdout, b"7\n");
    for workers in [1, 8] {
        for emit_object in [false, true] {
            let output = dir.join(format!("unsupported-{workers}-{emit_object}"));
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(&source)
                .args(["--panic", "unwind", "--workers"])
                .arg(workers.to_string());
            if emit_object {
                command.arg("--emit-object");
            }
            let checked = Command::new(command.get_program())
                .args(command.get_args())
                .arg("--check")
                .output()
                .unwrap();
            assert!(checked.status.success(), "CHECK {command:?}: {checked:?}");
            assert!(!output.exists());
            let build = command.arg("-o").arg(&output).output().unwrap();
            assert!(!build.status.success(), "{command:?}: {build:?}");
            let diagnostic = String::from_utf8_lossy(&build.stderr);
            assert!(
                diagnostic.contains("unsupported") && diagnostic.contains("initialized"),
                "{command:?}: {build:?}"
            );
            assert!(!output.exists(), "unsupported BUILD published output");
        }
        let executable = dir.join(format!("abort-{workers}"));
        let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .args(["--panic", "abort", "--workers"])
            .arg(workers.to_string())
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        let actual = Command::new(&executable).output().unwrap();
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(actual.stdout, expected.stdout);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn c_constructor_exported_panicking_query_requires_runtime_entry_support() {
    let dir = std::env::temp_dir().join(format!("jr-panicking-constructor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("query.rs");
    std::fs::write(
        &source,
        r#"
fn helper()->bool { std::thread::panicking() }
#[unsafe(no_mangle)] pub extern "C" fn exported_panicking()->bool { helper() }
fn main(){println!("main");}
"#,
    )
    .unwrap();
    let constructor = dir.join("constructor.c");
    std::fs::write(
        &constructor,
        r#"
extern _Bool exported_panicking(void);
__attribute__((constructor)) static void query_before_main(void) {
    if (exported_panicking()) __builtin_trap();
}
"#,
    )
    .unwrap();
    let object = dir.join("constructor.o");
    let compiled = Command::new("cc")
        .arg("-c")
        .arg(&constructor)
        .arg("-o")
        .arg(&object)
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    let stock = dir.join("stock");
    let compiled =
        Command::new(std::env::var_os("JR_STOCK_RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(format!("-Clink-arg={}", object.display()))
            .arg(&source)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    let expected = Command::new(&stock).output().unwrap();
    assert!(expected.status.success(), "{expected:?}");
    assert_eq!(expected.stdout, b"main\n");
    for workers in [1, 8] {
        for opt in ["off", "final"] {
            let executable = dir.join(format!("native-{workers}-{opt}"));
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(&source)
                .args(["--opt", opt, "--workers"])
                .arg(workers.to_string())
                .arg("--native-object")
                .arg(&object);
            let checked = Command::new(command.get_program())
                .args(command.get_args())
                .arg("--check")
                .output()
                .unwrap();
            assert!(checked.status.success(), "CHECK {command:?}: {checked:?}");
            assert!(!executable.exists(), "CHECK published an executable");
            let build = command.arg("-o").arg(&executable).output().unwrap();
            assert!(!build.status.success(), "{command:?}: {build:?}");
            let diagnostic = String::from_utf8_lossy(&build.stderr);
            assert!(
                diagnostic.contains("unsupported") && diagnostic.contains("external C"),
                "{command:?}: {build:?}"
            );
            assert!(
                !executable.exists(),
                "unsupported BUILD published an executable"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn standalone_object_panicking_query_requires_initialized_native_entry() {
    let dir = std::env::temp_dir().join(format!("jr-panicking-object-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("query.rs");
    std::fs::write(
        &source,
        "#[unsafe(no_mangle)] pub extern \"C\" fn is_panicking()->bool { std::thread::panicking() } fn main() {}",
    )
    .unwrap();
    for workers in [1, 8] {
        let object = dir.join(format!("query-{workers}.o"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
        command
            .arg(&source)
            .args(["--emit-object", "--workers"])
            .arg(workers.to_string());
        let checked = Command::new(command.get_program())
            .args(command.get_args())
            .arg("--check")
            .output()
            .unwrap();
        assert!(checked.status.success(), "CHECK {command:?}: {checked:?}");
        assert!(!object.exists(), "CHECK published object bytes");
        let build = command.arg("-o").arg(&object).output().unwrap();
        assert!(!build.status.success(), "{command:?}: {build:?}");
        let diagnostic = String::from_utf8_lossy(&build.stderr);
        assert!(
            diagnostic.contains("unsupported") && diagnostic.contains("initialized native entry"),
            "{command:?}: {build:?}"
        );
        assert!(!object.exists(), "unsupported BUILD published object bytes");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn thread_panicking_state_matches_stock_across_cleanup_and_callees() {
    let dir = std::env::temp_dir().join(format!("jr-panicking-state-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let normal = r#"
fn query()->bool { std::thread::panicking() }
fn work(a:u64,b:u64,c:u64,d:u64,e:u64,f:u64,g:u64,h:u64,i:u64,j:u64,k:u64,l:u64)->u64 {
    let a=a+1;let b=b+2;let c=c+3;let d=d+4;let e=e+5;let f=f+6;
    let g=g+7;let h=h+8;let i=i+9;let j=j+10;let k=k+11;let l=l+12;
    a+b+c+d+e+f+g+h+i+j+k+l
}
struct D;
impl Drop for D { fn drop(&mut self) { println!("normal drop {}",query()); } }
struct Wrap { value:D }
trait T { fn value(&self)->bool; }
impl T for Wrap { fn value(&self)->bool { query() } }
fn main(){ println!("entry {}",query());println!("work {}",work(1,2,3,4,5,6,7,8,9,10,11,12));let d=Wrap{value:D};let view:&dyn T=&d;assert!(!view.value());drop(d);println!("after {}",query()); }
"#;
    let unwind = r#"
fn query()->bool { std::thread::panicking() }
fn nested(n:u32)->bool { if n==0 { query() } else { nested(n-1) } }
struct D(u8);
impl Drop for D { fn drop(&mut self) { println!("drop {} {} {}",self.0,query(),nested(3)); } }
fn fail(){let _inner=D(2);panic!("first panic");}
fn main(){println!("entry {}",query());let _outer=D(1);fail();println!("unreachable");}
"#;
    let double = r#"
fn query()->bool { std::thread::panicking() }
struct D(u8);
impl Drop for D { fn drop(&mut self) { println!("drop {} {}",self.0,query());if self.0==2 { panic!("second panic"); } } }
fn fail(){let _inner=D(2);panic!("first panic");}
fn main(){println!("entry {}",query());let _outer=D(1);fail();println!("unreachable");}
"#;
    let execute = |exe: &std::path::Path| {
        Command::new("sh")
            .args(["-c", "ulimit -c 0; exec \"$1\"", "panicking-test"])
            .arg(exe)
            .env("RUST_BACKTRACE", "0")
            .output()
            .unwrap()
    };
    for (name, source, strategy, stdout, exit, signal) in [
        (
            "normal-abort",
            normal,
            "abort",
            "entry false\nwork 156\nnormal drop false\nafter false\n",
            Some(0),
            None,
        ),
        (
            "normal-unwind",
            normal,
            "unwind",
            "entry false\nwork 156\nnormal drop false\nafter false\n",
            Some(0),
            None,
        ),
        (
            "unwind",
            unwind,
            "unwind",
            "entry false\ndrop 2 true true\ndrop 1 true true\n",
            Some(101),
            None,
        ),
        (
            "double",
            double,
            "unwind",
            "entry false\ndrop 2 true\n",
            None,
            Some(6),
        ),
        ("abort", unwind, "abort", "entry false\n", None, Some(6)),
    ] {
        let source_dir = dir.join(format!("source-{name}"));
        std::fs::create_dir_all(&source_dir).unwrap();
        let path = source_dir.join("main.rs");
        std::fs::write(&path, source).unwrap();
        let stock = dir.join(format!("stock-{name}"));
        let build =
            Command::new(std::env::var_os("JR_STOCK_RUSTC").unwrap_or_else(|| "rustc".into()))
                .args(["--edition=2024", "-C"])
                .arg(format!("panic={strategy}"))
                .arg(&path)
                .arg("-o")
                .arg(&stock)
                .output()
                .unwrap();
        assert!(build.status.success(), "{build:?}");
        let expected = execute(&stock);
        assert_eq!(
            expected.stdout,
            stdout.as_bytes(),
            "stock {name}: {expected:?}"
        );
        assert_eq!(expected.status.code(), exit, "stock {name}: {expected:?}");
        assert_eq!(
            expected.status.signal(),
            signal,
            "stock {name}: {expected:?}"
        );
        for workers in [1, 8] {
            for opt in ["off", "final"] {
                for c_entry in [false, true] {
                    let exe = dir.join(format!("native-{name}-{workers}-{opt}-{c_entry}"));
                    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                    command
                        .arg(&path)
                        .args(["--panic", strategy, "--opt", opt, "--workers"])
                        .arg(workers.to_string());
                    if c_entry {
                        command.args(["--native-library", "c"]);
                    }
                    let checked = Command::new(command.get_program())
                        .args(command.get_args())
                        .arg("--check")
                        .output()
                        .unwrap();
                    assert!(checked.status.success(), "CHECK {command:?}: {checked:?}");
                    assert!(!exe.exists(), "CHECK must not publish an executable");
                    let build = command.arg("-o").arg(&exe).output().unwrap();
                    assert!(build.status.success(), "{command:?}: {build:?}");
                    let actual = execute(&exe);
                    assert_eq!(actual.stdout, expected.stdout, "{command:?}: {actual:?}");
                    assert_eq!(
                        actual.status.code(),
                        expected.status.code(),
                        "{command:?}: {actual:?}"
                    );
                    assert_eq!(
                        actual.status.signal(),
                        expected.status.signal(),
                        "{command:?}: {actual:?}"
                    );
                    if name == "double" {
                        assert!(
                            String::from_utf8_lossy(&actual.stderr).contains("second panic"),
                            "{actual:?}"
                        );
                    }
                }
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
