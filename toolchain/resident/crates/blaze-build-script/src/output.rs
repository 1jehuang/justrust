//! What a build script prints, read as cargo 1.99 reads it: `BuildOutput::parse` in
//! `src/compiler/custom_build.rs` of rust-lang/cargo at 5f94df478. Lines that start with
//! neither `cargo:` nor `cargo::`, and lines that are not UTF-8, are the script's own.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use serde_derive::Serialize;

use crate::{
    Error,
    unit::{Package, Unit},
};

/// A message a script asked cargo to show.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Severity {
    Error,
    Warning,
}

/// A directory of `rustc-link-search` or `rustc-flags`' `-L`, as written (`native=/x`).
/// cargo passes those inside the script's `OUT_DIR` first, each kind in the order printed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum LibraryPath {
    CargoArtifact(PathBuf),
    External(PathBuf),
}

impl LibraryPath {
    fn new(path: PathBuf, out_dir: &Path) -> LibraryPath {
        match search_path_of(&path).starts_with(out_dir) {
            true => LibraryPath::CargoArtifact(path),
            false => LibraryPath::External(path),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            LibraryPath::CargoArtifact(it) | LibraryPath::External(it) => it,
        }
    }
}

/// The directory of a search path, without its kind (`native=`, `crate=`, `dependency=`,
/// `framework=`, `all=`).
pub fn search_path_of(path: &Path) -> &Path {
    match path.to_str().and_then(|it| it.split_once('=')) {
        Some(("native" | "crate" | "dependency" | "framework" | "all", path)) => Path::new(path),
        _ => path,
    }
}

/// Which of the package's targets a `rustc-link-arg*` is for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum LinkArgTarget {
    /// `rustc-link-arg`.
    All,
    /// `rustc-cdylib-link-arg` and `rustc-link-arg-cdylib`.
    Cdylib,
    /// `rustc-link-arg-bins`.
    Bin,
    /// `rustc-link-arg-bin=NAME=ARG`.
    SingleBin(String),
    /// `rustc-link-arg-tests`.
    Test,
    /// `rustc-link-arg-benches`.
    Bench,
    /// `rustc-link-arg-examples`.
    Example,
}

/// What a script printed, as cargo keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Output {
    /// `-L` for the package and everything that links it.
    pub library_paths: Vec<LibraryPath>,
    /// `-l` for the package's library (or its programs when it has none), as written:
    /// `[KIND[:MODIFIERS]=]NAME[:RENAME]`.
    pub library_links: Vec<String>,
    pub linker_args: Vec<(LinkArgTarget, String)>,
    /// `--cfg` for every target of the package.
    pub cfgs: Vec<String>,
    /// `--check-cfg` for every target of the package.
    pub check_cfgs: Vec<String>,
    /// Variables for rustc's runs on the package's targets, and for `cargo run` and `cargo
    /// test` of them.
    pub env: Vec<(String, String)>,
    /// What the scripts of packages that depend on this one directly see as
    /// `DEP_<links>_<key>`, when this one has `links`.
    pub metadata: Vec<(String, String)>,
    /// Relative to the package's directory, or absolute.
    pub rerun_if_changed: Vec<PathBuf>,
    pub rerun_if_env_changed: Vec<String>,
    pub log_messages: Vec<(Severity, String)>,
}

/// What reading a script's output depends on besides the output.
#[derive(Clone, Debug)]
pub struct Context<'a> {
    pub package: &'a Package,
    /// The `OUT_DIR` the script ran with, and the one it has now: a path the output names in
    /// the first is taken to be in the second, as when the target directory moved.
    pub out_dir_when_generated: &'a Path,
    pub out_dir: &'a Path,
    /// Whether cargo allows nightly features: its channel, or `RUSTC_BOOTSTRAP=1`.
    pub nightly_features_allowed: bool,
    /// `RUSTC_BOOTSTRAP` in cargo's environment.
    pub rustc_bootstrap: Option<String>,
}

impl<'a> Context<'a> {
    /// The context of `unit`'s run under a stable cargo whose environment is `cargo_env`.
    pub fn of(unit: &'a Unit, cargo_env: &[(OsString, OsString)]) -> Context<'a> {
        let var = |name: &str| {
            cargo_env
                .iter()
                .find(|(it, _)| it == OsStr::new(name))
                .and_then(|(_, value)| value.to_str().map(str::to_owned))
        };
        let channel = var("__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS")
            .or_else(|| (var("RUSTC_BOOTSTRAP").as_deref() == Some("1")).then(|| "dev".to_owned()));
        Context {
            package: &unit.package,
            out_dir_when_generated: &unit.out_dir,
            out_dir: &unit.out_dir,
            nightly_features_allowed: matches!(channel.as_deref(), Some("nightly" | "dev")),
            rustc_bootstrap: var("RUSTC_BOOTSTRAP"),
        }
    }
}

const DOCS: &str = "See https://doc.rust-lang.org/cargo/reference/build-scripts.html#outputs-of-the-build-script \
                    for more information about build script outputs.";

/// The keys of the old syntax that are not metadata: no new one can be added, since
/// `cargo:KEY=VALUE` is metadata for any other.
const RESERVED_PREFIXES: &[&str] = &[
    "rustc-flags=",
    "rustc-link-lib=",
    "rustc-link-search=",
    "rustc-link-arg-cdylib=",
    "rustc-cdylib-link-arg=",
    "rustc-link-arg-bins=",
    "rustc-link-arg-bin=",
    "rustc-link-arg-tests=",
    "rustc-link-arg-benches=",
    "rustc-link-arg-examples=",
    "rustc-link-arg=",
    "rustc-cfg=",
    "rustc-check-cfg=",
    "rustc-env=",
    "warning=",
    "rerun-if-changed=",
    "rerun-if-env-changed=",
];

fn has_reserved_prefix(flag: &str) -> bool {
    RESERVED_PREFIXES.iter().any(|it| flag.starts_with(it))
}

/// `rust-version` as cargo reads it: one to three numbers.
fn rust_version(text: &str) -> Option<(u64, Option<u64>, Option<u64>)> {
    let mut parts = text.trim().split('.');
    let mut number = |required: bool| match parts.next() {
        Some(it) => it.parse::<u64>().ok().map(Some),
        None => (!required).then_some(None),
    };
    let version = (number(true)??, number(false)?, number(false)?);
    parts.next().is_none().then_some(version)
}

/// `cargo::` is from 1.77: a package whose `rust-version` is older cannot use it.
fn check_new_syntax(cx: &Context<'_>, descr: &str, flag: &str) -> Result<(), Error> {
    let Some(msrv) = cx.package.rust_version.as_deref() else { return Ok(()) };
    let Some((major, minor, patch)) = rust_version(msrv) else { return Ok(()) };
    let compatible = major == 1 && (minor.unwrap_or(0), patch.unwrap_or(0)) >= (77, 0);
    if compatible {
        return Ok(());
    }
    let shown =
        [Some(major), minor, patch].iter().flatten().map(u64::to_string).collect::<Vec<_>>();
    let msrv = shown.join(".");
    let suggestion = if has_reserved_prefix(flag) {
        format!("Switch to the old `cargo:{flag}` syntax (note the single colon).\n")
    } else if let Some(old) = flag.strip_prefix("metadata=") {
        format!(
            "Switch to the old `cargo:{old}` syntax instead of `cargo::{flag}` (note the single colon).\n"
        )
    } else {
        String::new()
    };
    Err(Error(format!(
        "the `cargo::` syntax for build script output instructions was added in \
         Rust 1.77.0, but the minimum supported Rust version of `{descr}` is {msrv}.\n\
         {suggestion}{DOCS}"
    )))
}

/// `KEY=VALUE`, the value's end trimmed.
fn key_value<'a>(
    whence: &str,
    line: &str,
    data: &'a str,
    syntax: &str,
) -> Result<(&'a str, &'a str), Error> {
    match data.split_once('=') {
        Some((key, value)) => Ok((key, value.trim_end())),
        None => Err(Error(format!(
            "invalid output in {whence}: `{line}`\n\
             Expected a line with `{syntax}KEY=VALUE` with an `=` character, \
             but none was found.\n\
             {DOCS}"
        ))),
    }
}

/// Reads what a script printed.
pub fn parse(input: &[u8], cx: &Context<'_>) -> Result<Output, Error> {
    let package = cx.package;
    let descr = package.descr();
    let whence = format!("build script of `{descr}`");
    let mut out = Output::default();
    let (Some(generated), Some(out_dir_text)) =
        (cx.out_dir_when_generated.to_str(), cx.out_dir.to_str())
    else {
        return Err(Error("an `OUT_DIR` that is not UTF-8".to_owned()));
    };
    for line in input.split(|b| *b == b'\n') {
        let Ok(line) = std::str::from_utf8(line) else { continue };
        let line = line.trim();
        let old_syntax;
        let (key, value) = if let Some(data) = line.strip_prefix("cargo::") {
            check_new_syntax(cx, &descr, data)?;
            old_syntax = false;
            key_value(&whence, line, data, "cargo::")?
        } else if let Some(data) = line.strip_prefix("cargo:") {
            old_syntax = true;
            match has_reserved_prefix(data) {
                true => key_value(&whence, line, data, "cargo:")?,
                false => ("metadata", data),
            }
        } else {
            continue;
        };
        let value = match generated.is_empty() {
            true => value.to_owned(),
            false => value.replace(generated, out_dir_text),
        };
        let syntax = if old_syntax { "cargo:" } else { "cargo::" };
        let missing_target = |kind: &str| {
            Error(format!(
                "invalid instruction `{syntax}{key}` from {whence}\n\
                 The package {descr} does not have a {kind} target."
            ))
        };
        let targets = &package.targets;
        match key {
            "rustc-flags" => {
                let (paths, links) = parse_rustc_flags(&value, &whence)?;
                out.library_links.extend(links);
                out.library_paths
                    .extend(paths.into_iter().map(|it| LibraryPath::new(it, cx.out_dir)));
            }
            "rustc-link-lib" => out.library_links.push(value),
            "rustc-link-search" => {
                out.library_paths.push(LibraryPath::new(PathBuf::from(value), cx.out_dir))
            }
            "rustc-link-arg-cdylib" | "rustc-cdylib-link-arg" => {
                if !targets.iter().any(|it| it.is_cdylib()) {
                    out.log_messages.push((
                        Severity::Warning,
                        format!(
                            "{syntax}{key} was specified in the build script of {descr}, \
                             but that package does not contain a cdylib target\n\
                             \n\
                             Allowing this was an unintended change in the 1.50 \
                             release, and may become an error in the future. \
                             For more information, see \
                             <https://github.com/rust-lang/cargo/issues/9562>."
                        ),
                    ));
                }
                out.linker_args.push((LinkArgTarget::Cdylib, value));
            }
            "rustc-link-arg-bins" => {
                if !targets.iter().any(|it| it.is_bin()) {
                    return Err(missing_target("bin"));
                }
                out.linker_args.push((LinkArgTarget::Bin, value));
            }
            "rustc-link-arg-bin" => {
                let Some((bin, arg)) = value.split_once('=') else {
                    return Err(Error(format!(
                        "invalid instruction `{syntax}{key}={value}` from {whence}\n\
                         The instruction should have the form {syntax}{key}=BIN=ARG"
                    )));
                };
                if !targets.iter().any(|it| it.is_bin() && it.name == bin) {
                    return Err(Error(format!(
                        "invalid instruction `{syntax}{key}` from {whence}\n\
                         The package {descr} does not have a bin target with the name `{bin}`."
                    )));
                }
                out.linker_args.push((LinkArgTarget::SingleBin(bin.to_owned()), arg.to_owned()));
            }
            "rustc-link-arg-tests" => {
                if !targets.iter().any(|it| it.is_test()) {
                    return Err(missing_target("test"));
                }
                out.linker_args.push((LinkArgTarget::Test, value));
            }
            "rustc-link-arg-benches" => {
                if !targets.iter().any(|it| it.is_bench()) {
                    return Err(missing_target("benchmark"));
                }
                out.linker_args.push((LinkArgTarget::Bench, value));
            }
            "rustc-link-arg-examples" => {
                if !targets.iter().any(|it| it.is_example()) {
                    return Err(missing_target("example"));
                }
                out.linker_args.push((LinkArgTarget::Example, value));
            }
            "rustc-link-arg" => out.linker_args.push((LinkArgTarget::All, value)),
            "rustc-cfg" => out.cfgs.push(value),
            "rustc-check-cfg" => out.check_cfgs.push(value),
            "rustc-env" => {
                let (name, val) = parse_rustc_env(&value, &whence)?;
                if name != "RUSTC_BOOTSTRAP" {
                    out.env.push((name, val));
                    continue;
                }
                // A crate setting it would leave the stability guarantees; what cargo was
                // run with can allow it, and then it only warns.
                let library = package.library_crate_name();
                let allowed_by_env = library.as_deref().is_some_and(|name| {
                    cx.rustc_bootstrap
                        .as_deref()
                        .is_some_and(|it| it.split(',').any(|it| it == name))
                });
                let message = format!(
                    "cannot set `RUSTC_BOOTSTRAP={val}` from {whence}.\n\
                     note: crates cannot set `RUSTC_BOOTSTRAP` themselves, as doing so would \
                     subvert the stability guarantees of Rust for your project."
                );
                if cx.nightly_features_allowed || allowed_by_env {
                    out.log_messages.push((Severity::Warning, message));
                } else {
                    let name = library.as_deref().unwrap_or("1");
                    return Err(Error(format!(
                        "{message}\n\
                         help: If you're sure you want to do this in your project, set the \
                         environment variable `RUSTC_BOOTSTRAP={name}` before running cargo \
                         instead."
                    )));
                }
            }
            "error" => out.log_messages.push((Severity::Error, value)),
            "warning" => out.log_messages.push((Severity::Warning, value)),
            "rerun-if-changed" => out.rerun_if_changed.push(PathBuf::from(value)),
            "rerun-if-env-changed" => out.rerun_if_env_changed.push(value),
            "metadata" => {
                let syntax = if old_syntax { "cargo:" } else { "cargo::metadata=" };
                let (key, value) = key_value(&whence, line, &value, syntax)?;
                out.metadata.push((key.to_owned(), value.to_owned()));
            }
            _ => {
                return Err(Error(format!(
                    "invalid output in {whence}: `{line}`\n\
                     Unknown key: `{key}`.\n\
                     {DOCS}"
                )));
            }
        }
    }
    Ok(out)
}

/// `rustc-flags`: only `-l` and `-L`, with or without a space before the value.
pub fn parse_rustc_flags(value: &str, whence: &str) -> Result<(Vec<PathBuf>, Vec<String>), Error> {
    let mut flags = value.trim().split(char::is_whitespace).filter(|it| !it.trim().is_empty());
    let (mut paths, mut links) = (Vec::new(), Vec::new());
    while let Some(flag) = flags.next() {
        if !flag.starts_with("-l") && !flag.starts_with("-L") {
            return Err(Error(format!(
                "only `-l` and `-L` flags are allowed in {whence}: `{}`",
                value
            )));
        }
        let (flag, mut rest) = flag.split_at(2);
        if rest.is_empty() {
            rest = flags.next().ok_or_else(|| {
                Error(format!("flag in rustc-flags has no value in {whence}: {rest}"))
            })?;
        }
        match flag {
            "-l" => links.push(rest.to_owned()),
            _ => paths.push(PathBuf::from(rest)),
        }
    }
    Ok((paths, links))
}

/// `rustc-env=NAME=VALUE`.
pub fn parse_rustc_env(value: &str, whence: &str) -> Result<(String, String), Error> {
    match value.split_once('=') {
        Some((name, value)) => Ok((name.to_owned(), value.to_owned())),
        None => Err(Error(format!("Variable rustc-env has no value in {whence}: {value}"))),
    }
}

/// A `rustc-link-lib` as rustc reads `-l`: `[KIND[:MODIFIERS]=]NAME[:RENAME]`. cargo passes
/// it on as written; rustc says what is wrong with one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkLib<'a> {
    /// `static`, `dylib`, `framework`, `raw-dylib` or `link-arg`; none when not written.
    pub kind: Option<&'a str>,
    /// `+bundle`, `-whole-archive` and the rest, each with whether it is turned on.
    pub modifiers: Vec<(bool, &'a str)>,
    pub name: &'a str,
    /// The name the library is linked under, after `:`.
    pub rename: Option<&'a str>,
}

impl<'a> LinkLib<'a> {
    pub fn parse(spec: &'a str) -> LinkLib<'a> {
        let (kind, name) = match spec.split_once('=') {
            Some((kind, name)) => (Some(kind), name),
            None => (None, spec),
        };
        let (kind, modifiers) = match kind.map(|it| it.split_once(':').unwrap_or((it, ""))) {
            Some((kind, modifiers)) => (Some(kind), modifiers),
            None => (None, ""),
        };
        let modifiers = modifiers
            .split(',')
            .filter(|it| !it.is_empty())
            .map(|it| match it.split_at(1) {
                ("+", name) => (true, name),
                ("-", name) => (false, name),
                _ => (true, it),
            })
            .collect();
        let (name, rename) = match name.split_once(':') {
            Some((name, rename)) => (name, Some(rename)),
            None => (name, None),
        };
        LinkLib { kind, modifiers, name, rename }
    }

    /// Whether `modifier` is turned on.
    pub fn has(&self, modifier: &str) -> bool {
        self.modifiers.iter().rev().find(|(_, it)| *it == modifier).is_some_and(|(on, _)| *on)
    }
}

#[cfg(test)]
mod tests;
