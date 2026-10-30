//! `cargo report`: `future-incompatibilities` prints a report a build left in the build
//! directory, as cargo 1.99.0's `commands/report.rs` and `future_incompat.rs` do. `timings`,
//! `sessions` and `rebuilds` are unstable in cargo; without `-Z build-analysis` they are
//! refused as cargo refuses them.

use std::collections::BTreeMap;
use std::io::{IsTerminal as _, Write as _};

use anyhow::Context as _;
use blaze_manifest::metadata::root_manifest;
use blaze_manifest::{Context, Workspace};
use serde::Deserialize;

use super::args::{Args, Read, switch, valued};
use super::manifest::{USAGE, config, failed};

/// What `report future-incompatibilities` prints ahead of the report.
const PREAMBLE: &str = "\
The following warnings were discovered during the build. These warnings are an
indication that the packages contain code that will become an error in a
future release of Rust. These warnings typically cover changes to close
soundness problems, unintended or undocumented behavior, or critical problems
that cannot be fixed in a backwards-compatible fashion, and are not expected
to be in wide use.

Each warning should contain a link for more information on what the warning
means and how to resolve it.
";

/// The file in the build directory builds leave their reports in.
const FILE: &str = ".future-incompat-report.json";

/// The version of [`FILE`] this cargo reads.
const ON_DISK_VERSION: u32 = 0;

const HELP: &str = "\
Generate and display various kinds of reports

Usage: cargo report [OPTIONS] <COMMAND>

Commands:
  future-incompatibilities  Reports any crates which will eventually stop compiling
  timings                   Reports the build timings of previous sessions (unstable)
  sessions                  Reports the previous sessions (unstable)
  rebuilds                  Reports rebuild reasons from previous sessions (unstable)

Options:
  -v, --verbose...               Use verbose output (-vv very verbose/build.rs output)
  -q, --quiet                    Do not print cargo log messages
      --color <WHEN>             Coloring [possible values: auto, always, never]
      --config <KEY=VALUE|PATH>  Override a configuration value
  -Z <FLAG>                      Unstable (nightly-only) flags to Cargo, see 'cargo -Z help' for
                                 details
  -h, --help                     Print help

Manifest Options:
      --locked   Assert that `Cargo.lock` will remain unchanged
      --offline  Run without accessing the network
      --frozen   Equivalent to specifying both --locked and --offline

Run `cargo help report` for more detailed information.";

#[derive(Deserialize)]
struct OnDiskReports {
    version: u32,
    #[allow(dead_code)]
    next_id: u32,
    reports: Vec<OnDiskReport>,
}

#[derive(Deserialize)]
struct OnDiskReport {
    id: u32,
    suggestion_message: String,
    per_package: BTreeMap<String, String>,
}

/// `cargo report <COMMAND>`.
pub(super) fn command(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [
        valued(&["--id"]),
        valued(&["-p", "--package"]),
        valued(&["--manifest-path"]),
        switch(&["--open"]),
        valued(&["--limit"]),
    ];
    let read = match args.read(&flags) {
        Ok(read) => read,
        Err(why) => {
            super::super::say::error(&why);
            eprintln!(
                "\nUsage: cargo report [OPTIONS] <COMMAND>\n\nFor more information, try '--help'."
            );
            return Ok(USAGE);
        }
    };
    match read.positional.first().map(String::as_str) {
        None if read.has("--help") => {
            println!("{HELP}");
            Ok(0)
        }
        None => {
            eprintln!("{HELP}");
            Ok(USAGE)
        }
        Some("future-incompatibilities" | "future-incompat") => {
            match future_incompatibilities(&read) {
                Ok(code) => Ok(code),
                Err(err) => Ok(failed(&err)),
            }
        }
        Some(unstable @ ("timings" | "sessions" | "rebuilds")) => {
            let allowed = read.values("-Z").iter().any(|it| it == "build-analysis");
            let why = match allowed {
                false => anyhow::anyhow!(
                    "the `cargo report {unstable}` command is unstable, pass `-Z build-analysis` \
                     to enable it\nSee https://github.com/rust-lang/cargo/issues/15844 for more \
                     information about the `cargo report {unstable}` command."
                ),
                true => anyhow::anyhow!(
                    "blaze has no `cargo report {unstable}` yet: it reads the logs `-Z \
                     build-analysis` has cargo's builds write, which blaze's builds do not; \
                     nothing was done"
                ),
            };
            Ok(failed(&why))
        }
        Some(other) => {
            super::super::say::error(&format!("unrecognized subcommand '{other}'"));
            eprintln!(
                "\nUsage: cargo report [OPTIONS] <COMMAND>\n\nFor more information, try '--help'."
            );
            Ok(USAGE)
        }
    }
}

fn future_incompatibilities(read: &Read) -> anyhow::Result<i32> {
    let ctx = Context::new(config("report", read)?);
    let root = root_manifest(None, &ctx)?;
    let ws = Workspace::new(&root, &ctx)?;
    let path = ws.build_dir().join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("no reports are currently available")
        }
        Err(error) => return Err(error).context("failed to read report"),
    };
    let reports: OnDiskReports = serde_json::from_str(&text).context("failed to load report")?;
    anyhow::ensure!(
        reports.version == ON_DISK_VERSION,
        "unable to read reports; reports were saved from a future version of Cargo"
    );
    let id = match read.value("--id") {
        Some(id) => match id.parse::<u32>() {
            Ok(id) => id,
            Err(_) => {
                // clap's error, with the status of a refused command line, as cargo has it.
                eprint!("{}: invalid value: could not parse `{id}` as a number", red("error"));
                return Ok(USAGE);
            }
        },
        None => {
            reports.reports.last().map(|it| it.id).context("no reports are currently available")?
        }
    };
    let Some(report) = reports.reports.iter().find(|it| it.id == id) else {
        let available: Vec<String> = reports.reports.iter().map(|it| it.id.to_string()).collect();
        anyhow::bail!(
            "could not find report with ID {id}\nAvailable IDs are: {}",
            available.join(", ")
        );
    };
    let mut shown = report.suggestion_message.clone() + "\n";
    match read.value("--package") {
        Some(package) => match report.per_package.get(&package) {
            Some(it) => shown += it,
            None => {
                let available: Vec<&str> = report.per_package.keys().map(String::as_str).collect();
                anyhow::bail!(
                    "could not find package with ID `{package}`\n
                Available packages are: {}\n
                Omit the `--package` flag to display a report for all packages",
                    available.join(", ")
                );
            }
        },
        None => shown += &report.per_package.values().cloned().collect::<Vec<_>>().join("\n"),
    }
    let colored = match read.value("--color").or_else(|| std::env::var("CARGO_TERM_COLOR").ok()) {
        Some(it) if it == "always" => true,
        Some(it) if it == "never" => false,
        _ => std::io::stdout().is_terminal(),
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{PREAMBLE}");
    let _ = out.write_all(if colored { shown } else { plain(&shown) }.as_bytes());
    Ok(0)
}

fn red(word: &str) -> String {
    match super::super::say::colored() {
        true => format!("\x1b[1m\x1b[91m{word}\x1b[0m"),
        false => word.to_owned(),
    }
}

/// `text` without its terminal escapes: the colors (`ESC [ ... m`) and links (`ESC ] ... ESC \`)
/// a report was saved with.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn colors_and_links_are_taken_out() {
        let text = "\x1b[1m\x1b[33mwarning\x1b[0m: \x1b]8;;https://x\x1b\\x\x1b]8;;\x1b\\ end\x07";
        assert_eq!(super::plain(text), "warning: x end\x07");
    }
}
