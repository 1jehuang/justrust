//! The `llvm` feature builds against the one LLVM blaze pins (`llvm.json` beside this script:
//! LLVM 23.1.4, built by .github/scripts/llvm.py and fetched by its sha256) and no other.
//!
//! `LLVM_SYS_231_PREFIX` names the prefix. Its `blaze-llvm.json`, written by the build, must name
//! the pinned commit and version, and its `llvm-config` must say that version: neither a
//! distribution's LLVM nor Homebrew's is a source, and a prefix that is not the pinned one stops
//! the build with one message.
//!
//! inkwell's llvm-sys (vendor/llvm-sys) links that prefix statically, and on Linux the C++
//! runtime LLVM is written against, libstdc++, too: a binary built with the feature needs no
//! libLLVM and no C++ runtime where it runs. On macOS the C++ runtime is the system's libc++.
//! The linking is llvm-sys's and not this script's because a crate's native libraries come right
//! after its rlib on the link line, and inkwell, which calls LLVM, comes after this crate.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

const PREFIX: &str = "LLVM_SYS_231_PREFIX";
const MARKER: &str = "blaze-llvm.json";

fn main() {
    println!("cargo::rerun-if-env-changed={PREFIX}");
    println!("cargo::rerun-if-changed=llvm.json");
    if env::var_os("CARGO_FEATURE_LLVM").is_none() {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let pin = read_json(&manifest.join("llvm.json"));
    let (version, commit) = (field(&pin, "version"), field(&pin, "commit"));

    let prefix = env::var(PREFIX).unwrap_or_default();
    let refuse = |why: String| {
        println!(
            "cargo::error=the `llvm` feature builds against the LLVM blaze pins ({version}, \
             {} at {}): {why}. Enter `nix develop`, or fetch it with \
             `python3 .github/scripts/llvm.py fetch --target <triple> --dir <dir>` and set \
             {PREFIX} to that directory",
            field(&pin, "repo"),
            &commit[..12],
        );
    };
    if prefix.is_empty() {
        return refuse(format!("{PREFIX} is unset"));
    }
    let prefix = PathBuf::from(prefix);
    let marker = prefix.join(MARKER);
    println!("cargo::rerun-if-changed={}", marker.display());
    if !marker.exists() {
        return refuse(format!(
            "`{}` has no {MARKER}, so it is not a build of it",
            prefix.display()
        ));
    }
    let built = read_json(&marker);
    if field(&built, "commit") != commit || field(&built, "version") != version {
        return refuse(format!(
            "`{}` is LLVM {} at {}",
            prefix.display(),
            field(&built, "version"),
            field(&built, "commit")
        ));
    }
    let exe = if env::var_os("CARGO_CFG_WINDOWS").is_some() { ".exe" } else { "" };
    let config = prefix.join("bin").join(format!("llvm-config{exe}"));
    let said = ask(&config, &["--version"]);
    if said.trim() != version {
        return refuse(format!("`{}` says {}", config.display(), said.trim()));
    }
}

fn ask(program: &Path, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{} {args:?}: {e}", program.display()));
    assert!(
        out.status.success(),
        "{} {args:?}: {}",
        program.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("UTF-8 output")
}

/// The JSON object in `path`, kept as text: a build script with no dependencies reads the few
/// string fields it needs with [`field`].
fn read_json(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The string value of the top-level `"key": "value"` in `json` (none of the values it is asked
/// for holds a quote or a backslash).
fn field(json: &str, key: &str) -> String {
    let at = json.find(&format!("\"{key}\"")).unwrap_or_else(|| panic!("no \"{key}\" in {json}"));
    let rest = &json[at + key.len() + 2..];
    let rest = rest.trim_start().strip_prefix(':').expect("a `:` after the key").trim_start();
    let rest = rest.strip_prefix('"').unwrap_or_else(|| panic!("\"{key}\" is not a string"));
    rest[..rest.find('"').expect("a closing quote")].to_owned()
}
