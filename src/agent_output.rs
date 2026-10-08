//! Compact output for coding agents.
//!
//! `justrust check/test/build/clippy/run` stream cargo's output through this
//! filter. The full output is still saved with the run, so nothing is lost,
//! but the agent's context only gets what it needs to act on:
//!
//! - errors, always, in full,
//! - the first few warnings in full, then only a count,
//! - failing tests with their panic output, plus anything a program or test prints,
//! - none of cargo's progress chatter (`Compiling`, `Running`, `Finished`, ...)
//!   or the per-test `... ok` lines.

/// Cargo status verbs that are pure progress noise for an agent.
const NOISE_VERBS: &[&str] = &[
    "Compiling",
    "Checking",
    "Fresh",
    "Dirty",
    "Running",
    "Finished",
    "Executable",
    "Doc-tests",
    "Downloaded",
    "Downloading",
    "Locking",
    "Updating",
    "Adding",
    "Documenting",
    "Generated",
    "Packaging",
    "Unpacking",
    "Building",
    "Removed",
];

pub const DEFAULT_MAX_WARNINGS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    None,
    Show,
    Hide,
}

#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Hidden {
    pub progress_lines: usize,
    pub passing_test_lines: usize,
    pub warnings_shown: usize,
    pub warnings_hidden: usize,
}

impl Hidden {
    pub fn merge(&mut self, o: Hidden) {
        self.progress_lines += o.progress_lines;
        self.passing_test_lines += o.passing_test_lines;
        self.warnings_shown += o.warnings_shown;
        self.warnings_hidden += o.warnings_hidden;
    }
}

pub struct Filter {
    stdout: bool,
    test_output: bool,
    max_warnings: usize,
    block: Block,
    last_kept_blank: bool,
    kept_any: bool,
    pub hidden: Hidden,
}

impl Filter {
    /// `stdout`: cargo's stdout carries program and test output, stderr carries
    /// cargo and rustc messages. `test_output`: stdout comes from libtest.
    pub fn new(stdout: bool, test_output: bool, max_warnings: usize) -> Filter {
        Filter {
            stdout,
            test_output,
            max_warnings,
            block: Block::None,
            last_kept_blank: true,
            kept_any: false,
            hidden: Hidden::default(),
        }
    }

    /// Decide whether to show one line (already stripped of ANSI codes and
    /// the trailing newline).
    pub fn keep(&mut self, line: &str) -> bool {
        let keep = if self.stdout {
            self.keep_stdout(line)
        } else {
            self.keep_stderr(line)
        };
        if keep {
            let blank = line.trim().is_empty();
            // Collapse runs of blank lines and drop leading ones.
            if blank && (self.last_kept_blank || !self.kept_any) {
                return false;
            }
            self.last_kept_blank = blank;
            self.kept_any = true;
        }
        keep
    }

    fn keep_stdout(&mut self, line: &str) -> bool {
        if !self.test_output {
            return true;
        }
        let t = line.trim();
        if t.starts_with("running ") && t.ends_with(" tests") || t == "running 1 test" {
            self.hidden.progress_lines += 1;
            return false;
        }
        if t.starts_with("test ")
            && (t.ends_with(" ... ok")
                || t.ends_with(" ... ignored")
                || t.contains(" ... ignored, "))
        {
            self.hidden.passing_test_lines += 1;
            return false;
        }
        if t.starts_with("test result: ok.") {
            self.hidden.progress_lines += 1;
            return false;
        }
        true
    }

    fn keep_stderr(&mut self, line: &str) -> bool {
        if line.trim().is_empty() {
            let shown = self.block == Block::Show;
            self.block = Block::None;
            return shown;
        }
        let trimmed = line.trim_start();
        let verb = trimmed.split_whitespace().next().unwrap_or("");
        let indented = line.len() != trimmed.len();
        // Cargo status lines are right-aligned verbs ("   Compiling foo v1").
        if NOISE_VERBS.contains(&verb) && (indented || verb == "Finished") {
            self.block = Block::None;
            self.hidden.progress_lines += 1;
            return false;
        }
        if line.starts_with("error") {
            self.block = Block::Show;
            return true;
        }
        if line.starts_with("warning") {
            // "warning: `crate` (lib) generated 12 warnings" repeats the count.
            if line.contains(" generated ") && line.contains(" warning") {
                self.block = Block::None;
                return false;
            }
            if self.hidden.warnings_shown < self.max_warnings {
                self.hidden.warnings_shown += 1;
                self.block = Block::Show;
                return true;
            }
            self.hidden.warnings_hidden += 1;
            self.block = Block::Hide;
            return false;
        }
        match self.block {
            Block::Hide => false,
            Block::Show | Block::None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut Filter, text: &str) -> Vec<String> {
        text.lines()
            .filter(|l| f.keep(l))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn hides_cargo_progress_and_keeps_errors() {
        let mut f = Filter::new(false, false, 3);
        let out = run(
            &mut f,
            "   Compiling foo v0.1.0 (/x)\n\
             error[E0308]: mismatched types\n \
             --> src/main.rs:1:22\n  \
             |\n\
             1 | fn broken() -> u32 { \"x\" }\n\
             \n\
             error: could not compile `foo` (bin \"foo\") due to 1 previous error\n",
        );
        assert_eq!(out[0], "error[E0308]: mismatched types");
        assert!(out.iter().any(|l| l.contains("--> src/main.rs:1:22")));
        assert!(out.last().unwrap().starts_with("error: could not compile"));
        assert!(!out.iter().any(|l| l.contains("Compiling")));
        assert_eq!(f.hidden.progress_lines, 1);
    }

    #[test]
    fn limits_warnings() {
        let mut f = Filter::new(false, false, 1);
        let out = run(
            &mut f,
            "warning: unused variable: `a`\n --> src/lib.rs:1:5\n\n\
             warning: unused variable: `b`\n --> src/lib.rs:2:5\n\n\
             warning: `foo` (lib) generated 2 warnings\n\
             \x20   Finished `dev` profile [unoptimized] target(s) in 0.1s\n",
        );
        assert_eq!(
            out,
            vec!["warning: unused variable: `a`", " --> src/lib.rs:1:5", ""]
        );
        assert_eq!(f.hidden.warnings_shown, 1);
        assert_eq!(f.hidden.warnings_hidden, 1);
    }

    #[test]
    fn keeps_failing_tests_only() {
        let mut f = Filter::new(true, true, 3);
        let out = run(
            &mut f,
            "\nrunning 3 tests\n\
             test a ... ok\n\
             test b ... FAILED\n\
             test c ... ignored\n\
             \n\
             failures:\n\
             \n\
             ---- b stdout ----\n\
             thread 'b' panicked at src/lib.rs:9:5:\n\
             assertion failed\n\
             \n\
             test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
        );
        assert_eq!(out[0], "test b ... FAILED");
        assert!(out.iter().any(|l| l.contains("panicked")));
        assert!(out.last().unwrap().starts_with("test result: FAILED"));
        assert_eq!(f.hidden.passing_test_lines, 2);
    }

    #[test]
    fn passes_program_output_through() {
        let mut f = Filter::new(true, false, 3);
        assert!(f.keep("test result: ok. this is my program talking"));
        assert!(f.keep("running 5 tests"));
    }
}
