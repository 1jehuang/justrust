//! `JUSTRUST_SHADOW=1` on a tiny workspace: the first run builds normally and
//! records the snapshot, a leaf-only edit runs the leaf's tests in the
//! shadow crate under the same names, an edit elsewhere builds normally
//! again, and the user's tree never gains a file.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch() -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("jr-shadow-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if e.file_name() == "target" {
                continue;
            }
            if p.is_dir() {
                todo.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn run(ws: &Path, home: &Path) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_justrust"));
    // This test itself runs under justrust/cargo: drop their environment
    // (RUSTC pointing at a shim, run dirs, cargo's per-crate vars).
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy().into_owned();
        if k.starts_with("JUSTRUST_") || k.starts_with("CARGO_") || k.starts_with("RUSTC") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .args(["test", "-p", "big", "--lib", "--", "leaf"])
        .current_dir(ws)
        .env("JUSTRUST_SHADOW", "1")
        .env("JUSTRUST_HOME", home)
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_REMOTE", "off")
        .env("JUSTRUST_BUILDD", "0")
        .env("JUSTRUST_SLOTS", "0")
        .env_remove("JUSTRUST_RUN_DIR")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn leaf_edit_routes_to_shadow_and_other_edit_does_not() {
    let d = scratch();
    let ws = d.join("ws");
    let home = d.join("home");
    write(
        &ws.join("Cargo.toml"),
        "[workspace]\nmembers = [\"big\"]\nresolver = \"2\"\n[profile.dev]\ndebug = 0\n",
    );
    write(
        &ws.join("big/Cargo.toml"),
        "[package]\nname = \"big\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        &ws.join("big/src/lib.rs"),
        "mod leaf;\nmod other;\npub fn api() -> u32 { leaf::double(other::three()) }\n",
    );
    write(
        &ws.join("big/src/other.rs"),
        "pub(crate) fn three() -> u32 { 3 }\n",
    );
    let leaf = ws.join("big/src/leaf.rs");
    let leaf_src = "pub(crate) fn double(x: u32) -> u32 { x * 2 }\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn doubles() { assert_eq!(double(2), 4); }\n}\n";
    write(&leaf, leaf_src);
    let before = files(&ws);

    let (ok, out) = run(&ws, &home);
    assert!(ok, "{out}");
    assert!(out.contains("normal build"), "{out}");
    assert!(!out.contains("leaf crate"), "{out}");

    // Leaf-only edit: routed, same test name.
    write(&leaf, &leaf_src.replace("x * 2", "x + x"));
    let (ok, out) = run(&ws, &home);
    assert!(ok, "{out}");
    assert!(out.contains("leaf crate jr-shadow-leaf"), "{out}");
    let last = std::fs::read_dir(home.join("runs"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .max()
        .unwrap();
    let s: serde_json::Value =
        serde_json::from_slice(&std::fs::read(last.join("summary.json")).unwrap()).unwrap();
    assert_eq!(s["tests"]["passed"], 1, "{out}");
    let log = std::fs::read_to_string(last.join("output.jsonl")).unwrap();
    assert!(log.contains("leaf::tests::doubles ... ok"), "{log}");

    // A failing leaf edit fails the run (no stale result).
    write(&leaf, &leaf_src.replace("x * 2", "x * 3"));
    let (ok, out) = run(&ws, &home);
    assert!(!ok, "{out}");
    assert!(out.contains("leaf::tests::doubles"), "{out}");
    write(&leaf, leaf_src);

    // Edit outside the leaf: normal build again.
    write(
        &ws.join("big/src/other.rs"),
        "pub(crate) fn three() -> u32 { 1 + 2 }\n",
    );
    let (ok, out) = run(&ws, &home);
    assert!(ok, "{out}");
    assert!(out.contains("normal build"), "{out}");
    assert!(!out.contains("leaf crate"), "{out}");

    // Then routed again.
    let (ok, out) = run(&ws, &home);
    assert!(ok, "{out}");
    assert!(out.contains("leaf crate"), "{out}");

    // The user's tree never gained or lost a file (Cargo.lock aside).
    let after: Vec<PathBuf> = files(&ws)
        .into_iter()
        .filter(|p| p.file_name().is_none_or(|n| n != "Cargo.lock"))
        .collect();
    assert_eq!(before, after);
    let _ = std::fs::remove_dir_all(&d);
}
