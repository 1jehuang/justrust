//! `cargo blaze clippy`: clippy's lints, blaze's own (`crates/blaze-lints`), run by the
//! daemon's checker over the crates it loads as `cargo blaze check` would, and written as
//! `cargo clippy` writes them: each build of a crate cargo would make a unit of, with its
//! diagnostics as rustc writes them and the line cargo ends a unit's warnings with. No cargo,
//! rustc or clippy-driver runs.
//!
//! What is not cargo's yet: a crate built twice for one build (for the host and the target)
//! is linted once, `build.rustflags` and `target.*.rustflags` are not read (`RUSTFLAGS` is),
//! and code only a build without `cfg(test)` has is not seen when tests are linted too.

use std::{
    io::IsTerminal as _,
    path::{Path, PathBuf},
    time::Instant,
};

use blaze_lints::serve::{Answer, Ask, Linted, Message};
use cargo_metadata::{DependencyKind, Metadata, Package, Target, TargetKind};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use super::super::{
    FAILED,
    cli::{Asked, Invocation},
    client::{self, Start},
    daemon::{Reply, Request},
    messages::{self, Format},
    say, state,
};

/// The flags `cargo clippy` takes that `cargo check` does not.
const CLIPPYS: &[&str] =
    &["--no-deps", "--fix", "--allow-dirty", "--allow-staged", "--allow-no-vcs", "--broken-code"];

/// `cargo blaze clippy <rest>`; returns the status to exit with.
pub(crate) fn command(rest: &[String]) -> anyhow::Result<i32> {
    let (ours, clippy_args) = match rest.iter().position(|it| it == "--") {
        Some(at) => (&rest[..at], &rest[at + 1..]),
        None => (rest, &[][..]),
    };
    if let Some(fix) = ours.iter().find(|it| it.as_str() == "--fix") {
        say::error(&format!(
            "blaze's clippy does not apply its suggestions yet (`{fix}`); nothing was changed"
        ));
        return Ok(FAILED);
    }
    let no_deps = ours.iter().any(|it| it == "--no-deps");
    let mut typed = vec!["check".to_owned()];
    typed.extend(ours.iter().filter(|it| !CLIPPYS.contains(&it.as_str())).cloned());
    let Asked::Build(asked) = Asked::parse(typed) else {
        anyhow::bail!(
            "`cargo blaze clippy` reads its flags as `cargo check`'s, and they did not read"
        );
    };
    let format = match Format::of(&asked.flags.message_format) {
        Ok(format) => format,
        Err(why) => {
            say::error(&why);
            return Ok(1);
        }
    };
    let started = Instant::now();
    let metadata = state::metadata(&asked);
    let plan = units(&asked, &metadata, no_deps)?;
    let ask = ask(&asked, &metadata, &plan.units, clippy_args, format)?;
    let ahead = state::checked(&asked)?;
    let answer =
        match client::ask(Request::Lint(ahead, serde_json::to_string(&ask)?), Start::IfMissing)? {
            Some(Reply::Linted { answer }) => serde_json::from_str::<Answer>(&answer)?,
            Some(Reply::Checked { errors: Some(errors) }) => {
                messages::turned_down(format, &[errors], &metadata);
                return Ok(FAILED);
            }
            Some(Reply::Cold { reason }) => {
                say::error(&format!("blaze could not lint the workspace: {reason}"));
                return Ok(FAILED);
            }
            reply => anyhow::bail!("the daemon answered a lint request with {reply:?}"),
        };
    if !answer.failed.is_empty() {
        for (_, why) in &answer.failed {
            say::error(why);
        }
        return Ok(FAILED);
    }
    let failed = written(&plan, &answer, format, clippy_args);
    if !failed {
        let took = started.elapsed().as_secs_f64();
        let tag = crate::cargo::blaze_tag();
        let finished = format!("`{}` profile [{tag}] target(s) in {took:.2}s", asked.profile());
        say::status(asked.flags.quiet, "Finished", &finished);
    }
    if format.is_json() {
        println!(r#"{{"reason":"build-finished","success":{}}}"#, !failed);
    }
    Ok(if failed { FAILED } else { 0 })
}

/// A build of a crate cargo would make: its package, its target, and whether it is built
/// with `--test`.
struct Unit<'a> {
    package: &'a Package,
    target: &'a Target,
    test: bool,
}

impl Unit<'_> {
    /// cargo's `description_named`: `lib`, `bin "app"`.
    fn described(&self) -> String {
        let target = self.target;
        match target.kind.first() {
            Some(TargetKind::Bin) => format!("bin \"{}\"", target.name),
            Some(TargetKind::Example) => format!("example \"{}\"", target.name),
            Some(TargetKind::Test) => format!("test \"{}\"", target.name),
            Some(TargetKind::Bench) => format!("bench \"{}\"", target.name),
            Some(TargetKind::CustomBuild) => "build script".to_owned(),
            _ => "lib".to_owned(),
        }
    }

    /// Whether cargo names it a test build: one with `--test` of a target that is no test or
    /// bench.
    fn named_test(&self) -> bool {
        self.test && !self.target.is_test() && !self.target.is_bench()
    }

    /// cargo's `descriptive_pkg_name`: `` `app` (bin "app" test) ``.
    fn named(&self) -> String {
        let test = if self.named_test() { " test" } else { "" };
        format!("`{}` ({}{test})", self.package.name, self.described())
    }
}

/// The units of a build, each after those it depends on, and the packages of the workspace
/// each package depends on.
struct Plan<'a> {
    units: Vec<Unit<'a>>,
    deps: FxHashMap<&'a str, Vec<&'a str>>,
}

/// The units `asked` builds: every target selected of each package selected, and the library
/// of each other package of the workspace they depend on, which `cargo clippy` lints too.
fn units<'a>(
    asked: &Invocation,
    metadata: &'a Metadata,
    no_deps: bool,
) -> anyhow::Result<Plan<'a>> {
    let members = metadata.workspace_packages();
    let selected = selected(asked, metadata)?;
    let flags = &asked.flags;
    let all = asked.typed.iter().any(|it| it == "--all-targets");
    let mut deps: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
    for package in &members {
        let of_it = package.dependencies.iter().filter(|dep| dep.path.is_some());
        let named = of_it.filter_map(|dep| members.iter().find(|it| it.name.as_str() == dep.name));
        deps.insert(package.name.as_str(), named.map(|it| it.name.as_str()).collect());
    }
    let mut units = Vec::new();
    for package in &selected {
        for target in &package.targets {
            let kind = |it: TargetKind| target.kind.contains(&it);
            let lib = target.is_lib() || kind(TargetKind::ProcMacro) || kind(TargetKind::RLib);
            let (plain, test) = match () {
                _ if target.is_custom_build() => (false, false),
                _ if all => {
                    let tested = target.test;
                    match () {
                        _ if lib || target.is_bin() => (true, tested),
                        _ if target.is_example() => (true, false),
                        _ => (false, true),
                    }
                }
                _ if flags.selects_targets => selects(&asked.typed, target, lib),
                _ => (lib || target.is_bin(), false),
            };
            if plain {
                units.push(Unit { package, target, test: false });
            }
            if test {
                units.push(Unit { package, target, test: true });
            }
        }
    }
    if no_deps {
        return Ok(Plan { units, deps });
    }
    let tested = units.iter().any(|it| it.test);
    let mut reached: FxHashSet<&str> = selected.iter().map(|it| it.name.as_str()).collect();
    let mut first = Vec::new();
    for package in &selected {
        depended_on(package, &members, tested, &mut reached, &mut first);
    }
    let libs = first.into_iter().filter_map(|package| {
        let target = package.targets.iter().find(|it| it.is_lib() || it.is_proc_macro())?;
        Some(Unit { package, target, test: false })
    });
    let mut ordered: Vec<Unit<'a>> = libs.collect();
    ordered.extend(units);
    Ok(Plan { units: ordered, deps })
}

/// Adds the packages of the workspace `package` depends on that are not `reached` yet to
/// `found`, each after those it depends on. Its dev-dependencies count when it is built with
/// its tests.
fn depended_on<'a>(
    package: &'a Package,
    members: &[&'a Package],
    tests: bool,
    reached: &mut FxHashSet<&'a str>,
    found: &mut Vec<&'a Package>,
) {
    for dep in &package.dependencies {
        if dep.kind == DependencyKind::Development && !tests || dep.path.is_none() {
            continue;
        }
        let Some(member) = members.iter().find(|it| it.name.as_str() == dep.name) else { continue };
        if reached.insert(member.name.as_str()) {
            depended_on(member, members, false, reached, found);
            found.push(member);
        }
    }
}

/// Whether `target` is built without `--test` and with it for the target flags typed.
fn selects(typed: &[String], target: &Target, lib: bool) -> (bool, bool) {
    let has = |flag: &str| typed.iter().any(|it| it == flag);
    let named = |flag: &str| {
        let mut words = typed.iter();
        let mut names = Vec::new();
        while let Some(word) = words.next() {
            if word == flag {
                names.extend(words.next().cloned());
            } else if let Some(name) = word.strip_prefix(flag).and_then(|it| it.strip_prefix('=')) {
                names.push(name.to_owned());
            }
        }
        names.contains(&target.name)
    };
    let tests = has("--tests") && target.test;
    let plain = (lib && has("--lib"))
        || (target.is_bin() && (has("--bins") || named("--bin")))
        || (target.is_example() && (has("--examples") || named("--example")));
    let test = tests && (lib || target.is_bin() || target.is_test() || target.is_example())
        || (target.is_test() && named("--test"))
        || (target.is_bench() && (has("--benches") || named("--bench")));
    (plain, test)
}

/// The packages `asked` builds: those `-p` names, every one for `--workspace`, else the one
/// of the manifest, or for a virtual one its `default-members`, else every member.
fn selected<'a>(asked: &Invocation, metadata: &'a Metadata) -> anyhow::Result<Vec<&'a Package>> {
    let members = metadata.workspace_packages();
    let flags = &asked.flags;
    if !flags.packages.is_empty() {
        let mut found = Vec::new();
        for named in &flags.packages {
            let name = named.split('@').next().unwrap_or(named);
            let package = members.iter().find(|it| it.name.as_str() == name);
            found.push(*package.ok_or_else(|| {
                anyhow::anyhow!("package `{named}` is not a member of the workspace")
            })?);
        }
        return Ok(found);
    }
    if flags.workspace {
        return Ok(members
            .into_iter()
            .filter(|it| !flags.excludes.contains(&it.name.to_string()))
            .collect());
    }
    let cwd = std::env::current_dir()?;
    let manifest = match &flags.manifest_path {
        Some(it) => cwd.join(it),
        None => cwd
            .ancestors()
            .map(|it| it.join("Cargo.toml"))
            .find(|it| it.exists())
            .unwrap_or_default(),
    };
    if let Some(package) = members.iter().find(|it| it.manifest_path.as_std_path() == manifest) {
        return Ok(vec![*package]);
    }
    let root = metadata.workspace_root.as_std_path();
    let defaults = default_members(&root.join("Cargo.toml"));
    let chosen: Vec<&Package> = members
        .iter()
        .filter(|it| {
            defaults
                .iter()
                .any(|dir| it.manifest_path.as_std_path() == root.join(dir).join("Cargo.toml"))
        })
        .copied()
        .collect();
    Ok(if chosen.is_empty() { members } else { chosen })
}

/// The `[workspace] default-members` of the root manifest at `path`.
fn default_members(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let Ok(table) = text.parse::<toml::Table>() else { return Vec::new() };
    let members = table
        .get("workspace")
        .and_then(|it| it.get("default-members"))
        .and_then(|it| it.as_array());
    members.into_iter().flatten().filter_map(|it| it.as_str().map(str::to_owned)).collect()
}

/// The lint request of the packages the units are of.
fn ask(
    asked: &Invocation,
    metadata: &Metadata,
    units: &[Unit<'_>],
    clippy_args: &[String],
    format: Format,
) -> anyhow::Result<Ask> {
    let root_manifest = metadata.workspace_root.as_std_path().join("Cargo.toml");
    let rustflags = rustflags();
    let conf_dir =
        std::env::var_os("CLIPPY_CONF_DIR").filter(|it| !it.is_empty()).map(PathBuf::from);
    let mut packages: Vec<blaze_lints::serve::Package> = Vec::new();
    for unit in units {
        let package = unit.package;
        if packages.iter().any(|it| it.name == package.name.as_str()) {
            continue;
        }
        let manifest = package.manifest_path.as_std_path();
        let lints =
            blaze_lints::manifest::lints(manifest, &root_manifest).map_err(anyhow::Error::msg)?;
        let mut flags = blaze_lints::manifest::flags(&lints);
        flags.extend(rustflags.iter().cloned());
        flags.extend(clippy_args.iter().cloned());
        packages.push(blaze_lints::serve::Package {
            name: package.name.to_string(),
            manifest_dir: manifest.parent().map(Path::to_owned).unwrap_or_default(),
            flags,
            conf_dir: conf_dir.clone(),
            rust_version: package.rust_version.as_ref().map(|it| it.to_string()),
        });
    }
    let color = colour(&asked.typed);
    let (short, ansi) = match format {
        Format::Human => (false, color),
        Format::Short => (true, color),
        Format::Json { short, ansi, render } => (short, ansi || render && color),
    };
    let width = (format == Format::Human || format == Format::Short).then(terminal_width).flatten();
    Ok(Ask { packages, short, ansi, width })
}

/// What `RUSTFLAGS` gives rustc, as cargo reads it: `CARGO_ENCODED_RUSTFLAGS` split at `\x1f`,
/// else `RUSTFLAGS` split at whitespace.
fn rustflags() -> Vec<String> {
    if let Ok(encoded) = std::env::var("CARGO_ENCODED_RUSTFLAGS") {
        return encoded.split('\x1f').filter(|it| !it.is_empty()).map(str::to_owned).collect();
    }
    let flags = std::env::var("RUSTFLAGS").unwrap_or_default();
    flags.split_whitespace().map(str::to_owned).collect()
}

/// Whether diagnostics are in colour, as `--color` and cargo's terminal say.
fn colour(typed: &[String]) -> bool {
    let mut words = typed.iter();
    let mut asked = None;
    while let Some(word) = words.next() {
        if word == "--color" {
            asked = words.next().cloned();
        } else if let Some(value) = word.strip_prefix("--color=") {
            asked = Some(value.to_owned());
        }
    }
    match asked.as_deref() {
        Some("always") => true,
        Some("never") => false,
        _ => std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    }
}

/// The columns of the terminal standard error is, which cargo passes rustc as
/// `--diagnostic-width`. None on Windows, which has no `TIOCGWINSZ`.
#[cfg(not(unix))]
fn terminal_width() -> Option<usize> {
    None
}

/// The columns of the terminal standard error is, which cargo passes rustc as
/// `--diagnostic-width`.
#[cfg(unix)]
fn terminal_width() -> Option<usize> {
    if !std::io::stderr().is_terminal() {
        return None;
    }
    // SAFETY: `TIOCGWINSZ` writes a `winsize` and nothing else.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let got = unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) };
    (got == 0 && size.ws_col > 0).then_some(usize::from(size.ws_col))
}

/// What cargo counts of a unit's warnings.
#[derive(Default)]
struct Count {
    total: usize,
    duplicates: usize,
    /// `None` once an error makes `cargo clippy --fix` not apply.
    fixable: Option<usize>,
    errors: usize,
}

/// Writes each unit's diagnostics as `format` asks, with the line cargo ends a unit's warnings
/// with and the error it ends a failed unit with. Says whether a unit failed.
/// A unit that depends on a library that failed is not built.
fn written(plan: &Plan<'_>, answer: &Answer, format: Format, clippy_args: &[String]) -> bool {
    let mut emitted: FxHashSet<String> = FxHashSet::default();
    let mut failed = false;
    // The packages whose library failed, or was not built for one it depends on that did.
    let mut broken: FxHashSet<&str> = FxHashSet::default();
    let skipped = |unit: &Unit<'_>, broken: &FxHashSet<&str>| {
        let name = unit.package.name.as_str();
        let lib = unit.target.is_lib() || unit.target.is_proc_macro();
        let deps = plan.deps.get(name).map(Vec::as_slice).unwrap_or_default();
        deps.iter().any(|it| broken.contains(it)) || !lib && broken.contains(name)
    };
    for (at, unit) in plan.units.iter().enumerate() {
        let name = unit.package.name.as_str();
        let lib = unit.target.is_lib() || unit.target.is_proc_macro();
        if skipped(unit, &broken) {
            if lib && !unit.test {
                broken.insert(name);
            }
            continue;
        }
        let src = unit.target.src_path.as_std_path();
        let Some(linted) = answer.crates.iter().find(|it| it.root == src) else { continue };
        let messages = messages_of(linted, unit.test);
        let mut count = Count { fixable: Some(0), ..Count::default() };
        for message in messages {
            if message.error {
                count.errors += 1;
                count.fixable = None;
            } else {
                count.total += 1;
            }
            match format {
                Format::Json { .. } => {
                    compiler_message(unit, &message.json);
                }
                Format::Human | Format::Short => {
                    let new = emitted.insert(message.rendered.clone());
                    if new {
                        eprint!("{}", message.rendered);
                    } else if !message.error {
                        count.duplicates += 1;
                    }
                    if new
                        && !message.error
                        && message.fixable
                        && let Some(fixable) = &mut count.fixable
                    {
                        *fixable += 1;
                    }
                }
            }
        }
        if count.errors > 0 && lib && !unit.test {
            broken.insert(name);
        }
        if count.total > 0 && !format.is_json() {
            summary(unit, &count, clippy_args);
        }
        if count.errors > 0 {
            // cargo says so at its first failure while other units are still being built.
            let waiting = !failed && plan.units[at + 1..].iter().any(|it| !skipped(it, &broken));
            failed = true;
            let errors = match count.errors {
                1 => " due to 1 previous error".to_owned(),
                n => format!(" due to {n} previous errors"),
            };
            let warnings = match count.total {
                0 => String::new(),
                1 => "; 1 warning emitted".to_owned(),
                n => format!("; {n} warnings emitted"),
            };
            say::error(&format!("could not compile {}{errors}{warnings}", unit.named()));
            if waiting {
                say::warning("build failed, waiting for other jobs to finish...");
            }
        }
    }
    failed
}

fn messages_of(linted: &Linted, test: bool) -> &[Message] {
    match (&linted.test, test) {
        (Some(test), true) => test,
        _ => &linted.plain,
    }
}

/// cargo's `report_warning_count`: `` warning: `app` (lib) generated 2 warnings (run `cargo
/// clippy --fix --lib -p app` to apply 1 suggestion) ``.
fn summary(unit: &Unit<'_>, count: &Count, clippy_args: &[String]) {
    let mut message = format!("{} generated ", unit.named());
    match count.total {
        1 => message.push_str("1 warning"),
        n => message.push_str(&format!("{n} warnings")),
    }
    match count.duplicates {
        0 => {}
        1 => message.push_str(" (1 duplicate)"),
        n => message.push_str(&format!(" ({n} duplicates)")),
    }
    if let Some(fixable) = count.fixable.filter(|it| *it > 0) {
        let mut args = format!("{} -p {}", unit.described(), unit.package.name);
        if unit.named_test() {
            args.push_str(" --tests");
        }
        let suggestions = match fixable {
            1 => "1 suggestion".to_owned(),
            n => format!("{n} suggestions"),
        };
        // `cargo clippy` hands clippy-driver what follows `--` each ended by a separator, and
        // cargo writes them back with that last one as a space.
        let mut lints: String = clippy_args.iter().map(|it| format!("{it} ")).collect();
        lints.truncate(lints.trim_ascii_end().len());
        message.push_str(&format!(
            " (run `cargo clippy --fix --{args} -- {lints}` to apply {suggestions})"
        ));
    }
    say::warning(&message);
}

/// Writes `message`, rustc's JSON of a diagnostic, as cargo's `compiler-message` of `unit`.
fn compiler_message(unit: &Unit<'_>, message: &str) {
    #[derive(Serialize)]
    struct Head<'a> {
        reason: &'static str,
        package_id: &'a str,
        manifest_path: &'a str,
        target: OfTarget<'a>,
    }
    #[derive(Serialize)]
    struct OfTarget<'a> {
        kind: &'a [TargetKind],
        crate_types: &'a [cargo_metadata::CrateType],
        name: &'a str,
        src_path: &'a str,
        edition: &'a str,
        doc: bool,
        doctest: bool,
        test: bool,
    }
    let target = unit.target;
    let head = Head {
        reason: "compiler-message",
        package_id: &unit.package.id.repr,
        manifest_path: unit.package.manifest_path.as_str(),
        target: OfTarget {
            kind: &target.kind,
            crate_types: &target.crate_types,
            name: &target.name,
            src_path: target.src_path.as_str(),
            edition: target.edition.as_str(),
            doc: target.doc,
            doctest: target.doctest,
            test: target.test,
        },
    };
    let Ok(mut line) = serde_json::to_string(&head) else { return };
    line.pop();
    println!("{line},\"message\":{message}}}");
}
