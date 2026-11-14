//! Controlled real-acquire diagnostic. This is not a full build benchmark.
//! Run explicitly with `justrust test real_acquire_warm_reuse_ab -- --ignored --nocapture`.
use super::*;

const CHILD_ENV: &str = "JUSTRUST_SLOT_ACQUIRE_FIXTURE_CHILD";
const RESULT: &str = "slot-acquire-fixture:";
const NO_REFLINK: &str = "JUSTRUST_SLOT_FIXTURE_NO_REFLINK";

#[test]
fn acquire_fixture_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let args: Vec<OsString> = ["test", "--", "fixture-filter"]
        .into_iter()
        .map(OsString::from)
        .collect();
    let start = Instant::now();
    let acquired = if std::env::var_os(NO_REFLINK).is_some() {
        acquire_with_probe(&args, |_| false)
    } else {
        acquire(&args)
    };
    if std::env::var(NO_REFLINK).as_deref() == Ok("shared") {
        assert!(
            acquired.is_none(),
            "idle ext4 must retain its warm shared target"
        );
        println!("{RESULT}{}", serde_json::json!({"shared": true}));
        return;
    }
    let slot = acquired.expect("fixture must acquire an isolated slot");
    assert!(slot.info.slot.is_some() || slot.info.how == "fallback");
    assert_eq!(&slot.args[slot.args.len() - 2..], &args[1..]);
    println!(
        "{RESULT}{}",
        serde_json::json!({"elapsed_secs": start.elapsed().as_secs_f64(), "info": slot.info})
    );
}

struct FixtureRoot(PathBuf);
impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn prepare_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join(".cargo")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"slot-acquire-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();
    std::fs::write(
        root.join(".cargo/config.toml"),
        "[build]\ntarget-dir = \"target\"\n",
    )
    .unwrap();
    let shared = root.join("target/debug");
    std::fs::create_dir_all(shared.join(".fingerprint")).unwrap();
    std::fs::create_dir_all(shared.join("deps")).unwrap();
    // Identical synthetic file trees in both arms. We measure real metadata,
    // flock, reflink and slot selection, not compiler work or Cargo freshness.
    let content = vec![42; 4096];
    for n in 0..256 {
        std::fs::write(
            shared.join(format!("deps/fixture-{n:016x}.rmeta")),
            &content,
        )
        .unwrap();
    }
}

fn run_acquisition(root: &Path, warm: bool, session: usize) -> serde_json::Value {
    run_acquisition_mode(root, warm, session, false)
}

fn run_acquisition_mode(
    root: &Path,
    warm: bool,
    session: usize,
    no_reflink: bool,
) -> serde_json::Value {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "slots::contention_tests::acquire_fixture_child",
            "--nocapture",
        ])
        .current_dir(root)
        .env(CHILD_ENV, "1")
        .env("JUSTRUST_SLOTS", "4")
        .env("JUSTRUST_JOIN", "0")
        .env("JUSTRUST_SLOT_WARM_REUSE", if warm { "1" } else { "0" })
        .env(
            "JUSTRUST_SLOT_SESSION",
            format!("fixture-session-{session}"),
        )
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .env_remove("CARGO_UNSTABLE_FINE_GRAIN_LOCKING");
    if no_reflink {
        child.env(NO_REFLINK, "1");
    } else {
        child.env_remove(NO_REFLINK);
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result = stdout
        .lines()
        .find_map(|line| line.split_once(RESULT).map(|(_, result)| result))
        .expect("child must report actual slot acquisition");
    serde_json::from_str(result).unwrap()
}

#[test]
fn real_acquire_without_reflinks_keeps_uncontended_shared_target() {
    let root = FixtureRoot(std::env::temp_dir().join(format!(
        "justrust-slot-idle-no-reflink-{}",
        std::process::id()
    )));
    prepare_fixture(&root.0);
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "slots::contention_tests::acquire_fixture_child",
            "--nocapture",
        ])
        .current_dir(&root.0)
        .env(CHILD_ENV, "1")
        .env(NO_REFLINK, "shared")
        .env("JUSTRUST_SLOTS", "4")
        .env("JUSTRUST_JOIN", "0")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .env_remove("CARGO_UNSTABLE_FINE_GRAIN_LOCKING")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.0.join("target/justrust-slots/0").exists());
}

#[test]
fn real_acquire_without_reflinks_is_isolated_and_reuses_warm_state() {
    let root = FixtureRoot(
        std::env::temp_dir().join(format!("justrust-slot-no-reflink-{}", std::process::id())),
    );
    prepare_fixture(&root.0);
    let shared = root.0.join("target/debug");
    let shared_lock = try_lock(&shared.join(".cargo-build-lock")).unwrap();
    let first = run_acquisition_mode(&root.0, true, 0, true);
    assert_eq!(first["info"]["slot"], 0);
    assert_eq!(first["info"]["how"], "new");
    assert_eq!(first["info"]["seed_secs"], 0.0);
    let slot = root.0.join("target/justrust-slots/0/debug");
    assert!(slot.is_dir());
    assert_eq!(std::fs::read_dir(&slot).unwrap().count(), 0);
    // Shared Cargo is still busy, yet acquisition returned without waiting.
    assert!(try_lock(&shared.join(".cargo-build-lock")).is_none());
    drop(shared_lock);

    std::fs::create_dir_all(slot.join("deps")).unwrap();
    std::fs::create_dir_all(slot.join(".fingerprint")).unwrap();
    std::fs::write(slot.join("deps/private-artifact"), b"private").unwrap();
    // An unlocked shared tree must not trigger reflink-only refresh either.
    let sticky = run_acquisition_mode(&root.0, true, 0, true);
    assert_eq!(sticky["info"]["how"], "sticky");
    assert_eq!(sticky["info"]["refreshed_units"], 0);
    let warm = run_acquisition_mode(&root.0, true, 1, true);
    assert_eq!(warm["info"]["how"], "warm");
    assert_eq!(std::fs::read_dir(slot.join("deps")).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(slot.join("deps/private-artifact")).unwrap(),
        b"private"
    );
    assert!(!root.0.join("target/justrust-slots/.reflink-ok").exists());

    // A busy slot still allocates a different isolated target, not shared.
    let _lease = try_lock(&root.0.join("target/justrust-slots/0.lock")).unwrap();
    let parallel = run_acquisition_mode(&root.0, true, 2, true);
    assert_eq!(parallel["info"]["slot"], 1);
    assert_eq!(parallel["info"]["how"], "new");
    assert_eq!(parallel["info"]["busy"], 1);
}

#[test]
fn real_acquire_without_reflinks_respects_legacy_contention_and_capacity() {
    let root = FixtureRoot(std::env::temp_dir().join(format!(
        "justrust-slot-legacy-no-reflink-{}",
        std::process::id()
    )));
    prepare_fixture(&root.0);
    let shared = root.0.join("target/debug");
    let _writer = try_lock(&shared.join(".cargo-lock")).unwrap();
    let first = run_acquisition_mode(&root.0, true, 0, true);
    assert_eq!(first["info"]["slot"], 0);
    assert_eq!(first["info"]["seed_secs"], 0.0);
    assert_eq!(std::fs::read_dir(shared.join("deps")).unwrap().count(), 256);
    assert_eq!(
        std::fs::read(shared.join("deps/fixture-0000000000000000.rmeta")).unwrap(),
        vec![42; 4096]
    );
    // All leases held means fail open, never overwrite a busy private tree.
    let slots = root.0.join("target/justrust-slots");
    let _leases: Vec<_> = (0..4)
        .map(|n| try_lock(&slots.join(format!("{n}.lock"))).unwrap())
        .collect();
    let output = run_acquisition_mode(&root.0, true, 1, true);
    assert!(output["info"]["slot"].is_null());
    assert_eq!(output["info"]["busy"], 4);
    assert!(!slots.join("1").exists());
}

#[test]
#[ignore = "explicit real-acquire A/B diagnostic, requires reflink filesystem and Cargo"]
fn real_acquire_warm_reuse_ab() {
    // /tmp can be tmpfs even when the checkout and scratch directory are
    // btrfs. Keep test copies on the caller-selected scratch filesystem.
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap().join("target"));
    let root = FixtureRoot(base.join(format!("slot-acquire-ab-{}", std::process::id())));
    std::fs::create_dir_all(&root.0).unwrap();
    assert!(
        reflink_supported(&root.0),
        "A/B diagnostic requires reflinks, refusing to report a fallback as a measurement"
    );
    for warm in [false, true] {
        let workspace = root.0.join(if warm { "warm" } else { "baseline" });
        prepare_fixture(&workspace);
        let samples: Vec<_> = (0..4)
            .map(|session| run_acquisition(&workspace, warm, session))
            .collect();
        let trees = std::fs::read_dir(workspace.join("target/justrust-slots"))
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && entry.file_name().to_string_lossy().parse::<usize>().is_ok()
            })
            .count();
        assert_eq!(trees, if warm { 1 } else { 4 });
        assert_eq!(samples[0]["info"]["how"], "new");
        let mut elapsed_secs = 0.0;
        let mut seed_secs = 0.0;
        for sample in &samples {
            assert_eq!(sample["info"]["join_wait_secs"], 0.0);
            elapsed_secs += sample["elapsed_secs"].as_f64().unwrap();
            seed_secs += sample["info"]["seed_secs"].as_f64().unwrap();
        }
        for sample in &samples[1..] {
            assert_eq!(sample["info"]["how"], if warm { "warm" } else { "new" });
            if warm {
                assert_eq!(sample["info"]["seed_secs"], 0.0);
            }
        }
        let copied_entries: usize = (0..trees)
            .map(|slot| {
                std::fs::read_dir(
                    workspace.join(format!("target/justrust-slots/{slot}/debug/deps")),
                )
                .unwrap()
                .count()
            })
            .sum();
        assert_eq!(copied_entries, trees * 256);
        println!(
            "{}",
            serde_json::json!({
                "diagnostic": "real-slot-acquire-only",
                "warm_reuse": warm,
                "target_trees": trees,
                "copied_dep_entries": copied_entries,
                "acquire_secs": elapsed_secs,
                "seed_secs": seed_secs,
                "samples": samples,
            })
        );
    }
}
