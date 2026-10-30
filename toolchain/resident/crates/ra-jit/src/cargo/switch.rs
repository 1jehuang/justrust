//! `ra-jit enable` and `ra-jit disable`: whether plain cargo builds the crates of the user's
//! workspaces through ra-jit. Cargo is told in its own configuration, the file in its home
//! directory, as `build.rustc-workspace-wrapper`. What the file was before is kept next to
//! it, and `disable` puts it back. `cargo fmt` and `cargo clippy`, which cargo does not build
//! in, are linked to blaze's ([`links`]).

mod links;

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

/// The table and the key of cargo's configuration that name the wrapper.
const TABLE: &str = "build";
const KEY: &str = "rustc-workspace-wrapper";

/// The names cargo reads its configuration from in `.cargo` of a directory.
pub(crate) const CONFIGS: [&str; 2] = [".cargo/config.toml", ".cargo/config"];

/// The variable that names the wrapper for one command, ahead of any configuration.
pub(crate) const VARIABLE: &str = "RUSTC_WORKSPACE_WRAPPER";

/// What `enable` did, for `disable` to undo and for the wrapper to run what was there.
#[derive(Serialize, Deserialize)]
pub(crate) struct Enabled {
    /// The configuration file that was written.
    pub(crate) config: PathBuf,
    /// The file as it was, when there was one.
    before: Option<String>,
    /// The file as `enable` wrote it.
    written: String,
    /// The wrapper the user had there, which ra-jit runs rustc through.
    pub(crate) chained: Option<String>,
    /// The wrapper cargo is told now.
    pub(crate) wrapper: PathBuf,
    /// The commands linked to blaze's.
    #[serde(default)]
    links: Vec<links::Link>,
}

/// Cargo's home directory, as cargo finds it.
pub(crate) fn cargo_home() -> Option<PathBuf> {
    let set = std::env::var_os("CARGO_HOME").filter(|it| !it.is_empty()).map(PathBuf::from);
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    set.or_else(|| Some(Path::new(&home()?).join(".cargo")))
}

/// The configuration files of cargo's home: the one cargo reads now and the one it read
/// before it had the extension.
pub(crate) fn home_configs() -> Vec<PathBuf> {
    cargo_home()
        .map(|it| ["config.toml", "config"].map(|name| it.join(name)).to_vec())
        .unwrap_or_default()
}

/// The wrapper that stands in for rustc: `ra-jit-rustc`, installed next to this ra-jit.
pub(crate) fn wrapper_path() -> std::io::Result<PathBuf> {
    let this = std::env::current_exe()?;
    Ok(this.with_file_name(format!("ra-jit-rustc{}", std::env::consts::EXE_SUFFIX)))
}

/// Whether `path` is the wrapper, under either of its names.
pub(crate) fn is_wrapper(path: &Path) -> bool {
    let stem = path.file_stem().and_then(|it| it.to_str());
    stem.is_some_and(|it| it == "ra-jit-rustc" || it == "ra-jit-rustc-first")
}

impl Enabled {
    fn path() -> anyhow::Result<PathBuf> {
        Ok(cargo_home().context("no CARGO_HOME or HOME")?.join("ra-jit-enabled.json"))
    }

    /// What `enable` did, when ra-jit is enabled.
    pub(crate) fn read() -> Option<Enabled> {
        let text = std::fs::read_to_string(Enabled::path().ok()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write(&self) -> anyhow::Result<()> {
        let path = Enabled::path()?;
        let new = path.with_extension(format!("{}.new", std::process::id()));
        std::fs::write(&new, serde_json::to_vec_pretty(self)?)?;
        Ok(std::fs::rename(&new, &path)?)
    }
}

/// `ra-jit enable`: has cargo run rustc through ra-jit in every workspace of the user.
pub(crate) fn enable() -> anyhow::Result<i32> {
    let wrapper = wrapper_path()?;
    anyhow::ensure!(
        wrapper.exists(),
        "{} is not installed next to blaze; plain cargo is not changed",
        wrapper.display()
    );
    let home = cargo_home().context("no CARGO_HOME or HOME")?;
    if let Some(mut enabled) = Enabled::read() {
        if enabled.wrapper == wrapper && wrapper_in(&enabled.config).as_deref() == wrapper.to_str()
        {
            let mut made = Vec::new();
            let lines = links::make(&home, &enabled.links, &mut made);
            if made.is_empty() && lines.as_ref().is_ok_and(Vec::is_empty) {
                println!("blaze is enabled already in {}", enabled.config.display());
            }
            enabled.links.retain(|it| !made.iter().any(|made| made.path == it.path));
            enabled.links.extend(made);
            enabled.write()?;
            lines?.iter().for_each(|it| println!("{it}"));
            return Ok(0);
        }
        // Another ra-jit, or a configuration that was changed since: start over from what
        // the user has.
        disable()?;
    }
    let configs = home_configs();
    let config = configs.iter().find(|it| it.exists()).or(configs.first());
    let config = config.context("no CARGO_HOME or HOME")?.clone();
    let before = match std::fs::read_to_string(&config) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("reading {}", config.display())),
    };
    let text = before.clone().unwrap_or_default();
    let table = parsed(&text).with_context(|| format!("{} is not TOML", config.display()))?;
    let chained = wrapper_of(&table).filter(|it| Path::new(it) != wrapper);
    let named = wrapper.to_str().context("the path of ra-jit-rustc is not text")?.to_owned();
    let named = named.as_str();
    let written = with_wrapper(&text, named).with_context(|| {
        format!(
            "{} sets `{TABLE}` in a way blaze does not edit; add `{KEY} = \"{named}\"` to it",
            config.display()
        )
    })?;
    let mut enabled =
        Enabled { config: config.clone(), before, written, chained, wrapper, links: Vec::new() };
    enabled.write()?;
    if let Some(dir) = config.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&config, &enabled.written)
        .with_context(|| format!("writing {}", config.display()))?;
    println!("blaze is enabled: {} has `{TABLE}.{KEY} = \"{named}\"`", config.display());
    if let Some(chained) = &enabled.chained {
        println!("rustc runs through `{chained}` as before, under blaze");
    }
    let mut made = Vec::new();
    let lines = links::make(&home, &[], &mut made);
    enabled.links = made;
    enabled.write()?;
    lines?.iter().for_each(|it| println!("{it}"));
    let cwd = std::env::current_dir()?;
    for line in around(&cwd).into_iter().chain(overrides(&cwd)) {
        println!("{line}");
    }
    Ok(0)
}

/// `ra-jit disable`: puts cargo's configuration back as it was before `enable`. A file
/// nobody wrote since is the same file again, byte for byte; in one that was edited the
/// one key is put back.
pub(crate) fn disable() -> anyhow::Result<i32> {
    let Some(enabled) = Enabled::read() else {
        println!("blaze is not enabled");
        return Ok(0);
    };
    links::undo(&enabled.links)?.iter().for_each(|it| println!("{it}"));
    let config = &enabled.config;
    let now = std::fs::read_to_string(config).ok();
    match (&now, &enabled.before) {
        (Some(now), None) if *now == enabled.written => std::fs::remove_file(config)?,
        (Some(now), Some(before)) if *now == enabled.written => std::fs::write(config, before)?,
        (Some(now), _) => {
            let before = enabled.before.as_deref().unwrap_or_default();
            let line = parsed(before).ok().and_then(|_| wrapper_line(before).map(|it| it.1));
            let text = without_wrapper(now, line.as_deref()).with_context(|| {
                format!(
                    "{} was changed in a way blaze does not edit; remove `{KEY}` from it",
                    config.display()
                )
            })?;
            std::fs::write(config, text)?;
        }
        (None, _) => {}
    }
    std::fs::remove_file(Enabled::path()?)?;
    println!("blaze is disabled: {} is as it was", config.display());
    Ok(0)
}

/// The lines of `ra-jit status` about plain cargo: whether it builds through ra-jit, and
/// what in `cwd` has it not.
pub(crate) fn status(cwd: &Path) -> Vec<String> {
    let Some(enabled) = Enabled::read() else {
        return vec!["plain cargo: not through blaze (`blaze enable` turns it on)".to_owned()];
    };
    let mut lines = vec![format!(
        "plain cargo: through blaze, by `{TABLE}.{KEY}` in {}",
        enabled.config.display()
    )];
    if let Some(chained) = &enabled.chained {
        lines.push(format!("  rustc runs through `{chained}`, the wrapper there before"));
    }
    if wrapper_in(&enabled.config).as_deref() != enabled.wrapper.to_str() {
        lines.push(format!("  but {} no longer names blaze", enabled.config.display()));
    }
    if let Some(home) = cargo_home() {
        lines.extend(links::status(&home, &enabled.links).into_iter().map(|it| format!("  {it}")));
    }
    lines.extend(around(cwd).into_iter().chain(overrides(cwd)).map(|it| format!("  {it}")));
    lines
}

/// The key of the table and the variable that name the wrapper cargo runs every rustc
/// through, ra-jit's wrapper too: a cache like sccache. It is left as it is.
const OUTER_KEY: &str = "rustc-wrapper";
const OUTER_VARIABLE: &str = "RUSTC_WRAPPER";

/// What cargo runs ra-jit's wrapper through in `dir`, when the user has such a wrapper.
fn around(dir: &Path) -> Option<String> {
    let set = std::env::var(OUTER_VARIABLE).ok().filter(|it| !it.is_empty());
    let configured = || {
        let configs = dir.ancestors().flat_map(|it| CONFIGS.map(|name| it.join(name)));
        configs.chain(home_configs()).find_map(|it| {
            let table = parsed(&std::fs::read_to_string(it).ok()?).ok()?;
            table.get(TABLE)?.get(OUTER_KEY)?.as_str().map(str::to_owned)
        })
    };
    let outer = set.or_else(configured)?;
    Some(format!("`{outer}` stays cargo's `{OUTER_KEY}`: it runs blaze's wrapper as it ran rustc"))
}

/// The variable `cargo ra-jit` names the workspace's own `rustc-workspace-wrapper` in, for
/// ra-jit's wrapper to run rustc through: the one it puts in that wrapper's place.
pub(crate) const CHAINED: &str = "RA_JIT_CHAINED_WRAPPER";

/// The variables cargo reads the workspace's wrapper from, ahead of every configuration
/// file: its own, then the one that stands for the key of its configuration.
const WRAPPER_VARIABLES: [&str; 2] = [VARIABLE, "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"];

/// The `rustc-workspace-wrapper` cargo would run in `dir`, which `cargo ra-jit` overrides
/// with its own. An empty one is the project saying rustc runs through no wrapper, as cargo
/// reads it; `None` is that it names none, or names ra-jit's, and leaves it to `ra-jit enable`.
pub(crate) fn project_wrapper(dir: &Path) -> Option<std::ffi::OsString> {
    let set = WRAPPER_VARIABLES.iter().find_map(|it| std::env::var(it).ok());
    wrapper_for(dir, set, &home_configs())
}

/// The wrapper cargo would run in `dir`, in cargo's order: the variable `set` ahead of every
/// file, a file nearer `dir` ahead of one further up, and the files of cargo's home, `home`,
/// last. A path with a separator is relative to `dir` when a variable names it and to the
/// directory that holds `.cargo` when a file does; a bare name is looked up in `PATH`.
fn wrapper_for(dir: &Path, set: Option<String>, home: &[PathBuf]) -> Option<std::ffi::OsString> {
    let (root, named) = match set {
        Some(set) => (dir.to_owned(), set),
        None => {
            let nearer = dir.ancestors().flat_map(|it| CONFIGS.map(|name| it.join(name)));
            let mut configs = nearer.filter(|it| !home.contains(it)).chain(home.iter().cloned());
            configs.find_map(|config| {
                let named = wrapper_in(&config)?;
                Some((config.parent()?.parent()?.to_owned(), named))
            })?
        }
    };
    if is_wrapper(Path::new(&named)) {
        return None;
    }
    if !named.contains('/') && !named.contains(std::path::MAIN_SEPARATOR) {
        return Some(named.into());
    }
    Some(root.join(named).into_os_string())
}

/// Whether the user has cargo run every rustc through a wrapper of their own in `dir`.
pub(crate) fn user_wraps(dir: &Path) -> bool {
    let set = std::env::var_os("CARGO_BUILD_RUSTC_WRAPPER").is_some_and(|it| !it.is_empty());
    set || around(dir).is_some()
}

/// What has cargo not run ra-jit in `dir` while it is enabled: cargo reads the variable
/// ahead of every file, and a file nearer the workspace ahead of the one in its home.
fn overrides(dir: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let ours = wrapper_path().ok();
    if let Some(set) =
        std::env::var_os(VARIABLE).filter(|it| Some(Path::new(it)) != ours.as_deref())
    {
        let set = set.to_string_lossy();
        lines.push(format!(
            "not in this shell: {VARIABLE} is set to `{set}`, which cargo reads first"
        ));
    }
    let home = home_configs();
    let nearer = dir
        .ancestors()
        .flat_map(|it| CONFIGS.map(|name| it.join(name)))
        .filter(|it| !home.contains(it));
    for config in nearer {
        if let Some(named) = wrapper_in(&config).filter(|it| Some(Path::new(it)) != ours.as_deref())
        {
            lines.push(format!(
                "not below {}: its configuration sets `{TABLE}.{KEY} = \"{named}\"`, which cargo reads first",
                config.parent().and_then(Path::parent).unwrap_or(dir).display()
            ));
        }
    }
    lines
}

fn parsed(text: &str) -> Result<Table, toml::de::Error> {
    text.parse::<Table>()
}

fn wrapper_of(table: &Table) -> Option<String> {
    table.get(TABLE)?.get(KEY)?.as_str().map(str::to_owned)
}

/// The wrapper the configuration file `config` names, when it names one.
fn wrapper_in(config: &Path) -> Option<String> {
    wrapper_of(&parsed(&std::fs::read_to_string(config).ok()?).ok()?)
}

/// Where the key is set in `text`: the number of the line, and the line. It is looked for
/// as `rustc-workspace-wrapper = ` under `[build]` and as `build.rustc-workspace-wrapper = `
/// above the first table, which is how the file is written by hand.
fn wrapper_line(text: &str) -> Option<(usize, String)> {
    let sets = |line: &str, key: &str| {
        line.trim_start().strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let mut table = None;
    for (at, line) in text.lines().enumerate() {
        if let Some(header) = header(line) {
            table = Some(header);
            continue;
        }
        let found = match table {
            None => sets(line, &format!("{TABLE}.{KEY}")),
            Some(table) => table == TABLE && sets(line, KEY),
        };
        if found {
            return Some((at, line.to_owned()));
        }
    }
    None
}

/// The name of the table the line starts, when it starts one.
fn header(line: &str) -> Option<&str> {
    let line = line.trim();
    let line = line.split_once('#').map_or(line, |(before, _)| before.trim_end());
    Some(line.strip_prefix('[')?.strip_suffix(']')?.trim())
}

/// `text` with line `at` replaced by `with`, or without the line. The file ends its lines
/// as it did.
fn with_line(text: &str, at: usize, with: Option<&str>, replaces: bool) -> String {
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<&str> = text.lines().collect();
    match (with, replaces) {
        (Some(line), true) => lines[at] = line,
        (Some(line), false) => lines.insert(at, line),
        (None, _) => {
            lines.remove(at);
        }
    }
    lines.iter().flat_map(|it| [it, ending]).collect()
}

/// `text` with the key set to `wrapper` and nothing else changed. `None` when the file sets
/// the table in a way that is not edited here: the result is read back, and has to be the
/// file it was but for the one key.
fn with_wrapper(text: &str, wrapper: &str) -> Option<String> {
    let before = parsed(text).ok()?;
    let value = Value::String(wrapper.to_owned()).to_string();
    let under_table = format!("{KEY} = {value}");
    let header_at = text.lines().position(|it| header(it) == Some(TABLE));
    let written = match (wrapper_line(text), header_at) {
        (Some((at, line)), _) => {
            let dotted = line.trim_start().starts_with(TABLE);
            let line = if dotted { format!("{TABLE}.{under_table}") } else { under_table };
            with_line(text, at, Some(&line), true)
        }
        (None, Some(at)) => with_line(text, at + 1, Some(&under_table), false),
        (None, None) if before.contains_key(TABLE) => {
            with_line(text, 0, Some(&format!("{TABLE}.{under_table}")), false)
        }
        (None, None) => {
            let gap = match text {
                "" => "",
                text if text.ends_with('\n') => "\n",
                _ => "\n\n",
            };
            format!("{text}{gap}[{TABLE}]\n{under_table}\n")
        }
    };
    let mut expected = before;
    let table = expected.entry(TABLE).or_insert_with(|| Value::Table(Table::new()));
    table.as_table_mut()?.insert(KEY.to_owned(), Value::String(wrapper.to_owned()));
    (parsed(&written).ok()? == expected).then_some(written)
}

/// `text` with the line that sets the key replaced by `line`, the one that was there
/// before `enable`, or without it. `None` when what is left is not the file but for the
/// key.
fn without_wrapper(text: &str, line: Option<&str>) -> Option<String> {
    let Some((at, _)) = wrapper_line(text) else { return Some(text.to_owned()) };
    let written = with_line(text, at, line, true);
    let mut expected = parsed(text).ok()?;
    let was = line.and_then(|_| wrapper_of(&parsed(&written).ok()?));
    let table = expected.get_mut(TABLE)?.as_table_mut()?;
    match was {
        Some(was) => table.insert(KEY.to_owned(), Value::String(was)),
        None => table.remove(KEY),
    };
    (parsed(&written).ok()? == expected).then_some(written)
}

#[cfg(test)]
mod tests {
    use super::{with_wrapper, without_wrapper, wrapper_line};

    const OURS: &str = "/opt/ra-jit/ra-jit-rustc";

    #[test]
    fn the_key_goes_under_the_table_that_is_there() {
        let text = "# mine\n[net]\noffline = false\n\n[build] # all\njobs = 4\n";
        let written = with_wrapper(text, OURS).unwrap();
        assert_eq!(
            written,
            "# mine\n[net]\noffline = false\n\n[build] # all\nrustc-workspace-wrapper = \"/opt/ra-jit/ra-jit-rustc\"\njobs = 4\n"
        );
        assert_eq!(without_wrapper(&written, None).unwrap(), text);
    }

    #[test]
    fn a_file_without_the_table_gets_one() {
        assert_eq!(
            with_wrapper("", OURS).unwrap(),
            "[build]\nrustc-workspace-wrapper = \"/opt/ra-jit/ra-jit-rustc\"\n"
        );
        let written = with_wrapper("[net]\noffline = true", OURS).unwrap();
        assert!(written.starts_with("[net]\noffline = true\n\n[build]\n"), "{written}");
    }

    #[test]
    fn a_dotted_table_stays_dotted() {
        let text = "build.rustc-wrapper = \"sccache\"\n[net]\noffline = true\n";
        let written = with_wrapper(text, OURS).unwrap();
        assert!(written.starts_with("build.rustc-workspace-wrapper = "), "{written}");
        assert_eq!(without_wrapper(&written, None).unwrap(), text);
    }

    #[test]
    fn the_wrapper_that_was_there_comes_back() {
        let text = "[build]\r\nrustc-workspace-wrapper = 'mine' # kept\r\n";
        let (_, line) = wrapper_line(text).unwrap();
        let written = with_wrapper(text, OURS).unwrap();
        assert!(!written.contains("mine") && written.contains("\r\n"), "{written}");
        assert_eq!(without_wrapper(&written, Some(&line)).unwrap(), text);
    }

    #[test]
    fn a_table_that_is_not_lines_is_left_alone() {
        assert_eq!(with_wrapper("build = { jobs = 2 }\n", OURS), None);
        assert_eq!(with_wrapper("[build\n", OURS), None);
    }

    #[test]
    fn the_same_key_of_another_table_is_not_the_key() {
        let text = "[target.x86_64-unknown-linux-gnu]\nrustc-workspace-wrapper = \"other\"\n";
        assert_eq!(wrapper_line(text), None);
        assert!(
            with_wrapper(text, OURS)
                .unwrap()
                .ends_with("[build]\nrustc-workspace-wrapper = \"/opt/ra-jit/ra-jit-rustc\"\n")
        );
    }

    #[test]
    fn the_project_wrapper_is_relative_to_the_directory_above_its_config() {
        let root = std::env::temp_dir().join(format!("ra-jit-wrapper-{}", std::process::id()));
        let inner = root.join("crates").join("a");
        std::fs::create_dir_all(root.join(".cargo")).unwrap();
        std::fs::create_dir_all(&inner).unwrap();
        let config = root.join(".cargo").join("config.toml");
        let write = |text: &str| std::fs::write(&config, text).unwrap();
        let found = |set: Option<&str>, home: &[std::path::PathBuf]| {
            super::wrapper_for(&inner, set.map(str::to_owned), home)
        };
        write("[build]\nrustc-workspace-wrapper = \"scripts/frontend\"\n");
        let relative = Some(root.join("scripts/frontend").into_os_string());
        assert_eq!(found(None, &[]), relative);
        write("[build]\nrustc-workspace-wrapper = \"frontend\"\n");
        assert_eq!(found(None, &[]), Some("frontend".into()));
        write("[build]\njobs = 4\n");
        assert_eq!(found(None, &[]), None);
        // An empty wrapper is none, which is not the same as naming none.
        write("[build]\nrustc-workspace-wrapper = \"\"\n");
        assert_eq!(found(None, &[]), Some("".into()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_variable_comes_first_and_cargo_home_last() {
        let root = std::env::temp_dir().join(format!("ra-jit-order-{}", std::process::id()));
        let inner = root.join("project").join("crates").join("a");
        let home = root.join("cargo-home");
        std::fs::create_dir_all(root.join("project").join(".cargo")).unwrap();
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let project = root.join("project").join(".cargo").join("config.toml");
        let homes = [home.join("config.toml"), home.join("config")];
        let wrapper = |named: &str| format!("[build]\nrustc-workspace-wrapper = \"{named}\"\n");
        let found = |set: Option<&str>| super::wrapper_for(&inner, set.map(str::to_owned), &homes);
        std::fs::write(&homes[0], wrapper("home-wrapper")).unwrap();
        assert_eq!(found(None), Some("home-wrapper".into()));
        std::fs::write(&project, wrapper("project-wrapper")).unwrap();
        assert_eq!(found(None), Some("project-wrapper".into()));
        assert_eq!(found(Some("bin/set")), Some(inner.join("bin/set").into_os_string()));
        assert_eq!(found(Some("")), Some("".into()));
        assert_eq!(found(Some("/opt/ra-jit/ra-jit-rustc")), None);
        std::fs::remove_file(&project).unwrap();
        std::fs::write(&homes[0], wrapper("/opt/ra-jit/ra-jit-rustc")).unwrap();
        assert_eq!(found(None), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
