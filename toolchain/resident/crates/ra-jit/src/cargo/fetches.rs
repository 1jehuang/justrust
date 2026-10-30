//! What build scripts download, pinned as Nix pins a fixed-output derivation: by the sha256
//! of the bytes, recorded once, and handed to the script with no network.
//!
//! A build script runs with no network ([`super::script_sandbox`]). A crate that downloads a
//! prebuilt library in its build script (rusty_v8's V8, ort's ONNX Runtime, livekit's
//! libwebrtc) is given the download instead, through the crate's own variable for a local
//! copy, as crate2nix's `defaultCrateOverrides` gives it to a Nix build: [`OVERRIDES`] says,
//! for each crate blaze knows, which variable and where the download is.
//!
//! `blaze lock --fetches` ([`command`]) reads `Cargo.lock`, and for each of its packages that
//! [`OVERRIDES`] has, for each target asked (the host by default), works out the address,
//! downloads it, keeps the bytes in blaze's store by their sha256, and writes what it
//! downloaded into [`FILE`] beside `Cargo.lock`: not into `Cargo.lock`, whose fields a cargo
//! that does not know them drops when it writes it again. An entry is kept by the package's
//! name, version and what `Cargo.lock` pins it by (its checksum, or its git source), the
//! target, and the features of it that pick the download.
//!
//! A build ([`hand`], from [`super::script_sandbox::plan`]) finds the entry of the package
//! whose script it runs, writes the store's bytes out below the target directory (unpacked,
//! where the variable names a directory) and sets the variable to them. The path is below
//! the profile's directory, which the key of the script's run names by what stands for it
//! (`script.rs`'s `Places`), so the run is the same on every machine; what it names is
//! told by the sha256 in it. A package of [`OVERRIDES`] with no entry, or one for another
//! version or checksum, fails, naming it and the command that records it: a build never
//! downloads.
//!
//! [`FILE`] is TOML: `version` (of the whole file, [`VERSION`]), a `[[fetch]]` table for each
//! download ([`Entry`]), and `[[system]]` tables, which are the system libraries' resolution
//! and whose fields this module leaves to the code that writes them.
//!
//! The bytes are an entry of the store kept by their sha256 alone
//! ([`ra_jit_store::Store::open_by_content`]), so every build of blaze finds them, and a
//! remote cache carries them as it carries any entry.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::remote::{self, Https, Traffic};
use crate::sandbox::{Line, Reached};

/// The file beside `Cargo.lock` with what build scripts download.
pub(crate) const FILE: &str = "Cargo.fetches.lock";

/// The format of [`FILE`].
const VERSION: u32 = 1;

/// The command that writes [`FILE`], as an error names it.
const COMMAND: &str = "blaze lock --fetches";

/// The directory below the profile's where downloads are written out for build scripts.
const WRITTEN: &str = "blaze-fetches";

/// How long a download waits for the server to answer, or for the next of its bytes.
const PATIENCE: Duration = Duration::from_secs(60);

/// How a crate's build script is given what it would download.
pub(crate) struct Override {
    /// The package whose build script it is.
    pub(crate) package: &'static str,
    pub(crate) give: Give,
}

pub(crate) enum Give {
    /// A value of a variable that has the script download nothing.
    Value { var: &'static str, value: &'static str },
    /// A download, named by the variable the script reads a local copy from.
    Fetch {
        var: &'static str,
        /// The package's features that pick what is downloaded.
        features: &'static [&'static str],
        /// The download for a package, `None` for a target there is none for.
        download: fn(&Of<'_>) -> anyhow::Result<Option<Download>>,
        hand: Hand,
    },
}

/// What the variable names.
#[derive(Clone, Copy)]
pub(crate) enum Hand {
    /// The file downloaded.
    File,
    /// What the archive downloaded holds, unpacked below `into` (where `{target}` is the
    /// target and `{sha256}` the archive's) in a directory of its own; the variable names that
    /// directory, or what [`Download::names`] says below it.
    Unpacked { archive: Archive, into: &'static str },
}

#[derive(Clone, Copy)]
pub(crate) enum Archive {
    Zip,
    /// A tar in a raw LZMA2 stream of a 64 MiB dictionary, as ort's CDN serves it.
    TarLzma2,
}

/// A package, as an override works out its download.
pub(crate) struct Of<'a> {
    pub(crate) version: &'a str,
    pub(crate) target: &'a str,
    /// Which of the override's features are on, sorted.
    pub(crate) features: &'a [String],
    /// The package's directory.
    pub(crate) source: &'a Path,
    /// The directory of a dependency of the package, by its name.
    pub(crate) dependency: &'a dyn Fn(&str) -> Option<PathBuf>,
}

impl Of<'_> {
    fn on(&self, feature: &str) -> bool {
        self.features.iter().any(|it| it == feature)
    }
}

pub(crate) struct Download {
    pub(crate) url: String,
    /// Below the unpacked download, what the variable names: empty for the directory itself.
    pub(crate) names: String,
}

/// The crates blaze hands a download, crate2nix's `defaultCrateOverrides` for build scripts.
/// Each cites the source of the crate it read, at the version given.
pub(crate) const OVERRIDES: &[Override] = &[
    // rusty_v8 (`v8` 152.2.0): with `RUSTY_V8_ARCHIVE` set it reads that file and downloads
    // nothing (build.rs:1559), gzipped or not (build.rs:1656). The address is
    // `DEFAULT_ARTIFACT_BASE` (build.rs:793) `/v{version}/` and `static_lib_name` of
    // `{features}_{profile}_{target}` (build.rs:1550, 784): the profile is `release` unless
    // `V8_FORCE_DEBUG` (build.rs:762), which a script in the sandbox is not given, and the
    // features `_ptrcomp` and `_sandbox` (build.rs:773). Its Rust binding is in the published
    // crate (`gen/`, build.rs:1735), so nothing else is downloaded.
    Override {
        package: "v8",
        give: Give::Fetch {
            var: "RUSTY_V8_ARCHIVE",
            features: &["v8_enable_pointer_compression", "v8_enable_sandbox"],
            download: v8,
            hand: Hand::File,
        },
    },
    // ort (`ort-sys` 2.0.0-rc.15): it downloads the prebuilt ONNX Runtime its
    // `build/download/dist.tsv` names for the target and its execution-provider features
    // (resolve.rs:18, 51) into `{ORT_CACHE_DIR}/dfbin/{target}/{sha256}`, unless that directory
    // is there (build/main.rs:120, src/internal/dirs.rs:133). `ORT_LIB_LOCATION` would take
    // its path for a build of one's own instead (build/main.rs:44), which links differently,
    // so the cache is what it is given, with the archive unpacked as it unpacks it
    // (extract.rs:70).
    Override {
        package: "ort-sys",
        give: Give::Fetch {
            var: "ORT_CACHE_DIR",
            features: ORT_FEATURES,
            download: ort,
            hand: Hand::Unpacked { archive: Archive::TarLzma2, into: "dfbin/{target}/{sha256}" },
        },
    },
    // webrtc-sys (livekit's, and zed's fork at 0.3.23): `webrtc-sys-build` downloads
    // `webrtc-{triple}.zip` of the release `WEBRTC_TAG` (src/lib.rs:30, 95, 193) unless
    // `LK_CUSTOM_WEBRTC` names a directory (src/lib.rs:79), which the build script reads
    // `include` and `lib` of (build.rs:98). The zip holds that directory as `{triple}`.
    Override {
        package: "webrtc-sys",
        give: Give::Fetch {
            var: "LK_CUSTOM_WEBRTC",
            features: &[],
            download: webrtc,
            hand: Hand::Unpacked { archive: Archive::Zip, into: "" },
        },
    },
    // helix-term 25.7.1 fetches every tree-sitter grammar with git and compiles them into
    // `runtime/grammars` of the checkout unless `HELIX_DISABLE_AUTO_GRAMMAR_BUILD` is set
    // (build.rs:4). Those are some two hundred repositories at revisions of
    // `languages.toml`, which the editor fetches and builds itself (`hx --grammar fetch`,
    // `hx --grammar build`); as nixpkgs' helix, the build leaves them out.
    Override {
        package: "helix-term",
        give: Give::Value { var: "HELIX_DISABLE_AUTO_GRAMMAR_BUILD", value: "1" },
    },
];

/// The features of ort-sys that pick its download (resolve.rs:55): each a part of the
/// feature set its `dist.tsv` names, but `cuda`, whose part is `cuda12` or `cuda13`.
const ORT_FEATURES: &[&str] = &[
    "training", "tensorrt", "openvino", "onednn", "directml", "nnapi", "coreml", "xnnpack", "rocm",
    "acl", "armnn", "tvm", "migraphx", "rknpu", "vitis", "cann", "qnn", "webgpu", "azure", "nvrtx",
    "vsinpu", "cuda",
];

fn v8(of: &Of<'_>) -> anyhow::Result<Option<Download>> {
    let mut features = String::new();
    // `v8_enable_sandbox` turns pointer compression on.
    if of.on("v8_enable_pointer_compression") || of.on("v8_enable_sandbox") {
        features.push_str("_ptrcomp");
    }
    if of.on("v8_enable_sandbox") {
        features.push_str("_sandbox");
    }
    let target = of.target;
    let file = match target.contains("-windows") {
        true => format!("rusty_v8{features}_release_{target}.lib.gz"),
        false => format!("librusty_v8{features}_release_{target}.a.gz"),
    };
    let base = "https://github.com/denoland/rusty_v8/releases/download";
    Ok(Some(Download { url: format!("{base}/v{}/{file}", of.version), names: String::new() }))
}

fn ort(of: &Of<'_>) -> anyhow::Result<Option<Download>> {
    let table = of.source.join("build/download/dist.tsv");
    let text = std::fs::read_to_string(&table).with_context(|| {
        format!(
            "reading {}: blaze's override of ort-sys reads the downloads it names, which ort-sys \
             has from 2.0.0-rc.11",
            table.display()
        )
    })?;
    if of.on("cuda") {
        anyhow::bail!(
            "ort-sys with `cuda` picks its download by the CUDA it finds on the machine \
             (`ORT_CUDA_VERSION`, `CUDA_HOME`, `nvcc`), which blaze's override does not yet"
        );
    }
    let wanted: BTreeSet<&str> = of.features.iter().map(String::as_str).collect();
    let mut candidates: Vec<(BTreeSet<&str>, &str)> = text
        .lines()
        .skip(1)
        .filter(|it| !it.is_empty() && !it.starts_with('#'))
        .filter_map(|line| {
            let columns: Vec<&str> = line.split('\t').collect();
            let [target, features, url, _hash] = columns[..] else { return None };
            let features = match features {
                "none" => BTreeSet::new(),
                them => them.split(',').collect(),
            };
            (target == of.target).then_some((features, url))
        })
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    // As `resolve_dist` picks: one of exactly the features, else the first of those with the
    // most of them, when that has them all.
    candidates.sort_by_key(|it| std::cmp::Reverse(it.0.intersection(&wanted).count()));
    let exact = candidates.iter().find(|it| it.0 == wanted);
    let (features, url) = exact.unwrap_or(&candidates[0]);
    if features.intersection(&wanted).count() != wanted.len() {
        anyhow::bail!(
            "ort-sys {} has no prebuilt ONNX Runtime for {} with the features {wanted:?}",
            of.version,
            of.target
        );
    }
    Ok(Some(Download { url: (*url).to_owned(), names: String::new() }))
}

fn webrtc(of: &Of<'_>) -> anyhow::Result<Option<Download>> {
    let build = (of.dependency)("webrtc-sys-build").context("it has no webrtc-sys-build")?;
    let lib = build.join("src/lib.rs");
    let text =
        std::fs::read_to_string(&lib).with_context(|| format!("reading {}", lib.display()))?;
    let tag = between(&text, "pub const WEBRTC_TAG: &str = \"", "\"")
        .with_context(|| format!("{} has no `WEBRTC_TAG`", lib.display()))?;
    let end = text.find("/{}/{}.zip\"");
    let start = end.and_then(|end| Some(text[..end].rfind('"')? + 1));
    let address = start.zip(end).map(|(start, end)| &text[start..end]).with_context(|| {
        format!("{} has no address of the form `\"https://.../{{}}/{{}}.zip\"`", lib.display())
    })?;
    let (arch, os) = (of.target.split('-').next().unwrap_or_default(), target_os(of.target));
    let os = match os {
        "windows" => "win",
        "macos" => "mac",
        "ios" if of.target.ends_with("-sim") => "ios-simulator",
        "ios" => "ios-device",
        other => other,
    };
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    let triple = format!("{os}-{arch}-release");
    Ok(Some(Download { url: format!("{address}/{tag}/webrtc-{triple}.zip"), names: triple }))
}

/// `target_os` of a target triple, as its `cfg` has it, for the triples the overrides know.
fn target_os(target: &str) -> &str {
    let parts: Vec<&str> = target.split('-').collect();
    match parts[..] {
        [_, "apple", "darwin", ..] => "macos",
        [_, "apple", "ios", ..] => "ios",
        [_, _, "windows", ..] => "windows",
        [_, _, "linux", ..] | [_, "linux", ..] => {
            if target.contains("android") {
                "android"
            } else {
                "linux"
            }
        }
        [_, _, os, ..] => os,
        _ => target,
    }
}

fn between<'a>(text: &'a str, before: &str, after: &str) -> Option<&'a str> {
    let at = text.find(before)? + before.len();
    Some(&text[at..at + text[at..].find(after)?])
}

/// One download of [`FILE`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub(crate) package: String,
    pub(crate) version: String,
    /// What `Cargo.lock` pins the package by: its checksum, or its source where it has none.
    pub(crate) locked: String,
    pub(crate) target: String,
    /// The package's features that pick the download that are on, sorted.
    #[serde(default)]
    pub(crate) features: Vec<String>,
    /// The variable the build script is given it by.
    pub(crate) var: String,
    pub(crate) url: String,
    pub(crate) sha256: String,
    pub(crate) size: u64,
    /// What the variable names below the unpacked download, when not the directory itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) names: String,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Lock {
    /// Sorted.
    pub(crate) fetches: Vec<Entry>,
    /// The `[[system]]` tables: what the system libraries of build scripts resolve to (a
    /// crate's nixpkgs attributes and their store paths, by target). Their fields are the
    /// system resolver's; `blaze lock --fetches` writes them back as it read them.
    pub(crate) system: Vec<toml::Table>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    version: u32,
    #[serde(default)]
    fetch: Vec<Entry>,
    #[serde(default)]
    system: Vec<toml::Table>,
}

impl Lock {
    pub(crate) fn read(text: &str) -> anyhow::Result<Lock> {
        let written: Written = toml::from_str(text)?;
        if written.version != VERSION {
            anyhow::bail!(
                "it is of version {}, and this blaze reads version {VERSION}: `{COMMAND}` writes \
                 it again",
                written.version
            );
        }
        let mut fetches = written.fetch;
        fetches.sort();
        Ok(Lock { fetches, system: written.system })
    }

    /// The text of [`FILE`]: each entry in order, each field in order.
    pub(crate) fn text(&self) -> String {
        let quoted = |it: &str| toml::Value::String(it.to_owned()).to_string();
        let mut text = format!(
            "# Written by `{COMMAND}`: what build scripts download, each by its sha256. A build\n\
             # hands each to its script through the crate's own variable, with no network.\n\
             version = {VERSION}\n"
        );
        let mut fetches = self.fetches.clone();
        fetches.sort();
        for it in &fetches {
            let features: Vec<String> = it.features.iter().map(|it| quoted(it)).collect();
            text += &format!(
                "\n[[fetch]]\npackage = {}\nversion = {}\nlocked = {}\ntarget = {}\nfeatures = [{}]\n\
                 var = {}\nurl = {}\nsha256 = {}\nsize = {}\n",
                quoted(&it.package),
                quoted(&it.version),
                quoted(&it.locked),
                quoted(&it.target),
                features.join(", "),
                quoted(&it.var),
                quoted(&it.url),
                quoted(&it.sha256),
                it.size,
            );
            if !it.names.is_empty() {
                text += &format!("names = {}\n", quoted(&it.names));
            }
        }
        if !self.system.is_empty() {
            #[derive(Serialize)]
            struct Systems<'a> {
                system: &'a [toml::Table],
            }
            let system = toml::to_string(&Systems { system: &self.system })
                .expect("a table read from TOML writes as TOML");
            text += &format!("\n{system}");
        }
        text
    }

    fn of_root(root: &Path) -> anyhow::Result<Option<Lock>> {
        let path = root.join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                Lock::read(&text).with_context(|| format!("reading {}", path.display())).map(Some)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

/// A package of `Cargo.lock`.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Locked {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source: Option<String>,
    pub(crate) checksum: Option<String>,
}

impl Locked {
    /// What it is pinned by: its checksum, its source where it has none (git), and `path`
    /// for a package of the checkout.
    pub(crate) fn pin(&self) -> String {
        self.checksum.clone().or_else(|| self.source.clone()).unwrap_or_else(|| "path".to_owned())
    }
}

/// The packages of the `Cargo.lock` at `path`.
pub(crate) fn cargo_lock(path: &Path) -> anyhow::Result<Vec<Locked>> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        package: Vec<Locked>,
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file: File =
        toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
    Ok(file.package)
}

/// The package of `locked` named `name` at `version` whose directory is `dir`: of several,
/// the one whose source `dir` is in (a git checkout's, named by the commit's start, or a
/// registry's).
fn locked_at<'a>(
    locked: &'a [Locked],
    name: &str,
    version: &str,
    dir: &Path,
) -> anyhow::Result<&'a Locked> {
    let them: Vec<&Locked> =
        locked.iter().filter(|it| it.name == name && it.version == version).collect();
    if let [one] = them[..] {
        return Ok(one);
    }
    let dir = dir.to_string_lossy();
    let fits = |it: &&&Locked| match it.source.as_deref() {
        Some(source) if source.starts_with("git+") => {
            let commit = source.rsplit('#').next().unwrap_or_default();
            dir.contains("/git/checkouts/") && commit.len() >= 7 && dir.contains(&commit[..7])
        }
        Some(_) => dir.contains("/registry/src/"),
        None => !dir.contains("/registry/src/") && !dir.contains("/git/checkouts/"),
    };
    match them.iter().filter(fits).collect::<Vec<_>>()[..] {
        [one] => Ok(one),
        [] => anyhow::bail!("Cargo.lock has no `{name}` {version} whose source is {dir}"),
        _ => anyhow::bail!("Cargo.lock has `{name}` {version} of several sources"),
    }
}

/// What a build script is given in place of downloading: its variables, and what it is to
/// read.
#[derive(Debug, Default)]
pub(crate) struct Handed {
    pub(crate) env: Vec<(String, String)>,
    pub(crate) readable: Vec<PathBuf>,
}

/// What the build script cargo runs with `env`, writing `out_dir`, of the workspace at
/// `root`, is given in place of what it would download, by [`OVERRIDES`]. A variable of an
/// override the workspace gives scripts itself (`build-script-env`, `given`) is left as it is.
/// `Err` names the package and what to run when [`FILE`] has no download of it for this build.
pub(crate) fn hand(
    env: &[(String, String)],
    out_dir: &Path,
    root: Option<&Path>,
    given: &BTreeSet<String>,
) -> anyhow::Result<Handed> {
    hand_with(OVERRIDES, env, out_dir, root, given, super::daemon::store().as_deref())
}

fn hand_with(
    overrides: &[Override],
    env: &[(String, String)],
    out_dir: &Path,
    root: Option<&Path>,
    given: &BTreeSet<String>,
    store: Option<&Path>,
) -> anyhow::Result<Handed> {
    let var = |name: &str| env.iter().find(|it| it.0 == name).map(|it| it.1.as_str());
    let package = var("CARGO_PKG_NAME").unwrap_or_default();
    let mut handed = Handed::default();
    for it in overrides.iter().filter(|it| it.package == package) {
        let (name, features, hand) = match &it.give {
            Give::Value { var, .. } | Give::Fetch { var, .. } if given.contains(*var) => continue,
            Give::Value { var, value } => {
                handed.env.push(((*var).to_owned(), (*value).to_owned()));
                continue;
            }
            Give::Fetch { var, features, hand, .. } => (*var, *features, *hand),
        };
        let version = var("CARGO_PKG_VERSION").unwrap_or_default();
        let target = var("TARGET").unwrap_or_default();
        let dir = Path::new(var("CARGO_MANIFEST_DIR").unwrap_or_default());
        let what = format!(
            "the build script of `{package}` {version} downloads what `{name}` names a local \
             copy of, which a build is not to download"
        );
        let found = root.map(Path::to_owned).or_else(|| locked_dir(out_dir, var("PWD")));
        let root = found
            .as_deref()
            .with_context(|| format!("{what}, and no Cargo.lock is above {}", out_dir.display()))?;
        let locked = cargo_lock(&root.join("Cargo.lock")).with_context(|| what.clone())?;
        let locked = locked_at(&locked, package, version, dir)?.pin();
        let Some(lock) = Lock::of_root(root)? else {
            anyhow::bail!("{what}, and {} has no {FILE}: `{COMMAND}` records it", root.display());
        };
        let on: Vec<String> = features
            .iter()
            .filter(|it| {
                var(&format!("CARGO_FEATURE_{}", it.to_uppercase().replace('-', "_"))).is_some()
            })
            .map(|it| (*it).to_owned())
            .collect();
        let of_package =
            |it: &&Entry| it.package == package && it.var == name && it.target == target;
        let entries: Vec<&Entry> = lock.fetches.iter().filter(of_package).collect();
        let entry = match entries.iter().find(|it| it.version == version && it.locked == locked) {
            None if entries.is_empty() => anyhow::bail!(
                "{what}, and {FILE} has none of it for {target}: `{COMMAND}` records it{}",
                if var("HOST") == Some(target) {
                    String::new()
                } else {
                    format!(" (`--target {target}`)")
                }
            ),
            None => anyhow::bail!(
                "{what}, and {FILE} has it for {}, not for `{package}` {version} as Cargo.lock \
                 locks it ({locked}): `{COMMAND}` records it again",
                entries
                    .iter()
                    .map(|it| format!("{} ({})", it.version, it.locked))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some(entry) => *entry,
        };
        if entry.features != on {
            anyhow::bail!(
                "{what}; {FILE} has it for the features {:?} of it, and this build has {on:?} on: \
                 `{COMMAND}` records it for the features the workspace's resolve has",
                entry.features
            );
        }
        let profile = profile_dir(out_dir)?;
        let written = profile.join(WRITTEN);
        let path = write_out(entry, hand, &written, store)
            .with_context(|| format!("handing `{package}` {version} {}", entry.url))?;
        let path = path.to_str().context("a target directory whose name is not text")?.to_owned();
        handed.env.push((name.to_owned(), path));
        handed.readable.push(written);
    }
    Ok(handed)
}

/// The directory of the `Cargo.lock` of a build of a package in no workspace (a lone
/// package's `Cargo.lock` is beside its manifest): the nearest above the target directory
/// `out_dir` is in, else above `pwd`.
fn locked_dir(out_dir: &Path, pwd: Option<&str>) -> Option<PathBuf> {
    let has = |it: &&Path| it.join("Cargo.lock").is_file() && it.join("Cargo.toml").is_file();
    let above_target =
        profile_dir(out_dir).ok().and_then(|it| it.ancestors().find(has).map(Path::to_owned));
    above_target.or_else(|| Path::new(pwd?).ancestors().find(has).map(Path::to_owned))
}

/// The profile's directory `out_dir` is below: what is above its `build` directory.
fn profile_dir(out_dir: &Path) -> anyhow::Result<PathBuf> {
    let text = out_dir.to_str().context("an OUT_DIR whose name is not text")?;
    let at = text.replace('\\', "/").rfind("/build/").filter(|it| *it > 0);
    let at = at.with_context(|| format!("the OUT_DIR {text} is below no `build` directory"))?;
    Ok(PathBuf::from(&text[..at]))
}

/// The key the store keeps a download of `sha256` under.
fn key(sha256: &str) -> anyhow::Result<[u8; 32]> {
    let bytes: Option<Vec<u8>> = (0..sha256.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(sha256.get(at..at + 2)?, 16).ok())
        .collect();
    let bytes =
        bytes.filter(|it| it.len() == 32).with_context(|| format!("`{sha256}` is no sha256"))?;
    let mut key = super::keyed::Key::new("a download by its sha256");
    key.field(&bytes);
    Ok(key.done())
}

fn sha256(bytes: &[u8]) -> String {
    crate::content::hex(&Sha256::digest(bytes))
}

fn open_store(store: Option<&Path>) -> anyhow::Result<ra_jit_store::Store> {
    let dir =
        store.context("blaze keeps downloads in its store, which `BLAZE_CACHE=off` turns off")?;
    ra_jit_store::Store::open_by_content(dir)
        .with_context(|| format!("opening blaze's store in {}", dir.display()))
}

/// Writes the download of `entry` out below `dir`, in a directory named by its sha256, as
/// `hand` says, unless it is there; the result is what the variable names.
fn write_out(
    entry: &Entry,
    hand: Hand,
    dir: &Path,
    store: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let at = dir.join(&entry.sha256);
    let named = match hand {
        Hand::File => at.join(file_name(&entry.url)),
        Hand::Unpacked { into, .. } => at.join(placed(into, entry)).join(&entry.names),
    };
    // What is there was moved there whole.
    if at.exists() {
        return Ok(named);
    }
    let store = open_store(store)?;
    let bytes = store.get(&key(&entry.sha256)?).with_context(|| {
        format!(
            "blaze's store does not have it (sha256 {}): `{COMMAND}` downloads it again",
            entry.sha256
        )
    })?;
    if sha256(&bytes) != entry.sha256 {
        anyhow::bail!("blaze's store has other bytes for sha256 {}", entry.sha256);
    }
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let staging = dir.join(format!(".{}.{}", entry.sha256, std::process::id()));
    _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let written = match hand {
        Hand::File => {
            std::fs::write(staging.join(file_name(&entry.url)), &bytes).map_err(Into::into)
        }
        Hand::Unpacked { archive, into } => {
            unpack(archive, &bytes, &staging.join(placed(into, entry)))
        }
    };
    if let Err(e) = written {
        _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    if std::fs::rename(&staging, &at).is_err() {
        _ = std::fs::remove_dir_all(&staging);
        anyhow::ensure!(at.exists(), "moving {} into place", at.display());
    }
    Ok(named)
}

fn placed(into: &str, entry: &Entry) -> String {
    into.replace("{target}", &entry.target).replace("{sha256}", &entry.sha256)
}

/// The last part of the path of `url`.
fn file_name(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path)
}

fn unpack(archive: Archive, bytes: &[u8], into: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(into)?;
    match archive {
        Archive::Zip => zip::ZipArchive::new(Cursor::new(bytes))?.extract(into)?,
        Archive::TarLzma2 => {
            let raw = lzma_rust2::Lzma2Reader::new(Cursor::new(bytes), 1 << 26, None);
            tar::Archive::new(raw).unpack(into)?;
        }
    }
    Ok(())
}

/// Downloads `url`, whole.
fn download(url: &str) -> anyhow::Result<Vec<u8>> {
    let (base, name) = url.rsplit_once('/').with_context(|| format!("{url} is no address"))?;
    let base = url::Url::parse(base).with_context(|| format!("{url} is no address"))?;
    let web = Https::open(&base, PATIENCE, Arc::new(Traffic::new(1)))?;
    let runtime = remote::runtime()?;
    runtime.block_on(async {
        let mut body =
            web.read_from(name, 0).await?.with_context(|| format!("{url} is not there"))?;
        let mut bytes = Vec::new();
        loop {
            let next = tokio::time::timeout(PATIENCE, remote::https_next(&mut body)).await;
            match next.map_err(|_| anyhow::anyhow!("no bytes from {url} for {PATIENCE:?}"))?? {
                Some(piece) => bytes.extend_from_slice(&piece),
                None => break,
            }
        }
        anyhow::Ok(bytes)
    })
}

/// A package of the workspace's resolve for a target, as [`lock`] reads it.
pub(crate) struct Resolved {
    pub(crate) locked: Locked,
    pub(crate) source: PathBuf,
    /// The features on.
    pub(crate) features: Vec<String>,
    /// The directory of each dependency, by its name.
    pub(crate) dependencies: BTreeMap<String, PathBuf>,
}

/// The entries of [`FILE`] for `resolved`, the packages of each target's resolve by target:
/// each download `overrides` names for one, from `old` where it has the same one whose bytes
/// the store has, else downloaded by `fetch` and kept in the store.
fn lock(
    overrides: &[Override],
    resolved: &BTreeMap<String, Vec<Resolved>>,
    old: &Lock,
    store: &ra_jit_store::Store,
    fetch: &dyn Fn(&str) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<Lock> {
    let mut fetches = Vec::new();
    let mut downloaded: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for (target, packages) in resolved {
        for package in packages {
            let name = &package.locked.name;
            for it in overrides.iter().filter(|it| it.package == *name) {
                let Give::Fetch { var, features, download, .. } = &it.give else { continue };
                let on: Vec<String> = features
                    .iter()
                    .filter(|it| package.features.iter().any(|on| on == *it))
                    .map(|it| (*it).to_owned())
                    .collect();
                let dependency = |name: &str| package.dependencies.get(name).cloned();
                let of = Of {
                    version: &package.locked.version,
                    target,
                    features: &on,
                    source: &package.source,
                    dependency: &dependency,
                };
                let what = || format!("the download of `{name}` {} for {target}", of.version);
                let Some(found) = download(&of).with_context(what)? else {
                    super::say::note(&format!(
                        "`{name}` {} downloads nothing for {target}",
                        of.version
                    ));
                    continue;
                };
                let mut entry = Entry {
                    package: name.clone(),
                    version: package.locked.version.clone(),
                    locked: package.locked.pin(),
                    target: target.clone(),
                    features: on.clone(),
                    var: (*var).to_owned(),
                    url: found.url,
                    sha256: String::new(),
                    size: 0,
                    names: found.names,
                };
                let before = old
                    .fetches
                    .iter()
                    .find(|it| Entry { sha256: String::new(), size: 0, ..(*it).clone() } == entry);
                let (sha256, size) = match (before, downloaded.get(&entry.url)) {
                    (_, Some(done)) => done.clone(),
                    (Some(before), None) if store.has(&key(&before.sha256)?) => {
                        (before.sha256.clone(), before.size)
                    }
                    (before, None) => {
                        super::say::status(false, "Downloading", &entry.url);
                        let bytes = fetch(&entry.url).with_context(what)?;
                        let sha256 = sha256(&bytes);
                        if let Some(before) = before.filter(|it| it.sha256 != sha256) {
                            anyhow::bail!(
                                "{} is not what {FILE} locked: its sha256 was {} and is {sha256}; \
                                 if the change is wanted, take its entry out of {FILE} and run \
                                 `{COMMAND}` again",
                                entry.url,
                                before.sha256
                            );
                        }
                        store.insert(&key(&sha256)?, &bytes);
                        store.seal();
                        anyhow::ensure!(
                            store.has(&key(&sha256)?),
                            "blaze's store did not keep {} ({} bytes)",
                            entry.url,
                            bytes.len()
                        );
                        (sha256, bytes.len() as u64)
                    }
                };
                downloaded.insert(entry.url.clone(), (sha256.clone(), size));
                (entry.sha256, entry.size) = (sha256, size);
                fetches.push(entry);
            }
        }
    }
    // A target not asked keeps what it had.
    fetches.extend(old.fetches.iter().filter(|it| !resolved.contains_key(&it.target)).cloned());
    fetches.sort();
    fetches.dedup();
    Ok(Lock { fetches, system: old.system.clone() })
}

/// `blaze lock --fetches [--target <triple>]... [--manifest-path <path>]`: writes [`FILE`]
/// for the workspace's `Cargo.lock`, downloading what it does not have. Returns the status to
/// exit with.
pub(crate) fn command(args: &[String]) -> anyhow::Result<i32> {
    let usage = "usage: blaze lock --fetches [--target <TRIPLE>]... [--manifest-path <PATH>]";
    let (mut fetches, mut targets, mut manifest_path) = (false, Vec::new(), None);
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let (flag, joined) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (word.as_str(), None),
        };
        let mut value = || joined.clone().or_else(|| words.next().cloned());
        match flag {
            "--fetches" => fetches = true,
            "--target" => targets.extend(value()),
            "--manifest-path" => manifest_path = value().map(PathBuf::from),
            "-h" | "--help" => {
                println!(
                    "{usage}\n\nWrites {FILE} beside Cargo.lock: each download a build script of the workspace needs, for each target (the host's when none is given), by its sha256."
                );
                return Ok(0);
            }
            _ => {
                super::say::error(&format!("unexpected argument `{word}`\n\n{usage}"));
                return Ok(super::FAILED);
            }
        }
    }
    if !fetches {
        super::say::error(&format!(
            "`blaze lock` writes only {FILE} yet, the downloads of build scripts: `{COMMAND}`"
        ));
        return Ok(super::FAILED);
    }
    let cwd = std::env::current_dir()?;
    let start =
        manifest_path.as_deref().and_then(Path::parent).map_or(cwd.clone(), |it| cwd.join(it));
    let lone = || start.ancestors().find(|it| it.join("Cargo.toml").is_file()).map(Path::to_owned);
    let root = super::sandboxed::workspace_root(&start)?
        .or_else(lone)
        .with_context(|| format!("{} is in no package", start.display()))?;
    let locked = cargo_lock(&root.join("Cargo.lock"))?;
    let path = root.join(FILE);
    let old = Lock::of_root(&root)?.unwrap_or_default();
    let lock = if !locked.iter().any(|it| {
        OVERRIDES.iter().any(|o| o.package == it.name && matches!(o.give, Give::Fetch { .. }))
    }) {
        super::say::status(
            false,
            "Locked",
            "no build script of the workspace downloads what blaze hands it",
        );
        if old.system.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(0),
            };
        }
        Lock { fetches: Vec::new(), system: old.system }
    } else {
        if targets.is_empty() {
            targets.push(super::own::manifest::host_triple()?);
        }
        let mut resolved = BTreeMap::new();
        for target in &targets {
            resolved
                .insert(target.clone(), resolve(&cwd, manifest_path.as_deref(), target, &locked)?);
        }
        let store = open_store(super::daemon::store().as_deref())?;
        lock(OVERRIDES, &resolved, &old, &store, &download)?
    };
    let writing = path.with_extension("lock.writing");
    std::fs::write(&writing, lock.text())
        .and_then(|()| std::fs::rename(&writing, &path))
        .with_context(|| format!("writing {}", path.display()))?;
    super::say::status(
        false,
        "Locked",
        &format!("{} downloads in {}", lock.fetches.len(), path.display()),
    );
    Ok(0)
}

/// The packages of `locked` that [`OVERRIDES`] downloads for, as the workspace resolves them
/// for `target` with blaze's own `cargo metadata`, which fetches their sources.
fn resolve(
    cwd: &Path,
    manifest_path: Option<&Path>,
    target: &str,
    locked: &[Locked],
) -> anyhow::Result<Vec<Resolved>> {
    let cfg = super::own::manifest::target_cfg(target)?;
    let asked = blaze_metadata::Asked {
        cwd,
        env: std::env::vars().collect(),
        manifest_path,
        config: &[],
        features: &[],
        all_features: false,
        no_default_features: false,
        build: None,
        platform: Some(blaze_metadata::TargetPlatform { triple: target, cfg: &cfg }),
        offline: false,
        store: None,
        rustc_version: Some(crate::cargo::toolchain::ours_release()),
    };
    let json = blaze_metadata::metadata(&asked)?;
    let metadata: cargo_metadata::Metadata = serde_json::from_str(&json)?;
    let resolve = metadata.resolve.context("`cargo metadata` resolved nothing")?;
    let package_of =
        |id: &cargo_metadata::PackageId| metadata.packages.iter().find(|it| it.id == *id);
    let dir_of = |it: &cargo_metadata::Package| {
        it.manifest_path.parent().map(|it| it.as_std_path().to_owned())
    };
    let mut found = Vec::new();
    for node in &resolve.nodes {
        let Some(package) = package_of(&node.id) else { continue };
        if !OVERRIDES.iter().any(|it| it.package == package.name.as_str()) {
            continue;
        }
        let version = package.version.to_string();
        let source = package.source.as_ref().map(|it| it.repr.clone());
        let Some(locked) = locked.iter().find(|it| {
            it.name == package.name.as_str() && it.version == version && it.source == source
        }) else {
            continue;
        };
        let dependencies = node
            .deps
            .iter()
            .filter_map(|dep| {
                Some((package_of(&dep.pkg)?.name.to_string(), dir_of(package_of(&dep.pkg)?)?))
            })
            .collect();
        found.push(Resolved {
            locked: locked.clone(),
            source: dir_of(package).context("a package with no directory")?,
            features: node.features.iter().map(|it| it.to_string()).collect(),
            dependencies,
        });
    }
    Ok(found)
}

/// What a build script that failed having been refused the network (`lines`, its record)
/// is told of downloads: that blaze has no override for `package`, which addresses it tried
/// and what an override would be; or, for a package [`OVERRIDES`] has, that it still tried.
pub(crate) fn no_override_note(
    package: &str,
    lines: &[Line],
    stdout: &[u8],
    stderr: &[u8],
) -> Option<String> {
    let mut tried: Vec<String> =
        lines.iter().filter(|it| it.reached == Reached::Net).map(|it| it.what.clone()).collect();
    if tried.is_empty() {
        return None;
    }
    tried.sort();
    tried.dedup();
    let printed = [String::from_utf8_lossy(stdout), String::from_utf8_lossy(stderr)].concat();
    let mut hosts: Vec<&str> = printed
        .match_indices("://")
        .filter_map(|(at, _)| {
            let scheme = printed[..at].rsplit(|c: char| !c.is_ascii_alphabetic()).next()?;
            matches!(scheme, "http" | "https").then_some(())?;
            let rest = &printed[at + 3..];
            let end = rest
                .find(|c: char| c == '/' || c == ':' || c.is_whitespace() || "\"'`)>".contains(c));
            Some(&rest[..end.unwrap_or(rest.len())]).filter(|it| !it.is_empty())
        })
        .collect();
    hosts.sort();
    hosts.dedup();
    let tried =
        [tried.join(", "), hosts.iter().map(|it| format!("`{it}`")).collect::<Vec<_>>().join(", ")]
            .into_iter()
            .filter(|it| !it.is_empty())
            .collect::<Vec<_>>()
            .join("; it names ");
    if OVERRIDES.iter().any(|it| it.package == package) {
        return Some(format!(
            "the build script of `{package}` tried the network ({tried}) though blaze's override \
             of it (crates/ra-jit/src/cargo/fetches.rs) gave it what it downloads: the override \
             is not what this version of it reads"
        ));
    }
    let mut vars: Vec<String> = String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("cargo:")?;
            line.strip_prefix(':').unwrap_or(line).strip_prefix("rerun-if-env-changed=")
        })
        .map(str::to_owned)
        .collect();
    vars.sort();
    vars.dedup();
    let var = match &vars[..] {
        [] => "<VAR>".to_owned(),
        [one] => one.to_string(),
        _ => format!("<one of {}>", vars.join(", ")),
    };
    Some(format!(
        "the build script of `{package}` tried the network ({tried}), which a build never has, and \
         blaze has no override that hands `{package}` a locked download. One goes in `OVERRIDES` \
         of crates/ra-jit/src/cargo/fetches.rs: `Override {{ package: \"{package}\", give: \
         Give::Fetch {{ var: \"{var}\", features: &[], download: <its address for a version and \
         target>, hand: Hand::File }} }}`, `{var}` being the variable its build script reads a \
         local copy from; if it reads none, it needs a patch to read a local file"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(package: &str, target: &str) -> Entry {
        Entry {
            package: package.to_owned(),
            version: "1.0.0".to_owned(),
            locked: "ab".repeat(32),
            target: target.to_owned(),
            features: Vec::new(),
            var: "FAKE_ARCHIVE".to_owned(),
            url: format!("https://example.com/{package}-{target}.bin"),
            sha256: "cd".repeat(32),
            size: 3,
            names: String::new(),
        }
    }

    #[test]
    fn the_lock_file_reads_back_and_is_in_one_order() {
        let mut quoted = entry("z\"q", "x86_64-unknown-linux-gnu");
        quoted.features = vec!["a".to_owned(), "b".to_owned()];
        quoted.names = "linux-x64-release".to_owned();
        let them = [
            quoted,
            entry("b", "aarch64-apple-darwin"),
            entry("b", "aarch64-unknown-linux-gnu"),
            entry("a", "x"),
        ];
        let text = Lock { fetches: them.to_vec(), system: Vec::new() }.text();
        let reversed =
            Lock { fetches: them.iter().rev().cloned().collect(), system: Vec::new() }.text();
        assert_eq!(text, reversed, "the same text whatever the order it was made in");
        let read = Lock::read(&text).unwrap();
        assert_eq!(read.text(), text);
        // The system section is the system resolver's: read and written back as it is.
        let system = "\n[[system]]\npackage = \"openssl-sys\"\nnixpkgs = [\"openssl.dev\"]\n";
        let with_system = Lock::read(&format!("{text}{system}")).unwrap();
        assert_eq!(with_system.system.len(), 1);
        assert_eq!(Lock::read(&with_system.text()).unwrap(), with_system);
        let order: Vec<(&str, &str)> =
            read.fetches.iter().map(|it| (&*it.package, &*it.target)).collect();
        assert_eq!(
            order,
            [
                ("a", "x"),
                ("b", "aarch64-apple-darwin"),
                ("b", "aarch64-unknown-linux-gnu"),
                ("z\"q", "x86_64-unknown-linux-gnu")
            ]
        );
        assert!(Lock::read(&text.replace("version = 1", "version = 2")).is_err());
        assert!(Lock::read(&format!("{text}unknown = 1\n")).is_err());
    }

    #[test]
    fn each_override_names_the_download_of_its_crate() {
        let none = |_: &str| None;
        let of = |version, target, features: &[String]| -> String {
            let of =
                Of { version, target, features, source: Path::new("/nowhere"), dependency: &none };
            v8(&of).unwrap().unwrap().url
        };
        assert_eq!(
            of("152.2.0", "x86_64-unknown-linux-gnu", &[]),
            "https://github.com/denoland/rusty_v8/releases/download/v152.2.0/librusty_v8_release_x86_64-unknown-linux-gnu.a.gz"
        );
        assert!(
            of("152.2.0", "x86_64-pc-windows-msvc", &["v8_enable_sandbox".to_owned()]).ends_with(
                "/v152.2.0/rusty_v8_ptrcomp_sandbox_release_x86_64-pc-windows-msvc.lib.gz"
            )
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("build/src")).unwrap();
        std::fs::write(
            dir.path().join("build/src/lib.rs"),
            "pub const WEBRTC_TAG: &str = \"webrtc-0001d84-4\";\n\
             pub fn download_url() -> String {\n    format!(\n        \"https://github.com/zed-industries/livekit-rust-sdks/releases/download/{}/{}.zip\",\n",
        )
        .unwrap();
        let build = dir.path().join("build");
        let dependency = |name: &str| (name == "webrtc-sys-build").then(|| build.clone());
        let of = Of {
            version: "0.3.23",
            target: "aarch64-unknown-linux-gnu",
            features: &[],
            source: dir.path(),
            dependency: &dependency,
        };
        let found = webrtc(&of).unwrap().unwrap();
        assert_eq!(
            found.url,
            "https://github.com/zed-industries/livekit-rust-sdks/releases/download/webrtc-0001d84-4/webrtc-linux-arm64-release.zip"
        );
        assert_eq!(found.names, "linux-arm64-release");
        let of = Of { target: "aarch64-apple-darwin", ..of };
        assert!(webrtc(&of).unwrap().unwrap().url.ends_with("/webrtc-mac-arm64-release.zip"));
        std::fs::create_dir_all(dir.path().join("build/download")).unwrap();
        std::fs::write(
            dir.path().join("build/download/dist.tsv"),
            "target\tfeature_set\turl\tsha256_hash\n\
             aarch64-apple-darwin\tcoreml\thttps://cdn/x+coreml\t00\n\
             aarch64-apple-darwin\tcoreml,webgpu\thttps://cdn/x+coreml,webgpu\t00\n\
             aarch64-unknown-linux-gnu\tnone\thttps://cdn/a\t00\n",
        )
        .unwrap();
        let ort_of = |target, features: &[String]| {
            let of = Of {
                version: "2.0.0-rc.15",
                target,
                features,
                source: dir.path(),
                dependency: &none,
            };
            ort(&of).map(|it| it.map(|it| it.url))
        };
        assert_eq!(
            ort_of("aarch64-unknown-linux-gnu", &[]).unwrap().as_deref(),
            Some("https://cdn/a")
        );
        assert_eq!(
            ort_of("aarch64-apple-darwin", &["webgpu".to_owned()]).unwrap().as_deref(),
            Some("https://cdn/x+coreml,webgpu")
        );
        assert_eq!(
            ort_of("aarch64-apple-darwin", &[]).unwrap().as_deref(),
            Some("https://cdn/x+coreml")
        );
        assert!(ort_of("aarch64-unknown-linux-gnu", &["webgpu".to_owned()]).is_err());
        assert_eq!(ort_of("riscv64gc-unknown-linux-gnu", &[]).unwrap(), None);
    }

    /// Serves `bytes` at every address to each connection on 127.0.0.1, for as long as the
    /// test runs; the result is its address.
    fn serve(bytes: Vec<u8>) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut asked = [0; 4096];
                _ = stream.read(&mut asked);
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    bytes.len()
                );
                _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&bytes));
            }
        });
        address
    }

    static FAKE_URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();

    fn fake(of: &Of<'_>) -> anyhow::Result<Option<Download>> {
        let url = FAKE_URL.get().unwrap();
        Ok(Some(Download {
            url: format!("{url}/files/fake-{}-{}.bin", of.version, of.target),
            names: String::new(),
        }))
    }

    const FAKE: &[Override] = &[
        Override {
            package: "fake-fetch",
            give: Give::Fetch {
                var: "FAKE_ARCHIVE",
                features: &[],
                download: fake,
                hand: Hand::File,
            },
        },
        Override {
            package: "fake-fetch",
            give: Give::Value { var: "FAKE_NO_DOWNLOAD", value: "1" },
        },
    ];

    #[test]
    fn a_locked_download_is_handed_to_the_build_script_and_a_missing_or_stale_one_is_named() {
        let bytes = b"the prebuilt library".to_vec();
        _ = FAKE_URL.set(serve(bytes.clone()));
        let dir = tempfile::tempdir().unwrap();
        let (root, store_dir) = (dir.path().join("ws"), dir.path().join("store"));
        std::fs::create_dir_all(&root).unwrap();
        let checksum = "ab".repeat(32);
        let cargo_lock = |checksum: &str| {
            let text = format!(
                "version = 4\n\n[[package]]\nname = \"fake-fetch\"\nversion = \"1.0.0\"\n\
                 source = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{checksum}\"\n"
            );
            std::fs::write(root.join("Cargo.lock"), text).unwrap();
        };
        cargo_lock(&checksum);
        let target = "x86_64-unknown-linux-gnu";
        let source = dir.path().join("registry/src/index/fake-fetch-1.0.0");
        let locked = cargo_lock_of(&root);
        let resolved = BTreeMap::from([(
            target.to_owned(),
            vec![Resolved {
                locked: locked[0].clone(),
                source: source.clone(),
                features: Vec::new(),
                dependencies: BTreeMap::new(),
            }],
        )]);
        let store = ra_jit_store::Store::open_by_content(&store_dir).unwrap();
        let lock = super::lock(FAKE, &resolved, &Lock::default(), &store, &download).unwrap();
        drop(store);
        assert_eq!(lock.fetches.len(), 1);
        let it = &lock.fetches[0];
        assert_eq!((&*it.sha256, it.size), (&*sha256(&bytes), bytes.len() as u64));
        assert_eq!((&*it.locked, &*it.var, &*it.target), (&*checksum, "FAKE_ARCHIVE", target));
        // Locked again, nothing is downloaded: the store has it.
        let store = ra_jit_store::Store::open_by_content(&store_dir).unwrap();
        let refuse = |url: &str| -> anyhow::Result<Vec<u8>> { panic!("downloaded {url} again") };
        assert_eq!(super::lock(FAKE, &resolved, &lock, &store, &refuse).unwrap(), lock);
        drop(store);

        let out_dir = root.join("target/debug/build/fake-fetch-0123/out");
        let env: Vec<(String, String)> = [
            ("CARGO_PKG_NAME", "fake-fetch"),
            ("CARGO_PKG_VERSION", "1.0.0"),
            ("TARGET", target),
            ("CARGO_MANIFEST_DIR", source.to_str().unwrap()),
            ("OUT_DIR", out_dir.to_str().unwrap()),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .to_vec();
        let given = BTreeSet::new();
        let hand = |given: &BTreeSet<String>| {
            hand_with(FAKE, &env, &out_dir, Some(&root), given, Some(&store_dir))
        };
        let said = hand(&given).unwrap_err().to_string();
        assert!(
            said.contains("`fake-fetch` 1.0.0") && said.contains(COMMAND) && said.contains(FILE),
            "{said}"
        );
        std::fs::write(root.join(FILE), lock.text()).unwrap();
        let handed = hand(&given).unwrap();
        let profile = root.join("target/debug");
        let file = profile.join(WRITTEN).join(&it.sha256).join(format!("fake-1.0.0-{target}.bin"));
        assert_eq!(
            handed.env,
            [
                ("FAKE_ARCHIVE".to_owned(), file.to_str().unwrap().to_owned()),
                ("FAKE_NO_DOWNLOAD".to_owned(), "1".to_owned())
            ]
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert_eq!(handed.readable, [profile.join(WRITTEN)]);
        // A lone package, in no workspace, has its Cargo.lock beside its manifest.
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"ws\"\n").unwrap();
        let lone = hand_with(FAKE, &env, &out_dir, None, &given, Some(&store_dir)).unwrap();
        assert_eq!(lone.env, handed.env);
        // Written out once, it is not read from the store again.
        std::fs::remove_dir_all(&store_dir).unwrap();
        assert_eq!(hand(&given).unwrap().env, handed.env);
        // What the workspace gives itself is left as it gives it.
        let workspace = BTreeSet::from(["FAKE_ARCHIVE".to_owned(), "FAKE_NO_DOWNLOAD".to_owned()]);
        assert!(hand(&workspace).unwrap().env.is_empty());

        cargo_lock(&"ef".repeat(32));
        let said = format!("{:#}", hand(&given).unwrap_err());
        assert!(
            said.contains("`fake-fetch` 1.0.0")
                && said.contains(&"ef".repeat(32))
                && said.contains(COMMAND),
            "{said}"
        );
        cargo_lock(&checksum);
        std::fs::write(root.join(FILE), Lock::default().text()).unwrap();
        let said = hand(&given).unwrap_err().to_string();
        assert!(said.contains("has none of it for x86_64-unknown-linux-gnu"), "{said}");
    }

    fn cargo_lock_of(root: &Path) -> Vec<Locked> {
        cargo_lock(&root.join("Cargo.lock")).unwrap()
    }

    #[test]
    fn a_download_unpacked_is_named_below_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut zipped = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut zipped);
        zip.start_file(
            "linux-x64-release/lib/libwebrtc.a",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"!<arch>\n").unwrap();
        zip.finish().unwrap();
        let bytes = zipped.into_inner();
        let store_dir = dir.path().join("store");
        let store = ra_jit_store::Store::open_by_content(&store_dir).unwrap();
        let sha = sha256(&bytes);
        store.insert(&key(&sha).unwrap(), &bytes);
        store.seal();
        drop(store);
        let mut it = entry("webrtc-sys", "x86_64-unknown-linux-gnu");
        (it.sha256, it.names) = (sha.clone(), "linux-x64-release".to_owned());
        let hand = Hand::Unpacked { archive: Archive::Zip, into: "dfbin/{target}/{sha256}" };
        let named = write_out(&it, hand, &dir.path().join("w"), Some(&store_dir)).unwrap();
        let below =
            dir.path().join("w").join(&sha).join("dfbin/x86_64-unknown-linux-gnu").join(&sha);
        assert_eq!(named, below.join("linux-x64-release"));
        assert_eq!(std::fs::read(named.join("lib/libwebrtc.a")).unwrap(), b"!<arch>\n");
    }

    #[test]
    fn a_script_refused_the_network_is_told_what_override_it_lacks() {
        let line =
            |reached, what: &str| Line { reached, test: String::new(), what: what.to_owned() };
        let lines = [line(Reached::Net, "140.82.112.4:443"), line(Reached::Read, "/x")];
        let stdout = b"cargo:rerun-if-env-changed=SOME_SYS_ARCHIVE\n";
        let stderr =
            b"Error: failed to GET https://github.com/someone/some-sys/releases/a.zip: refused\n";
        let note = no_override_note("some-sys", &lines, stdout, stderr).unwrap();
        assert!(
            note.contains("`some-sys`")
                && note.contains("140.82.112.4:443")
                && note.contains("`github.com`"),
            "{note}"
        );
        assert!(
            note.contains("var: \"SOME_SYS_ARCHIVE\"")
                && note.contains("a patch to read a local file"),
            "{note}"
        );
        assert!(no_override_note("some-sys", &lines[1..], stdout, stderr).is_none());
        assert!(no_override_note("v8", &lines, b"", b"").unwrap().contains("blaze's override"));
    }

    #[test]
    fn of_several_versions_in_cargo_lock_the_one_of_the_directory_is_taken() {
        let locked = |source: &str| Locked {
            name: "webrtc-sys".to_owned(),
            version: "0.3.23".to_owned(),
            source: Some(source.to_owned()),
            checksum: None,
        };
        let them = [
            locked(
                "git+https://github.com/zed-industries/livekit-rust-sdks?rev=0a1c#0a1c519cfce9b365229026b55de9b9dbdb6fed3c",
            ),
            locked("registry+https://github.com/rust-lang/crates.io-index"),
        ];
        let git = Path::new("/h/.cargo/git/checkouts/livekit-rust-sdks-d281/0a1c519/webrtc-sys");
        assert!(locked_at(&them, "webrtc-sys", "0.3.23", git).unwrap().pin().starts_with("git+"));
        let registry = Path::new("/h/.cargo/registry/src/index/webrtc-sys-0.3.23");
        assert!(
            locked_at(&them, "webrtc-sys", "0.3.23", registry)
                .unwrap()
                .pin()
                .starts_with("registry+")
        );
        assert!(locked_at(&them, "webrtc-sys", "9.9.9", registry).is_err());
    }
}
