//! The exact stock rustc used as an external differential oracle.
//!
//! Lookup order: `JR_STOCK_RUSTC`, then `/usr/bin/rustc`, then the pinned
//! rustup toolchain `~/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu`
//! (how hosted build machines provide it without root). The location never
//! decides correctness: `stock_rustc()` verifies, once per test process, that
//! the binary is exactly rustc 1.98.1 (commit 48a229cea, LLVM 22.1.8) for
//! x86_64-unknown-linux-gnu, and panics otherwise. A different compiler can
//! never silently become the oracle.
#![allow(dead_code)]

use std::process::Command;
use std::sync::OnceLock;

const RELEASE: &str = "release: 1.98.1";
const COMMIT: &str = "commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985";
const HOST: &str = "host: x86_64-unknown-linux-gnu";
const LLVM: &str = "LLVM version: 22.1.8";

pub fn stock_rustc() -> String {
    static VERIFIED: OnceLock<String> = OnceLock::new();
    VERIFIED
        .get_or_init(|| {
            // An explicit override, else /usr/bin/rustc, else the pinned rustup
            // toolchain (hosted build machines install it there without root).
            let rustup = std::env::var("HOME").ok().map(|home| {
                format!("{home}/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin/rustc")
            });
            let path = std::env::var("JR_STOCK_RUSTC")
                .ok()
                .filter(|path| !path.is_empty())
                .or_else(|| {
                    std::iter::once("/usr/bin/rustc".to_string())
                        .chain(rustup)
                        .find(|path| std::path::Path::new(path).exists())
                })
                .unwrap_or_else(|| "/usr/bin/rustc".to_string());
            let output = Command::new(&path)
                .env_remove("RUSTC_BOOTSTRAP")
                .arg("-vV")
                .output()
                .unwrap_or_else(|error| panic!("stock oracle {path}: {error}"));
            let text = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = text.lines().map(str::trim).collect();
            for required in [RELEASE, COMMIT, HOST, LLVM] {
                assert!(
                    output.status.success() && lines.contains(&required),
                    "stock oracle {path} is not the pinned rustc ({required} missing):\n{text}"
                );
            }
            path
        })
        .clone()
}
