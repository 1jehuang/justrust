//! Frozen24 original bounded divergent macros plus three function-local shadow cases.
//! No formatting or unwinding support. Original module shadow cases remain unsupported.
//! Receipt: /home/jeremy/.jcode/scratch/divergent-macros-stock/results.json
//! Receipt SHA256: 207a0d998f3135e0045c773be69d49590be97ebd0e5fddd3a2e780f671959525
//! Stock measured first with /usr/bin/rustc 1.98.1, edition2024, panic=abort, no BOOTSTRAP.
//! Original stock: 16 abort, 6 live, 2 reject. Original native: 15 abort, 3 live, 5 unsupported, 1 invalid.
//! Appended stock/native: 3 live. Total native: 15 abort, 6 live, 5 parse unsupported, 1 check invalid.
//! Append receipt: /home/jeremy/.jcode/scratch/divergent-macros-local-shadow-stock/results.json
//! Append receipt SHA256: 11400249fae3a4cc7fcf930850f330c78755ede37e65a4ff08188019447dc5af
//! The original nonliteral source stops at its earlier string-binding parser boundary.
//! Root owns native observations. Harness reused from panic.rs/literal_output.rs.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::compiler::{self, frontend, ir};
use justrust_engine::{Key, Report};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Native {
    Live,
    Abort,
    Invalid,
    Unsupported,
}
struct Case {
    name: &'static str,
    source: &'static str,
    source_bytes: usize,
    source_fingerprint: u64,
    source_sha256: &'static str,
    native: Native,
    boundary: &'static str,
    stock_compile_code: i32,
    stock_code: Option<i32>,
    stock_stdout: &'static str,
    phase: &'static str,
    message: &'static str,
    location: &'static str,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: r####"unreachable_empty"####,
            source: r####"fn main(){ unreachable!(); println!("after"); }
"####,
            source_bytes: 48,
            source_fingerprint: 7220920466499288220,
            source_sha256: r####"c7bff6b464392c4fb62e1d3b2b87bc7e1c0a51261319953235784faf0c26a660"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"internal error: entered unreachable code"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unreachable_custom"####,
            source: r####"fn main(){ unreachable!("custom"); println!("after"); }
"####,
            source_bytes: 56,
            source_fingerprint: 12250536288202028141,
            source_sha256: r####"024121b0953bb42758091d15447fa2850b7b41a4eb5fe1f7a3234fcd01d81065"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"internal error: entered unreachable code: custom"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unreachable_escaped"####,
            source: r####"fn main(){ unreachable!("{{λ}}\nnext"); }
"####,
            source_bytes: 43,
            source_fingerprint: 17897112818107314108,
            source_sha256: r####"3d4f74e47eb34d69f9eac055dc6f9580ff7edfd869f7bcb05239a997480a5e77"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"internal error: entered unreachable code: {λ}
next"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unreachable_delimiter"####,
            source: r####"fn main(){ unreachable!["bracket",]; }
"####,
            source_bytes: 39,
            source_fingerprint: 2861508872280095959,
            source_sha256: r####"fe9fd5251b712a34ee592afc263ae025007b16a5b6cfefae54b7a4d8e4c0f467"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"internal error: entered unreachable code: bracket"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unreachable_dead"####,
            source: r####"fn main(){ if false { unreachable!{"dead"} } println!("live"); }
"####,
            source_bytes: 65,
            source_fingerprint: 15528171191239885798,
            source_sha256: r####"f4b1d8fb51f181639dd9449c95527da9283e1e36f73d0d4c3d104229603693d5"####,
            native: Native::Live,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r####"live
"####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: r####"unreachable_once"####,
            source: r####"fn gate()->bool{ eprintln!("before"); true } fn main(){ if gate() { unreachable!("once"); } eprintln!("after"); }
"####,
            source_bytes: 114,
            source_fingerprint: 5224854204373509650,
            source_sha256: r####"1f12240f26e392182146859b201bdf7f9e2d8154ef7b3398117b69d8cd9e3318"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"internal error: entered unreachable code: once"####,
            location: r####"1:69"####,
        },
        Case {
            name: r####"unreachable_shadow"####,
            source: r####"macro_rules! unreachable { ($($x:tt)*) => { println!("shadow") }; } fn main(){ unreachable!(not a format); }
"####,
            source_bytes: 109,
            source_fingerprint: 15031042266080237263,
            source_sha256: r####"2162fa5fdcd4dbe33336c8150af0cc07507b0f3d3300a1d2877632f60c554cc3"####,
            native: Native::Unsupported,
            boundary: "module macros in expression position are unsupported",
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r####"shadow
"####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: r####"todo_empty"####,
            source: r####"fn main(){ todo!(); println!("after"); }
"####,
            source_bytes: 41,
            source_fingerprint: 10898310748364195114,
            source_sha256: r####"8c3319c1425e6b478deb4c38bd4a5a82b89dad0be7c39ef1960e10d49c7a705e"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not yet implemented"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"todo_custom"####,
            source: r####"fn main(){ todo!("custom"); println!("after"); }
"####,
            source_bytes: 49,
            source_fingerprint: 17940707689458642791,
            source_sha256: r####"39d08d1b0bb8145c36c67f36dad73e5289d0c40b468ba6a614aef22083e4b16e"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not yet implemented: custom"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"todo_escaped"####,
            source: r####"fn main(){ todo!("{{λ}}\nnext"); }
"####,
            source_bytes: 36,
            source_fingerprint: 3554216938884000766,
            source_sha256: r####"f119a8441f0f08671d0bb0c1d966454b513428b1b61a5448d4c53ba92fb6eaeb"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not yet implemented: {λ}
next"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"todo_delimiter"####,
            source: r####"fn main(){ todo!["bracket",]; }
"####,
            source_bytes: 32,
            source_fingerprint: 14944225505963686317,
            source_sha256: r####"befb1af6d524e0d4eb32434fe2fbdf7002c3f26228673c5aafc4d3229a37e915"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not yet implemented: bracket"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"todo_dead"####,
            source: r####"fn main(){ if false { todo!{"dead"} } println!("live"); }
"####,
            source_bytes: 58,
            source_fingerprint: 8237050325676212026,
            source_sha256: r####"b6acded41e0556f1ff4383560ca8e455f13070104388ce9adabe6e514ee95a77"####,
            native: Native::Live,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r####"live
"####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: r####"todo_once"####,
            source: r####"fn gate()->bool{ eprintln!("before"); true } fn main(){ if gate() { todo!("once"); } eprintln!("after"); }
"####,
            source_bytes: 107,
            source_fingerprint: 5407831997909928698,
            source_sha256: r####"5f49e57bac95d6caee481a0fbfa110bcd7a5fa22435460ef129c02245b7a9894"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not yet implemented: once"####,
            location: r####"1:69"####,
        },
        Case {
            name: r####"todo_shadow"####,
            source: r####"macro_rules! todo { ($($x:tt)*) => { println!("shadow") }; } fn main(){ todo!(not a format); }
"####,
            source_bytes: 95,
            source_fingerprint: 17662184406229945119,
            source_sha256: r####"1656c8550101b2c692228ab5dcb58c6612bb60fb199aef077b6b65387745977b"####,
            native: Native::Unsupported,
            boundary: "module macros in expression position are unsupported",
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r####"shadow
"####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: r####"unimplemented_empty"####,
            source: r####"fn main(){ unimplemented!(); println!("after"); }
"####,
            source_bytes: 50,
            source_fingerprint: 521035663729444507,
            source_sha256: r####"fda17f6e87862b161908038777f26eeb911610bfd06311d76ff6fa92a9c1e454"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not implemented"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unimplemented_custom"####,
            source: r####"fn main(){ unimplemented!("custom"); println!("after"); }
"####,
            source_bytes: 58,
            source_fingerprint: 7682196931683750634,
            source_sha256: r####"cdc9aac3b9b757c8ad19de0420d9db8e0d36cc50d961e9bcd5770f452ea7300b"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not implemented: custom"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unimplemented_escaped"####,
            source: r####"fn main(){ unimplemented!("{{λ}}\nnext"); }
"####,
            source_bytes: 45,
            source_fingerprint: 2329479900076824597,
            source_sha256: r####"335bf6a9684a844106fc023b3f3e1ba567347d864ede2602bb193e7fe7bcf150"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not implemented: {λ}
next"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unimplemented_delimiter"####,
            source: r####"fn main(){ unimplemented!["bracket",]; }
"####,
            source_bytes: 41,
            source_fingerprint: 2188969939254553806,
            source_sha256: r####"a85fadd7a6286c46fe440a128840c8bcfc0cab516b8cffc11deb44e24d998496"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not implemented: bracket"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"unimplemented_dead"####,
            source: r####"fn main(){ if false { unimplemented!{"dead"} } println!("live"); }
"####,
            source_bytes: 67,
            source_fingerprint: 4361739081873701245,
            source_sha256: r####"4d3963fcd82a274b425a64deb260cb84ac1e139e93f7e63ccf098781b82e5a18"####,
            native: Native::Live,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r####"live
"####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: r####"unimplemented_once"####,
            source: r####"fn gate()->bool{ eprintln!("before"); true } fn main(){ if gate() { unimplemented!("once"); } eprintln!("after"); }
"####,
            source_bytes: 116,
            source_fingerprint: 8596135947284994187,
            source_sha256: r####"ee91014d1f13114e18f14deb52bae3c36362c80614101b4e1999b47b1e9a8f30"####,
            native: Native::Abort,
            boundary: r####""####,
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not implemented: once"####,
            location: r####"1:69"####,
        },
        Case {
            name: r####"unimplemented_shadow"####,
            source: r####"macro_rules! unimplemented { ($($x:tt)*) => { println!("shadow") }; } fn main(){ unimplemented!(not a format); }
"####,
            source_bytes: 113,
            source_fingerprint: 13309970629709715707,
            source_sha256: r####"4d5779d46c446a4879378941ddef408b61f7f8189821e806bbc8b23571456d14"####,
            native: Native::Unsupported,
            boundary: "module macros in expression position are unsupported",
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r####"shadow
"####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: r####"format"####,
            source: r####"fn main(){ todo!("value {}", 7); }
"####,
            source_bytes: 35,
            source_fingerprint: 16789343040777769838,
            source_sha256: r####"590370c9bc1576ce362fe9a6704c891229b85ebacb62b4f3a65572ce4add7d74"####,
            native: Native::Unsupported,
            boundary: r####"todo! formatting placeholders are unsupported"####,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r####""####,
            message: r####"not yet implemented: value 7"####,
            location: r####"1:12"####,
        },
        Case {
            name: r####"nonliteral"####,
            source: r####"fn main(){ let s="text"; unimplemented!(s); }
"####,
            source_bytes: 46,
            source_fingerprint: 7270409704440910109,
            source_sha256: r####"3c61dce056f61d1209e013c2075d7821d0318934517aa354aba30f7b2886a278"####,
            native: Native::Unsupported,
            boundary: r####"unimplemented! nonliteral arguments are unsupported"####,
            phase: "parse",
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r####""####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: r####"dead_invalid"####,
            source: r####"fn main(){ if false { unreachable!(); let _x:i32=true; } }
"####,
            source_bytes: 59,
            source_fingerprint: 2750352263357822162,
            source_sha256: r####"71e0e65904fdea1f2436b24c0ccca72abf86ac89ca0b6aac45a3905328a1cb4b"####,
            native: Native::Invalid,
            boundary: r####"type"####,
            phase: "check",
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r####""####,
            message: r####""####,
            location: r####""####,
        },
        Case {
            name: "unreachable_local_shadow",
            source: r####"fn main(){ macro_rules! unreachable { ($($x:tt)*) => { println!("shadow") }; } unreachable!(not a format); }
"####,
            source_bytes: 109,
            source_fingerprint: 2879452016549107923,
            source_sha256: "a7f491c6085a0a315c94768aa8e46c7c1b122ffcfdf8e1de44dfa347ffd7f6d3",
            native: Native::Live,
            boundary: "",
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: "shadow\n",
            message: "",
            location: "",
        },
        Case {
            name: "todo_local_shadow",
            source: r####"fn main(){ macro_rules! todo { ($($x:tt)*) => { println!("shadow") }; } todo!(not a format); }
"####,
            source_bytes: 95,
            source_fingerprint: 15906514686395904425,
            source_sha256: "8a47f48e9f2f84722c1ac6a13170ea91e3f08bbcee936561439660cd665495aa",
            native: Native::Live,
            boundary: "",
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: "shadow\n",
            message: "",
            location: "",
        },
        Case {
            name: "unimplemented_local_shadow",
            source: r####"fn main(){ macro_rules! unimplemented { ($($x:tt)*) => { println!("shadow") }; } unimplemented!(not a format); }
"####,
            source_bytes: 113,
            source_fingerprint: 17135502895764652565,
            source_sha256: "00844d2da4da27442a1a7ac90f81f9514e1508c27726edafcaccd9020c5c9e92",
            native: Native::Live,
            boundary: "",
            phase: "check",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: "shadow\n",
            message: "",
            location: "",
        },
    ]
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!(
            "jr-divergent-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        new_file(&self.path(name)).write_all(bytes).unwrap();
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap()
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
// Linux x86_64 ABI, matching this test's target gate. This links the libc already
// used by std and adds no dependency. Only setrlimit is called after fork.
#[repr(C)]
struct Rlimit {
    current: u64,
    maximum: u64,
}
unsafe extern "C" {
    fn setrlimit(resource: std::ffi::c_int, limit: *const Rlimit) -> std::ffi::c_int;
    fn prctl(option: std::ffi::c_int, ...) -> std::ffi::c_int;
}
fn disable_core(command: &mut Command) {
    // SAFETY: the closure only uses stack data, a libc syscall and last_os_error.
    unsafe {
        command.pre_exec(|| {
            let limit = Rlimit {
                current: 0,
                maximum: 0,
            };
            // A piped core_pattern can invoke an external dumper even with
            // RLIMIT_CORE=0. Keep abort observations independent of that service.
            if setrlimit(4, &limit) == 0 && prctl(4, 0usize, 0usize, 0usize, 0usize) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    capture_bounded(command, scratch, seconds, 65536)
}
fn capture_bounded(command: &mut Command, scratch: &Scratch, seconds: u64, limit: u64) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    disable_core(command);
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    let mut retries = 0;
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error) if error.raw_os_error() == Some(26) && retries < 20 => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("spawn {command:?}: {error}"),
        }
    };
    let mut child = Running(child);
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= limit,
                "output bound: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(fs::metadata(path).unwrap().len() <= limit);
    }
    Captured {
        status,
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn ordinary_exit(result: &Captured, code: i32) {
    assert_eq!(result.status.signal(), None, "{result:?}");
    assert_eq!(result.status.code(), Some(code), "{result:?}");
}
fn no_compiler_tools(command: &mut Command, scratch: &Scratch) {
    command
        .current_dir(&scratch.0)
        .env_remove("RUSTC_BOOTSTRAP")
        .env("RUSTC", scratch.path("no-rustc"))
        .env("PATH", scratch.path("no-tools"));
}
fn panic_runtime(case: &Case, result: &Captured, source: &Path, stock: bool) {
    assert_eq!(
        result.stdout,
        case.stock_stdout.as_bytes(),
        "{} exact stdout {result:?}",
        case.name
    );
    if case.stock_code == Some(-6) {
        assert_eq!(
            result.status.signal(),
            Some(6),
            "{} requires SIGABRT {result:?}",
            case.name
        );
        assert_eq!(result.status.code(), None);
        let stderr = String::from_utf8(result.stderr.clone()).unwrap();
        let before = if case.name.ends_with("_once") {
            "before\n"
        } else {
            ""
        };
        let location = format!("{}:{}", source.display(), case.location);
        if stock {
            let prefix = format!("{before}\nthread 'main' (");
            let remaining = stderr
                .strip_prefix(&prefix)
                .expect("exact stock panic prefix");
            let (pid, tail) = remaining
                .split_once(") panicked at ")
                .expect("stock PID frame");
            assert!(!pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
            let note =
                "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n";
            assert_eq!(tail, format!("{location}:\n{}\n{note}", case.message));
        } else {
            assert_eq!(
                stderr,
                format!("{before}panicked at {location}:\n{}\n", case.message),
                "{} exact native frame",
                case.name
            );
        }
    } else {
        ordinary_exit(result, 0);
        assert!(result.stderr.is_empty(), "{result:?}");
    }
}
fn runtime(case: &Case, path: &Path, source: &Path, scratch: &Scratch, stock: bool) {
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(Command::new(path).env("RUST_BACKTRACE", "0"), scratch, 5);
    panic_runtime(case, &result, source, stock);
}
#[derive(Default)]
struct CellFailures(Vec<String>);
impl CellFailures {
    fn check(&mut self, label: impl Into<String>, check: impl FnOnce()) {
        let label = label.into();
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check)) {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            let failure = format!("{label}: {message}");
            eprintln!("DIVERGENT CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("DIVERGENT CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} panic failures:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}
fn fingerprint(source: &str) -> u64 {
    source.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
#[test]
fn frozen_divergent_sources_and_strict_phases() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 27);
    let mut partition = [0; 4];
    let mut failures = CellFailures::default();
    for case in &all {
        partition[match case.native {
            Native::Live => 0,
            Native::Abort => 1,
            Native::Unsupported => 2,
            Native::Invalid => 3,
        }] += 1;
        failures.check(format!("phase {}", case.name), || {
            assert_eq!(case.source.len(), case.source_bytes);
            assert_eq!(fingerprint(case.source), case.source_fingerprint);
            assert_eq!(case.source_sha256.len(), 64);
            assert!(case.source.ends_with('\n'));
            let parsed = frontend::parse(case.source);
            if case.phase == "parse" {
                diagnostic(
                    case,
                    &parsed.expect_err("strict parse rejection").to_string(),
                );
            } else {
                let program = parsed.expect("must pass parse");
                let checked = frontend::check(&program);
                if case.native == Native::Invalid {
                    diagnostic(
                        case,
                        &checked.expect_err("strict check rejection").to_string(),
                    );
                } else {
                    checked.expect("all bodies checked");
                }
            }
        });
    }
    assert_eq!(partition, [6, 15, 5, 1]);
    failures.finish();
}
fn stock_all() {
    static STOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STOCK.get_or_init(stock_all_uncached);
}
fn stock_all_uncached() {
    let scratch = Scratch::new();
    let rustc = "/usr/bin/rustc";
    let version = capture(
        Command::new(rustc).env_remove("RUSTC_BOOTSTRAP").arg("-vV"),
        &scratch,
        10,
    );
    ordinary_exit(&version, 0);
    eprintln!(
        "DIVERGENT ACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut failures = CellFailures::default();
    for case in cases() {
        failures.check(format!("stock {}", case.name), || {
            let scratch = Scratch::new();
            scratch.write("source.rs", case.source.as_bytes());
            let source = scratch.path("source.rs");
            let image = scratch.path("stock");
            let result = capture(
                Command::new(rustc)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUST_BACKTRACE", "0")
                    .env("LC_ALL", "C")
                    .args([
                        "--edition=2024",
                        "--crate-name=divergent_oracle",
                        "--color=never",
                    ])
                    .arg("-Cpanic=abort")
                    .arg(&source)
                    .arg("-o")
                    .arg(&image),
                &scratch,
                20,
            );
            ordinary_exit(&result, case.stock_compile_code);
            assert!(result.stdout.is_empty(), "{result:?}");
            if case.stock_compile_code == 0 {
                runtime(&case, &image, &source, &scratch, true);
            } else {
                assert!(!image.exists());
                let text = String::from_utf8_lossy(&result.stderr);
                let codes: Vec<_> = text
                    .lines()
                    .filter_map(|line| line.strip_prefix("error[")?.split_once(']').map(|v| v.0))
                    .collect();
                let expected: &[&str] = if case.name == "dead_invalid" {
                    &["E0308"]
                } else {
                    &[]
                };
                assert_eq!(codes, expected, "{}: {result:?}", case.name);
                if case.name == "nonliteral" {
                    assert!(text.contains("format argument must be a string literal"));
                }
            }
        });
    }
    failures.finish();
}
#[test]
fn stock_divergent_exact_sources() {
    stock_all();
}
fn diagnostic(case: &Case, text: &str) {
    assert!(!text.is_empty(), "{}: empty diagnostic", case.name);
    assert!(
        !text.contains("panicked at"),
        "{}: compiler panic: {text}",
        case.name
    );
    match case.native {
        Native::Invalid => {
            // Ordinary invalid source cannot be laundered into a capability failure.
            assert!(!text.contains("unsupported"), "{}: {text}", case.name);
            if case.boundary == "type" {
                assert!(
                    text.contains("expected") && text.contains("I32") && text.contains("Bool"),
                    "{}: wrong ordinary type error: {text}",
                    case.name
                );
            } else {
                assert!(text.contains(case.boundary), "{}: {text}", case.name);
            }
        }
        Native::Unsupported => {
            // Confirmed parser capability boundary, not a native observation.
            assert!(
                text.contains(case.boundary),
                "{}: expected earlier boundary {:?}: {text}",
                case.name,
                case.boundary
            );
        }
        _ => panic!("{} unexpectedly rejected: {text}", case.name),
    }
}
fn evaluated_once(report: &Report, kind: u32, index: usize, case: &Case) {
    let key = Key::new(kind, vec![index as u64]);
    let nodes: Vec<_> = report
        .node_evaluations
        .iter()
        .filter(|(actual, _)| *actual == key)
        .collect();
    assert_eq!(nodes.len(), 1, "{}: missing/duplicate {key:?}", case.name);
    assert_eq!(nodes[0].1, 1, "{}: repeated {key:?}", case.name);
}
fn successful_all_body_graph(case: &Case, compiled: &compiler::Compilation, check: bool) {
    let program = native_prelude::parse(case.source).expect("positive frozen source parses");
    frontend::check(&program).expect("all bodies checked independent of reachability");
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    assert_eq!(compiled.source_bytes, case.source.len());
    assert_eq!(compiled.source_files, 1);
    assert_eq!(compiled.executable.is_none(), check);
    assert_eq!(compiled.report.failures, 0);
    assert_eq!(compiled.report.events_dropped, 0);
    assert!(compiled.report.capture_complete);
    let mut source_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        evaluated_once(&compiled.report, 110, index, case);
        let universal = match plan.callable_provider(declaration).unwrap() {
            frontend::CallableProvider::Source { .. } => {
                source_bodies += usize::from(!plan.is_prelude_decl(declaration).unwrap());
                plan.callable_env(declaration).unwrap().is_generic()
            }
            frontend::CallableProvider::Recipe { .. } => true,
            frontend::CallableProvider::UnavailableProvided { .. } => {
                panic!("unavailable body admitted")
            }
        };
        if universal {
            evaluated_once(&compiled.report, 111, index, case);
            evaluated_once(&compiled.report, 112, index, case);
        }
    }
    assert_eq!(compiled.functions, source_bodies);
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 110)
            .count(),
        plan.callable_count()
    );
    assert_eq!(compiled.control_flow.len(), compiled.analyses.len());
    assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
    for kind in [102, 108, 109] {
        let nodes: Vec<_> = compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == kind)
            .collect();
        assert_eq!(
            nodes.len(),
            compiled.control_flow.len(),
            "{}:{kind}",
            case.name
        );
        assert!(nodes.iter().all(|(_, count)| *count == 1));
    }
    let objects: Vec<_> = compiled
        .report
        .node_evaluations
        .iter()
        .filter(|(key, _)| key.kind == 103)
        .collect();
    assert_eq!(
        objects.len(),
        if check {
            0
        } else {
            compiled.control_flow.len()
        },
        "{}: codegen only in BUILD",
        case.name
    );
    assert!(objects.iter().all(|(_, count)| *count == 1));
}
fn cli_all() {
    let mut failures = CellFailures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "CLI {} workers{workers} check{check} {:?}",
                        case.name, case.native
                    ),
                    || {
                        let scratch = Scratch::new();
                        scratch.write("source.rs", case.source.as_bytes());
                        let source = scratch.path("source.rs");
                        let image = scratch.path("native");
                        let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                        no_compiler_tools(&mut command, &scratch);
                        command
                            .arg(&source)
                            .args(["--workers", &workers.to_string()]);
                        if check {
                            command.arg("--check");
                        } else {
                            command.arg("-o").arg(&image);
                        }
                        let result = capture(&mut command, &scratch, 20);
                        let accepted = matches!(case.native, Native::Live | Native::Abort);
                        ordinary_exit(&result, i32::from(!accepted));
                        if !accepted {
                            assert!(result.stdout.is_empty());
                            diagnostic(&case, &String::from_utf8_lossy(&result.stderr));
                        }
                        if accepted && !check {
                            runtime(&case, &image, &source, &scratch, false);
                            runtimes += 1;
                        } else {
                            for name in ["native", "source", "a.out"] {
                                assert!(
                                    !scratch.path(name).exists(),
                                    "{}: unexpected artifact {name}",
                                    case.name
                                );
                            }
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 108);
    failures.check("runtime count", || assert_eq!(runtimes, 42));
    failures.finish();
}
fn graph_all() {
    let mut failures = CellFailures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "graph {} workers{workers} check{check} {:?}",
                        case.name, case.native
                    ),
                    || {
                        let scratch = Scratch::new();
                        scratch.write("source.rs", case.source.as_bytes());
                        let source = scratch.path("source.rs");
                        match compiler::compile_file_observed(source.clone(), workers, check) {
                            Ok(compiled) => {
                                assert!(
                                    matches!(case.native, Native::Live | Native::Abort),
                                    "{} unexpectedly accepted",
                                    case.name
                                );
                                successful_all_body_graph(&case, &compiled, check);
                                if !check {
                                    let path = scratch.path("graph-native");
                                    scratch.write(
                                        "graph-native",
                                        compiled.executable.as_ref().unwrap(),
                                    );
                                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                                        .unwrap();
                                    runtime(&case, &path, &source, &scratch, false);
                                    runtimes += 1;
                                }
                            }
                            Err(failure) => {
                                diagnostic(&case, &failure.to_string());
                                let report =
                                    failure.report.expect("failure retains observed graph");
                                assert!(report.capture_complete);
                                assert_eq!(report.events_dropped, 0);
                                assert!(report.failures > 0);
                                assert!(
                                    !report.published_kind(105),
                                    "{}: rejected source emitted object",
                                    case.name
                                );
                            }
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 108);
    failures.check("graph runtime count", || assert_eq!(runtimes, 42));
    failures.finish();
}
fn isolated(name: &str, marker: &str, run: fn()) {
    if std::env::var_os(marker).is_some() {
        run();
        return;
    }
    // Stock comes first via the independently measured system compiler. The
    // native child then has no stock compiler or linker available in PATH.
    stock_all();
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    command
        .args(["--exact", name, "--nocapture"])
        .env(marker, "1");
    let result = capture_bounded(&mut command, &scratch, 240, 8 * 1024 * 1024);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    ordinary_exit(&result, 0);
}
#[test]
fn native_divergent_workers1_8_check_build_actual_elf() {
    isolated(
        "native_divergent_workers1_8_check_build_actual_elf",
        "JR_DIVERGENT_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_divergent_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_divergent_workers1_8_check_build_all_body_graph",
        "JR_DIVERGENT_GRAPH_CHILD",
        graph_all,
    );
}
