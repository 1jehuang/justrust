//! The exact stock rustc used as an external differential oracle.
//!
//! Lookup order: `JR_STOCK_RUSTC` alone when set, else the first of
//! `/usr/bin/rustc` and the pinned rustup toolchain that verifies, `~/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu`
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
            // An explicit override is the only candidate when set. Otherwise
            // try /usr/bin/rustc, then the pinned rustup toolchain, and use the
            // first one that verifies as exactly the pinned release. A system
            // rustc upgrade must not shadow the pinned toolchain.
            let candidates: Vec<String> = match std::env::var("JR_STOCK_RUSTC")
                .ok()
                .filter(|path| !path.is_empty())
            {
                Some(path) => vec![path],
                None => std::iter::once("/usr/bin/rustc".to_string())
                    .chain(std::env::var("HOME").ok().map(|home| {
                        format!(
                            "{home}/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin/rustc"
                        )
                    }))
                    .collect(),
            };
            let mut rejected = Vec::new();
            for path in candidates {
                let output = match Command::new(&path)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .arg("-vV")
                    .output()
                {
                    Ok(output) => output,
                    Err(error) => {
                        rejected.push(format!("{path}: {error}"));
                        continue;
                    }
                };
                let text = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<&str> = text.lines().map(str::trim).collect();
                match [RELEASE, COMMIT, HOST, LLVM]
                    .into_iter()
                    .find(|required| !(output.status.success() && lines.contains(required)))
                {
                    None => return path,
                    Some(required) => rejected.push(format!(
                        "{path} is not the pinned rustc ({required} missing):\n{text}"
                    )),
                }
            }
            panic!(
                "no stock oracle is the pinned rustc:\n{}",
                rejected.join("\n")
            )
        })
        .clone()
}
