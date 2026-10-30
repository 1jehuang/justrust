use std::{
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{DirBuilderExt, PermissionsExt},
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Helper(Child);
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}
unsafe extern "C" {
    fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
    fn poll(fds: *mut PollFd, count: usize, timeout: i32) -> i32;
}

#[test]
fn controlled_link_diagnostic_capture_is_bounded_and_preserves_failures() {
    use std::io::{self, Read};
    for name in ["stdout", "stderr"] {
        for size in [
            0,
            super::MAX_DIAGNOSTIC_BYTES,
            super::MAX_DIAGNOSTIC_BYTES + 1,
        ] {
            let (bytes, error) = super::drain(io::repeat(b'x').take(size as u64), name);
            assert_eq!(bytes.len(), size.min(super::MAX_DIAGNOSTIC_BYTES));
            assert_eq!(error.is_some(), size > super::MAX_DIAGNOSTIC_BYTES);
            if let Some(error) = error {
                assert!(error.to_string().contains(name));
                assert!(error.to_string().contains("diagnostics truncated"));
            }
        }
    }
    struct Broken(bool);
    impl Read for Broken {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.0 {
                return Err(io::Error::other("diagnostic read failed"));
            }
            self.0 = true;
            out[..3].copy_from_slice(b"bad");
            Ok(3)
        }
    }
    let (bytes, error) = super::drain(Broken(false), "stderr");
    assert_eq!(bytes, b"bad");
    assert!(
        error
            .unwrap()
            .to_string()
            .contains("diagnostic read failed")
    );
}

#[test]
fn controlled_link_floods_never_publish_success_or_wait_for_wall_limit() {
    let root = std::env::temp_dir().join(format!(
        "jr-link-flood-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let scratch = Scratch(root);
    fs::create_dir(scratch.0.join("out")).unwrap();
    let limit = super::MAX_DIAGNOSTIC_BYTES;
    for (fd, size, linger) in [
        (1, limit, false),
        (2, limit, false),
        (1, limit + 1, false),
        (2, limit + 1, false),
        (1, limit + 1, true),
        (2, limit + 1, true),
    ] {
        let source = scratch.0.join("probe.c");
        fs::write(&source, format!(r#"
static long call(long n,long a,long b,long c){{long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}}
void _start(void){{
 char bytes[4096];for(long i=0;i<4096;i++)bytes[i]='x';
 long left={size};while(left>0){{long n=left>4096?4096:left;long written=call(1,{fd},(long)bytes,n);if(written<=0)break;left-=written;}}
 long delay[2]={{0,100000000}};if({linger})for(;;)call(35,(long)delay,0,0);
 call(60,0,0,0);__builtin_unreachable();
}}
"#, linger=u8::from(linger))).unwrap();
        let compiled = Command::new("cc")
            .args([
                "-nostdlib",
                "-static",
                "-fno-stack-protector",
                "-no-pie",
                "-O2",
            ])
            .arg(&source)
            .arg("-o")
            .arg(scratch.0.join("probe"))
            .output()
            .unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        fs::set_permissions(scratch.0.join("probe"), fs::Permissions::from_mode(0o555)).unwrap();
        let mut command = Command::new(scratch.0.join("probe"));
        command.env_clear();
        let start = Instant::now();
        let result = super::output(&mut command, &scratch.0);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "diagnostic cancellation stalled"
        );
        if size == limit {
            let output = result.unwrap();
            assert!(output.status.success());
            assert_eq!(
                if fd == 1 {
                    &output.stdout
                } else {
                    &output.stderr
                },
                &vec![b'x'; limit]
            );
        } else {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(if fd == 1 { "stdout" } else { "stderr" }));
            assert!(error.contains("diagnostic limit (diagnostics truncated)"));
            assert!(error.len() <= limit + 512);
        }
    }
}

#[test]
fn controlled_link_initial_auxv_random_is_normalized_across_launches() {
    let root = std::env::temp_dir().join(format!(
        "jr-link-auxv-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let scratch = Scratch(root);
    fs::create_dir(scratch.0.join("out")).unwrap();
    let source = scratch.0.join("probe.S");
    fs::write(
        &source,
        r#"
.global _start
.bss
.balign 8
observed: .skip 32
.text
_start:
 mov (%rsp),%rcx
 lea 16(%rsp,%rcx,8),%rsi
1: lodsq
 test %rax,%rax
 jnz 1b
 xor %r8d,%r8d
2: lodsq
 mov %rax,%rdx
 lodsq
 test %rdx,%rdx
 jz 6f
 cmp $25,%rdx
 jne 3f
 test %rax,%rax
 jz 9f
 mov (%rax),%rcx
 mov %rcx,observed(%rip)
 mov 8(%rax),%rcx
 mov %rcx,observed+8(%rip)
 or $1,%r8d
3: cmp $33,%rdx
 jne 4f
 mov %rax,observed+16(%rip)
4: cmp $4,%rdx
 jne 5f
 mov %rax,observed+24(%rip)
 or $2,%r8d
5: cmp $3,%rdx
 je 8f
 cmp $9,%rdx
 jne 2b
8: test %rax,%rax
 jz 9f
 jmp 2b
6: cmp $3,%r8d
 jne 9f
 mov $1,%eax
 mov $1,%edi
 lea observed(%rip),%rsi
 mov $32,%edx
 syscall
 cmp $32,%rax
 jne 9f
 mov $60,%eax
 xor %edi,%edi
 syscall
9: mov $60,%eax
 mov $1,%edi
 syscall
"#,
    )
    .unwrap();
    let compiled = Command::new("cc")
        .args(["-nostdlib", "-static", "-no-pie"])
        .arg(&source)
        .arg("-o")
        .arg(scratch.0.join("probe"))
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    fs::set_permissions(scratch.0.join("probe"), fs::Permissions::from_mode(0o555)).unwrap();
    let mut expected = b"jr-link-seed-v1!".to_vec();
    expected.extend_from_slice(&0u64.to_le_bytes());
    expected.extend_from_slice(&56u64.to_le_bytes());
    for launch in 0..4 {
        let mut command = Command::new(scratch.0.join("probe"));
        command.env_clear();
        // Exercise both an empty and a nonempty initial environment and argv.
        if launch % 2 == 1 {
            command.args(["one", "two"]).env("PROBE", "present");
        }
        let output = super::output(&mut command, &scratch.0).unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, expected, "launch {launch}");
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn controlled_link_exec_stop_precedes_first_image_write_and_reaps_setup_errors() {
    use super::super::{
        link_sandbox,
        link_sandbox_exec::Guard,
        link_sandbox_io::{Broker, bootstrap_exec, receive},
    };
    let root = std::env::temp_dir().join(format!(
        "jr-link-exec-stop-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let scratch = Scratch(root);
    fs::create_dir(scratch.0.join("out")).unwrap();
    // First instruction sequence writes to inherited stdout, without any
    // brokered syscall that could itself hide an early execution race.
    let source = scratch.0.join("probe.S");
    fs::write(&source, ".global _start\n.text\n_start:\nmov $1,%eax\nmov $1,%edi\nlea message(%rip),%rsi\nmov $5,%edx\nsyscall\nmov $60,%eax\nxor %edi,%edi\nsyscall\nmessage: .ascii \"ready\"\n").unwrap();
    let compiled = Command::new("cc")
        .args(["-nostdlib", "-static", "-no-pie"])
        .arg(&source)
        .arg("-o")
        .arg(scratch.0.join("probe"))
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    fs::set_permissions(scratch.0.join("probe"), fs::Permissions::from_mode(0o555)).unwrap();
    for failure in [0, 1, 2] {
        let mut broker = Broker::new(&scratch.0).unwrap();
        let mut command = Command::new(scratch.0.join("probe"));
        command
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let socket = link_sandbox::confine(&mut command, &scratch.0).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (mut child, listener, process) = std::thread::scope(|scope| {
            let receiver = scope.spawn(move || {
                let listener = receive(&socket).unwrap();
                let (pid, process) = bootstrap_exec(&listener).unwrap();
                (listener, pid, process)
            });
            let child = Guard::new(command.spawn().unwrap());
            let (listener, pid, process) = receiver.join().unwrap();
            assert_eq!(pid, child.child.id());
            (child, listener, process)
        });
        if failure == 2 {
            assert!(child.normalize_launch(&process).is_err());
            // Receiver completed but no wait consumed the exec-stop yet.
            // Cleanup must not mistake that pending stop for task death.
            let pid = child.child.id();
            let mut stdout = child.child.stdout.take().unwrap();
            drop(child);
            assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
            use std::io::Read;
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).unwrap();
            assert!(bytes.is_empty());
            continue;
        }
        child.await_exec().unwrap();
        let mut stdout = child.child.stdout.take().unwrap();
        let stderr = child.child.stderr.take().unwrap();
        let mut event = PollFd {
            fd: stdout.as_raw_fd(),
            events: 1,
            revents: 0,
        };
        assert_eq!(
            unsafe { poll(&mut event, 1, 100) },
            0,
            "untrusted image wrote before parent setup"
        );
        child.normalize_launch(&process).unwrap();
        broker.bind_process(process).unwrap();
        if failure == 1 {
            // Deliberately fail stdio admission with a regular file while the
            // image is stopped. Drop must kill and truly reap, not cache stop.
            assert!(
                broker
                    .set_stdio(&fs::File::open(&source).unwrap(), &stderr)
                    .is_err()
            );
            let pid = child.child.id();
            drop(child);
            assert!(
                !PathBuf::from(format!("/proc/{pid}")).exists(),
                "stopped task was not reaped"
            );
            use std::io::Read;
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).unwrap();
            assert!(bytes.is_empty(), "failed parent setup allowed image write");
        } else {
            broker.set_stdio(&stdout, &stderr).unwrap();
            assert_eq!(
                unsafe { poll(&mut event, 1, 100) },
                0,
                "image ran while still traced"
            );
            child.detach().unwrap();
            assert_eq!(
                unsafe { poll(&mut event, 1, 3000) },
                1,
                "ready image never wrote"
            );
            use std::io::Read;
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"ready");
            assert!(child.child.wait().unwrap().success());
        }
        drop(listener);
    }
    // Failed exec never produces an exec stop. It must still leave neither a
    // hanging spawn nor a traced orphan behind.
    let start = Instant::now();
    let mut command = Command::new(scratch.0.join("missing"));
    assert!(super::output(&mut command, &scratch.0).is_err());
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn controlled_linker_dies_with_its_compiler() {
    const CHILD: &str = "JR_LINK_PARENT_DEATH_FIXTURE_ROOT";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        let mut command = Command::new(root.join("probe"));
        command.env_clear();
        let result = super::output(&mut command, &root);
        panic!("fixture compiler unexpectedly returned before being killed: {result:?}");
    }
    let root = std::env::temp_dir().join(format!(
        "jr-link-lifetime-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let scratch = Scratch(root);
    fs::create_dir(scratch.0.join("out")).unwrap();
    let source = scratch.0.join("probe.c");
    fs::write(&source, r#"
static long call(long n,long a,long b,long c){long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}
void _start(void){
 long fd=call(2,(long)"out/ready",65,0600);
 if(fd<0||call(1,fd,(long)"ready",5)!=5) {call(60,1,0,0);__builtin_unreachable();}
 call(3,fd,0,0);
 long delay[2]={0,100000000};
 for(;;)call(35,(long)delay,0,0);
}
"#).unwrap();
    let compiled = Command::new("cc")
        .args([
            "-nostdlib",
            "-static",
            "-fno-stack-protector",
            "-no-pie",
            "-O2",
        ])
        .arg(&source)
        .arg("-o")
        .arg(scratch.0.join("probe"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    fs::set_permissions(scratch.0.join("probe"), fs::Permissions::from_mode(0o555)).unwrap();
    let mut helper = Helper(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "compiler::c_abi::link_sandbox_run::tests::controlled_linker_dies_with_its_compiler", "--nocapture"])
        .env(CHILD, &scratch.0).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    let pid = loop {
        if fs::read(scratch.0.join("out/ready")).is_ok_and(|v| v == b"ready") {
            // The tool cannot query its process identity. Observe the helper's
            // sole child from the supervising test instead. Command::spawn
            // runs on a test thread, so inspect every task's children list.
            // Our live, unreaped helper pins this process identity throughout.
            let tasks = fs::read_dir(format!("/proc/{}/task", helper.0.id())).unwrap();
            let mut children = std::collections::BTreeSet::new();
            for task in tasks {
                let path = task.unwrap().path().join("children");
                let Ok(text) = fs::read_to_string(path) else {
                    continue;
                };
                for pid in text.split_whitespace() {
                    children.insert(pid.parse::<u32>().unwrap());
                }
            }
            assert_eq!(
                children.len(),
                1,
                "fixture compiler child identity ambiguous"
            );
            break *children.first().unwrap();
        }
        assert!(
            helper.0.try_wait().unwrap().is_none(),
            "fixture compiler exited before starting its linker"
        );
        assert!(
            Instant::now() < deadline,
            "fixture linker did not announce readiness"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    // pidfds refer to the exact task, including after death/PID reuse. Never
    // send cleanup signals to an unpinned numeric process ID.
    let fd = unsafe { syscall(434, pid, 0u32) };
    assert!(fd >= 0, "{}", std::io::Error::last_os_error());
    let pidfd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let mut event = PollFd {
        fd: pidfd.as_raw_fd(),
        events: 1,
        revents: 0,
    };
    assert_eq!(
        unsafe { poll(&mut event, 1, 0) },
        0,
        "fixture linker already exited"
    );
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    for name in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
        let line = status.lines().find(|line| line.starts_with(name)).unwrap();
        assert_eq!(
            u64::from_str_radix(line.split_whitespace().nth(1).unwrap(), 16).unwrap(),
            0,
            "{line}"
        );
    }
    helper.0.kill().unwrap();
    helper.0.wait().unwrap();
    let exited = unsafe { poll(&mut event, 1, 3000) };
    if exited != 1 || event.revents & 1 == 0 {
        // Bounded, task-identity-safe cleanup if the assertion exposes a bug.
        unsafe {
            syscall(424, pidfd.as_raw_fd(), 9i32, std::ptr::null::<u8>(), 0u32);
        }
        panic!(
            "sandboxed linker survived its compiler: poll={exited}, events={}",
            event.revents
        );
    }
}
