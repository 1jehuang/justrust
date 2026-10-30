//! `cargo search`, `owner`, `yank`, `login` and `logout`: a registry's web API asked with
//! blaze-registry's client, with the token its credential providers give
//! (`blaze_registry::auth`), as cargo 1.99.0's `commands/{search,owner,yank,login,logout}.rs`
//! and `ops/registry` do.

use std::io::IsTerminal as _;
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use blaze_manifest::metadata::root_manifest;
use blaze_manifest::{Config, Context, Workspace};
use blaze_registry::api::Api;
use blaze_registry::auth::{Auth, AuthorizationError, Named, Operation, Shell};
use blaze_resolve::{CRATES_IO_REGISTRY, SourceId};

use super::super::say;
use super::args::{Args, Read, switch, valued};
use super::manifest::{USAGE, config, failed};

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

/// One of the commands: its name, usage line, help above [`COMMON`], and flags.
struct Command {
    name: &'static str,
    usage: &'static str,
    head: &'static str,
    /// Whether it takes words that are no flags, for clap's tip on an unknown flag.
    positional: bool,
}

const SEARCH: Command = Command {
    name: "search",
    usage: "cargo search [OPTIONS] [QUERY]...",
    head: "Search packages in the registry. Default registry is crates.io

Usage: cargo search [OPTIONS] [QUERY]...

Arguments:
  [QUERY]...  

Options:
      --limit <LIMIT>            Limit the number of results (default: 10, max: 100)
      --index <INDEX>            Registry index URL to search packages in
      --registry <REGISTRY>      Registry to search packages in
",
    positional: true,
};

const OWNER: Command = Command {
    name: "owner",
    usage: "cargo owner [OPTIONS] [CRATE]",
    head: "Manage the owners of a crate on the registry

Usage: cargo owner [OPTIONS] [CRATE]

Arguments:
  [CRATE]  

Options:
  -a, --add <LOGIN>              Name of a user or team to invite as an owner
  -r, --remove <LOGIN>           Name of a user or team to remove as an owner
  -l, --list                     List owners of a crate
      --index <INDEX>            Registry index URL to modify owners for
      --registry <REGISTRY>      Registry to modify owners for
      --token <TOKEN>            API token to use when authenticating
",
    positional: true,
};

const YANK: Command = Command {
    name: "yank",
    usage: "cargo yank [OPTIONS] [CRATE]",
    head: "Remove a pushed crate from the index

Usage: cargo yank [OPTIONS] [CRATE]

Arguments:
  [CRATE]  

Options:
      --version <VERSION>        The version to yank or un-yank
      --undo                     Undo a yank, putting a version back into the index
      --index <INDEX>            Registry index URL to yank from
      --registry <REGISTRY>      Registry to yank from
      --token <TOKEN>            API token to use when authenticating
",
    positional: true,
};

const LOGIN: Command = Command {
    name: "login",
    usage: "cargo login [OPTIONS] [-- [args]...]",
    head: "Log in to a registry.

Usage: cargo login [OPTIONS] [-- [args]...]

Arguments:
  [args]...  Additional arguments for the credential provider

Options:
      --registry <REGISTRY>      Registry to use
",
    positional: true,
};

const LOGOUT: Command = Command {
    name: "logout",
    usage: "cargo logout [OPTIONS]",
    head: "Remove an API token from the registry locally

Usage: cargo logout [OPTIONS]

Options:
      --registry <REGISTRY>      Registry to use
",
    positional: false,
};

impl Command {
    /// The flags, or the status clap ends with: help printed, or the command line refused.
    fn read(&self, typed: &[String], flags: &[super::args::Flag]) -> Result<Read, i32> {
        let read = match Args::new(typed).read(flags) {
            Ok(read) => read,
            Err(why) => {
                let unknown =
                    typed.iter().find(|it| why == format!("unexpected argument '{it}' found"));
                let Some(word) = unknown.filter(|it| it.starts_with("--")) else {
                    return Err(self.refuse(&why, self.usage));
                };
                let flag = word.split_once('=').map_or(word.as_str(), |it| it.0);
                if let Some(similar) = similar(&flag[2..], flags) {
                    let positional = self.positional_usage(&Read::default());
                    let usage = format!("cargo {} {similar}{positional}", self.name);
                    let similar = similar.split(' ').next().unwrap_or_default();
                    let why = format!("{why}\n\n  tip: a similar argument exists: '{similar}'");
                    return Err(self.refuse(&why, &usage));
                }
                let tip = match self.positional {
                    true => format!("\n\n  tip: to pass '{flag}' as a value, use '-- {flag}'"),
                    false => String::new(),
                };
                return Err(self.refuse(&format!("{why}{tip}"), self.usage));
            }
        };
        if read.has("--help") {
            println!(
                "{}{COMMON}\n\nRun `cargo help {}` for more detailed information.",
                self.head, self.name
            );
            return Err(0);
        }
        if read.has("--registry") && read.has("--index") {
            let positional = self.positional_usage(&read);
            let usage = format!("cargo {} --registry <REGISTRY>{positional}", self.name);
            return Err(self.refuse(
                "the argument '--registry <REGISTRY>' cannot be used with '--index <INDEX>'",
                &usage,
            ));
        }
        Ok(read)
    }

    /// The words that are no flags as clap's usage of the arguments given shows them.
    fn positional_usage(&self, read: &Read) -> &'static str {
        match self.name {
            "search" => " [QUERY]...",
            _ if read.positional.is_empty() => " [CRATE]",
            _ => " <CRATE>",
        }
    }

    fn refuse(&self, why: &str, usage: &str) -> i32 {
        say::error(why);
        eprintln!("\nUsage: {usage}\n\nFor more information, try '--help'.");
        USAGE
    }
}

/// The long flags every command takes, with whether each takes a value, as clap knows them.
const GLOBAL_LONGS: &[(&str, bool)] = &[
    ("verbose", false),
    ("quiet", false),
    ("color", true),
    ("config", true),
    ("frozen", false),
    ("locked", false),
    ("offline", false),
    ("help", false),
];

/// clap's `did_you_mean_flag`: the long flag most like `typed` by Jaro similarity, if any is
/// more than 0.7 alike, as clap's usage shows it (`--limit <LIMIT>`).
fn similar(typed: &str, flags: &[super::args::Flag]) -> Option<String> {
    let own = flags.iter().filter_map(|it| {
        let long = it.names.iter().find(|it| it.starts_with("--"))?;
        Some((&long[2..], it.valued))
    });
    let mut found: Vec<(f64, &str, bool)> = own
        .chain(GLOBAL_LONGS.iter().copied())
        .map(|(long, valued)| (jaro(typed, long), long, valued))
        .filter(|(confidence, ..)| *confidence > 0.7)
        .collect();
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (_, long, valued) = found.pop()?;
    Some(match valued {
        true => format!("--{long} <{}>", long.to_uppercase()),
        false => format!("--{long}"),
    })
}

/// strsim's Jaro similarity of two strings, by characters.
fn jaro(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut taken = vec![false; b.len()];
    let mut matched_a = Vec::new();
    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        if let Some(j) = (lo..hi).find(|&j| !taken[j] && b[j] == *ca) {
            taken[j] = true;
            matched_a.push(*ca);
        }
    }
    let m = matched_a.len();
    if m == 0 {
        return 0.0;
    }
    let matched_b = b.iter().zip(&taken).filter(|(_, t)| **t).map(|(c, _)| *c);
    let transpositions = matched_a.iter().zip(matched_b).filter(|(x, y)| **x != *y).count() / 2;
    let m = m as f64;
    (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64) / m) / 3.0
}

/// `--registry` or `--index`, or `registry.default`.
enum RegistryOrIndex {
    Registry(String),
    Index(String),
}

/// `--registry`, `--index`, or the `registry.default` configured, as cargo's
/// `registry_or_index`.
fn registry_or_index(read: &Read, config: &Config) -> anyhow::Result<Option<RegistryOrIndex>> {
    Ok(match (read.value("--registry"), read.value("--index")) {
        (None, None) => config.default_registry()?.map(RegistryOrIndex::Registry),
        (None, Some(index)) => {
            url::Url::parse(&index).map_err(|e| anyhow!("invalid url `{index}`: {e}"))?;
            Some(RegistryOrIndex::Index(index))
        }
        (Some(registry), _) => {
            cargo_util_schemas::manifest::RegistryName::new(&registry)?;
            Some(RegistryOrIndex::Registry(registry))
        }
    })
}

/// The registry a command asks, as cargo's `RegistrySourceIds`.
struct Target {
    /// What its token is kept under: crates.io's git index for crates.io.
    original: SourceId,
    /// What is asked: crates.io's sparse index for crates.io, else `original`.
    replacement: SourceId,
    /// Its name: `crates-io`, or `--registry`'s; none for `--index`.
    name: Option<String>,
}

impl Target {
    fn named(&self) -> Named<'_> {
        Named { source: &self.original, name: self.name.as_deref() }
    }

    fn described(&self) -> String {
        self.named().described()
    }

    /// cargo's `display_index`.
    fn index(&self) -> String {
        match self.original.is_crates_io() {
            true => "crates.io index".to_owned(),
            false => format!("`{}` index", self.named().shown()),
        }
    }
}

/// cargo's `get_source_id`: the registry named, with only crates.io's own replacement (its
/// sparse index); with none named, a `[source]` replacing crates.io is an error.
fn target(ctx: &Context, asked: Option<&RegistryOrIndex>) -> anyhow::Result<Target> {
    let crates_io = || (SourceId::crates_io(), Some(CRATES_IO_REGISTRY.to_owned()));
    let (original, name) = match asked {
        None => crates_io(),
        Some(RegistryOrIndex::Index(url)) => (blaze_registry::registry_source(url)?, None),
        Some(RegistryOrIndex::Registry(name)) if name == CRATES_IO_REGISTRY => crates_io(),
        Some(RegistryOrIndex::Registry(name)) => {
            let index = ctx.config.registry_index(name)?;
            (SourceId::for_alt_registry(&index, name)?, Some(name.clone()))
        }
    };
    let builtin =
        blaze_metadata::registry_sources(ctx, false, None)?.map().replacement(original)?;
    let replaced =
        blaze_metadata::registry_sources(ctx, true, None)?.map().replacement(original)?;
    if asked.is_none() && replaced != builtin {
        let shown = match replaced_by(&ctx.config)? {
            Some((name, true)) => bail!(
                "crates-io is replaced with remote registry {name};\ninclude `--registry {name}` or `--registry crates-io`"
            ),
            Some((name, false)) if replaced.is_remote_registry() => format!("registry `{name}`"),
            _ => replaced.to_string(),
        };
        bail!(
            "crates-io is replaced with non-remote-registry source {shown};\ninclude `--registry crates-io` to use crates.io"
        );
    }
    Ok(Target { original, replacement: builtin, name })
}

/// The last name of crates.io's chain of `replace-with`, and whether it is a `[registries]`
/// name rather than a `[source]` one: cargo names a replacement by it.
fn replaced_by(config: &Config) -> anyhow::Result<Option<(String, bool)>> {
    let mut name = CRATES_IO_REGISTRY.to_owned();
    let mut last = None;
    while let Some(with) = config.get_string(&format!("source.{name}.replace-with"))? {
        name = with.val;
        let mut is_source = false;
        for key in ["registry", "directory", "local-registry", "git"] {
            is_source |= config.get_string(&format!("source.{name}.{key}"))?.is_some();
        }
        last = Some((name.clone(), !is_source));
        if !is_source || last.as_ref().is_some_and(|(it, _)| it == CRATES_IO_REGISTRY) {
            break;
        }
    }
    Ok(last)
}

/// The run's flags every command reads, and its credentials.
struct Run {
    ctx: Context,
    auth: Arc<Auth>,
    quiet: bool,
}

impl Run {
    fn status(&self, word: &str, rest: &str) {
        say::status(self.quiet, word, rest);
    }

    fn warn(&self, text: &str) {
        if !self.quiet {
            say::warning(text);
        }
    }

    fn note(&self, text: &str) {
        if !self.quiet {
            say::shell_note(text);
        }
    }
}

/// What the credential providers say, through cargo's shell.
struct Said {
    quiet: bool,
    verbose: bool,
}

impl Shell for Said {
    fn warn(&self, text: &str) {
        if !self.quiet {
            say::warning(text);
        }
    }

    fn note(&self, text: &str) {
        if !self.quiet {
            say::shell_note(text);
        }
    }

    fn status(&self, word: &str, text: &str) {
        say::status(self.quiet, word, text);
    }

    fn verbose(&self) -> bool {
        self.verbose
    }
}

/// cargo's `ops::registry::registry`: the registry's `config.json` (asked again when
/// `force_update`), its API, and the token `operation` needs.
fn registry(
    run: &Run,
    target: &Target,
    from_command_line: Option<&str>,
    asked: Option<&RegistryOrIndex>,
    force_update: bool,
    operation: Option<Operation<'_>>,
) -> anyhow::Result<Api> {
    let is_index = matches!(asked, Some(RegistryOrIndex::Index(_)));
    if is_index && operation.is_some() && from_command_line.is_none() {
        bail!("command-line argument --index requires --token to be specified");
    }
    if let Some(token) = from_command_line {
        run.auth.keep_command_line_token(&target.original, token);
    }
    let sources = blaze_metadata::registry_sources(&run.ctx, false, Some(run.auth.clone()))?;
    let registry = sources.registry(target.replacement)?;
    if force_update {
        registry.invalidate();
    }
    if !registry.config_is_cached() {
        run.status("Updating", &target.index());
    }
    let config = registry
        .config()
        .with_context(|| format!("failed to update {}", target.described()))?
        .with_context(|| format!("{} has no config.json", target.described()))?;
    let Some(api) = config.api else {
        bail!("{} does not support API commands", target.described());
    };
    let token = match operation.is_some() || config.auth_required {
        true => {
            let operation = operation.unwrap_or(Operation::Read);
            Some(run.auth.token(target.named(), None, operation, &[], false)?)
        }
        false => None,
    };
    let http = blaze_registry::http::HyperHttp::shared()?;
    Ok(Api::new(api, token, http, config.auth_required))
}

fn read_line() -> anyhow::Result<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

/// The run of one command, its `--config`s merged.
fn start(name: &str, read: &Read) -> anyhow::Result<Run> {
    let ctx = Context::new(config(name, read)?);
    let (quiet, verbose) = (read.has("--quiet"), read.has("--verbose"));
    let auth = Arc::new(Auth::new(ctx.config.reload()?, Box::new(Said { quiet, verbose })));
    Ok(Run { ctx, auth, quiet })
}

fn ended(done: anyhow::Result<()>) -> anyhow::Result<i32> {
    Ok(match done {
        Ok(()) => 0,
        Err(err) => failed(&err),
    })
}

/// `cargo search [QUERY]...`.
pub(super) fn search(typed: &[String]) -> anyhow::Result<i32> {
    let flags = [valued(&["--limit"]), valued(&["--index"]), valued(&["--registry"])];
    let read = match SEARCH.read(typed, &flags) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let run = start("search", &read)?;
    let asked = match registry_or_index(&read, &run.ctx.config) {
        Ok(it) => it,
        Err(err) => return Ok(failed(&err)),
    };
    let limit = match read.value("--limit") {
        Some(limit) => match limit.parse::<u32>() {
            Ok(limit) => limit,
            Err(_) => {
                say::error_unended(&format!(
                    "invalid value: could not parse `{limit}` as a number"
                ));
                return Ok(USAGE);
            }
        },
        None => 10,
    };
    let query = read.positional.join("+");
    ended(searched(&run, &query, asked.as_ref(), limit.min(100)))
}

fn searched(
    run: &Run,
    query: &str,
    asked: Option<&RegistryOrIndex>,
    limit: u32,
) -> anyhow::Result<()> {
    let target = target(&run.ctx, asked)?;
    let api = registry(run, &target, None, asked, false, None)?;
    let (crates, total) = api.search(query, limit).with_context(|| {
        format!("failed to retrieve search results from the registry at {}", api.host())
    })?;
    let names: Vec<String> =
        crates.iter().map(|it| format!("{} = \"{}\"", it.name, it.max_version)).collect();
    let margin = names.iter().map(String::len).max().unwrap_or_default() + 4;
    let length = 80.max(128usize.saturating_sub(margin));
    let colored = say::colored_out();
    let mut out = String::new();
    for (name, it) in names.into_iter().zip(&crates) {
        let line = match &it.description {
            Some(description) => {
                let description = truncate_with_ellipsis(&description.replace('\n', " "), length);
                format!("{name: <margin$}# {description}")
            }
            None => name,
        };
        let mut fragments = line.split(query).peekable();
        while let Some(fragment) = fragments.next() {
            out.push_str(fragment);
            if fragments.peek().is_some() {
                match colored {
                    true => out.push_str(&format!("\x1b[1m\x1b[92m{query}\x1b[0m")),
                    false => out.push_str(query),
                }
            }
        }
        out.push('\n');
    }
    if total > limit && limit < 100 {
        out.push_str(&format!(
            "... and {} crates more (use --limit N to see more)\n",
            total - limit
        ));
    } else if total > limit && limit >= 100 {
        let extra = match target.original.is_crates_io() {
            true => {
                let url = url::Url::parse_with_params("https://crates.io/search", [("q", query)])?;
                format!(" (go to {url} to see more)")
            }
            false => String::new(),
        };
        out.push_str(&format!("... and {} crates more{extra}\n", total - limit));
    }
    print!("{out}");
    if total > 0 {
        let literal = match say::colored() {
            true => "\x1b[1m\x1b[96mcargo info <name>\x1b[0m",
            false => "cargo info <name>",
        };
        run.note(&format!("to learn more about a package, run `{literal}`"));
    }
    Ok(())
}

/// cargo's `truncate_with_ellipsis`: at most `max` characters, the last an ellipsis when cut.
fn truncate_with_ellipsis(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut prefix: String = chars.by_ref().take(max - 1).collect();
    if chars.next().is_some() {
        prefix.push('…');
    }
    prefix
}

/// The package of the working directory, for `owner` and `yank` with none named.
fn current_package(run: &Run) -> anyhow::Result<String> {
    let root = root_manifest(None, &run.ctx)?;
    let ws = Workspace::new(&root, &run.ctx)?;
    let current = ws.current_manifest().to_owned();
    match ws.members().find(|(path, _)| *path == current) {
        Some((_, manifest)) => Ok(manifest.package_id.name.clone()),
        None => bail!(
            "manifest path `{}` is a virtual manifest, but this command requires running against an actual package in this workspace",
            current.display()
        ),
    }
}

/// `cargo owner [CRATE]`.
pub(super) fn owner(typed: &[String]) -> anyhow::Result<i32> {
    let flags = [
        valued(&["-a", "--add"]),
        valued(&["-r", "--remove"]),
        switch(&["-l", "--list"]),
        valued(&["--index"]),
        valued(&["--registry"]),
        valued(&["--token"]),
    ];
    let read = match OWNER.read(typed, &flags) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    if let Some(extra) = read.positional.get(1) {
        return Ok(OWNER.refuse(&format!("unexpected argument '{extra}' found"), OWNER.usage));
    }
    let run = start("owner", &read)?;
    ended((|| {
        let asked = registry_or_index(&read, &run.ctx.config)?;
        let name = match read.positional.first() {
            Some(name) => name.clone(),
            None => current_package(&run)?,
        };
        let target = target(&run.ctx, asked.as_ref())?;
        let token = read.value("--token");
        let api = registry(
            &run,
            &target,
            token.as_deref(),
            asked.as_ref(),
            true,
            Some(Operation::Owners { name: &name }),
        )?;
        let host = api.host().to_owned();
        let to_add = read.values("--add");
        if !to_add.is_empty() {
            let users: Vec<&str> = to_add.iter().map(String::as_str).collect();
            let msg = api.add_owners(&name, &users).with_context(|| {
                format!("failed to invite owners to crate `{name}` on registry at {host}")
            })?;
            run.status("Owner", &msg);
        }
        let to_remove = read.values("--remove");
        if !to_remove.is_empty() {
            let users: Vec<&str> = to_remove.iter().map(String::as_str).collect();
            run.status("Owner", &format!("removing {users:?} from crate {name}"));
            api.remove_owners(&name, &users).with_context(|| {
                format!("failed to remove owners from crate `{name}` on registry at {host}")
            })?;
        }
        if read.has("--list") {
            let owners = api.list_owners(&name).with_context(|| {
                format!("failed to list owners of crate `{name}` on registry at {host}")
            })?;
            let mut out = String::new();
            for it in owners {
                out.push_str(&it.login);
                match (it.name, it.email) {
                    (Some(name), Some(email)) => out.push_str(&format!(" ({name} <{email}>)\n")),
                    (Some(it), None) | (None, Some(it)) => out.push_str(&format!(" ({it})\n")),
                    (None, None) => out.push('\n'),
                }
            }
            print!("{out}");
        }
        Ok(())
    })())
}

/// `cargo yank [CRATE] --version <VERSION> [--undo]`.
pub(super) fn yank(typed: &[String]) -> anyhow::Result<i32> {
    let flags = [
        valued(&["--version", "--vers"]),
        switch(&["--undo"]),
        valued(&["--index"]),
        valued(&["--registry"]),
        valued(&["--token"]),
    ];
    let read = match YANK.read(typed, &flags) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    if let Some(extra) = read.positional.get(1) {
        return Ok(YANK.refuse(&format!("unexpected argument '{extra}' found"), YANK.usage));
    }
    let run = start("yank", &read)?;
    ended((|| {
        let (krate, version) = resolve_crate(read.positional.first(), read.value("--version"))?;
        let Some(version) = version else { bail!("`--version` is required") };
        let asked = registry_or_index(&read, &run.ctx.config)?;
        let name = match krate {
            Some(name) => name,
            None => current_package(&run)?,
        };
        let target = target(&run.ctx, asked.as_ref())?;
        let token = read.value("--token");
        let operation = match read.has("--undo") {
            true => Operation::Unyank { name: &name, vers: &version },
            false => Operation::Yank { name: &name, vers: &version },
        };
        let api = registry(&run, &target, token.as_deref(), asked.as_ref(), true, Some(operation))?;
        let spec = format!("{name}@{version}");
        if read.has("--undo") {
            run.status("Unyank", &spec);
            api.unyank(&name, &version).with_context(|| {
                format!("failed to undo a yank from the registry at {}", api.host())
            })?;
        } else {
            run.status("Yank", &spec);
            api.yank(&name, &version)
                .with_context(|| format!("failed to yank from the registry at {}", api.host()))?;
        }
        Ok(())
    })())
}

/// `name@version` taken apart, and the version checked, as cargo's `resolve_crate`.
fn resolve_crate(
    krate: Option<&String>,
    version: Option<String>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let (mut krate, mut version) = (krate.cloned(), version);
    if let Some((k, v)) = krate.as_deref().and_then(|it| it.split_once('@')) {
        if version.is_some() {
            bail!("cannot specify both `@{v}` and `--version`");
        }
        if k.is_empty() {
            bail!("missing crate name for `@{v}`");
        }
        let (k, v) = (k.to_owned(), v.to_owned());
        krate = Some(k);
        version = Some(v);
    }
    if let Some(version) = &version
        && semver::Version::parse(version).is_err()
    {
        let err = semver::Version::parse(version).unwrap_err();
        let why = match version.strip_prefix('v') {
            Some(stripped) => format!(
                "the version provided, `{version}` is not a valid SemVer version\n\nhelp: try changing the version to `{stripped}`"
            ),
            None => format!("invalid version `{version}`"),
        };
        return Err(anyhow::Error::new(err).context(why));
    }
    Ok((krate, version))
}

/// `cargo login [-- [args]...]`, with the token from standard input (or, deprecated, the
/// one word given).
pub(super) fn login(typed: &[String]) -> anyhow::Result<i32> {
    let flags = [valued(&["--registry"])];
    let (before, provider_args) = match typed.iter().position(|it| it == "--") {
        Some(at) => (&typed[..at], &typed[at + 1..]),
        None => (typed, &typed[typed.len()..]),
    };
    let read = match LOGIN.read(before, &flags) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    if let Some(extra) = read.positional.get(1) {
        return Ok(LOGIN.refuse(&format!("unexpected argument '{extra}' found"), LOGIN.usage));
    }
    let run = start("login", &read)?;
    ended((|| {
        let asked = registry_or_index(&read, &run.ctx.config)?;
        let from_command_line = read.positional.first().cloned();
        if from_command_line.is_some() {
            run.warn(
                "`cargo login <token>` is deprecated in favor of reading `<token>` from stdin",
            );
        }
        let target = target(&run.ctx, asked.as_ref())?;
        let login_url = match registry(
            &run,
            &target,
            from_command_line.as_deref(),
            asked.as_ref(),
            false,
            None,
        ) {
            Ok(api) => Some(format!("{}/me", api.host())),
            Err(err) => match err.downcast::<AuthorizationError>() {
                Ok(err) => err.login_url,
                Err(err) => return Err(err),
            },
        };
        let token = from_command_line.or_else(|| {
            if std::io::stdin().is_terminal() {
                return None;
            }
            read_line().ok().filter(|it| !it.is_empty())
        });
        let args: Vec<&str> = provider_args.iter().map(String::as_str).collect();
        run.auth.login(target.named(), token.as_deref(), login_url.as_deref(), &args)
    })())
}

/// `cargo logout`.
pub(super) fn logout(typed: &[String]) -> anyhow::Result<i32> {
    let flags = [valued(&["--registry"])];
    let read = match LOGOUT.read(typed, &flags) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    if let Some(extra) = read.positional.first() {
        return Ok(LOGOUT.refuse(&format!("unexpected argument '{extra}' found"), LOGOUT.usage));
    }
    let run = start("logout", &read)?;
    ended((|| {
        let asked = registry_or_index(&read, &run.ctx.config)?;
        let target = target(&run.ctx, asked.as_ref())?;
        run.auth.logout(target.named())
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_cut_as_cargo_cuts_them() {
        assert_eq!(truncate_with_ellipsis("abcdef", 4), "abc…");
        assert_eq!(truncate_with_ellipsis("abcd", 4), "abc…");
        assert_eq!(truncate_with_ellipsis("abc", 4), "abc");
        assert_eq!(truncate_with_ellipsis("ééééé", 3), "éé…");
    }

    #[test]
    fn versions_with_the_crate() {
        let it =
            |k: &str, v: Option<&str>| resolve_crate(Some(&k.to_owned()), v.map(str::to_owned));
        assert_eq!(it("a@1.0.0", None).unwrap(), (Some("a".into()), Some("1.0.0".into())));
        assert!(
            it("a@1.0.0", Some("1.0.0")).unwrap_err().to_string().contains("cannot specify both")
        );
        assert!(it("@1.0.0", None).unwrap_err().to_string().contains("missing crate name"));
        assert!(format!("{:#}", it("a", Some("v1.0.0")).unwrap_err()).contains("try changing"));
    }
}
