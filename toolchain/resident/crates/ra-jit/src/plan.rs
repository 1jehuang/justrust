//! What cargo builds: the units of a build of one package, with the features and the profile
//! of each. `cargo metadata` has neither as a build of a single package has them.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::Context;
use cfg::{CfgAtom, CfgDiff};
use intern::{Symbol, sym};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_derive::{Deserialize, Serialize};

/// What is said of a profile with `lto`. ra-jit compiles every function alone, so there is
/// nothing for it to optimize across functions, and the program does what rustc's does: only
/// how fast and how large it is differ. The libraries it links against are built without.
const LTO_ACCEPTED: &str = "lto: accepted, no cross-function optimization";

/// Which build of a package the daemon stands in for, as the flags of `cargo build` say it.
/// `cargo ra-jit` and `ra-jit` itself both say a build with this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Selection {
    /// The name of the profile: `dev` unless `--release` or `--profile` says otherwise.
    pub(crate) profile: String,
    pub(crate) features: Vec<String>,
    pub(crate) no_default_features: bool,
    pub(crate) all_features: bool,
    /// The packages asked for with `-p`: none is what `cargo build` builds where it is run.
    pub(crate) packages: Vec<String>,
    /// Whether every member of the workspace is built.
    pub(crate) workspace: bool,
    /// The members `--exclude` leaves out of the workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) excludes: Vec<String>,
    /// The values of `-Z`, which cargo is run with as it was.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) unstable: Vec<String>,
    /// The triple built for, when it is not the host.
    pub(crate) target: Option<String>,
    /// Whether tests, examples and benchmarks are built, and so what they depend on.
    pub(crate) dev_deps: bool,
    /// The `--config` values given, which can change a profile or what is built as much as
    /// the configuration files can.
    #[serde(default)]
    pub(crate) config: Vec<String>,
    /// Where the build was asked for, when not in the package's directory: the manifest given
    /// with `--manifest-path`, or the directory cargo looked for one from. What `--workspace`,
    /// `-p` and no package at all select is decided there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) asked_in: Option<PathBuf>,
}

impl Default for Selection {
    fn default() -> Selection {
        Selection {
            profile: "dev".to_owned(),
            features: Vec::new(),
            no_default_features: false,
            all_features: false,
            packages: Vec::new(),
            workspace: false,
            excludes: Vec::new(),
            unstable: Vec::new(),
            target: None,
            dev_deps: false,
            config: Vec::new(),
            asked_in: None,
        }
    }
}

impl Selection {
    /// The selection as flags, which `cargo build` and `ra-jit` both read. Whether tests are
    /// built is not among them: cargo and ra-jit name that differently.
    pub(crate) fn flags(&self) -> Vec<String> {
        let mut flags = vec!["--profile".to_owned(), self.profile.clone()];
        if let Some(target) = &self.target {
            flags.extend(["--target".to_owned(), target.clone()]);
        }
        if !self.features.is_empty() {
            flags.extend(["--features".to_owned(), self.features.join(",")]);
        }
        if self.no_default_features {
            flags.push("--no-default-features".to_owned());
        }
        if self.all_features {
            flags.push("--all-features".to_owned());
        }
        for package in &self.packages {
            flags.extend(["--package".to_owned(), package.clone()]);
        }
        if self.workspace {
            flags.push("--workspace".to_owned());
        }
        for package in &self.excludes {
            flags.extend(["--exclude".to_owned(), package.clone()]);
        }
        for unstable in &self.unstable {
            flags.extend(["-Z".to_owned(), unstable.clone()]);
        }
        for config in &self.config {
            flags.extend(["--config".to_owned(), config.clone()]);
        }
        flags
    }
}

/// How a package is built for the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Built {
    pub(crate) features: BTreeSet<String>,
    pub(crate) profile: Profile,
    /// Whether the package is one cargo was asked to build, not one those depend on.
    pub(crate) primary: bool,
    /// Whether the package is built into the program, not only for what runs while building.
    pub(crate) for_target: bool,
}

/// What a build is for and who asks for it.
pub(crate) struct Asked<'a> {
    /// The triple built for.
    pub(crate) triple: &'a str,
    /// What `rustc --print cfg` says of the triple.
    pub(crate) cfg: &'a [CfgAtom],
    /// The variables of the process that asks, where they are not those of this one.
    pub(crate) env: &'a FxHashMap<String, Option<String>>,
}

/// cargo, run in `dir` with `env` for the variables of whoever asks.
fn cargo_in(dir: &Path, env: &FxHashMap<String, Option<String>>) -> std::process::Command {
    let mut command = toolchain::command("cargo", dir, env);
    // `cargo config get` is not stable yet.
    command.env("RUSTC_BOOTSTRAP", "1").stdin(Stdio::null());
    crate::answers::ready_cargo(&mut command);
    command
}

impl Asked<'_> {
    /// cargo, run in `dir` as whoever asks would run it.
    fn cargo(&self, dir: &Path) -> std::process::Command {
        cargo_in(dir, self.env)
    }
}

/// The settings of a profile that change what a program does.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Profile {
    /// `dev`, `release`, `test`, `bench` or a custom profile's name.
    #[serde(default)]
    pub(crate) name: String,
    pub(crate) opt_level: String,
    lto: String,
    pub(crate) debug_assertions: bool,
    pub(crate) overflow_checks: bool,
    pub(crate) panic: PanicStrategy,
    debuginfo: DebugLevel,
    split_debuginfo: Option<String>,
    strip: Strip,
    /// The profile's own `rustflags` (cargo's `-Z profile-rustflags`), which come after the
    /// others.
    #[serde(default)]
    pub(crate) rustflags: Vec<String>,
}

/// A `debug` setting as cargo passes it on: a level by its number or by its name.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum DebugLevel {
    Number(u8),
    Named(String),
}

/// A `strip` setting. One cargo deferred is its own choice for a build without debug info,
/// which has nothing the setting would take out.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Strip {
    Resolved(Stripped),
    Deferred(Stripped),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
enum Stripped {
    None,
    Named(String),
}

/// What a build tells a debugger of its code, and where that is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Described {
    pub(crate) level: Level,
    pub(crate) kept: Kept,
    /// Whether every function keeps its frame pointer (`-C force-frame-pointers`), for a
    /// profiler or the program's own backtraces to walk the frames by.
    pub(crate) frame_pointers: bool,
}

impl Described {
    pub(crate) const NOTHING: Described =
        Described { level: Level::Nothing, kept: Kept::Linked, frame_pointers: false };

    /// The most either tells, kept the furthest from the program either keeps it.
    fn or(self, other: Described) -> Described {
        Described {
            level: self.level.max(other.level),
            kept: self.kept.max(other.kept),
            frame_pointers: self.frame_pointers || other.frame_pointers,
        }
    }
}

/// How much a build tells a debugger of its code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    Nothing,
    /// Which line each instruction is of, and the functions.
    Lines,
    /// The variables and their types as well.
    Full,
}

/// Where what a debugger reads of the code is kept (`split-debuginfo`). The lines are with
/// the program always.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kept {
    /// In the program, which the linker copies it into.
    Linked,
    /// In the objects the program was linked from, which the program names.
    Objects,
    /// In one package next to the program.
    Package,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PanicStrategy {
    Unwind,
    Abort,
    ImmediateAbort,
}

impl PanicStrategy {
    fn name(self) -> &'static str {
        match self {
            PanicStrategy::Unwind => "unwind",
            PanicStrategy::Abort | PanicStrategy::ImmediateAbort => "abort",
        }
    }
}

impl Profile {
    /// Whether the profile asks for any debug info: what cargo tells a build script in `DEBUG`.
    pub(crate) fn debuginfo(&self) -> bool {
        !matches!(&self.debuginfo, DebugLevel::Number(0))
            && !matches!(&self.debuginfo, DebugLevel::Named(it) if it == "none")
    }

    /// The profile as rustc ends up with it once `flags` follow what cargo passes for it.
    fn with(&self, flags: &Flags) -> Profile {
        // cargo names a check only where it is not what rustc has for the level by itself, so
        // a level among the flags changes the ones cargo left unnamed.
        let unoptimized = self.opt_level == "0";
        let named_assertions =
            (self.debug_assertions != unoptimized).then_some(self.debug_assertions);
        let named_overflow =
            (self.overflow_checks != self.debug_assertions).then_some(self.overflow_checks);
        let unoptimized = flags.opt_level.as_ref().map_or(unoptimized, |it| it == "0");
        let debug_assertions = flags.debug_assertions.or(named_assertions).unwrap_or(unoptimized);
        Profile {
            debug_assertions,
            overflow_checks: flags.overflow_checks.or(named_overflow).unwrap_or(debug_assertions),
            panic: flags.panic.unwrap_or(self.panic),
            debuginfo: flags.debuginfo.clone().unwrap_or_else(|| self.debuginfo.clone()),
            split_debuginfo: flags.split_debuginfo.clone().or_else(|| self.split_debuginfo.clone()),
            strip: flags.strip.clone().map_or_else(
                || self.strip.clone(),
                |named| Strip::Resolved(Stripped::Named(named)),
            ),
            ..self.clone()
        }
    }

    /// What the profile has a debugger told. Where the profile asks for it to be kept apart
    /// from the program, which ra-jit does not do, that is the reason not to build.
    pub(crate) fn described(&self) -> anyhow::Result<Described> {
        let level = match &self.debuginfo {
            DebugLevel::Number(0) => Level::Nothing,
            DebugLevel::Number(1) => Level::Lines,
            DebugLevel::Number(2) => Level::Full,
            DebugLevel::Named(name) => match name.as_str() {
                "none" => Level::Nothing,
                "line-directives-only" | "line-tables-only" | "limited" => Level::Lines,
                "full" => Level::Full,
                _ => anyhow::bail!("`debug = \"{name}\"`: blaze does not know the level"),
            },
            DebugLevel::Number(level) => {
                anyhow::bail!("`debug = {level}`: blaze does not know the level")
            }
        };
        // Stripping the symbols is the linker's to do; of what ra-jit writes, what a debugger
        // reads is all either setting takes out.
        let stripped = match &self.strip {
            Strip::Resolved(Stripped::Named(what)) => match what.as_str() {
                "none" => false,
                "debuginfo" | "symbols" => true,
                _ => anyhow::bail!("`strip = \"{what}\"`: blaze does not know the setting"),
            },
            Strip::Resolved(Stripped::None) | Strip::Deferred(_) => false,
        };
        if stripped || level == Level::Nothing {
            return Ok(Described::NOTHING);
        }
        let kept = match self.split_debuginfo.as_deref() {
            None | Some("off") => Kept::Linked,
            Some("unpacked") => Kept::Objects,
            Some("packed") => Kept::Package,
            Some(split) => {
                anyhow::bail!("`split-debuginfo = \"{split}\"`: blaze does not know the setting")
            }
        };
        Ok(Described { level, kept, frame_pointers: false })
    }

    /// Whether rustc compiles the whole program again when it links it.
    fn links_from_bitcode(&self) -> bool {
        matches!(self.lto.as_str(), "fat" | "thin" | "true")
    }

    /// What the profile changes of the cfg `rustc --print cfg -O` reports.
    pub(crate) fn cfg(&self) -> CfgDiff {
        let flag = |name: Symbol| CfgAtom::Flag(name);
        let panic = |strategy: PanicStrategy| CfgAtom::KeyValue {
            key: sym::panic,
            value: Symbol::intern(strategy.name()),
        };
        let (mut enable, mut disable) = (Vec::new(), Vec::new());
        let mut set =
            |on: bool, atom: CfgAtom| if on { enable.push(atom) } else { disable.push(atom) };
        set(self.debug_assertions, flag(sym::debug_assertions));
        set(self.overflow_checks, flag(sym::overflow_checks));
        for strategy in [PanicStrategy::Unwind, PanicStrategy::Abort] {
            set(strategy.name() == self.panic.name(), panic(strategy));
        }
        CfgDiff::new(enable, disable)
    }
}

/// What the flags given to every rustc of a build say of what a profile has too. They come
/// after the profile's own on the command line and so win.
#[derive(Debug, Default, PartialEq, Eq)]
struct Flags {
    opt_level: Option<String>,
    debug_assertions: Option<bool>,
    overflow_checks: Option<bool>,
    panic: Option<PanicStrategy>,
    lto: bool,
    debuginfo: Option<DebugLevel>,
    split_debuginfo: Option<String>,
    strip: Option<String>,
    frame_pointers: bool,
    links_dead_code: bool,
    lints_capped: bool,
    /// What `-C link-arg` and `-C link-args` give the linker, in order: rustc passes them
    /// after the native libraries, and so does ra-jit's own link.
    link_args: Vec<String>,
}

impl Flags {
    /// The flags cargo passes in `dir` for what is `asked`: the first of
    /// `CARGO_ENCODED_RUSTFLAGS`, `RUSTFLAGS`, the `[target]` tables that name what is built
    /// for and `[build]` of the configuration that has any, as blaze-manifest reads them and
    /// cargo's `TargetInfo` picks them, with no cargo run.
    fn of(dir: &Path, asked: &Asked<'_>) -> anyhow::Result<Flags> {
        let flags = crate::answers::cargo_rustflags(&asked.cargo(dir), asked.triple)
            .context("the flags cargo's configuration gives rustc")?;
        Flags::parse(flags.iter().map(String::as_str))
    }

    /// The flags of `words`. `Err` names a `-Z` flag ra-jit does not build code with
    /// ([`crate::unstable`]); the others it ignores, or what they ask is what it does.
    fn parse<'a>(words: impl Iterator<Item = &'a str>) -> anyhow::Result<Flags> {
        let words: Vec<&str> = words.collect();
        if let Some(why) = crate::unstable::refusal(&words) {
            anyhow::bail!("the flags for rustc have {why}; blaze does not build with it");
        }
        let mut words = words.into_iter();
        let mut flags = Flags::default();
        while let Some(word) = words.next() {
            if ["--cap-lints", "--force-warn"].iter().any(|it| {
                word.strip_prefix(it).is_some_and(|rest| rest.is_empty() || rest.starts_with('='))
            }) {
                flags.lints_capped = true;
                continue;
            }
            let option = match word {
                "-O" => "opt-level=3",
                "-g" => "debuginfo=2",
                "-C" => words.next().unwrap_or_default(),
                _ => match word.strip_prefix("-C") {
                    Some(option) => option,
                    None => continue,
                },
            };
            let (name, value) = option.split_once('=').unwrap_or((option, ""));
            let on = match value {
                "y" | "yes" | "on" | "true" | "" => Some(true),
                "n" | "no" | "off" | "false" => Some(false),
                _ => None,
            };
            match name.replace('_', "-").as_str() {
                "opt-level" => flags.opt_level = Some(value.to_owned()),
                "debug-assertions" => flags.debug_assertions = on,
                "overflow-checks" => flags.overflow_checks = on,
                "panic" if value == "abort" => flags.panic = Some(PanicStrategy::Abort),
                "panic" if value == "unwind" => flags.panic = Some(PanicStrategy::Unwind),
                "lto" => flags.lto = on != Some(false),
                "linker-plugin-lto" => flags.lto = on != Some(false),
                "debuginfo" => {
                    flags.debuginfo = Some(match value.parse() {
                        Ok(level) => DebugLevel::Number(level),
                        Err(_) => DebugLevel::Named(value.to_owned()),
                    });
                }
                "split-debuginfo" => flags.split_debuginfo = Some(value.to_owned()),
                "link-arg" => flags.link_args.push(value.to_owned()),
                "link-args" => flags.link_args.extend(value.split_whitespace().map(str::to_owned)),
                "strip" => flags.strip = Some(value.to_owned()),
                // `always` and `non-leaf` keep them too, `non-leaf` in more functions than asked.
                "force-frame-pointers" => {
                    flags.frame_pointers = on.unwrap_or(matches!(value, "always" | "non-leaf"));
                }
                "link-dead-code" => flags.links_dead_code = on != Some(false),
                _ => {}
            }
        }
        Ok(flags)
    }
}

/// The packages of a build by their id, each as it is built for the target.
pub(crate) struct Plan {
    packages: FxHashMap<String, Built>,
    lto: bool,
    frame_pointers: bool,
    links_dead_code: bool,
    lints_capped: bool,
    link_args: Vec<String>,
}

/// What a plan says of a program as a whole.
#[derive(Clone)]
pub(crate) struct Whole {
    /// What the build has a debugger told of the code.
    pub(crate) debug: Described,
    /// Whether rustc links it from bitcode, with `lto`.
    pub(crate) lto: bool,
    /// Whether rustc keeps every function, `#[inline(always)]` ones too, as one copy in one
    /// codegen unit (`-C link-dead-code`), where LLVM inlines it only into that unit's code.
    pub(crate) links_dead_code: bool,
    /// Whether `--cap-lints` or `--force-warn` is among the flags, so that a lint the source
    /// denies may not fail rustc's build.
    pub(crate) lints_capped: bool,
    /// What the flags give the linker (`-C link-arg`, `-C link-args`), which the daemon's own
    /// link passes as rustc's does.
    pub(crate) link_args: Vec<String>,
}

impl Plan {
    /// The units of the build `selection` names of the package of `manifest`.
    pub(crate) fn of(
        manifest: &Path,
        selection: &Selection,
        asked: &Asked<'_>,
    ) -> anyhow::Result<Plan> {
        // The unit graph says which features and which profile each package of one build has.
        let graph = UnitGraph::asked(manifest, selection, asked)?;
        let (dir, _) = asked_where(manifest, selection);
        UnitGraph::plan(graph, &Flags::of(dir, asked)?)
    }

    /// What the build has a debugger told of the packages cargo was asked to build: the
    /// most any of them asks for, which is one answer for all the code ra-jit writes.
    pub(crate) fn described(&self) -> anyhow::Result<Described> {
        let mut described = Described::NOTHING;
        for built in self.packages.values().filter(|it| it.primary) {
            described = described.or(built.profile.described()?);
        }
        Ok(described)
    }

    /// What the plan says of the programs cargo was asked to build.
    pub(crate) fn whole(&self) -> anyhow::Result<Whole> {
        let debug = Described { frame_pointers: self.frame_pointers, ..self.described()? };
        let link_args = self.link_args.clone();
        Ok(Whole {
            debug,
            lto: self.lto,
            links_dead_code: self.links_dead_code,
            lints_capped: self.lints_capped,
            link_args,
        })
    }

    /// How the package cargo calls `id` is built, when the build has it.
    pub(crate) fn package(&self, id: &str) -> Option<&Built> {
        self.packages.get(&package_named(id))
    }

    /// The profile of the packages cargo was asked to build, for the target.
    pub(crate) fn target_profile(&self) -> Option<&Profile> {
        self.packages.values().find(|it| it.primary && it.for_target).map(|it| &it.profile)
    }
}

/// The one name of a package for the two ways cargo spells its id: `source#name@version`,
/// and `name version (source)` as the unit graph of an older cargo does, with the
/// commit of a git source after it.
pub(crate) fn package_named(id: &str) -> String {
    let old = id.strip_suffix(')').and_then(|it| it.split_once(" ("));
    if let Some((name_version, source)) = old
        && let Some((name, version)) = name_version.split_once(' ')
    {
        let source = source.split_once('#').map_or(source, |it| it.0);
        return format!("{source}#{name}@{version}");
    }
    let Some((source, named)) = id.split_once('#') else { return id.to_owned() };
    if named.contains('@') {
        return id.to_owned();
    }
    // A package whose directory or repository is called what it is has only its version.
    let path = source.split_once('?').map_or(source, |it| it.0);
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
    let name = name.strip_suffix(".git").unwrap_or(name);
    format!("{source}#{name}@{named}")
}

/// A build's units as `cargo build --unit-graph` prints them (cargo's `unit_graph.rs`, format
/// version 1): what the build compiles and runs, each with its package, target, profile,
/// features and the units it depends on. The build scripts and proc macros of a load are
/// built and run from it (`cargo/compile_time.rs`).
#[derive(Deserialize)]
pub(crate) struct UnitGraph {
    pub(crate) units: Vec<Unit>,
    pub(crate) roots: Vec<usize>,
}

#[derive(Deserialize)]
pub(crate) struct Unit {
    pub(crate) pkg_id: String,
    pub(crate) target: UnitTarget,
    pub(crate) profile: Profile,
    /// The triple the unit is built for, `None` for the host.
    #[serde(default)]
    pub(crate) platform: Option<String>,
    /// `build`, `check`, `test`, `run-custom-build`, ...
    #[serde(default)]
    pub(crate) mode: String,
    pub(crate) features: Vec<String>,
    pub(crate) dependencies: Vec<UnitDependency>,
}

#[derive(Deserialize)]
pub(crate) struct UnitTarget {
    pub(crate) kind: Vec<String>,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) src_path: PathBuf,
    #[serde(default = "edition_2015")]
    pub(crate) edition: String,
}

fn edition_2015() -> String {
    "2015".to_owned()
}

#[derive(Deserialize)]
pub(crate) struct UnitDependency {
    pub(crate) index: usize,
    /// The name the unit links it under.
    #[serde(default)]
    pub(crate) extern_crate_name: String,
}

impl Unit {
    /// Whether the unit runs while building: a build script or a proc macro. What it depends
    /// on is built for the host, with features and a profile of its own.
    pub(crate) fn runs_on_host(&self) -> bool {
        self.target.kind.iter().any(|it| it == "custom-build" || it == "proc-macro")
    }

    pub(crate) fn is_library(&self) -> bool {
        self.target.kind.iter().any(|it| it.ends_with("lib"))
    }
}

/// Where the build `selection` names of the package of `manifest` is asked for: the directory,
/// and the manifest when one is named.
pub(crate) fn asked_where<'a>(
    manifest: &'a Path,
    selection: &'a Selection,
) -> (&'a Path, Option<&'a Path>) {
    match &selection.asked_in {
        Some(asked_in) if asked_in.is_dir() => (asked_in.as_path(), None),
        Some(asked_in) => (asked_in.parent().unwrap_or(Path::new(".")), Some(asked_in.as_path())),
        None => (manifest.parent().unwrap_or(Path::new(".")), Some(manifest)),
    }
}

/// The unit graphs one load was told, by what they were asked with: a load asks for its
/// graph once for the plan and once for its build scripts (`cargo/compile_time.rs`).
type Told = FxHashMap<String, std::sync::Arc<UnitGraph>>;

std::thread_local! {
    static TOLD: std::cell::RefCell<Option<Told>> = const { std::cell::RefCell::new(None) };
}

/// The load of a workspace on this thread, while it lives: the unit graphs asked for in it
/// are made once. A load after it makes them again, as the manifests may have changed since.
pub(crate) struct Load {
    outer: Option<Told>,
}

impl Load {
    pub(crate) fn start() -> Load {
        Load { outer: TOLD.with(|it| it.replace(Some(Told::default()))) }
    }
}

impl Drop for Load {
    fn drop(&mut self) {
        TOLD.with(|it| *it.borrow_mut() = self.outer.take());
    }
}

impl UnitGraph {
    /// The units of the build `selection` names of the package of `manifest`, for what
    /// `asked` says: what the load under way was told already when it asked the same.
    pub(crate) fn asked(
        manifest: &Path,
        selection: &Selection,
        asked: &Asked<'_>,
    ) -> anyhow::Result<std::sync::Arc<UnitGraph>> {
        let (dir, manifest) = asked_where(manifest, selection);
        let mut vars: Vec<_> = asked.env.iter().collect();
        vars.sort();
        let cfg = format!("{:?}", asked.cfg);
        let key = serde_json::to_string(&(dir, manifest, selection, vars, asked.triple, cfg))?;
        let told = TOLD.with(|it| it.borrow().as_ref().and_then(|it| it.get(&key).cloned()));
        if let Some(graph) = told {
            return Ok(graph);
        }
        let graph = std::sync::Arc::new(UnitGraph::of(dir, manifest, selection, asked)?);
        TOLD.with(|it| {
            if let Some(told) = it.borrow_mut().as_mut() {
                told.insert(key, graph.clone());
            }
        });
        Ok(graph)
    }

    /// The unit graph of the build `selection` names, asked in `dir` of the package of
    /// `manifest` when there is one: what `cargo build --unit-graph` says of it, made by
    /// blaze-units with no cargo run (harness/resolve.py `units` holds the two equal).
    fn of(
        dir: &Path,
        manifest: Option<&Path>,
        selection: &Selection,
        asked: &Asked<'_>,
    ) -> anyhow::Result<UnitGraph> {
        let graph = crate::cargo::metadata::units(selection, dir, manifest, asked)?;
        let json = serde_json::to_value(&graph).context("writing blaze's unit graph")?;
        serde_json::from_value(json).context("blaze's unit graph is not cargo's form")
    }

    /// The units that end up in the program: those the roots reach without going through one
    /// that runs on the host.
    pub(crate) fn for_target(&self) -> FxHashSet<usize> {
        let mut reached = FxHashSet::default();
        let mut queue: Vec<usize> =
            self.roots.iter().copied().filter(|&it| it < self.units.len()).collect();
        while let Some(index) = queue.pop() {
            let unit = &self.units[index];
            if unit.runs_on_host() || !reached.insert(index) {
                continue;
            }
            queue.extend(
                unit.dependencies.iter().map(|it| it.index).filter(|&it| it < self.units.len()),
            );
        }
        reached
    }

    fn plan(graph: std::sync::Arc<UnitGraph>, flags: &Flags) -> anyhow::Result<Plan> {
        let this = &*graph;
        let for_target = this.for_target();
        // The flag is not cargo's to know: the libraries are then built for a link without it
        // and rustc finds no bitcode in them.
        anyhow::ensure!(
            !flags.lto,
            "`-C lto` is among the flags: rustc compiles the whole program again as it links \
             it and leaves no library on the link line for blaze's functions to be linked against"
        );
        for unit in &this.units {
            if let Some(why) = crate::unstable::refusal(&unit.profile.rustflags) {
                anyhow::bail!(
                    "the profile's rustflags for `{}` have {why}; blaze does not build with it",
                    unit.target.name
                );
            }
        }
        let lto = for_target.iter().any(|&it| this.units[it].profile.links_from_bitcode());
        if lto {
            eprintln!("{LTO_ACCEPTED}");
        }
        // A package that is only built for the host, a proc macro, is as that build has it.
        let rank = |index: usize, unit: &Unit| (for_target.contains(&index), unit.is_library());
        let mut best: FxHashMap<&str, (usize, &Unit)> = FxHashMap::default();
        for (index, unit) in this.units.iter().enumerate() {
            let chosen = best.entry(unit.pkg_id.as_str()).or_insert((index, unit));
            if rank(index, unit) > rank(chosen.0, chosen.1) {
                *chosen = (index, unit);
            }
        }
        let roots: FxHashSet<&str> =
            this.roots.iter().filter_map(|&it| Some(this.units.get(it)?.pkg_id.as_str())).collect();
        let packages = best
            .into_iter()
            .map(|(id, (index, unit))| {
                let features = unit.features.iter().cloned().collect();
                let (profile, primary) = (unit.profile.with(flags), roots.contains(id));
                let for_target = for_target.contains(&index);
                (package_named(id), Built { features, profile, primary, for_target })
            })
            .collect();
        Ok(Plan {
            packages,
            lto,
            frame_pointers: flags.frame_pointers,
            links_dead_code: flags.links_dead_code,
            lints_capped: flags.lints_capped,
            link_args: flags.link_args.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Flags;

    #[test]
    fn force_frame_pointers_is_read_in_each_spelling() {
        let kept = |flags: &str| Flags::parse(flags.split_whitespace()).unwrap().frame_pointers;
        assert!(kept("-Cforce-frame-pointers=yes"));
        assert!(kept("--cap-lints allow -C force-frame-pointers"));
        assert!(kept("-C force_frame_pointers=always"));
        assert!(kept("-Cforce-frame-pointers=non-leaf"));
        assert!(!kept("-Cforce-frame-pointers=yes -Cforce-frame-pointers=no"));
        assert!(!kept("-C link-dead-code"));
    }

    #[test]
    fn link_dead_code_is_read_in_each_spelling() {
        let kept = |flags: &str| Flags::parse(flags.split_whitespace()).unwrap().links_dead_code;
        assert!(kept("-C link-dead-code"));
        assert!(kept("-Clink_dead_code=yes"));
        assert!(!kept("-C link-dead-code -C link-dead-code=no"));
        assert!(!kept("-C force-frame-pointers"));
    }

    #[test]
    fn the_flags_that_cap_lints_are_read() {
        let capped = |flags: &str| Flags::parse(flags.split_whitespace()).unwrap().lints_capped;
        assert!(capped("--cap-lints warn"));
        assert!(capped("-C opt-level=1 --cap-lints=allow"));
        assert!(capped("--force-warn non_snake_case"));
        assert!(!capped("-D warnings -A dead_code"));
        assert!(!capped("--cap-lintsx"));
    }

    #[test]
    fn link_args_are_read_in_order_in_each_spelling() {
        let words =
            ["-C", "link-arg=-Wl,-no_compact_unwind", "-Clink-args=-a -b", "-C", "link_arg=-c"];
        let args = Flags::parse(words.into_iter());
        assert_eq!(args.unwrap().link_args, ["-Wl,-no_compact_unwind", "-a", "-b", "-c"]);
    }

    #[test]
    fn unstable_flags_are_ignored_or_refused_by_name() {
        let flags = Flags::parse("-Zthreads=8 -Z time-passes -Copt-level=1".split_whitespace());
        assert_eq!(flags.unwrap().opt_level.as_deref(), Some("1"));
        let refused = Flags::parse("-Zthreads=8 -Z location-detail=none".split_whitespace());
        assert!(refused.unwrap_err().to_string().contains("-Z location-detail=none"));
    }
}
