//! Confirm a toolchain-class failure against stock tools.
//!
//! When `failure` blames the toolchain, a detached low-priority process
//! reruns the same command with the rustup toolchain, no wrapper, no
//! caches and a private target dir. Stock passing proves the failure was
//! ours (a correctness bug); stock failing too means the code or the
//! command, or a nightly-only project. The verdict lands in the run dir as
//! `stock.json` and shows in `justrust show` and `justrust runs --ours`.
//! Nothing here changes the agent's run or waits on it.

use crate::paths;
use crate::sys::Detach;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const ARG: &str = "__stock-verify";
const FILE: &str = "stock.json";
/// A cold stock build of a large workspace can take a while. Give up after this.
const TIMEOUT_SECS: u64 = 30 * 60;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Verdict {
    /// "passed": stock succeeded, the failure was ours. "failed": stock
    /// failed too. "error": the check could not run.
    pub outcome: String,
    pub exit: Option<i32>,
    pub secs: f64,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Verdict {
    pub fn line(&self) -> String {
        match self.outcome.as_str() {
            "passed" => format!(
                "stock toolchain PASSED in {:.0}s: confirmed justrust/toolchain bug",
                self.secs
            ),
            "failed" => format!(
                "stock toolchain also failed (exit {}): likely the code, command or a nightly-only project",
                self.exit.map_or("?".into(), |c| c.to_string())
            ),
            _ => format!(
                "stock check could not run: {}",
                self.detail.as_deref().unwrap_or("unknown")
            ),
        }
    }
}

pub fn load(run_dir: &Path) -> Option<Verdict> {
    serde_json::from_slice(&std::fs::read(run_dir.join(FILE)).ok()?).ok()
}

fn enabled() -> bool {
    !matches!(
        std::env::var("JUSTRUST_STOCK_VERIFY").as_deref(),
        Ok("0" | "off" | "no" | "false")
    ) && std::env::var_os("JUSTRUST_STOCK_VERIFYING").is_none()
}

/// The stock command for a recorded one: compile-only, since the
/// classifier only blames the toolchain for build failures.
pub fn stock_args(args: &[String]) -> Option<Vec<String>> {
    let (head, _) = match args.iter().position(|a| a == "--") {
        Some(i) => args.split_at(i),
        None => (args, &[][..]),
    };
    let sub = head.first()?.as_str();
    let mut out: Vec<String> = match sub {
        "check" | "c" | "build" | "b" | "clippy" | "rustc" => head.to_vec(),
        "test" | "t" | "bench" => {
            let mut v = head.to_vec();
            if !v.iter().any(|a| a == "--no-run") {
                v.push("--no-run".into());
            }
            v
        }
        "run" | "r" => {
            let mut v = head.to_vec();
            v[0] = "build".into();
            v
        }
        _ => return None,
    };
    // Our own target dir is set through the environment.
    let mut i = 0;
    while i < out.len() {
        if out[i] == "--target-dir" {
            out.drain(i..(i + 2).min(out.len()));
        } else if out[i].starts_with("--target-dir=") {
            out.remove(i);
        } else {
            i += 1;
        }
    }
    Some(out)
}

/// Start verification for a run classified as a toolchain failure.
pub fn maybe_spawn(id: &str) {
    if !enabled() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .args([ARG, id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .detached()
        .spawn();
}

/// The detached worker.
pub fn main(id: Option<String>) -> ! {
    crate::sys::lower_priority();
    if let Some(id) = id
        && let Ok(dir) = paths::runs_dir().map(|d| d.join(&id))
    {
        let v = verify(&dir).unwrap_or_else(|e| Verdict {
            outcome: "error".into(),
            exit: None,
            secs: 0.0,
            args: Vec::new(),
            detail: Some(format!("{e:#}")),
        });
        if let Ok(json) = serde_json::to_vec_pretty(&v) {
            let _ = std::fs::write(dir.join(FILE), json);
        }
    }
    std::process::exit(0)
}

fn verify(dir: &Path) -> Result<Verdict> {
    let s: crate::summary::Summary =
        serde_json::from_slice(&std::fs::read(dir.join("summary.json"))?)?;
    let args = stock_args(&s.args).context("command has no stock compile equivalent")?;
    // One verification at a time: a cold stock build is heavy.
    let root = paths::home()?.join("stock-verify");
    std::fs::create_dir_all(&root)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("lock"))?;
    lock.lock()?;
    let target = root.join(format!("target-{:016x}", fxhash(&s.cwd)));
    let me = std::env::current_exe()?;
    let me = me.canonicalize().unwrap_or(me);
    let cargo = stock_tool("cargo", &me).context("no stock cargo on PATH")?;
    let rustc = stock_tool("rustc", &me).context("no stock rustc on PATH")?;
    let mut cmd = Command::new(&cargo);
    cmd.args(&args)
        .current_dir(&s.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("JUSTRUST_")
            || k.starts_with("RUSTC")
            || k == "RUSTDOC"
            || k == "CARGO_BUILD_RUSTC_WRAPPER"
            || k == "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"
            || k == "CARGO_BUILD_RUSTC"
            || k == "CARGO_TARGET_DIR"
            || k == "CARGO_BUILD_TARGET_DIR"
        {
            cmd.env_remove(k.as_ref());
        }
    }
    cmd.env("RUSTC", &rustc)
        .env("CARGO_BUILD_RUSTC_WRAPPER", "")
        .env("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "")
        .env("CARGO_TARGET_DIR", &target)
        .env("JUSTRUST_STOCK_VERIFYING", "1");
    let start = paths::now();
    let mut child = cmd.spawn()?;
    let stderr = child.stderr.take();
    let tail = std::thread::spawn(move || {
        use std::io::BufRead;
        let mut last = std::collections::VecDeque::new();
        if let Some(e) = stderr {
            for l in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                if l.trim_start().starts_with("error") {
                    last.push_back(l);
                    if last.len() > 3 {
                        last.pop_front();
                    }
                }
            }
        }
        last.into_iter().collect::<Vec<_>>().join(" | ")
    });
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break Some(st);
        }
        if paths::now() - start > TIMEOUT_SECS as f64 {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    let errors = tail.join().unwrap_or_default();
    let secs = paths::now() - start;
    // Toolchain failures are rare; never let verification targets pile up.
    let _ = std::fs::remove_dir_all(&target);
    Ok(match status {
        None => Verdict {
            outcome: "error".into(),
            exit: None,
            secs,
            args,
            detail: Some(format!("timed out after {TIMEOUT_SECS}s")),
        },
        Some(st) => Verdict {
            outcome: if st.success() { "passed" } else { "failed" }.into(),
            exit: st.code(),
            secs,
            args,
            detail: (!errors.is_empty()).then_some(errors),
        },
    })
}

/// A cargo/rustc on PATH that is not justrust (the `cargo` proxy symlink).
fn stock_tool(name: &str, me: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe = crate::sys::exe_name(name);
    std::env::split_paths(&path)
        .map(|d| d.join(&exe))
        .filter(|p| p.is_file())
        .find(|p| p.canonicalize().map_or(true, |c| c != *me))
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stock_args_are_compile_only_and_drop_target_dir() {
        assert_eq!(
            stock_args(&v(&["test", "-p", "x", "--target-dir", "/t", "--", "filt"])),
            Some(v(&["test", "-p", "x", "--no-run"]))
        );
        assert_eq!(
            stock_args(&v(&["run", "--bin", "a", "--target-dir=/t", "--", "x"])),
            Some(v(&["build", "--bin", "a"]))
        );
        assert_eq!(
            stock_args(&v(&["check", "--all-targets"])),
            Some(v(&["check", "--all-targets"]))
        );
        assert_eq!(stock_args(&v(&["fmt"])), None);
    }
}
