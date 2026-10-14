//! Exercise bundled native archive invalidation through the real rustc shim.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn command(program: impl AsRef<std::ffi::OsStr>, cwd: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("JUSTRUST_") || name.starts_with("CARGO_") || name.starts_with("RUSTC")
        {
            cmd.env_remove(key);
        }
    }
    cmd
}

fn success(cmd: &mut Command) -> Output {
    let output = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(
        output.status.success(),
        "{cmd:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bundled_native_rlib_change_invalidates_shim_depcache() {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let fixture = Fixture(base.join(format!(
            "jr-native-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    let root = &fixture.0;
    let cargo_home = root.join("cargo-home");
    let source = cargo_home.join("registry/src/fixture/consumer-0.0.0");
    let deps = root.join("target/debug/deps");
    let native = root.join("native");
    let run = root.join("recorded");
    for dir in [&source, &deps, &native, &run] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(
        source.join("consumer.rs"),
        "fn main() { println!(\"{}\", wrapper::value()); }\n",
    )
    .unwrap();
    std::fs::write(root.join("wrapper.rs"), "#[link(name = \"helper\", kind = \"static\", modifiers = \"+bundle\")]\nunsafe extern \"C\" { fn helper() -> u32; }\npub fn value() -> u32 { unsafe { helper() } }\n").unwrap();
    let rustc = std::env::var_os("JUSTRUST_REAL_RUSTC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|dir| dir.join("rustc"))
                .find(|path| path.is_file())
                .expect("rustc must be on PATH")
        });
    let shim = root.join("rustc");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_justrust"), &shim).unwrap();
    let rlib = deps.join("libwrapper.rlib");
    let binary = deps.join("consumer-native");
    let mut metadata = None;
    for (version, expected_cache) in [(1, "miss"), (2, "miss"), (2, "hit")] {
        if expected_cache == "miss" {
            std::fs::write(
                native.join("helper.c"),
                format!("unsigned helper(void) {{ return {version}; }}\n"),
            )
            .unwrap();
            success(command("cc", root).args(["-c", "native/helper.c", "-o", "native/helper.o"]));
            success(command("ar", root).args(["crs", "native/libhelper.a", "native/helper.o"]));
            success(
                command(&rustc, root)
                    .args([
                        "--crate-name",
                        "wrapper",
                        "--crate-type",
                        "rlib",
                        "--edition=2021",
                        "-C",
                        "metadata=native-cache-fixed",
                        "wrapper.rs",
                        "-L",
                        "native=native",
                        "-o",
                    ])
                    .arg(&rlib),
            );
            // Verify that only the native payload changed, not Rust metadata.
            let bytes = success(command("ar", root).arg("p").arg(&rlib).arg("lib.rmeta")).stdout;
            assert!(!bytes.is_empty(), "rlib has no metadata");
            if let Some(previous) = &metadata {
                assert_eq!(previous, &bytes, "Rust metadata changed");
            } else {
                metadata = Some(bytes);
            }
        }
        // Remove outputs so the third compile must restore them, not reuse them.
        if binary.exists() {
            std::fs::remove_file(&binary).unwrap();
        }
        let mut cmd = command(&shim, &source);
        cmd.args([
            "--crate-name",
            "consumer",
            "--crate-type",
            "bin",
            "--edition=2021",
            "--emit=dep-info,link",
            "-C",
            "extra-filename=-native",
            "consumer.rs",
            "--extern",
        ])
        .arg(format!("wrapper={}", rlib.display()))
        .arg("--out-dir")
        .arg(&deps)
        .env("CARGO_HOME", &cargo_home)
        .env("CARGO_MANIFEST_DIR", &source)
        .env("CARGO_CRATE_NAME", "consumer")
        .env("CARGO_PKG_NAME", "consumer")
        .env("CARGO_PKG_VERSION", "0.0.0")
        .env("JUSTRUST_REAL_RUSTC", &rustc)
        .env("JUSTRUST_RUN_DIR", &run)
        .env("JUSTRUST_HOME", root.join("home"))
        .env("JUSTRUST_DEPCACHE_DIR", root.join("cache"))
        .env("JUSTRUST_DEPCACHE", "1")
        .env("JUSTRUST_PREBUILT", "0")
        .env("JUSTRUST_REMOTE_CACHE", "0")
        .env("JUSTRUST_PEER_CACHE", "0")
        .env("JUSTRUST_PASSES", "0")
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_NO_TELEMETRY", "1");
        success(&mut cmd);
        let actual = success(&mut command(&binary, &source));
        let log = std::fs::read_to_string(run.join("units.jsonl")).unwrap();
        let unit = log.lines().last().unwrap();
        let stdout = String::from_utf8_lossy(&actual.stdout);
        eprintln!(
            "native version {version}: output {}, unit {unit}",
            stdout.trim()
        );
        if stdout.trim() != version.to_string() {
            // On regression, prove that an uncached identical invocation sees
            // the new native payload rather than blaming the fixture/linker.
            success(cmd.env("JUSTRUST_DEPCACHE", "0"));
            let uncached = success(&mut command(&binary, &source));
            eprintln!(
                "uncached version {version}: output {}",
                String::from_utf8_lossy(&uncached.stdout).trim()
            );
        }
        assert_eq!(
            stdout.trim(),
            version.to_string(),
            "stale bundled native archive after version {version}"
        );
        assert!(unit.contains("\"local\":false"), "{unit}");
        assert!(
            unit.contains(&format!("\"depcache\":\"{expected_cache}\"")),
            "{unit}"
        );
        eprintln!("native version {version}: output {version}, depcache {expected_cache}");
    }
}
