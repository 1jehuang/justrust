//! Transfer regression tests, separate from platform/format validation fixtures.
use super::*;

#[test]
fn exclusive_binary_selection_is_conservative() {
    let args = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        exclusive_bins(&args(&["build", "--bin", "app", "--bin=tool"])),
        Some(vec!["app".into(), "tool".into()])
    );
    assert_eq!(
        exclusive_bins(&args(&[
            "zigbuild",
            "--bin=app",
            "--target=x86_64-pc-windows-gnu"
        ])),
        Some(vec!["app.exe".into()])
    );
    assert_eq!(
        exclusive_bins(&args(&["build", "--bin=app", "--", "--lib"])),
        Some(vec!["app".into()])
    );
    for selector in [
        "--lib",
        "--bins",
        "--example=demo",
        "--examples",
        "--test",
        "--tests",
        "--bench=x",
        "--benches",
        "--all-targets",
    ] {
        assert!(
            exclusive_bins(&args(&["build", "--bin=app", selector])).is_none(),
            "{selector}"
        );
    }
    for values in [
        vec!["build"],
        vec!["test", "--bin=app"],
        vec!["rustc", "--bin=app"],
        vec!["build", "--bin"],
        vec!["build", "--bin="],
        vec!["build", "--bin=../app"],
        vec!["build", "--bin=apps/*"],
        vec!["build", "--bin=app*"],
        vec!["build", "--bin", "--lib"],
    ] {
        assert!(exclusive_bins(&args(&values)).is_none(), "{values:?}");
    }
}

#[test]
fn explicit_binary_scan_retains_runtime_libraries_and_mixed_selectors_fall_back() {
    let fixture = Fixture::new("bins", b"selected app");
    for name in [
        "stale-tool",
        "examples/demo",
        "libsupport.so",
        "libsupport.so.1",
        "support.dll",
        "libsupport.dylib",
    ] {
        let path = fixture.remote.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"other artifact").unwrap();
        crate::sys::set_mode(&path, 0o755).unwrap();
    }
    let names = |args: &[&str]| {
        let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        build_candidates(&fixture.remote, &args)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&["build", "--bin=app"]),
        [
            "app",
            "libsupport.dylib",
            "libsupport.so",
            "libsupport.so.1",
            "support.dll"
        ]
    );
    let all = names(&["build"]);
    assert_eq!(all.len(), 7);
    assert_eq!(names(&["build", "--bin=app", "--examples"]), all);
    assert_eq!(names(&["build", "--bin=app", "--lib"]), all);
    assert_eq!(names(&["build", "--bin=app*"]), all);
}

#[test]
fn explicit_binary_filter_reduces_unrequested_payload_without_dropping_shared_libraries() {
    if !available() {
        return;
    }
    let mut fixture = Fixture::new("bin-wire", b"selected app");
    for name in ["stale-tool", "libsupport.so"] {
        let path = fixture.remote.join(name);
        std::fs::write(&path, vec![21; 1 << 20]).unwrap();
        crate::sys::set_mode(&path, 0o755).unwrap();
    }
    fixture.req.have.clear();
    let legacy = fixture.frames();
    fixture.req.args = vec!["build".into(), "--bin=app".into()];
    let filtered = fixture.frames();
    let files = |frames: &[(Frame, Vec<u8>)]| {
        frames
            .iter()
            .filter_map(|(f, _)| match f {
                Frame::File { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(files(&legacy), ["app", "libsupport.so", "stale-tool"]);
    assert_eq!(files(&filtered), ["app", "libsupport.so"]);
    let before: usize = legacy.iter().map(|(_, data)| data.len()).sum();
    let after: usize = filtered.iter().map(|(_, data)| data.len()).sum();
    assert!(before > after);
    eprintln!("exclusive --bin artifact payload: legacy={before} bytes, filtered={after} bytes");
}

struct Fixture {
    root: PathBuf,
    remote: PathBuf,
    sent: PathBuf,
    saved: PathBuf,
    id_path: PathBuf,
    req: Request,
}

impl Fixture {
    fn new(label: &str, body: &[u8]) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "jr-transfer-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let remote = root.join("remote");
        let sent = root.join("sent");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&sent).unwrap();
        let path = remote.join("app");
        std::fs::write(&path, body).unwrap();
        crate::sys::set_mode(&path, 0o755).unwrap();
        let id = id_of(&path.metadata().unwrap());
        let key = path.display().to_string().replace('/', "%");
        let saved = sent.join(&key);
        let id_path = sent.join(format!("{key}.id"));
        std::fs::copy(&path, &saved).unwrap();
        std::fs::write(&id_path, &id).unwrap();
        // A deterministic metadata-only rebuild, without sleeping for mtime resolution.
        std::fs::File::open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_234_567_890),
            ))
            .unwrap();
        assert_ne!(id_of(&path.metadata().unwrap()), id);
        let req = Request {
            cwd: String::new(),
            args: vec![],
            start: 0.0,
            have: vec![Have {
                name: "app".into(),
                id: Some(id),
                mtime: 0.0,
            }],
            test_run: None,
            anchors: false,
        };
        Self {
            root,
            remote,
            sent,
            saved,
            id_path,
            req,
        }
    }

    fn frames(&self) -> Vec<(Frame, Vec<u8>)> {
        let mut wire = Vec::new();
        serve_dir(&self.remote, &self.req, &self.sent, &mut wire).unwrap();
        let mut reader = BufReader::new(wire.as_slice());
        let mut frames = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let frame: Frame = serde_json::from_str(&line).unwrap();
            let len = match &frame {
                Frame::File { len, .. } => *len,
                _ => 0,
            };
            let mut data = vec![0; len as usize];
            reader.read_exact(&mut data).unwrap();
            let end = frame == Frame::End;
            frames.push((frame, data));
            if end {
                return frames;
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn identical_rebuild_skips_payload_repeatedly_and_retains_client_base() {
    let body = vec![42; 131_073];
    let fixture = Fixture::new("same", &body);
    let original_id = std::fs::read(&fixture.id_path).unwrap();
    for _ in 0..3 {
        let frames = fixture.frames();
        assert_eq!(frames.len(), 2, "only Hello and End, no file payload");
        assert!(matches!(frames[0].0, Frame::Hello { .. }));
        assert_eq!(frames[1].0, Frame::End);
        assert_eq!(std::fs::read(&fixture.id_path).unwrap(), original_id);
        assert_eq!(std::fs::read(&fixture.saved).unwrap(), body);
    }
    assert_eq!(std::fs::read_dir(&fixture.sent).unwrap().count(), 2);
}

#[test]
fn metadata_only_rebuild_eliminates_legacy_delta_payload() {
    if !available() {
        return;
    }
    let body = vec![42; 1 << 20];
    let fixture = Fixture::new("wire-evidence", &body);
    let compressed = fixture.root.join("legacy.zst");
    // This is precisely the previous server's delta compression command.
    let status = Command::new("zstd")
        .args(["-q", "-f", "-3", "-T0", "--long=31"])
        .arg(format!("--patch-from={}", fixture.saved.display()))
        .arg(fixture.remote.join("app"))
        .arg("-o")
        .arg(&compressed)
        .status()
        .unwrap();
    assert!(status.success());
    let before = compressed.metadata().unwrap().len();
    assert!(before > 0);
    let after: usize = fixture.frames().iter().map(|(_, data)| data.len()).sum();
    assert_eq!(after, 0);
    eprintln!("metadata-only artifact payload: legacy={before} bytes, exact-reuse={after} bytes");
}

#[test]
fn identical_selected_test_keeps_the_selected_manifest() {
    let mut fixture = Fixture::new("selected", b"selected executable snapshot");
    fixture.req.test_run = Some("successful-run".into());
    let manifest = vec![serde_json::json!({
        "reason": "compiler-artifact", "profile": {"test": true},
        "executable": fixture.remote.join("app"),
    })];
    let mut wire = Vec::new();
    serve_selected_dir(
        &fixture.remote,
        &fixture.req,
        &fixture.sent,
        &mut wire,
        Some(manifest.clone()),
    )
    .unwrap();
    let frames: Vec<Frame> = std::str::from_utf8(&wire)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(frames.len(), 3);
    assert_eq!(
        frames[1],
        Frame::Selected {
            artifacts: manifest
        }
    );
    assert_eq!(frames[2], Frame::End);
}

#[test]
fn exact_comparison_rejects_byte_size_mode_changes_and_missing_files() {
    let body = vec![17; 131_073];
    let fixture = Fixture::new("compare", &body);
    let path = fixture.remote.join("app");
    assert!(identical_artifact(&path, &fixture.saved).unwrap());
    let mut changed = std::fs::read(&path).unwrap();
    changed[65_537] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    assert!(!identical_artifact(&path, &fixture.saved).unwrap());
    std::fs::copy(&fixture.saved, &path).unwrap();
    crate::sys::set_mode(&path, 0o700).unwrap();
    assert!(!identical_artifact(&path, &fixture.saved).unwrap());
    std::fs::write(&path, b"short").unwrap();
    assert!(!identical_artifact(&path, &fixture.saved).unwrap());
    std::fs::remove_file(&path).unwrap();
    assert!(identical_artifact(&path, &fixture.saved).is_err());
}

#[test]
fn changed_outputs_and_unproven_clients_still_receive_files() {
    if !available() {
        return;
    }
    let body = vec![19; 131_073];
    for case in ["bytes", "mode", "unknown", "missing", "full"] {
        let mut fixture = Fixture::new(case, &body);
        match case {
            "bytes" => {
                let mut body = std::fs::read(&fixture.saved).unwrap();
                body[65_537] ^= 1;
                std::fs::write(fixture.remote.join("app"), body).unwrap();
            }
            "mode" => crate::sys::set_mode(&fixture.remote.join("app"), 0o700).unwrap(),
            "unknown" => fixture.req.have[0].id = Some("another-client-base".into()),
            "missing" => std::fs::remove_file(&fixture.saved).unwrap(),
            "full" => fixture.req.have.clear(),
            _ => unreachable!(),
        }
        let frames = fixture.frames();
        let files: Vec<_> = frames
            .iter()
            .filter(|(f, _)| matches!(f, Frame::File { .. }))
            .collect();
        assert_eq!(files.len(), 1, "{case}");
        let (
            Frame::File {
                mode, size, patch, ..
            },
            data,
        ) = files[0]
        else {
            unreachable!()
        };
        assert_eq!(*patch, matches!(case, "bytes" | "mode"));
        let local = fixture.root.join("local");
        std::fs::create_dir_all(&local).unwrap();
        // The original client copy is the unchanged fixture body.
        std::fs::write(local.join("app"), vec![19; 131_073]).unwrap();
        let base = patch.then(|| local.join("app"));
        let dst = apply(
            &local,
            "app",
            *mode,
            *size,
            base.as_deref(),
            data,
            Validation::NativeBuild,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dst).unwrap(),
            std::fs::read(fixture.remote.join("app")).unwrap()
        );
    }
}
