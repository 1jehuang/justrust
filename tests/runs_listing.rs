//! Public history listing keeps full-scan ordering and filters while reading a bounded tail.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn list(home: &Path, cwd: &Path, limit: usize, flags: &[&str]) -> Vec<Value> {
    let output = Command::new(env!("CARGO_BIN_EXE_justrust"))
        .current_dir(cwd)
        .env("JUSTRUST_HOME", home)
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .args(["runs", "--json", "-n", &limit.to_string()])
        .args(flags)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn history_cli_filters_and_limits_the_latest_row_per_run() {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&base).unwrap();
    #[cfg(unix)]
    let base = base.canonicalize().unwrap();
    let fixture = Fixture(base.join(format!("jr-runs-listing-{}", std::process::id())));
    let home = fixture.0.join("home");
    let cwd = fixture.0.join("workspace");
    let elsewhere = fixture.0.join("elsewhere");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let mut entries = Vec::new();
    for i in 0..600 {
        let mut entry = json!({
            // Two receipts for each run, with deliberately different metadata.
            "id": format!("run-{}", i / 2),
            "cwd": if i % 3 == 0 { &cwd } else { &elsewhere },
            "args": ["test", &format!("case-{i}")],
            "start": i as f64,
            "exit": if matches!(i % 8, 0 | 3 | 7) { 101 } else { 0 },
            "wall": 0.0, "cpu_secs": 0.0, "compile": 0.0, "test_run": 0.0,
            "units_compiled": 0, "top_unit": null, "compile_failed": false,
            "agent_session": null
        });
        if entry["exit"] != 0 {
            entry["failure"] = json!(if i % 8 == 7 { "user_code" } else { "infra" });
        }
        entries.push(entry);
    }
    let mut index = entries
        .iter()
        .map(|e| e.to_string() + "\n")
        .collect::<String>();
    assert!(index.len() > 64 * 1024);
    index.push_str("invalid record\n{\"id\":\"torn");
    std::fs::write(home.join("index.jsonl"), index).unwrap();

    for here in [false, true] {
        for mode in ["all", "failed", "ours"] {
            let mut flags = Vec::new();
            if here {
                flags.push("--here");
            }
            match mode {
                "failed" => flags.push("--failed"),
                "ours" => flags.push("--ours"),
                _ => {}
            }
            for limit in [0, 1, 17, 200, 1000] {
                let mut expected: Vec<_> = entries
                    // Only the second receipt in each pair is authoritative,
                    // even when the first receipt would match a filter.
                    .iter()
                    .skip(1)
                    .step_by(2)
                    .filter(|e| {
                        if here
                            && !e["cwd"]
                                .as_str()
                                .unwrap()
                                .starts_with(cwd.to_string_lossy().as_ref())
                        {
                            return false;
                        }
                        match mode {
                            "failed" => e["exit"] != 0,
                            "ours" => e["exit"] != 0 && e["failure"] == "infra",
                            _ => true,
                        }
                    })
                    .cloned()
                    .collect();
                expected.drain(..expected.len().saturating_sub(limit));
                assert_eq!(
                    list(&home, &cwd, limit, &flags),
                    expected,
                    "here={here} mode={mode} limit={limit}"
                );
            }
        }
    }
    std::fs::remove_file(home.join("index.jsonl")).unwrap();
    assert!(list(&home, &cwd, 200, &[]).is_empty());
}
