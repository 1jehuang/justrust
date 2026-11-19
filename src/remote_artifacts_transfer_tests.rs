//! Transfer regression tests, separate from platform/format validation fixtures.
use super::*;

#[test]
fn streamed_frame_stops_at_next_header_and_rejects_short_payload_before_install() {
    if !available() {
        return;
    }
    let fixture = Fixture::new("stream", b"new output");
    let encoded = Command::new("zstd")
        .args(["-q", "-c"])
        .arg(&fixture.saved)
        .output()
        .unwrap();
    assert!(encoded.status.success());
    let len = encoded.stdout.len() as u64;
    let mut wire = encoded.stdout.clone();
    wire.extend_from_slice(b"next header\n");
    let mut cursor = wire.as_slice();
    let dest = apply_frame(
        &fixture.root,
        "received",
        0o644,
        10,
        None,
        (&mut cursor).take(len),
        Validation::NativeBuild,
    )
    .unwrap();
    assert_eq!(cursor, b"next header\n");
    assert_eq!(std::fs::read(&dest).unwrap(), b"new output");
    std::fs::write(&dest, b"retain this").unwrap();
    let short = encoded.stdout.as_slice().take(len + 1);
    assert!(
        apply_frame(
            &fixture.root,
            "received",
            0o644,
            10,
            None,
            short,
            Validation::NativeBuild
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"retain this");
    assert!(!fixture.root.join(".received.jrtmp").exists());
}

#[test]
fn prediction_reconstructs_exactly_and_rejects_a_changed_base_before_install() {
    if !available() {
        return;
    }
    let mut bytes: Vec<u8> = (0..40_001usize)
        .map(|i| ((i.wrapping_mul(31) ^ (i >> 5)) & 255) as u8)
        .collect();
    let fixture = Fixture::new("prediction-round-trip", &bytes);
    let old = fixture.root.join("old");
    let new = fixture.root.join("new");
    let wire = fixture.root.join("wire.zst");
    std::fs::write(&old, &bytes).unwrap();
    bytes.splice(8192..8192, [7, 9, 11]);
    bytes[30_000] ^= 0x81;
    std::fs::write(&new, &bytes).unwrap();
    prediction::compress(&old, &new, bytes.len() as u64, &wire).unwrap();
    let mut data = std::fs::read(&wire).unwrap();
    let len = data.len() as u64;
    data.extend_from_slice(b"next frame");
    let apply_prediction = |input: &mut &[u8]| {
        apply_frame(
            &fixture.root,
            "out",
            0o644,
            bytes.len() as u64,
            Some(Basis::Predicted(&old)),
            input.take(len),
            Validation::NativeBuild,
        )
    };
    let mut input = data.as_slice();
    let dst = apply_prediction(&mut input).unwrap();
    assert_eq!(input, b"next frame");
    assert_eq!(std::fs::read(&dst).unwrap(), bytes);
    std::fs::write(&old, vec![0; 40_001]).unwrap();
    std::fs::write(&dst, b"previous valid output").unwrap();
    assert!(is_delta_failure(
        &apply_prediction(&mut data.as_slice()).unwrap_err()
    ));
    assert_eq!(std::fs::read(&dst).unwrap(), b"previous valid output");
}

#[test]
fn legacy_request_and_frame_do_not_negotiate_prediction() {
    let req: Request =
        serde_json::from_str(r#"{"cwd":"/p","args":[],"start":0,"have":[],"anchors":true}"#)
            .unwrap();
    assert!(!req.prediction_v1);
    let frame: Frame = serde_json::from_str(
        r#"{"t":"File","name":"x","id":"i","mode":420,"size":1,"patch":true,"len":1}"#,
    )
    .unwrap();
    assert!(matches!(
        frame,
        Frame::File {
            prediction_v1: false,
            ..
        }
    ));
}

#[cfg(unix)]
#[test]
#[ignore = "uses a 256 MiB source and basis to exercise the real encoder-selection threshold"]
fn large_negotiated_prediction_reaches_verified_install() {
    const SIZE: u64 = 256 << 20;
    let mut f = Fixture::new("negotiated-large", b"before");
    f.req.prediction_v1 = true;
    let src = f.remote.join("app");
    let basis = f.root.join("basis");
    for p in [&src, &f.saved] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(p)
            .unwrap()
            .set_len(SIZE)
            .unwrap();
    }
    std::fs::copy(&f.saved, &basis).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&src)
        .unwrap()
        .write_all(b"after!")
        .unwrap();
    let frames = f.frames();
    let (
        Frame::File {
            prediction_v1: true,
            patch: true,
            base: None,
            size,
            ..
        },
        bytes,
    ) = &frames[1]
    else {
        panic!("expected negotiated prediction (requires a host with at least 1.5 GiB available)");
    };
    let dst = apply_frame(
        &f.root,
        "installed",
        0o755,
        *size,
        Some(Basis::Predicted(&basis)),
        bytes.as_slice().take(bytes.len() as u64),
        Validation::NativeBuild,
    )
    .unwrap();
    assert!(identical_artifact(&src, &dst).unwrap());
}

#[test]
fn medium_prediction_checks_size_and_elf_magic() {
    let f = Fixture::new("medium-format", b"\x7fELF");
    for size in [16 << 20, (256 << 20) - 1] {
        assert!(prediction::medium_elf(&f.saved, size));
    }
    for size in [(16 << 20) - 1, 256 << 20] {
        assert!(!prediction::medium_elf(&f.saved, size));
    }
    for contents in [b"\x7fEL".as_slice(), b"!<ar", b"MZ\0\0"] {
        std::fs::write(&f.saved, contents).unwrap();
        assert!(!prediction::medium_elf(&f.saved, 16 << 20));
    }
}

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

#[test]
fn large_standalone_frame_round_trips_without_a_reference() {
    if !available() {
        return;
    }
    let mut fixture = Fixture::new("large-full", b"artifact bytes");
    fixture.req.have.clear();
    let src = fixture.remote.join("app");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&src)
        .unwrap()
        .set_len(32 << 20)
        .unwrap();
    let frames = fixture.frames();
    let (
        Frame::File {
            patch: false,
            base: None,
            size,
            ..
        },
        data,
    ) = &frames[1]
    else {
        panic!("expected standalone frame");
    };
    let dst = apply(
        &fixture.root,
        "out",
        0o755,
        *size,
        None,
        data,
        Validation::NativeBuild,
    )
    .unwrap();
    assert!(identical_artifact(&src, &dst).unwrap());
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
            prediction_v1: false,
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
        Self::decode(&wire)
    }

    fn decode(wire: &[u8]) -> Vec<(Frame, Vec<u8>)> {
        let mut reader = BufReader::new(wire);
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
fn selected_pipeline_preserves_order_modes_and_stops_on_broken_output() {
    if !available() {
        return;
    }
    let mut fixture = Fixture::new("selected-pipeline", &[]);
    fixture.req.anchors = true;
    fixture.req.have.clear();
    fixture.req.test_run = Some("selected-pipeline".into());
    let names = ["second", "first", "third"];
    let mut state = 123456789u32;
    let mut body: Vec<_> = (0..(1 << 18))
        .flat_map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()
        })
        .collect();
    let manifest: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let path = fixture.remote.join(name);
            body[i] ^= 1;
            std::fs::write(&path, &body).unwrap();
            crate::sys::set_mode(&path, 0o701 + i as u32).unwrap();
            serde_json::json!({"executable":path})
        })
        .collect();
    let transfer = |out: &mut dyn Write| {
        serve_selected_dir(
            &fixture.remote,
            &fixture.req,
            &fixture.sent,
            out,
            Some(manifest.clone()),
        )
    };
    // A slice writer fails during the first incompressible file, potentially
    // while the producer is waiting to hand over the next file.
    let mut short = [0u8; 8192];
    assert!(transfer(&mut &mut short[..]).is_err());
    assert!(
        !std::fs::read_dir(&fixture.sent).unwrap().any(|e| e
            .unwrap()
            .file_type()
            .unwrap()
            .is_dir())
    );

    let mut wire = Vec::new();
    transfer(&mut wire).unwrap();
    let frames = Fixture::decode(&wire);
    assert_eq!(frames.len(), 6);
    assert_eq!(
        frames[1].0,
        Frame::Selected {
            artifacts: manifest
        }
    );
    for (i, (frame, data)) in frames[2..5].iter().enumerate() {
        let Frame::File {
            name, mode, base, ..
        } = frame
        else {
            panic!("expected file")
        };
        assert_eq!(name.as_str(), names[i]);
        assert_eq!(*mode, 0o701 + i as u32);
        assert_eq!(base.as_deref(), (i != 0).then_some(names[0]));
        let payload = fixture.root.join("payload.zst");
        std::fs::write(&payload, data).unwrap();
        let mut decoder = Command::new("zstd");
        decoder.args(["-qdc", "--long=31"]);
        if let Some(base) = base {
            decoder.arg(format!(
                "--patch-from={}",
                fixture.root.join(base).display()
            ));
        }
        let output = decoder.arg(&payload).output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            std::fs::read(fixture.remote.join(name)).unwrap()
        );
        std::fs::write(fixture.root.join(name), output.stdout).unwrap();
    }
    assert_eq!(frames[5].0, Frame::End);
}

#[test]
fn selected_tests_reuse_exact_scoped_bases_across_slots() {
    if !available() {
        return;
    }
    let mut state = 0x1357_2468u32;
    let body: Vec<u8> = (0..262144)
        .flat_map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()
        })
        .collect();
    let mut fixture = Fixture::new("selected", &body);
    let slots = fixture.root.join("target/justrust-slots");
    std::fs::create_dir_all(&slots).unwrap();
    let first = slots.join("0");
    std::fs::rename(&fixture.remote, &first).unwrap();
    fixture.remote = first;
    fixture.req.test_run = Some("successful-run".into());
    let run = |dir: &Path, req: &Request| {
        let mut wire = Vec::new();
        let manifest = vec![
            serde_json::json!({"reason":"compiler-artifact", "profile":{"test":true}, "executable":dir.join("app")}),
        ];
        serve_selected_dir(dir, req, &fixture.sent, &mut wire, Some(manifest.clone())).unwrap();
        let frames = Fixture::decode(&wire);
        assert_eq!(
            frames[1].0,
            Frame::Selected {
                artifacts: manifest
            }
        );
        frames
            .into_iter()
            .find(|(f, _)| matches!(f, Frame::File { .. }))
    };
    // Legacy metadata ids cannot authorize a cross-slot basis.
    let (
        Frame::File {
            id, patch: false, ..
        },
        full,
    ) = run(&fixture.remote, &fixture.req).unwrap()
    else {
        panic!("expected full")
    };
    fixture.req.have[0].id = Some(id);
    let other = slots.join("1");
    std::fs::create_dir(&other).unwrap();
    let path = other.join("app");
    std::fs::copy(fixture.remote.join("app"), &path).unwrap();
    assert!(run(&other, &fixture.req).is_none());
    crate::sys::set_mode(&path, 0o700).unwrap();
    assert!(matches!(
        run(&other, &fixture.req).unwrap().0,
        Frame::File {
            mode: 0o700,
            patch: true,
            ..
        }
    ));
    crate::sys::set_mode(&path, 0o755).unwrap();
    let mut changed = std::fs::read(&path).unwrap();
    changed[123] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    let original_time = fixture
        .remote
        .join("app")
        .metadata()
        .unwrap()
        .modified()
        .unwrap();
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(original_time)
        .unwrap();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..3)
            .map(|_| scope.spawn(|| run(&other, &fixture.req)))
            .collect();
        for handle in handles {
            assert!(matches!(
                handle.join().unwrap().unwrap().0,
                Frame::File { patch: true, .. }
            ));
        }
    });
    let (
        Frame::File {
            patch: true,
            size,
            mode,
            ..
        },
        delta,
    ) = run(&other, &fixture.req).unwrap()
    else {
        panic!("expected cross-slot patch")
    };
    let dst = apply(
        &fixture.remote,
        "app",
        mode,
        size,
        Some(&fixture.remote.join("app")),
        &delta,
        Validation::NativeBuild,
    )
    .unwrap();
    assert_eq!(std::fs::read(dst).unwrap(), changed);
    assert!(delta.len() < full.len() / 10);
    eprintln!(
        "selected cross-slot payload: full={} unchanged=0 delta={}",
        full.len(),
        delta.len()
    );
    fixture.req.cwd = "other-tenant/workspace".into();
    assert!(matches!(
        run(&other, &fixture.req).unwrap().0,
        Frame::File { patch: false, .. }
    ));
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
                let path = fixture.remote.join("app");
                std::fs::write(&path, body).unwrap();
                // A same-size rewrite can land in the original filesystem clock
                // tick. Keep this changed-output case distinct from the saved
                // client's metadata identity without timing-dependent sleeps.
                std::fs::File::open(&path)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(
                        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_234_567_891),
                    ))
                    .unwrap();
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
