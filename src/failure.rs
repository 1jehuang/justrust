//! Who a failed run is on: the code being built, or us.
//!
//! Cargo exits 101 for a type error, a failing test, a rustc ICE and a
//! broken remote build alike, so the exit code alone cannot tell an agent
//! whether to fix its code or a justrust maintainer whether to fix the
//! toolchain. Every failed run gets a [`Failure`]: a class plus the line
//! that decided it. The exit code stays cargo's.
//!
//! Precedence is toolchain, then infra, then user code: an ICE that also
//! printed ordinary errors is still our bug, and a test run killed by the
//! OOM killer is not a test failure.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// The project's code, tests or command: the agent should act.
    UserCode,
    /// The compiler, linker, caches or justrust itself: our bug.
    Toolchain,
    /// The machine or network: disk, memory, signals, remote route.
    Infra,
    /// Interrupted by the user or a caller (Ctrl+C, SIGTERM).
    Interrupted,
    /// Failed without a recognized cause. Treat as ours until explained.
    Unknown,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::UserCode => "user_code",
            Class::Toolchain => "toolchain",
            Class::Infra => "infra",
            Class::Interrupted => "interrupted",
            Class::Unknown => "unknown",
        }
    }

    /// Failures a justrust maintainer should look at.
    pub fn is_ours(self) -> bool {
        matches!(self, Class::Toolchain | Class::Infra | Class::Unknown)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Failure {
    pub class: Class,
    /// Short human reason, e.g. "rustc internal compiler error".
    pub reason: String,
    /// The output line that decided the class, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// What the classifier needs from a run.
pub struct Input<'a> {
    pub exit: i32,
    pub subcommand: &'a str,
    /// Cargo's stderr. Test stdout is left out so a test that prints
    /// "internal compiler error" is not mistaken for an ICE.
    pub lines: &'a [&'a str],
    pub compile_failed: bool,
    pub compile_errors: usize,
    pub environment_fault: Option<&'a str>,
    pub tests_failed: u64,
    /// Exit codes of compiler units that failed.
    pub unit_exits: &'a [i32],
    /// Compiler units that ran at all. Zero means cargo stopped before
    /// building: resolution or a rejected command line.
    pub units_ran: usize,
}

const TOOLCHAIN: &[(&str, &str)] = &[
    ("internal compiler error", "rustc internal compiler error"),
    (
        "the compiler unexpectedly panicked",
        "rustc internal compiler error",
    ),
    ("thread 'rustc' panicked", "rustc internal compiler error"),
    ("query stack during panic", "rustc internal compiler error"),
    (
        "justrust: failed to exec",
        "justrust could not start a tool",
    ),
    ("justrust: remote build failed", "hosted build failed"),
    (
        "justrust: recording disabled",
        "justrust recorder failed to start",
    ),
    ("toolchain required:", "pinned toolchain unavailable"),
    ("found invalid metadata files", "corrupt cached artifact"),
    ("can't find crate for `std`", "sysroot missing std"),
    (
        "found crate `std` compiled by an incompatible version",
        "sysroot/toolchain mismatch",
    ),
    (
        "compiled by an incompatible version of rustc",
        "artifact/toolchain mismatch",
    ),
    ("error: linker `", "linker not runnable"),
];

const INFRA: &[(&str, &str)] = &[
    ("No space left on device", "disk full"),
    ("Disk quota exceeded", "disk quota exceeded"),
    ("memory allocation of", "out of memory"),
    ("Cannot allocate memory", "out of memory"),
    ("signal: 9, SIGKILL", "process killed (SIGKILL, likely OOM)"),
    ("unable to update registry", "registry unreachable"),
    ("failed to download", "download failed"),
    ("Could not resolve host", "network: DNS failure"),
    ("Problem with the SSL CA cert", "network: TLS CA setup"),
    ("SSL connect error", "network: TLS failure"),
    ("spurious network error", "network failure"),
    ("failed to get `", "dependency fetch failed"),
];

/// Cargo rejecting the command itself: the caller's fault, not ours.
const USAGE: &[&str] = &[
    "error: no library targets found",
    "error: no bin target named",
    "error: no example target named",
    "error: no test target named",
    "error: no bench target named",
    "requires the features:",
    "cannot be tested because",
    "error: package(s) `",
    "did not match any packages",
    "error: unexpected argument",
    "error: unrecognized subcommand",
    "error: failed to parse manifest",
    "error: invalid value",
    "error: a bin target must be available",
    "error: `cargo run` could not determine which binary",
    "error: no matching package named",
    "failed to select a version for",
];

/// The bracketed attribution appended to the agent verdict.
pub fn verdict_tag(f: &Failure) -> String {
    match f.class {
        Class::UserCode => format!("your code: {}", f.reason),
        Class::Toolchain => format!(
            "NOT your code: justrust/toolchain fault ({}); retry with JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_LOCAL_REASON=toolchain-fault",
            f.reason
        ),
        Class::Infra => format!("NOT your code: environment fault ({}); retry", f.reason),
        Class::Interrupted => "interrupted".to_owned(),
        Class::Unknown => format!(
            "cause unclassified{}; check the output above",
            f.evidence
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        ),
    }
}

/// Classify a run. `None` when it succeeded.
pub fn classify(i: &Input) -> Option<Failure> {
    if i.exit == 0 {
        return None;
    }
    let find = |table: &[(&str, &'static str)]| {
        i.lines.iter().find_map(|l| {
            table
                .iter()
                .find(|(needle, _)| l.contains(needle))
                .map(|(_, why)| (*why, l.trim().to_owned()))
        })
    };
    let mk = |class, reason: &str, evidence: Option<String>| {
        Some(Failure {
            class,
            reason: reason.to_owned(),
            evidence: evidence.map(|e| e.chars().take(240).collect()),
        })
    };

    if let Some((why, line)) = find(TOOLCHAIN) {
        return mk(Class::Toolchain, why, Some(line));
    }
    // rustc exits 1 for code errors and 101 when it panics.
    if i.unit_exits.contains(&101) {
        return mk(Class::Toolchain, "rustc exited 101 (compiler panic)", None);
    }
    if let Some(fault) = i.environment_fault {
        return mk(Class::Infra, fault, None);
    }
    if let Some((why, line)) = find(INFRA) {
        return mk(Class::Infra, why, Some(line));
    }
    match i.exit {
        130 | 143 => return mk(Class::Interrupted, "interrupted", None),
        137 => return mk(Class::Infra, "killed (SIGKILL, likely OOM)", None),
        _ => {}
    }
    // A compiler unit killed by a signal (negative or >128) is not a code error.
    if let Some(&code) = i.unit_exits.iter().find(|&&c| !(0..=128).contains(&c)) {
        return mk(
            Class::Infra,
            &format!("compiler killed by signal (exit {code})"),
            None,
        );
    }

    if i.compile_failed && i.compile_errors > 0 {
        let first = i
            .lines
            .iter()
            .find(|l| {
                let t = l.trim_start();
                t.starts_with("error[E") || t.starts_with("error:")
            })
            .map(|l| l.trim().to_owned());
        return mk(Class::UserCode, "compile error in the code", first);
    }
    if i.tests_failed > 0 {
        return mk(Class::UserCode, "tests failed", None);
    }
    if let Some(l) = i.lines.iter().find(|l| USAGE.iter().any(|u| l.contains(u))) {
        return mk(
            Class::UserCode,
            "cargo rejected the command",
            Some(l.trim().to_owned()),
        );
    }
    // `--message-format short` prints `path:1:2: error[E0308]: ...`, and
    // JSON output keeps the errors off stderr entirely; cargo's
    // "due to N previous errors" still says rustc reported code errors.
    if i.compile_failed
        && let Some(l) = i.lines.iter().find(|l| {
            l.contains(": error[E")
                || l.contains(": error: ")
                || (l.contains("could not compile") && l.contains("previous error"))
        })
    {
        return mk(
            Class::UserCode,
            "compile error in the code",
            Some(l.trim().to_owned()),
        );
    }
    if let Some(l) = i.lines.iter().find(|l| l.contains("panicked at")) {
        let why = match i.subcommand {
            "run" | "r" => "the program panicked",
            _ => "a test or build script panicked",
        };
        return mk(Class::UserCode, why, Some(l.trim().to_owned()));
    }
    if matches!(i.subcommand, "run" | "r")
        && i.lines
            .iter()
            .any(|l| l.contains("process didn't exit successfully"))
    {
        return mk(Class::UserCode, "the program exited non-zero", None);
    }
    if let Some(l) = i
        .lines
        .iter()
        .find(|l| l.contains("failed to run custom build command"))
    {
        return mk(
            Class::UserCode,
            "build script failed",
            Some(l.trim().to_owned()),
        );
    }
    if i.lines.iter().any(|l| l.contains("error: test failed")) {
        return mk(
            Class::UserCode,
            "test binary failed without a test result (crash or abort)",
            None,
        );
    }
    // Cargo refused before compiling anything, with its own `error:` and
    // no justrust message: a manifest, feature or argument problem.
    if i.units_ran == 0
        && !i.lines.iter().any(|l| l.contains("justrust"))
        && let Some(l) = i.lines.iter().find(|l| l.starts_with("error: "))
    {
        return mk(
            Class::UserCode,
            "cargo rejected the command",
            Some(l.trim().to_owned()),
        );
    }
    // Cargo execs the program after a clean build, so a failed `run` that
    // got this far is the program's own exit status.
    if matches!(i.subcommand, "run" | "r") && !i.compile_failed {
        return mk(Class::UserCode, "the program exited non-zero", None);
    }
    let evidence = i
        .lines
        .iter()
        .find(|l| l.trim_start().starts_with("error"))
        .map(|l| l.trim().to_owned());
    mk(Class::Unknown, "no recognized cause", evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(exit: i32, sub: &str, lines: &[&str]) -> Option<Failure> {
        classify(&Input {
            exit,
            subcommand: sub,
            lines,
            compile_failed: false,
            compile_errors: 0,
            environment_fault: None,
            tests_failed: 0,
            unit_exits: &[],
            units_ran: 0,
        })
    }

    #[test]
    fn success_is_unclassified() {
        assert_eq!(run(0, "test", &["error: whatever"]), None);
    }

    #[test]
    fn code_errors_are_user_code() {
        let f = classify(&Input {
            exit: 101,
            subcommand: "check",
            lines: &[
                "error[E0308]: mismatched types",
                "error: could not compile `x` (lib) due to 1 previous error",
            ],
            compile_failed: true,
            compile_errors: 1,
            environment_fault: None,
            tests_failed: 0,
            unit_exits: &[1],
            units_ran: 1,
        })
        .unwrap();
        assert_eq!(f.class, Class::UserCode);
        assert_eq!(
            f.evidence.as_deref(),
            Some("error[E0308]: mismatched types")
        );
    }

    #[test]
    fn ice_beats_ordinary_errors() {
        let f = classify(&Input {
            exit: 101,
            subcommand: "check",
            lines: &[
                "error[E0308]: mismatched types",
                "error: internal compiler error: compiler/rustc_middle/src/ty/mod.rs:1: boom",
            ],
            compile_failed: true,
            compile_errors: 2,
            environment_fault: None,
            tests_failed: 0,
            unit_exits: &[101],
            units_ran: 1,
        })
        .unwrap();
        assert_eq!(f.class, Class::Toolchain);
        assert!(f.class.is_ours());
    }

    #[test]
    fn rustc_101_without_text_is_toolchain() {
        let f = classify(&Input {
            exit: 101,
            subcommand: "build",
            lines: &[],
            compile_failed: true,
            compile_errors: 0,
            environment_fault: None,
            tests_failed: 0,
            unit_exits: &[101],
            units_ran: 1,
        })
        .unwrap();
        assert_eq!(f.class, Class::Toolchain);
    }

    #[test]
    fn failing_tests_are_user_code() {
        let f = classify(&Input {
            exit: 101,
            subcommand: "test",
            lines: &["thread 'x' panicked at src/lib.rs:1:1:"],
            compile_failed: false,
            compile_errors: 0,
            environment_fault: None,
            tests_failed: 3,
            unit_exits: &[],
            units_ran: 1,
        })
        .unwrap();
        assert_eq!(f.class, Class::UserCode);
        assert_eq!(f.reason, "tests failed");
    }

    #[test]
    fn oom_during_tests_is_infra() {
        let f = classify(&Input {
            exit: 101,
            subcommand: "test",
            lines: &["memory allocation of 1048576 bytes failed"],
            compile_failed: false,
            compile_errors: 0,
            environment_fault: None,
            tests_failed: 1,
            unit_exits: &[],
            units_ran: 1,
        })
        .unwrap();
        assert_eq!(f.class, Class::Infra);
    }

    #[test]
    fn real_unexplained_runs_from_2026_11_08() {
        // The six 101s that had neither compile errors nor test results.
        let cases: &[(&str, &[&str], Class)] = &[
            (
                "run",
                &["  [77] Problem with the SSL CA cert (path? access rights?)"],
                Class::Infra,
            ),
            (
                "run",
                &["thread 'main' (1) panicked at examples/connection_latency.rs:102:43:"],
                Class::UserCode,
            ),
            (
                "build",
                &["error: target `tui_bench` in package `jcode` requires the features: `dev-bins`"],
                Class::UserCode,
            ),
            (
                "test",
                &["error: no library targets found in package `jr-ops`"],
                Class::UserCode,
            ),
            (
                "test",
                &[
                    "error: package `ratatui-core` cannot be tested because it requires dev-dependencies and is not a member of the workspace",
                ],
                Class::UserCode,
            ),
        ];
        for (sub, lines, want) in cases {
            assert_eq!(run(101, sub, lines).unwrap().class, *want, "{lines:?}");
        }
    }

    #[test]
    fn misattributed_in_history() {
        let short = classify(&Input {
            exit: 101,
            subcommand: "check",
            lines: &[
                "tests/p.rs:457:67: error[E0608]: cannot index into a value of type `X`",
                "error: could not compile `e` (test \"p\") due to 2 previous errors",
            ],
            compile_failed: true,
            compile_errors: 0,
            environment_fault: None,
            tests_failed: 0,
            unit_exits: &[1],
            units_ran: 2,
        })
        .unwrap();
        assert_eq!(short.class, Class::UserCode);
        assert_eq!(run(1, "run", &[]).unwrap().class, Class::UserCode);
        for l in [
            "error: the package 'm' does not contain this feature: test-support",
            "error: could not find `Cargo.toml` in `/tmp` or any parent directory",
        ] {
            assert_eq!(run(101, "test", &[l]).unwrap().class, Class::UserCode);
        }
        // A justrust message before the build is never blamed on the code.
        assert_eq!(
            run(101, "test", &["justrust: oops", "error: x"])
                .unwrap()
                .class,
            Class::Unknown
        );
    }

    #[test]
    fn hosted_failure_and_signals() {
        assert_eq!(
            run(
                1,
                "test",
                &["justrust: remote build failed: connection reset"]
            )
            .unwrap()
            .class,
            Class::Toolchain
        );
        assert_eq!(run(143, "test", &[]).unwrap().class, Class::Interrupted);
        assert_eq!(run(137, "test", &[]).unwrap().class, Class::Infra);
        let f = classify(&Input {
            exit: 101,
            subcommand: "test",
            lines: &["error: something new"],
            compile_failed: false,
            compile_errors: 0,
            environment_fault: None,
            tests_failed: 0,
            unit_exits: &[],
            units_ran: 3,
        })
        .unwrap();
        assert_eq!(f.class, Class::Unknown);
        assert!(f.class.is_ours());
    }
}
