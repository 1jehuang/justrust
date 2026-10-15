use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jr-prototype"))
}

#[test]
fn help_and_invalid_inputs_are_explicit() {
    let help = binary().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Synthetic graph"));
    for args in [
        vec!["--workers", "0"],
        vec!["--shape", "not-a-fixture"],
        vec!["--revision", "short"],
        vec!["--size"],
        vec!["--unknown", "1"],
    ] {
        let out = binary().args(args).output().unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("jr-prototype:"));
    }
}

#[test]
fn cli_checks_reference_and_reports_complete_accounting() {
    for shape in [
        "wide",
        "chain",
        "diamond",
        "dynamic",
        "fixed-point",
        "multi-output",
        "build-check-test",
    ] {
        let out = binary()
            .args([
                "--shape",
                shape,
                "--size",
                "12",
                "--work",
                "10",
                "--workers",
                "4",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.contains("\"reference_checked\":true"), "{text}");
        assert!(text.contains("\"capture_complete\":true"), "{text}");
        assert!(text.contains("\"failures\":0"), "{text}");
        assert!(text.contains("\"revision\":null"), "{text}");
    }
}

#[test]
fn no_reference_is_labelled_and_trace_does_not_overwrite() {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = base.join(format!(
        "jr-prototype-test-{}-{}.trace",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let out = binary()
        .args(["--size", "4", "--work", "0", "--no-reference", "--trace"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"reference_checked\":false"));
    let before = std::fs::read(&path).unwrap();
    let out = binary().args(["--trace"]).arg(&path).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_file(path).unwrap();
}
