//! `justrust agents`: tell coding agents on this machine to use justrust.
//!
//! Adds a short instruction block (docs/AGENTS-snippet.md) to the global
//! instruction file of every agent that is installed:
//!
//! ```text
//! ~/.claude/CLAUDE.md   Claude Code   (when ~/.claude exists)
//! ~/.codex/AGENTS.md    Codex         (when ~/.codex exists)
//! ~/.gemini/GEMINI.md   Gemini CLI    (when ~/.gemini exists)
//! ~/AGENTS.md           Jcode         (when ~/.jcode or ~/AGENTS.md exists)
//! ```
//!
//! The block sits between marker comments, so running it again updates it in
//! place and `--remove` takes it out without touching anything else. A file
//! that already mentions justrust outside the markers (written by hand) is
//! left alone. The installer runs this; `JUSTRUST_NO_AGENTS=1` skips it.

use anyhow::Result;
use std::path::{Path, PathBuf};

pub const SNIPPET: &str = include_str!("../docs/AGENTS-snippet.md");
const BEGIN: &str = "<!-- justrust:begin (managed by `justrust agents`; `justrust agents --remove` takes it out) -->";
const BEGIN_PREFIX: &str = "<!-- justrust:begin";
const END: &str = "<!-- justrust:end -->";

#[derive(Debug, PartialEq)]
pub enum Edit {
    /// New file content.
    Write(String),
    /// Already up to date.
    Same,
    /// Mentions justrust outside the managed block: left alone.
    HandWritten,
}

fn block() -> String {
    format!("{BEGIN}\n{}\n{END}\n", SNIPPET.trim_end())
}

/// The managed block's byte range (including a trailing newline), if present.
fn managed_range(s: &str) -> Option<(usize, usize)> {
    let start = s.find(BEGIN_PREFIX)?;
    let end_rel = s[start..].find(END)?;
    let mut end = start + end_rel + END.len();
    if s[end..].starts_with('\n') {
        end += 1;
    }
    Some((start, end))
}

pub fn add(old: &str) -> Edit {
    let new = match managed_range(old) {
        Some((a, b)) => format!("{}{}{}", &old[..a], block(), &old[b..]),
        None => {
            if old.contains("justrust") {
                return Edit::HandWritten;
            }
            let sep = match old {
                "" => "",
                s if s.ends_with("\n\n") => "",
                s if s.ends_with('\n') => "\n",
                _ => "\n\n",
            };
            format!("{old}{sep}{}", block())
        }
    };
    if new == old {
        Edit::Same
    } else {
        Edit::Write(new)
    }
}

pub fn remove(old: &str) -> Edit {
    match managed_range(old) {
        None => Edit::Same,
        Some((a, b)) => {
            let mut head = old[..a].to_string();
            // Drop the blank line `add` put before the block.
            if head.ends_with("\n\n") {
                head.pop();
            }
            Edit::Write(format!("{head}{}", &old[b..]))
        }
    }
}

/// (agent, file) for every agent installed under `home`.
pub fn targets(home: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut t = Vec::new();
    if home.join(".claude").is_dir() {
        t.push(("Claude Code", home.join(".claude/CLAUDE.md")));
    }
    if home.join(".codex").is_dir() {
        t.push(("Codex", home.join(".codex/AGENTS.md")));
    }
    if home.join(".gemini").is_dir() {
        t.push(("Gemini CLI", home.join(".gemini/GEMINI.md")));
    }
    if home.join(".jcode").is_dir() || home.join("AGENTS.md").is_file() {
        t.push(("Jcode", home.join("AGENTS.md")));
    }
    t
}

pub fn command(remove_block: bool, dry_run: bool, quiet: bool) -> Result<()> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let targets = targets(&home);
    if targets.is_empty() && !quiet {
        println!("no coding agents found (looked for ~/.claude, ~/.codex, ~/.gemini, ~/.jcode)");
        println!("add docs/AGENTS-snippet.md to your agent's instructions by hand:");
        println!("\n{SNIPPET}");
    }
    for (agent, path) in targets {
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        let edit = if remove_block {
            remove(&old)
        } else {
            add(&old)
        };
        let shown = path.display();
        match edit {
            Edit::Same if !quiet => println!("{agent}: {shown} already up to date"),
            Edit::HandWritten if !quiet => {
                println!("{agent}: {shown} already mentions justrust, left as is")
            }
            Edit::Write(new) => {
                if dry_run {
                    println!("{agent}: would update {shown}");
                    continue;
                }
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let tmp = path.with_extension("justrust-tmp");
                std::fs::write(&tmp, &new)?;
                if let Ok(m) = std::fs::metadata(&path) {
                    let _ = std::fs::set_permissions(&tmp, m.permissions());
                }
                std::fs::rename(&tmp, &path)?;
                let verb = if remove_block {
                    "removed from"
                } else {
                    "added to"
                };
                println!("{agent}: justrust instructions {verb} {shown}");
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(e: Edit) -> String {
        match e {
            Edit::Write(s) => s,
            other => panic!("expected a write, got {other:?}"),
        }
    }

    #[test]
    fn adds_once_updates_in_place_and_removes_cleanly() {
        let orig = "# My rules\n\nBe nice.\n";
        let once = written(add(orig));
        assert!(once.starts_with(orig));
        assert!(once.contains(BEGIN) && once.contains(END) && once.contains("justrust check"));
        assert_eq!(add(&once), Edit::Same);

        // An older block is replaced, text after it is kept.
        let stale = format!("{orig}\n{BEGIN_PREFIX} old -->\nold text\n{END}\n## After\n");
        let fresh = written(add(&stale));
        assert!(!fresh.contains("old text") && fresh.ends_with("## After\n"));

        assert_eq!(written(remove(&once)), orig);
        assert_eq!(remove(orig), Edit::Same);
    }

    #[test]
    fn empty_and_unterminated_files() {
        let e = written(add(""));
        assert!(e.starts_with(BEGIN));
        assert_eq!(written(remove(&e)), "");
        assert!(written(add("x")).starts_with("x\n\n<!-- justrust:begin"));
    }

    #[test]
    fn hand_written_mentions_are_left_alone() {
        assert_eq!(add("use justrust check, not cargo\n"), Edit::HandWritten);
    }

    #[test]
    fn targets_follow_installed_agents() {
        let d = std::env::temp_dir().join(format!("jr-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".claude")).unwrap();
        std::fs::create_dir_all(d.join(".jcode")).unwrap();
        let t: Vec<_> = targets(&d).into_iter().map(|(a, _)| a).collect();
        assert_eq!(t, ["Claude Code", "Jcode"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
