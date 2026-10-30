//! `cargo new` and `cargo init`: blaze-new's package, with the command line, configuration
//! and shell of cargo 1.101.0's `commands/new.rs` and `commands/init.rs`.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use blaze_manifest::diagnostic::ManifestDiagnostic;
use blaze_manifest::{Context, Workspace};
use blaze_new::{NewOptions, Settings, VersionControl};

use super::args::{Args, switch, valued};
use super::manifest::{USAGE, config, failed, read_or_refuse_taking};
use crate::cargo::say;

const EDITIONS: [&str; 4] = ["2015", "2018", "2021", "2024"];

struct Said {
    quiet: bool,
}

impl blaze_new::Shell for Said {
    fn status(&mut self, word: &str, rest: &str) {
        say::status(self.quiet, word, rest);
    }

    fn warn(&mut self, text: &str) {
        if !self.quiet {
            say::warning(text);
        }
    }

    fn note(&mut self, text: &str) {
        if !self.quiet {
            say::note(text);
        }
    }

    fn line(&mut self, text: &str) {
        eprintln!("{text}");
    }
}

/// `cargo new <PATH>`, or with `init` `cargo init [PATH]`.
pub(super) fn command(name: &str, args: Args<'_>) -> anyhow::Result<i32> {
    let init = name == "init";
    let flags = [
        valued(&["--vcs"]),
        switch(&["--bin"]),
        switch(&["--lib"]),
        valued(&["--edition"]),
        valued(&["--name"]),
        valued(&["--registry"]),
    ];
    let usage = format!("cargo blaze {name} [OPTIONS] {}", if init { "[PATH]" } else { "<PATH>" });
    let read = match read_or_refuse_taking(&args, &flags, &usage, 1) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let refuse = |flag: &str, value: &str, possible: &[&str]| {
        say::error(&format!(
            "invalid value '{value}' for '{flag}'\n  [possible values: {}]",
            possible.join(", ")
        ));
        if let Some(similar) = similar(value, possible) {
            eprintln!("\n  tip: a similar value exists: '{similar}'");
        }
        eprintln!("\nFor more information, try '--help'.");
        Ok(USAGE)
    };
    let path = match (read.positional.first(), init) {
        (Some(path), _) => path.clone(),
        (None, true) => ".".to_owned(),
        (None, false) => {
            say::error("the following required arguments were not provided:\n  <PATH>");
            eprintln!("\nUsage: cargo {name} <PATH>\n\nFor more information, try '--help'.");
            return Ok(USAGE);
        }
    };
    let vcs = match read.value("--vcs") {
        None => None,
        Some(it) => match VersionControl::parse(&it) {
            Some(vcs) => Some(vcs),
            None => return refuse("--vcs <VCS>", &it, &VersionControl::VALUES),
        },
    };
    let edition = read.value("--edition");
    if let Some(it) = &edition
        && !EDITIONS.contains(&it.as_str())
    {
        return refuse("--edition <YEAR>", it, &EDITIONS);
    }
    let quiet = read.has("--quiet");
    let mut said = Said { quiet };
    let made = (|| {
        let config = config(name, &read)?;
        let cwd = config.cwd().to_owned();
        let registry = match read.value("--registry") {
            Some(registry) => Some(registry),
            None => config.get_string("registry.default")?.map(|it| it.val),
        };
        let opts = NewOptions::new(
            vcs,
            read.has("--bin"),
            read.has("--lib"),
            cwd.join(&path),
            read.value("--name"),
            edition,
            registry,
        )?;
        let configured = match config.get_string("cargo-new.vcs")? {
            None => None,
            Some(it) => Some(VersionControl::parse(&it.val).with_context(|| {
                format!(
                    "error in {}: `cargo-new.vcs` expected one of `git`, `hg`, `pijul`, `fossil`, `none`, found `{}`",
                    it.definition, it.val
                )
            })?),
        };
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let ctx = Context::new(config);
        let check_workspace = |manifest: &Path| match Workspace::new(manifest, &ctx) {
            Ok(ws) => {
                for warning in ws.workspace_warnings().iter().filter(|_| !quiet) {
                    say::warning(warning);
                }
                None
            }
            Err(err) => Some(rendered(&err, &cwd)),
        };
        let settings = Settings {
            cwd: &cwd,
            home: home.as_deref(),
            vcs: configured,
            check_workspace: &check_workspace,
        };
        match init {
            true => blaze_new::init(&opts, &settings, &mut said).map(drop),
            false => blaze_new::new(&opts, &settings, &mut said),
        }
    })();
    match made {
        Ok(()) => Ok(0),
        Err(err) => Ok(failed(&err)),
    }
}

/// `err` as cargo prints an error after a warning: the message with no `error:`, then each
/// cause.
fn rendered(err: &anyhow::Error, cwd: &Path) -> String {
    if let Some(diagnostic) = err.downcast_ref::<ManifestDiagnostic>() {
        return diagnostic.render(cwd);
    }
    let mut out = String::new();
    for (i, cause) in err.chain().enumerate() {
        if i == 0 {
            out.push_str(&cause.to_string());
            continue;
        }
        out.push_str("\n\nCaused by:");
        for line in cause.to_string().lines() {
            out.push('\n');
            if !line.is_empty() {
                out.push_str("  ");
                out.push_str(line);
            }
        }
    }
    out
}

/// The possible value clap's tip names for `value`: the last of the most similar by Jaro
/// similarity, if more than 0.7.
fn similar<'a>(value: &str, possible: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(f64, &str)> = None;
    for &it in possible {
        let confidence = jaro(value, it);
        if confidence > 0.7 && best.is_none_or(|(most, _)| confidence >= most) {
            best = Some((confidence, it));
        }
    }
    best.map(|(_, it)| it)
}

/// strsim's `jaro`.
fn jaro(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut b_used = vec![false; b.len()];
    let (mut matches, mut transpositions, mut b_consumed) = (0usize, 0usize, 0usize);
    for (i, ca) in a.iter().enumerate() {
        let from = i.saturating_sub(window);
        let to = (i + window + 1).min(b.len());
        if let Some(j) = (from..to).find(|&j| !b_used[j] && b[j] == *ca) {
            b_used[j] = true;
            matches += 1;
            if j < b_consumed {
                transpositions += 1;
            }
            b_consumed = j;
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let m = matches as f64;
    (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64) / m) / 3.0
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_tip_is_claps() {
        let editions = super::EDITIONS;
        assert_eq!(super::similar("2019", &editions), Some("2021"));
        assert_eq!(super::similar("2023", &editions), Some("2024"));
        assert_eq!(super::similar("svn", &["git", "hg", "pijul", "fossil", "none"]), None);
    }
}
