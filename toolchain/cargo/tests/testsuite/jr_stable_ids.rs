//! Stable crate ids for local incremental crates.
//!
//! With a compiler that advertises `jr-incr-lineage: v1`, a local crate keeps
//! one `-C metadata` however its dependencies resolved, and its unit hash
//! (file names, fingerprints) still tells the resolutions apart.

#![cfg(unix)]

use crate::prelude::*;
use cargo_test_support::{Project, project};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// `leaf` has a feature that `a` enables and `b` does not, so `mid`, which
/// uses `leaf`, has other dependencies in `build -p a` than in `build -p b`.
/// `bs` uses `mid` from its build script (leaf without the feature) and from
/// its program (leaf with it) in one build.
fn fixture() -> Project {
    project()
        .file(
            "Cargo.toml",
            "[workspace]\nmembers=['a','b','bs','mid','leaf']\nresolver='2'\n",
        )
        .file(
            "leaf/Cargo.toml",
            "[package]\nname='leaf'\nversion='0.1.0'\nedition='2021'\n[features]\nx=[]\n",
        )
        .file(
            "leaf/src/lib.rs",
            "pub fn value() -> u32 { if cfg!(feature = \"x\") { 10 } else { 1 } }",
        )
        .file(
            "mid/Cargo.toml",
            "[package]\nname='mid'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nleaf={path='../leaf'}\n",
        )
        .file("mid/src/lib.rs", "pub fn value() -> u32 { leaf::value() + 1 }")
        .file(
            "a/Cargo.toml",
            "[package]\nname='a'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nmid={path='../mid'}\nleaf={path='../leaf',features=['x']}\n",
        )
        .file("a/src/main.rs", "fn main() { print!(\"{}\", mid::value()) }")
        .file(
            "b/Cargo.toml",
            "[package]\nname='b'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nmid={path='../mid'}\n",
        )
        .file("b/src/main.rs", "fn main() { print!(\"{}\", mid::value()) }")
        .file(
            "bs/Cargo.toml",
            "[package]\nname='bs'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nmid={path='../mid'}\nleaf={path='../leaf',features=['x']}\n[build-dependencies]\nmid={path='../mid'}\n",
        )
        .file(
            "bs/build.rs",
            "fn main() { println!(\"cargo:rustc-env=BUILD_MID={}\", mid::value()); }",
        )
        .file(
            "bs/src/main.rs",
            "fn main() { print!(\"{} {}\", env!(\"BUILD_MID\"), mid::value()) }",
        )
        .build()
}

/// A compiler that is the real one and advertises the lineage protocol.
fn lineage_rustc(p: &Project) -> PathBuf {
    let path = p.root().join("lineage-rustc");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"-vV\" ]; then\n  rustc -vV || exit $?\n  echo 'jr-incr-lineage: v1'\nelse\n  exec rustc \"$@\"\nfi\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The real compiler without the lineage capability, whatever `rustc` on
/// `PATH` advertises (jr-rustc does).
fn plain_rustc(p: &Project) -> PathBuf {
    let path = p.root().join("plain-rustc");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"-vV\" ]; then\n  out=$(rustc -vV) || exit $?\n  printf '%s\\n' \"$out\" | grep -v '^jr-incr-lineage:'\nelse\n  exec rustc \"$@\"\nfi\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The (`-C metadata`, `-C extra-filename`) of every `mid` compilation in a
/// verbose build log, sorted.
fn mid_ids(log: &str) -> Vec<(String, String)> {
    let value = |line: &str, flag: &str| {
        line.split(flag)
            .nth(1)
            .and_then(|rest| rest.split([' ', '`']).next())
            .unwrap_or_else(|| panic!("no {flag} in {line}"))
            .to_string()
    };
    let mut ids: Vec<_> = log
        .lines()
        .filter(|line| line.contains("Running `") && line.contains("--crate-name mid "))
        .map(|line| {
            (
                value(line, "-C metadata="),
                value(line, "-C extra-filename="),
            )
        })
        .collect();
    ids.sort();
    ids
}

fn build(p: &Project, rustc: Option<&PathBuf>, args: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = p.cargo(&format!("build -v {args}"));
    cmd.env("CARGO_INCREMENTAL", "1");
    // Without a lineage compiler the build must not see the capability.
    let plain = plain_rustc(p);
    cmd.env("RUSTC", rustc.unwrap_or(&plain));
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.exec_with_output().unwrap();
    String::from_utf8(output.stderr).unwrap()
}

fn run(p: &Project, bin: &str) -> String {
    let output = std::process::Command::new(p.bin(bin)).output().unwrap();
    assert!(output.status.success(), "{bin} failed");
    String::from_utf8(output.stdout).unwrap()
}

#[cargo_test]
fn local_crate_keeps_its_id_when_its_dependencies_resolve_differently() {
    let p = fixture();
    let rustc = lineage_rustc(&p);

    let for_a = mid_ids(&build(&p, Some(&rustc), "-p a", &[]));
    assert_eq!(run(&p, "a"), "11");
    let for_b = mid_ids(&build(&p, Some(&rustc), "-p b", &[]));
    assert_eq!(run(&p, "b"), "2");
    assert_eq!((for_a.len(), for_b.len()), (1, 1), "{for_a:?} {for_b:?}");
    assert_eq!(for_a[0].0, for_b[0].0, "one crate id");
    assert_ne!(for_a[0].1, for_b[0].1, "two units, two sets of files");

    // Both resolutions stay built side by side.
    let again = build(&p, Some(&rustc), "-p a", &[]);
    assert!(!again.contains("--crate-name "), "rebuilt: {again}");
    assert_eq!(run(&p, "a"), "11");
}

#[cargo_test]
fn units_that_differ_only_in_dependencies_keep_distinct_ids_in_one_build() {
    let p = fixture();
    let rustc = lineage_rustc(&p);

    // The build script's `mid` and the program's `mid` meet in one build.
    let ids = mid_ids(&build(&p, Some(&rustc), "-p bs", &[]));
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0].0, ids[1].0, "{ids:?}");
    assert_ne!(ids[0].1, ids[1].1, "{ids:?}");
    assert_eq!(run(&p, "bs"), "2 11");
}

#[cargo_test]
fn local_crate_keeps_its_id_across_a_version_bump() {
    let manifest =
        |version: &str| format!("[package]\nname='mid'\nversion='{version}'\nedition='2021'\n");
    let p = project()
        .file("Cargo.toml", "[workspace]\nmembers=['a','mid']\n")
        .file("mid/Cargo.toml", &manifest("0.1.0"))
        .file(
            "mid/src/lib.rs",
            "pub fn version() -> &'static str { env!(\"CARGO_PKG_VERSION\") }",
        )
        .file(
            "a/Cargo.toml",
            "[package]\nname='a'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nmid={path='../mid'}\n",
        )
        .file("a/src/main.rs", "fn main() { print!(\"{}\", mid::version()) }")
        .build();
    let rustc = lineage_rustc(&p);

    let before = mid_ids(&build(&p, Some(&rustc), "-p a", &[]));
    assert_eq!(run(&p, "a"), "0.1.0");
    p.change_file("mid/Cargo.toml", &manifest("0.2.0"));
    let after = mid_ids(&build(&p, Some(&rustc), "-p a", &[]));
    assert_eq!(run(&p, "a"), "0.2.0");
    assert_eq!((before.len(), after.len()), (1, 1), "{before:?} {after:?}");
    assert_eq!(before[0].0, after[0].0, "one crate id");
    assert_ne!(before[0].1, after[0].1, "two units, two sets of files");

    // Stock ids tell the versions apart.
    let stock = |version: &str| {
        p.change_file("mid/Cargo.toml", &manifest(version));
        mid_ids(&build(&p, None, "-p a", &[]))
    };
    let (old, new) = (stock("0.1.0"), stock("0.2.0"));
    assert_eq!((old.len(), new.len()), (1, 1), "{old:?} {new:?}");
    assert_ne!(old[0].0, new[0].0);
}

#[cargo_test]
fn stock_ids_without_the_capability_or_when_disabled() {
    for (advertise, env) in [
        (false, &[][..]),
        (true, &[("JUSTRUST_STABLE_LOCAL_IDS", "0")][..]),
    ] {
        let p = fixture();
        let rustc = advertise.then(|| lineage_rustc(&p));
        let for_a = mid_ids(&build(&p, rustc.as_ref(), "-p a", env));
        let for_b = mid_ids(&build(&p, rustc.as_ref(), "-p b", env));
        assert_eq!((for_a.len(), for_b.len()), (1, 1), "{for_a:?} {for_b:?}");
        assert_ne!(for_a[0].0, for_b[0].0, "advertise={advertise}");
        assert_eq!(run(&p, "a"), "11");
        assert_eq!(run(&p, "b"), "2");
    }
}
