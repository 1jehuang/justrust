//! `cargo config get`: the configuration cargo reads, as cargo 1.99.0's
//! `ops/cargo_config.rs` prints it, by blaze-manifest's port of it (`config::get`). The
//! command is unstable in cargo: refused as cargo refuses it without `-Z unstable-options`,
//! the `unstable.unstable-options` key or `CARGO_UNSTABLE_UNSTABLE_OPTIONS`.

use std::io::Write as _;

use blaze_manifest::Config;
use blaze_manifest::config::get;

use super::manifest::USAGE;

/// The options every cargo command takes, after `Options:` in each help.
const COMMON: &str =
    "  -v, --verbose...               Use verbose output (-vv very verbose/build.rs output)
  -q, --quiet                    Do not print cargo log messages
      --color <WHEN>             Coloring [possible values: auto, always, never]
      --config <KEY=VALUE|PATH>  Override a configuration value
  -Z <FLAG>                      Unstable (nightly-only) flags to Cargo, see 'cargo -Z help' for
                                 details
  -h, --help                     Print help

Manifest Options:
      --locked   Assert that `Cargo.lock` will remain unchanged
      --offline  Run without accessing the network
      --frozen   Equivalent to specifying both --locked and --offline";

const HELP: &str = "\
Inspect configuration values

Usage: cargo config [OPTIONS] <COMMAND>

Commands:
  get  

Options:
";

const GET_HELP: &str = "\
Usage: cargo config get [OPTIONS] [key]

Arguments:
  [key]  The config key to display

Options:
      --format <format>          Display format [default: toml] [possible values: toml, json,
                                 json-value]
      --show-origin              Display where the config value is defined
      --merged <merged>          Whether or not to merge config values [default: yes] [possible
                                 values: yes, no]
";

/// Flags of every cargo command that `config get` has no use for, and whether each takes a
/// value. `--color` was read before the command ran.
const IGNORED: &[(&str, bool)] = &[
    ("-v", false),
    ("--verbose", false),
    ("-vv", false),
    ("-q", false),
    ("--quiet", false),
    ("--color", true),
    ("--locked", false),
    ("--offline", false),
    ("--frozen", false),
];

/// `cargo config <COMMAND>`, `args` being what follows `config`.
pub(super) fn command(args: &[String]) -> anyhow::Result<i32> {
    let mut words = Vec::new();
    let mut unstable = false;
    let mut at = args.iter();
    while let Some(arg) = at.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (arg.as_str(), None),
        };
        if let Some(&(_, valued)) = IGNORED.iter().find(|(it, _)| *it == name) {
            if valued && inline.is_none() {
                at.next();
            }
            continue;
        }
        let flag = match arg.strip_prefix("-Z") {
            Some("") => at.next().map(String::as_str),
            Some(flag) => Some(flag.strip_prefix('=').unwrap_or(flag)),
            None => None,
        };
        match flag {
            Some("unstable-options") => unstable = true,
            Some(_) => words.push(arg.clone()),
            None if arg == "--" => {
                words.extend(at.by_ref().cloned());
            }
            None => words.push(arg.clone()),
        }
    }
    let help = words.iter().any(|it| it == "-h" || it == "--help");
    match subcommand(&words) {
        None if help => {
            println!("{HELP}{COMMON}");
            return Ok(0);
        }
        None => {
            eprintln!("{HELP}{COMMON}");
            return Ok(USAGE);
        }
        Some("get") if help => {
            println!("{GET_HELP}{COMMON}");
            return Ok(0);
        }
        Some("get") => {}
        Some(other) => {
            eprintln!(
                "{}: unrecognized subcommand '{other}'\n\nUsage: cargo config [OPTIONS] \
                 <COMMAND>\n\nFor more information, try '--help'.",
                paint("error")
            );
            return Ok(USAGE);
        }
    }
    let cwd = std::env::current_dir()?;
    if !unstable && !allowed(&cwd) {
        eprintln!(
            "{}: the `cargo config` command is unstable, pass `-Z unstable-options` to enable \
             it\nSee https://github.com/rust-lang/cargo/issues/9301 for more information about \
             the `cargo config` command.",
            paint("error")
        );
        return Ok(super::super::FAILED);
    }
    let mut argv = vec!["config".to_owned()];
    argv.extend(words);
    let (code, out) = get::main(&cwd, environment(), &argv);
    print!("{}", out.stdout);
    let stderr: String = out.stderr.split_inclusive('\n').map(painted).collect();
    eprint!("{stderr}");
    std::io::stdout().flush()?;
    Ok(code.into())
}

/// The variables the configuration reads. `CARGO` is the cargo that ran `cargo blaze`; a
/// `cargo config` run from a shell has none.
fn environment() -> impl Iterator<Item = (String, String)> {
    std::env::vars().filter(|(name, _)| name != "CARGO")
}

/// The first word of `words` that is neither a flag nor a flag's value.
fn subcommand(words: &[String]) -> Option<&str> {
    let mut at = words.iter();
    while let Some(word) = at.next() {
        match word.as_str() {
            "--config" | "--format" | "--merged" => {
                at.next();
            }
            flag if flag.starts_with('-') => {}
            word => return Some(word),
        }
    }
    None
}

/// Whether the configuration enables the unstable options. A configuration that does not
/// load is `config get`'s to report.
fn allowed(cwd: &std::path::Path) -> bool {
    let Ok(config) = Config::load(cwd, environment(), &[]) else { return true };
    config.get::<Option<bool>>("unstable.unstable-options").map_or(true, |it| it == Some(true))
}

/// `line` with the word it starts with, `error`, `warning` or `note`, colored as cargo
/// colors it.
fn painted(line: &str) -> String {
    for word in ["error", "warning", "note"] {
        if let Some(rest) = line.strip_prefix(word).filter(|rest| rest.starts_with(':')) {
            return format!("{}{rest}", paint(word));
        }
    }
    line.to_owned()
}

fn paint(word: &str) -> String {
    let color = match word {
        "error" => "\x1b[1m\x1b[91m",
        "warning" => "\x1b[1m\x1b[93m",
        _ => "\x1b[1m",
    };
    match super::super::say::colored() {
        true => format!("{color}{word}\x1b[0m"),
        false => word.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::subcommand;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn a_flags_value_is_no_subcommand() {
        assert_eq!(subcommand(&words("--config build.jobs=9 get build")), Some("get"));
        assert_eq!(subcommand(&words("--format json get")), Some("get"));
        assert_eq!(subcommand(&words("--show-origin nope")), Some("nope"));
        assert_eq!(subcommand(&words("--help")), None);
    }
}
