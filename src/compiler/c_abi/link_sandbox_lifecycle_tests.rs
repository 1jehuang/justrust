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
