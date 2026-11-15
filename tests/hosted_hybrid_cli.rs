//! Managed agent hybrid admission, argument, status and empty-summary contract.
#![cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../src/remote_proto.rs"]
mod remote_proto;
use remote_proto::{Msg, VERSION, read_frame, write_frame};
use std::io::{BufReader, BufWriter};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

#[test]
fn admitted_hosted_hybrid_keeps_arguments_exit_and_unknown_cargo_summary() {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("jr-hosted-hybrid-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("root/project")).unwrap();
    std::fs::create_dir_all(dir.join("toolchain/bin")).unwrap();
    std::fs::create_dir_all(dir.join("private")).unwrap();
    std::fs::set_permissions(dir.join("private"), std::fs::Permissions::from_mode(0o700)).unwrap();
    for name in ["rustc", "cargo"] {
        std::fs::write(dir.join("toolchain/bin").join(name), "unused").unwrap();
    }
    let engine = dir.join("engine");
    std::fs::write(&engine, r#"#!/usr/bin/python3
import os, sys, time
assert sys.argv[1:3] == ['cargo', 'hybrid-ready'], sys.argv
if sys.argv[-1] == 'block':
    os.write(int(os.environ['JR_HYBRID_DECISION_FD']), b'committed\n')
    os.close(int(os.environ['JR_HYBRID_DECISION_FD']))
    print('managed-native-blocking', flush=True)
    time.sleep(30)
    sys.exit(91)
assert sys.argv[-3:] == ['--', '--exact', 'name with spaces'], sys.argv
assert os.environ['JUSTRUST_ON_REMOTE_AGENT'] == '1'
for name in ['JUSTRUST_HOSTED_HYBRID', 'JUSTRUST_HOSTED_HYBRID_ENGINE', 'JUSTRUST_HYBRID_PREWARM', 'JUSTRUST_HYBRID_FALLBACK']:
    assert name not in os.environ, name
os.write(int(os.environ['JR_HYBRID_DECISION_FD']), b'committed\n')
print('managed-native-committed', flush=True)
sys.exit(37)
"#).unwrap();
    std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_justrust"))
        .arg("__agent")
        .arg(dir.join("root"))
        .env("JUSTRUST_HOME", dir.join("home"))
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_REMOTE_TOOLCHAIN", "system")
        .env("JUSTRUST_HYBRID", "ready")
        .env("JUSTRUST_NATIVE_ENGINE", "/must-not-run")
        .env("JUSTRUST_HYBRID_PREWARM", "after-success")
        .env("JUSTRUST_HYBRID_FALLBACK", "/must-not-run")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut writer = BufWriter::new(child.stdin.take().unwrap());
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    write_frame(
        &mut writer,
        &Msg::Hello {
            version: VERSION,
            caps: vec![],
        },
        &[],
    )
    .unwrap();
    assert!(matches!(
        read_frame(&mut reader).unwrap().unwrap().0,
        Msg::Hello { .. }
    ));
    let env = vec![
        ("JUSTRUST_HOSTED_HYBRID".into(), "ready".into()),
        (
            "JUSTRUST_HOSTED_HYBRID_ENGINE".into(),
            engine.to_str().unwrap().into(),
        ),
        (
            "JUSTRUST_HOSTED_HYBRID_TOOLCHAIN".into(),
            dir.join("toolchain").to_str().unwrap().into(),
        ),
        (
            "JUSTRUST_HOSTED_HYBRID_HOME".into(),
            dir.join("private").to_str().unwrap().into(),
        ),
    ];
    write_frame(
        &mut writer,
        &Msg::Run {
            id: 1,
            cwd: "/project".into(),
            args: vec![
                "test".into(),
                "--".into(),
                "--exact".into(),
                "name with spaces".into(),
            ],
            env: env.clone(),
            roots: vec![],
            start: false,
            run_id: "hosted-native-no-cargo-summary".into(),
        },
        &[],
    )
    .unwrap();
    let mut output = Vec::new();
    loop {
        let (msg, payload) = read_frame(&mut reader)
            .unwrap()
            .expect("agent exited before result");
        match msg {
            Msg::Out { .. } => output.extend(payload),
            Msg::Exit { code, .. } => {
                assert_eq!(code, 37, "{}", String::from_utf8_lossy(&output));
                assert!(payload.is_empty(), "native must not borrow a Cargo summary");
                break;
            }
            Msg::Error { msg, .. } => panic!("{msg}"),
            _ => {}
        }
    }
    assert!(String::from_utf8_lossy(&output).contains("managed-native-committed"));
    // No opt-in must clear inherited local experiment configuration.
    write_frame(
        &mut writer,
        &Msg::Run {
            id: 2,
            cwd: "/project".into(),
            args: vec!["--version".into()],
            env: vec![],
            roots: vec![],
            start: false,
            run_id: String::new(),
        },
        &[],
    )
    .unwrap();
    loop {
        match read_frame(&mut reader).unwrap().unwrap().0 {
            Msg::Exit { id: 2, code, .. } => {
                assert_eq!(code, 0);
                break;
            }
            Msg::Error { msg, .. } => panic!("{msg}"),
            _ => {}
        }
    }
    // Cancellation must still target the normally published managed process group.
    write_frame(
        &mut writer,
        &Msg::Run {
            id: 3,
            cwd: "/project".into(),
            args: vec!["test".into(), "--".into(), "block".into()],
            env,
            roots: vec![],
            start: false,
            run_id: String::new(),
        },
        &[],
    )
    .unwrap();
    loop {
        let (msg, payload) = read_frame(&mut reader).unwrap().unwrap();
        if matches!(msg, Msg::Out { .. })
            && String::from_utf8_lossy(&payload).contains("managed-native-blocking")
        {
            break;
        }
        if let Msg::Error { msg, .. } = msg {
            panic!("{msg}");
        }
    }
    write_frame(&mut writer, &Msg::Cancel { id: 3 }, &[]).unwrap();
    loop {
        match read_frame(&mut reader).unwrap().unwrap().0 {
            Msg::Exit { id: 3, code, .. } => {
                assert_ne!(code, 0);
                break;
            }
            Msg::Error { msg, .. } => panic!("{msg}"),
            _ => {}
        }
    }
    drop(writer);
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Opt-in acceptance with the real frozen resident engine, not the Python fake.
/// Run on the target machine with JUSTRUST_TEST_NATIVE_ENGINE and
/// JUSTRUST_TEST_NATIVE_TOOLCHAIN naming matching immutable artifacts.
#[test]
#[ignore = "requires a frozen real native engine and matching toolchain"]
fn real_resident_test_cancel_reaps_user_process_and_keeps_daemon_reusable() {
    use std::io::Read;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn process_start(pid: u32) -> Option<String> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat.rsplit_once(')')?
            .1
            .split_whitespace()
            .nth(19)
            .map(str::to_owned)
    }
    struct Cleanup {
        user_process: Option<(u32, String)>,
        agent: Option<std::process::Child>,
        client: PathBuf,
        engine: PathBuf,
        toolchain: PathBuf,
        home: PathBuf,
        root: PathBuf,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some((pid, start)) = &self.user_process
                && process_start(*pid).as_ref() == Some(start)
            {
                // SAFETY: the PID start-time witness still identifies this fixture's child.
                unsafe {
                    libc::kill(*pid as i32, libc::SIGKILL);
                }
            }
            if let Some(agent) = &mut self.agent {
                let _ = agent.kill();
                let _ = agent.wait();
            }
            let _ = Command::new(&self.client)
                .args(["hybrid", "stop"])
                .env("JUSTRUST_NATIVE_ENGINE", &self.engine)
                .env("JUSTRUST_HYBRID_TOOLCHAIN", &self.toolchain)
                .env("JR_ENGINE_HOME", &self.home)
                .env_remove("JUSTRUST_HYBRID")
                .env_remove("JUSTRUST_HYBRID_PREWARM")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    let engine = PathBuf::from(
        std::env::var_os("JUSTRUST_TEST_NATIVE_ENGINE")
            .expect("set JUSTRUST_TEST_NATIVE_ENGINE to the frozen native engine"),
    );
    let toolchain = PathBuf::from(
        std::env::var_os("JUSTRUST_TEST_NATIVE_TOOLCHAIN")
            .expect("set JUSTRUST_TEST_NATIVE_TOOLCHAIN to its matching toolchain"),
    );
    let client = std::env::var_os("JUSTRUST_TEST_HOSTED_CLIENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_justrust")));
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").expect("HOME for native acceptance scratch"))
                .join(".justrust/acceptance")
        });
    std::fs::create_dir_all(&base).unwrap();
    let root = base.join(format!("jrc-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let home = root.join("native");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut cleanup = Cleanup {
        user_process: None,
        agent: None,
        client,
        engine,
        toolchain,
        home,
        root,
    };
    let project = cleanup.root.join("root/project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        "[package]\nname='jr-cancel-probe'\nversion='0.1.0'\nedition='2021'\n[workspace]\n",
    )
    .unwrap();
    let receipt = project.join("started");
    let body = format!(
        r#"
#[cfg(test)] mod tests {{
    #[test] fn long_probe() {{
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open({receipt:?}).unwrap();
        writeln!(file, "{{}}", std::process::id()).unwrap();
        file.sync_all().unwrap();
        println!("REAL_NATIVE_LONG_STARTED");
        std::thread::sleep(std::time::Duration::from_secs(60));
        panic!("cancellation failed to stop test");
    }}
    #[test] fn fast_probe() {{ println!("REAL_NATIVE_FAST_FINISHED"); }}
}}
"#,
        receipt = receipt.to_str().unwrap()
    );
    std::fs::write(project.join("src/lib.rs"), body).unwrap();
    let guest_home = std::env::var("HOME").expect("HOME for managed guest PATH");
    // PATH is a native worker identity input. Match start_run's guest PATH
    // preparation exactly rather than accidentally warming another worker.
    let managed_path = format!(
        "{guest_home}/.cargo/bin:/opt/homebrew/bin:{guest_home}/.homebrew/bin:/usr/local/bin:{}",
        std::env::var("PATH").unwrap_or_default()
    );
    let prepared = Command::new(&cleanup.client)
        .env("PATH", managed_path)
        .args(["hybrid", "prewarm", "test", "--lib"])
        .current_dir(&project)
        .env("CARGO_BUILD_JOBS", "2")
        .env("JUSTRUST_NATIVE_ENGINE", &cleanup.engine)
        .env("JUSTRUST_HYBRID_TOOLCHAIN", &cleanup.toolchain)
        .env("JR_ENGINE_HOME", &cleanup.home)
        .env_remove("JUSTRUST_HYBRID")
        .env_remove("JUSTRUST_HYBRID_PREWARM")
        .output()
        .unwrap();
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert!(!receipt.exists(), "prewarm executed a test body");
    let agent = Command::new(&cleanup.client)
        .arg("__agent")
        .arg(cleanup.root.join("root"))
        .env("JUSTRUST_HOME", cleanup.root.join("justrust"))
        .env("JUSTRUST_REMOTE_TOOLCHAIN", "system")
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .env("JUSTRUST_NO_UPDATE", "1")
        .env_remove("JUSTRUST_HYBRID")
        .env_remove("JUSTRUST_HYBRID_PREWARM")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    cleanup.agent = Some(agent);
    let agent = cleanup.agent.as_mut().unwrap();
    let mut writer = BufWriter::new(agent.stdin.take().unwrap());
    let stdout = agent.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(frame)) = read_frame(&mut reader) {
            if tx.send(frame).is_err() {
                break;
            }
        }
    });
    let recv = || {
        rx.recv_timeout(Duration::from_secs(20))
            .expect("managed agent response deadline")
    };
    write_frame(
        &mut writer,
        &Msg::Hello {
            version: VERSION,
            caps: vec![],
        },
        &[],
    )
    .unwrap();
    assert!(matches!(recv().0, Msg::Hello { .. }));
    let env = vec![
        ("CARGO_BUILD_JOBS".into(), "2".into()),
        ("JUSTRUST_HOSTED_HYBRID".into(), "ready".into()),
        (
            "JUSTRUST_HOSTED_HYBRID_ENGINE".into(),
            cleanup.engine.to_str().unwrap().into(),
        ),
        (
            "JUSTRUST_HOSTED_HYBRID_TOOLCHAIN".into(),
            cleanup.toolchain.to_str().unwrap().into(),
        ),
        (
            "JUSTRUST_HOSTED_HYBRID_HOME".into(),
            cleanup.home.to_str().unwrap().into(),
        ),
    ];
    let args = |name: &str| {
        vec![
            "test".into(),
            "--lib".into(),
            name.into(),
            "--".into(),
            "--nocapture".into(),
        ]
    };
    write_frame(
        &mut writer,
        &Msg::Run {
            id: 1,
            cwd: "/project".into(),
            args: args("long_probe"),
            env: env.clone(),
            roots: vec![],
            start: false,
            run_id: "real-native-cancel".into(),
        },
        &[],
    )
    .unwrap();
    let mut long_output = String::new();
    loop {
        let (msg, payload) = recv();
        match msg {
            Msg::Out { .. } => long_output.push_str(&String::from_utf8_lossy(&payload)),
            Msg::Error { msg, .. } => panic!("{msg}"),
            Msg::Exit { code, .. } => {
                panic!("test exited before cancellation ({code}): {long_output}")
            }
            _ => {}
        }
        if long_output.contains("REAL_NATIVE_LONG_STARTED") {
            break;
        }
    }
    let mut contents = String::new();
    std::fs::File::open(&receipt)
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    let lines: Vec<_> = contents.lines().collect();
    assert_eq!(lines.len(), 1, "test must start exactly once");
    let pid: u32 = lines[0].parse().unwrap();
    cleanup.user_process = Some((pid, process_start(pid).expect("live fixture test process")));
    assert!(
        !long_output.contains("Cargo fallback"),
        "must cancel actual resident execution: {long_output}"
    );
    assert!(PathBuf::from(format!("/proc/{pid}")).exists());
    let cancelled = Instant::now();
    write_frame(&mut writer, &Msg::Cancel { id: 1 }, &[]).unwrap();
    loop {
        let (msg, payload) = recv();
        match msg {
            Msg::Out { .. } => long_output.push_str(&String::from_utf8_lossy(&payload)),
            Msg::Exit { id: 1, code, .. } => {
                assert_ne!(code, 0);
                break;
            }
            Msg::Error { msg, .. } => panic!("{msg}"),
            _ => {}
        }
    }
    assert!(
        cancelled.elapsed() < Duration::from_secs(10),
        "test cancellation must be prompt"
    );
    let until = Instant::now() + Duration::from_secs(5);
    while PathBuf::from(format!("/proc/{pid}")).exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "cancel left actual user test process {pid} alive/unreaped"
    );
    assert_eq!(
        std::fs::read_to_string(&receipt).unwrap(),
        contents,
        "cancel or fallback replayed test"
    );
    write_frame(
        &mut writer,
        &Msg::Run {
            id: 2,
            cwd: "/project".into(),
            args: args("fast_probe"),
            env,
            roots: vec![],
            start: false,
            run_id: "real-native-after-cancel".into(),
        },
        &[],
    )
    .unwrap();
    let mut fast_output = String::new();
    loop {
        let (msg, payload) = recv();
        match msg {
            Msg::Out { .. } => fast_output.push_str(&String::from_utf8_lossy(&payload)),
            Msg::Exit { id: 2, code, .. } => {
                assert_eq!(code, 0, "{fast_output}");
                break;
            }
            Msg::Error { msg, .. } => panic!("{msg}"),
            _ => {}
        }
    }
    assert!(
        fast_output.contains("REAL_NATIVE_FAST_FINISHED"),
        "{fast_output}"
    );
    assert!(
        fast_output.contains("resident committed"),
        "daemon must remain reusable: {fast_output}"
    );
    assert!(!fast_output.contains("Cargo fallback"), "{fast_output}");
    assert_eq!(std::fs::read_to_string(&receipt).unwrap(), contents);
    drop(writer);
    assert!(cleanup.agent.as_mut().unwrap().wait().unwrap().success());
}
