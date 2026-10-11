//! Per-test timing. During a recorded `test`, cargo runs every test binary
//! through `justrust __test-runner <binary> <args>` (cargo's `runner` config,
//! which also covers doc-test binaries through rustdoc's `--test-runtool`).
//!
//! The runner adds libtest's `--report-time`, so each `test x ... ok` line
//! ends in `<1.234s>` and the report can name the slowest tests: one slow
//! test sets the wall time of the whole suite.
//!
//! `--report-time` is unstable. The runner sets `RUSTC_BOOTSTRAP` only for the
//! test process it execs, never for cargo or rustc, so no fingerprint or
//! incremental cache changes. It adds the flag only when the binary is a
//! libtest harness that understands it and the user did not pass it already.
//! Anything unexpected runs the binary unchanged (fail open).

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

/// Name of the internal subcommand.
pub const SUBCOMMAND: &str = "__test-runner";

/// The `--config` value that routes test binaries through the runner.
pub fn cargo_config(me: &Path) -> Option<String> {
    let me = me.to_str()?;
    // TOML basic strings: escape backslashes and quotes.
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!(
        "target.'cfg(all())'.runner=[\"{}\",\"{SUBCOMMAND}\"]",
        esc(me)
    ))
}

/// Whether to install the runner for these cargo args: tests only, and never
/// when the user or project already configured a runner (we would replace it).
pub fn wanted(subcommand: &str, args: &[OsString]) -> bool {
    if !matches!(subcommand, "test" | "t") {
        return false;
    }
    if std::env::var_os("JUSTRUST_TEST_TIMES").is_some_and(|v| v == "0") {
        return false;
    }
    if std::env::vars_os().any(|(k, _)| {
        let k = k.to_string_lossy();
        k.starts_with("CARGO_TARGET_") && k.ends_with("_RUNNER")
    }) {
        return false;
    }
    let mut prev_config = false;
    for a in args {
        let a = a.to_string_lossy();
        let value = a.strip_prefix("--config=").or(prev_config.then_some(&*a));
        if value.is_some_and(|v| v.contains("runner")) {
            return false;
        }
        prev_config = a == "--config";
    }
    !project_has_runner()
}

/// A `runner` in any `.cargo/config.toml` from the cwd up, or in `$CARGO_HOME`.
fn project_has_runner() -> bool {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.extend(cwd.ancestors().map(|d| d.join(".cargo")));
    }
    if let Some(h) = std::env::var_os("CARGO_HOME") {
        dirs.push(h.into());
    } else if let Some(h) = dirs::home_dir() {
        dirs.push(h.join(".cargo"));
    }
    dirs.iter().any(|d| {
        ["config.toml", "config"].iter().any(|f| {
            std::fs::read_to_string(d.join(f))
                .is_ok_and(|s| s.lines().any(|l| l.trim_start().starts_with("runner")))
        })
    })
}

/// Arguments to add for a libtest binary run with `user` args, or none.
pub fn extra_args(user: &[OsString], is_libtest: bool) -> Vec<&'static str> {
    if !is_libtest {
        return Vec::new();
    }
    let mut z_given = false;
    for a in user {
        let a = a.to_string_lossy();
        // Listing, benches, and JSON output (nextest-style consumers) parse
        // the output themselves. Leave them alone.
        if a == "--report-time"
            || a == "--list"
            || a == "--bench"
            || a.starts_with("--format")
            || a == "-Zunstable-options"
        {
            return Vec::new();
        }
        if a == "-Z" {
            z_given = true;
        }
    }
    if z_given {
        return Vec::new();
    }
    vec!["-Z", "unstable-options", "--report-time"]
}

/// Whether `bin` is a libtest harness that knows `--report-time`: the help
/// text for the flag is in its read-only data. Scanned in chunks, so a
/// 300 MB test binary costs a few tens of milliseconds and little memory.
fn is_libtest(bin: &Path) -> bool {
    use std::io::Read;
    const NEEDLE: &[u8] = b"Show execution time of each test";
    let Ok(mut f) = std::fs::File::open(bin) else {
        return false;
    };
    let finder = memchr::memmem::Finder::new(NEEDLE);
    let mut buf = vec![0u8; 1 << 20];
    let mut keep = 0usize;
    loop {
        let n = match f.read(&mut buf[keep..]) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        let filled = keep + n;
        if finder.find(&buf[..filled]).is_some() {
            return true;
        }
        keep = (NEEDLE.len() - 1).min(filled);
        buf.copy_within(filled - keep..filled, 0);
    }
}

/// `justrust __test-runner <binary> <args...>`: exec the binary.
pub fn main(mut args: impl Iterator<Item = OsString>) -> ! {
    let Some(bin) = args.next() else {
        eprintln!("justrust: {SUBCOMMAND} needs a program");
        std::process::exit(2);
    };
    let user: Vec<OsString> = args.collect();
    let extra = extra_args(&user, is_libtest(Path::new(&bin)));
    let mut cmd = Command::new(&bin);
    if !extra.is_empty() {
        // Any value other than "1" or a crate name only unlocks unstable
        // options in this process. Test code that runs cargo or rustc sees
        // the user's environment otherwise.
        if std::env::var_os("RUSTC_BOOTSTRAP").is_none() {
            cmd.env("RUSTC_BOOTSTRAP", "justrust-report-time");
        }
        cmd.args(&extra);
    }
    cmd.args(&user);
    let err = cmd.exec();
    eprintln!(
        "justrust: failed to run {}: {err}",
        Path::new(&bin).display()
    );
    std::process::exit(127);
}

/// Duration of one `test name ... ok <1.234s>` line: (name, seconds, passed).
pub fn parse_timed_line(line: &str) -> Option<(&str, f64, bool)> {
    let rest = line.trim().strip_prefix("test ")?;
    let (name, status) = rest.rsplit_once(" ... ")?;
    let (status, time) = status.split_once(" <")?;
    let secs: f64 = time.strip_suffix("s>")?.parse().ok()?;
    let passed = match status {
        "ok" => true,
        "FAILED" => false,
        _ => return None,
    };
    Some((name, secs, passed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn adds_report_time_only_to_plain_libtest_runs() {
        let add = vec!["-Z", "unstable-options", "--report-time"];
        assert_eq!(extra_args(&os(&[]), true), add);
        assert_eq!(extra_args(&os(&["fps_counter", "--nocapture"]), true), add);
        assert!(extra_args(&os(&[]), false).is_empty());
        for user in [
            &["--list"][..],
            &["--report-time"],
            &["-Z", "unstable-options", "--format", "json"],
            &["--format=terse"],
            &["--bench"],
        ] {
            assert!(extra_args(&os(user), true).is_empty(), "{user:?}");
        }
    }

    #[test]
    fn parses_timed_result_lines() {
        assert_eq!(
            parse_timed_line("test a::b ... ok <12.432s>"),
            Some(("a::b", 12.432, true))
        );
        assert_eq!(
            parse_timed_line("test x ... FAILED <0.250s>"),
            Some(("x", 0.25, false))
        );
        assert_eq!(parse_timed_line("test x ... ok"), None);
        assert_eq!(parse_timed_line("test x ... ignored"), None);
        assert_eq!(parse_timed_line("test result: ok. 1 passed"), None);
    }

    #[test]
    fn config_is_valid_toml_for_cargo() {
        let c = cargo_config(Path::new("/a b/jr\"x")).unwrap();
        let v: toml::Table =
            toml::from_str(&c.replacen("target.'cfg(all())'.runner", "r", 1)).expect("toml");
        assert_eq!(v["r"].as_array().unwrap()[0].as_str(), Some("/a b/jr\"x"));
    }

    #[test]
    fn detects_libtest_harness_across_chunk_boundaries() {
        let dir = std::env::temp_dir().join(format!("jr-libtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bin");
        let mut data = vec![0u8; (1 << 20) - 10];
        data.extend_from_slice(b"Show execution time of each test.");
        std::fs::write(&p, &data).unwrap();
        assert!(is_libtest(&p));
        std::fs::write(&p, b"not a test binary").unwrap();
        assert!(!is_libtest(&p));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
