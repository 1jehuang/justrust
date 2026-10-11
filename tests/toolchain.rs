//! `justrust toolchain` against a fake dist server (`file://`, no network):
//! install verifies checksums and assembles the prefix, a pinned project's
//! recorded runs use the pinned cargo and rustc, and a tampered component is
//! rejected without leaving a half-installed toolchain behind.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

const DATE: &str = "2026-10-08";
const COMPONENTS: &[(&str, &str)] = &[
    ("rustc", "rustc"),
    ("cargo", "cargo"),
    ("rust-std", "rust-std-x86_64-unknown-linux-gnu"),
    ("rust-src", "rust-src"),
    ("clippy-preview", "clippy-preview"),
    ("rustfmt-preview", "rustfmt-preview"),
    (
        "rustc-codegen-cranelift-preview",
        "rustc-codegen-cranelift-preview",
    ),
    ("rustc-dev", "rustc-dev"),
    ("llvm-tools-preview", "llvm-tools-preview"),
];

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("jr-toolchain-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sha256(p: &Path) -> String {
    let out = Command::new("sha256sum").arg(p).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn write_exe(p: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Files each fake component installs (relative to the prefix).
fn files_of(pkg: &str) -> Vec<(&'static str, String)> {
    let rustc = "#!/bin/sh\nif [ \"$1\" = -vV ]; then echo 'rustc 1.101.0-nightly (fake 2026-10-07)'; echo 'host: x86_64-unknown-linux-gnu'; exit 0; fi\nexit 0\n";
    // Records the environment a recorded run gives it, then succeeds.
    let cargo = "#!/bin/sh\n{ echo \"PATH0=${PATH%%:*}\"; echo \"RUSTC=$RUSTC\"; echo \"REAL=$JUSTRUST_REAL_RUSTC\"; echo \"ACTIVE=$JUSTRUST_TOOLCHAIN_ACTIVE\"; echo \"NIGHTLY=$JUSTRUST_TOOLCHAIN_NIGHTLY\"; echo \"ARGS=$*\"; } > \"$FAKE_CARGO_LOG\"\nexit 0\n";
    match pkg {
        "rustc" => vec![
            ("bin/rustc", rustc.into()),
            ("bin/rustdoc", "#!/bin/sh\n".into()),
            (
                "lib/rustlib/x86_64-unknown-linux-gnu/lib/shared-a",
                "a".into(),
            ),
        ],
        "cargo" => vec![("bin/cargo", cargo.into())],
        "rust-std" => vec![(
            "lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd-x.rlib",
            "std".into(),
        )],
        "rust-src" => vec![("lib/rustlib/src/rust/library/Cargo.toml", "".into())],
        "rustc-codegen-cranelift-preview" => vec![(
            "lib/rustlib/x86_64-unknown-linux-gnu/codegen-backends/librustc_codegen_cranelift.so",
            "so".into(),
        )],
        "rustc-dev" => vec![(
            "lib/rustlib/x86_64-unknown-linux-gnu/lib/librustc_driver.rlib",
            "dev".into(),
        )],
        "clippy-preview" => vec![("bin/cargo-clippy", "#!/bin/sh\n".into())],
        "rustfmt-preview" => vec![("bin/rustfmt", "#!/bin/sh\n".into())],
        _ => vec![(
            "lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-nm",
            "#!/bin/sh\n".into(),
        )],
    }
}

/// A dist tree in rust-installer format plus a channel manifest.
fn fake_dist(root: &Path, tamper: Option<&str>) -> String {
    let dist = root.join("dist").join(DATE);
    std::fs::create_dir_all(&dist).unwrap();
    let mut manifest = format!("manifest-version = \"2\"\ndate = \"{DATE}\"\n\n");
    manifest.push_str("[pkg.rustc]\nversion = \"1.101.0-nightly (fake 2026-10-07)\"\n\n");
    for (pkg, comp) in COMPONENTS {
        let top_name = format!("{pkg}-nightly-x86_64-unknown-linux-gnu");
        let build = root.join("build").join(&top_name);
        let cdir = build.join(comp);
        let mut list = String::new();
        for (rel, body) in files_of(pkg) {
            write_exe(&cdir.join(rel), &body);
            list.push_str(&format!("file:{rel}\n"));
        }
        std::fs::write(cdir.join("manifest.in"), list).unwrap();
        std::fs::write(build.join("components"), format!("{comp}\n")).unwrap();
        let tarball = dist.join(format!("{top_name}.tar.xz"));
        let ok = Command::new("tar")
            .arg("-cJf")
            .arg(&tarball)
            .arg("-C")
            .arg(root.join("build"))
            .arg(&top_name)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut hash = sha256(&tarball);
        if tamper == Some(*pkg) {
            hash = "0".repeat(64);
        }
        let target = if *pkg == "rust-src" {
            "\"*\""
        } else {
            "x86_64-unknown-linux-gnu"
        };
        manifest.push_str(&format!(
            "[pkg.{pkg}.target.{target}]\navailable = true\nxz_url = \"file://{}\"\nxz_hash = \"{hash}\"\n\n",
            tarball.display()
        ));
    }
    let mpath = dist.join("channel-rust-nightly.toml");
    std::fs::write(&mpath, manifest).unwrap();
    std::fs::write(
        dist.join("channel-rust-nightly.toml.sha256"),
        format!("{}  channel-rust-nightly.toml\n", sha256(&mpath)),
    )
    .unwrap();
    format!("file://{}", root.display())
}

fn justrust(home: &Path, server: &str, cwd: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_justrust"));
    c.current_dir(cwd)
        .env("JUSTRUST_HOME", home)
        .env("JUSTRUST_DIST_SERVER", server)
        .env("JUSTRUST_REMOTE", "0")
        .env("JUSTRUST_DEPCACHE", "0")
        .env_remove("JUSTRUST_TOOLCHAIN")
        .env_remove("JUSTRUST_RUN_DIR")
        .env_remove("JUSTRUST_DISABLE")
        .env_remove("CARGO_TARGET_DIR");
    c
}

#[test]
fn installs_pins_and_builds_with_the_pinned_toolchain() {
    let dir = scratch("ok");
    let server = fake_dist(&dir.join("server"), None);
    let home = dir.join("home");
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();

    let out = justrust(&home, &server, &project)
        .args(["toolchain", "pin", &format!("nightly-{DATE}")])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg = std::fs::read_to_string(project.join("justrust.toml")).unwrap();
    assert!(cfg.contains("channel = \"nightly-2026-10-08\""), "{cfg}");

    let prefix = home.join("toolchains/nightly-2026-10-08");
    for rel in [
        "bin/rustc",
        "bin/cargo",
        "bin/cargo-clippy",
        "bin/rustfmt",
        "lib/rustlib/src/rust/library/Cargo.toml",
        "lib/rustlib/x86_64-unknown-linux-gnu/codegen-backends/librustc_codegen_cranelift.so",
        // Shared directory filled by three components.
        "lib/rustlib/x86_64-unknown-linux-gnu/lib/shared-a",
        "lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd-x.rlib",
        "lib/rustlib/x86_64-unknown-linux-gnu/lib/librustc_driver.rlib",
        "justrust-toolchain.json",
    ] {
        assert!(prefix.join(rel).exists(), "missing {rel}");
    }
    let info = std::fs::read_to_string(prefix.join("justrust-toolchain.json")).unwrap();
    assert!(
        info.contains("rustc 1.101.0-nightly (fake 2026-10-07)"),
        "{info}"
    );
    // No staging leftovers.
    let names: Vec<String> = std::fs::read_dir(home.join("toolchains"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".staging") || n.starts_with(".trash"))
        .collect();
    assert!(names.is_empty(), "{names:?}");

    // A recorded build runs the pinned cargo, with the pinned rustc behind
    // the shim and the pinned bin dir first on PATH.
    let log = dir.join("cargo.log");
    let out = justrust(&home, &server, &project)
        .args(["cargo", "build"])
        .env("FAKE_CARGO_LOG", &log)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let seen = std::fs::read_to_string(&log).unwrap();
    let bin = prefix.join("bin");
    assert!(seen.contains(&format!("PATH0={}", bin.display())), "{seen}");
    assert!(
        seen.contains(&format!("REAL={}", bin.join("rustc").display())),
        "{seen}"
    );
    assert!(
        seen.contains("RUSTC=") && seen.contains("bin/shim/rustc"),
        "{seen}"
    );
    assert!(seen.contains("ACTIVE=nightly-2026-10-08"), "{seen}");
    assert!(seen.contains("NIGHTLY=1"), "{seen}");

    // `JUSTRUST_TOOLCHAIN=system` opts out for one run.
    let _ = std::fs::remove_file(&log);
    let out = justrust(&home, &server, &project)
        .args(["cargo", "build"])
        .env("FAKE_CARGO_LOG", &log)
        .env("JUSTRUST_TOOLCHAIN", "system")
        .env("JUSTRUST_REAL_CARGO", "/bin/true")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        !log.exists(),
        "system toolchain must not run the fake cargo"
    );

    // Unrecorded commands (`cargo fmt`, `metadata`) also use the pin.
    let _ = std::fs::remove_file(&log);
    let out = justrust(&home, &server, &project)
        .args(["cargo", "metadata"])
        .env("FAKE_CARGO_LOG", &log)
        .output()
        .unwrap();
    assert!(out.status.success());
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(seen.contains("ARGS=metadata"), "{seen}");

    let out = justrust(&home, &server, &project)
        .args(["toolchain", "unpin"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        !std::fs::read_to_string(project.join("justrust.toml"))
            .unwrap()
            .contains("channel")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rejects_a_tampered_component_and_fails_open() {
    let dir = scratch("tamper");
    let server = fake_dist(&dir.join("server"), Some("rustc-dev"));
    let home = dir.join("home");
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("justrust.toml"),
        "[toolchain]\nchannel = \"nightly-2026-10-08\"\n",
    )
    .unwrap();

    let out = justrust(&home, &server, &project)
        .args(["toolchain", "install"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("checksum mismatch"), "{err}");
    let left: Vec<_> = std::fs::read_dir(home.join("toolchains"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with(".install.lock"))
        .collect();
    assert!(left.is_empty(), "{left:?}");

    // A build fails open to the system toolchain and remembers the failure,
    // so the next build does not retry the download.
    let run = || {
        let out = justrust(&home, &server, &project)
            .args(["cargo", "build"])
            .env("JUSTRUST_REAL_CARGO", "/bin/true")
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let first = run();
    assert!(first.contains("could not install toolchain"), "{first}");
    let second = run();
    assert!(second.contains("failed to install"), "{second}");
    assert!(!second.contains("installing pinned toolchain"), "{second}");
    let _ = std::fs::remove_dir_all(&dir);
}
