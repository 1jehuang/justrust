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
