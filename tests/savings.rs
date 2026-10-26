//! Exercise the public CLI with isolated retained-history fixtures.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cli(home: &Path, cwd: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_justrust"))
        .current_dir(cwd)
        .env("JUSTRUST_HOME", home)
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .args(["savings", "--json"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn savings_cli_filters_deduplicates_and_discloses_missing_history() {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let fixture = Fixture(base.join(format!("jr-savings-{}", std::process::id())));
    let home = fixture.0.join("home");
    let cwd = fixture.0.join("workspace");
    let sibling = fixture.0.join("workspace-other");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let entry = |id: &str, path: &Path, start: f64| {
        json!({
            "id":id,"cwd":path,"start":start,"args":[],"exit":0,"wall":1.0,
            "cpu_secs":0.0,"compile":1.0,"test_run":0.0,"units_compiled":1,
            "top_unit":null,"compile_failed":false,"agent_session":null
        })
        .to_string()
    };
    let rows = [
        entry("raw", &cwd, now),
        entry("raw", &cwd, now),
        entry("persisted", &cwd, now),
        entry("missing", &cwd, now),
        entry("sibling", &sibling, now),
        entry("old", &cwd, now - 10.0 * 86400.0),
    ];
    std::fs::write(home.join("index.jsonl"), rows.join("\n")).unwrap();
    for id in ["raw", "persisted", "sibling", "old"] {
        std::fs::create_dir_all(home.join("runs").join(id)).unwrap();
    }
    std::fs::write(
        home.join("runs/raw/units.jsonl"),
        "{\"depcache\":\"remote\",\"depcache_saved_secs\":10,\"wall\":1}\n",
    )
    .unwrap();
    std::fs::write(
        home.join("runs/raw/scripts.jsonl"),
        "{\"hit\":true,\"saved_secs\":5,\"cache_hit_overhead_secs\":2}\n",
    )
    .unwrap();
    // Persisted accounting survives deletion of raw records, and takes precedence.
    let persisted = json!({"savings": {"dependency_hits":1,"original_task_secs":4.0,
        "measured_hit_overhead_secs":6.0,"estimated_task_secs_avoided":-2.0}});
    for id in ["persisted", "sibling", "old"] {
        std::fs::write(
            home.join("runs").join(id).join("summary.json"),
            persisted.to_string(),
        )
        .unwrap();
    }
    std::fs::write(
        home.join("runs/persisted/units.jsonl"),
        "{\"depcache\":\"hit\",\"depcache_saved_secs\":999,\"wall\":0}\n",
    )
    .unwrap();
    let report = cli(&home, &cwd, &["--days", "7", "--here"]);
    assert_eq!(report["indexed_runs"], 3);
    assert_eq!(report["measured_runs"], 2);
    assert_eq!(report["unavailable_runs"], 1);
    assert_eq!(report["totals"]["estimated_task_secs_avoided"], 10.0);
    assert_eq!(report["totals"]["dependency_hits"], 2);
    assert_eq!(report["totals"]["build_script_hits"], 1);
    assert_eq!(report["totals"]["hits_without_full_overhead"], 1);
    assert_eq!(report["metric"], "estimated_task_time_avoided");
    assert!(
        report["limitations"]
            .as_str()
            .unwrap()
            .contains("Not CPU time")
    );
    let all = cli(&home, &cwd, &[]);
    assert_eq!(all["indexed_runs"], 5);
    assert_eq!(all["totals"]["estimated_task_secs_avoided"], 6.0);
    let empty = cli(&home, &cwd, &["--days", "0"]);
    assert_eq!(empty["indexed_runs"], 0);
}
