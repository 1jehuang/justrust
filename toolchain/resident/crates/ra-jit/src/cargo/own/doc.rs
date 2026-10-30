//! `cargo blaze doc`: the HTML documentation of the packages asked for and of what they depend
//! on, written by blaze from rust-analyzer's HIR (`crates/blaze-doc`) where cargo doc writes
//! rustdoc's: `target/doc/<crate>/index.html`. No rustdoc runs. The workspace is loaded once in
//! this process, as the daemon's checker loads it, and the pages are written in parallel: a
//! one-shot load needs no daemon, and a daemon would hold a database only `doc` reads.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Context as _;
use blaze_doc::{Asked, Documented};
use cargo_metadata::{Metadata, Package, TargetKind};
use hir::Crate;
use ide_db::RootDatabase;
use rustc_hash::{FxHashMap, FxHashSet};

use super::args::{Args, Flag, switch, valued};
use crate::{
    cargo::{metadata, say, state::package_dir},
    plan::Selection,
};

/// The flags of `cargo doc`, besides those of every cargo command.
const FLAGS: &[Flag] = &[
    valued(&["-p", "--package"]),
    switch(&["--workspace", "--all"]),
    valued(&["--exclude"]),
    switch(&["--no-deps"]),
    switch(&["--document-private-items"]),
    switch(&["--open"]),
    valued(&["-F", "--features"]),
    switch(&["--all-features"]),
    switch(&["--no-default-features"]),
    valued(&["--target"]),
    valued(&["--target-dir"]),
    valued(&["--manifest-path"]),
    switch(&["--lib"]),
    switch(&["--bins"]),
    valued(&["--bin"]),
    switch(&["-r", "--release"]),
    valued(&["--profile"]),
    valued(&["--message-format"]),
    valued(&["--lockfile-path"]),
    // What changes nothing blaze writes.
    valued(&["-j", "--jobs"]),
    switch(&["--keep-going"]),
    switch(&["--ignore-rust-version"]),
];

const HELP: &str = "\
Write the documentation of the packages and of what they depend on, from blaze's own reading
of the source: no rustdoc runs.

Usage: cargo blaze doc [OPTIONS]

Options:
      --open                    Open the documentation in a browser after writing it
      --no-deps                 Leave out the documentation of dependencies
      --document-private-items  Document private items too
  -p, --package <SPEC>          Package to document
      --workspace               Document every package of the workspace
      --exclude <SPEC>          Package to leave out with --workspace
      --lib                     Document only the library
      --bins                    Document every binary
      --bin <NAME>              Document only the binary named
  -F, --features <FEATURES>     Features to turn on, separated by commas or spaces
      --all-features            Turn on every feature
      --no-default-features     Leave the default features off
      --target <TRIPLE>         Document for the target triple
      --target-dir <DIR>        Directory to write in, `target` by default
      --manifest-path <PATH>    Path to Cargo.toml
  -r, --release                 Read the source as the release profile does
      --profile <NAME>          Read the source as the profile named does
  -q, --quiet                   Say nothing but errors and warnings
  -v, --verbose                 Say how long each step took";

#[derive(Default, Debug)]
struct Flags {
    packages: Vec<String>,
    workspace: bool,
    excludes: Vec<String>,
    no_deps: bool,
    private: bool,
    open: bool,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    target: Option<String>,
    target_dir: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    lib: bool,
    bins: bool,
    bin: Vec<String>,
    profile: Option<String>,
    config: Vec<String>,
    unstable: Vec<String>,
    quiet: bool,
    verbose: bool,
    help: bool,
}

impl Flags {
    fn parse(typed: &[String]) -> Result<Flags, String> {
        let read = Args::new(typed).read(FLAGS)?;
        if let Some(word) = read.positional.first() {
            return Err(format!("unexpected argument '{word}' found"));
        }
        for refused in ["--message-format", "--lockfile-path"] {
            if read.has(refused) {
                return Err(format!("`cargo blaze doc` does not take `{refused}` yet"));
            }
        }
        Ok(Flags {
            packages: read.values("--package"),
            workspace: read.has("--workspace"),
            excludes: read.values("--exclude"),
            no_deps: read.has("--no-deps"),
            private: read.has("--document-private-items"),
            open: read.has("--open"),
            features: read.features(),
            all_features: read.has("--all-features"),
            no_default_features: read.has("--no-default-features"),
            target: read.value("--target"),
            target_dir: read.value("--target-dir").map(PathBuf::from),
            manifest_path: read.value("--manifest-path").map(PathBuf::from),
            lib: read.has("--lib"),
            bins: read.has("--bins"),
            bin: read.values("--bin"),
            profile: if read.has("--release") {
                Some("release".to_owned())
            } else {
                read.value("--profile")
            },
            config: read.values("--config"),
            unstable: read.values("-Z"),
            quiet: read.has("--quiet"),
            verbose: read.has("--verbose"),
            help: read.has("--help"),
        })
    }

    fn selection(&self, asked_in: Option<PathBuf>) -> Selection {
        Selection {
            profile: self.profile.clone().unwrap_or_else(|| "dev".to_owned()),
            features: self.features.clone(),
            no_default_features: self.no_default_features,
            all_features: self.all_features,
            packages: self.packages.clone(),
            workspace: self.workspace,
            excludes: self.excludes.clone(),
            unstable: self.unstable.clone(),
            target: self
                .target
                .clone()
                .or_else(|| std::env::var("CARGO_BUILD_TARGET").ok().filter(|it| !it.is_empty())),
            dev_deps: false,
            config: self.config.clone(),
            asked_in,
        }
    }
}

/// `cargo blaze doc [flags]`: the status cargo doc would end with.
pub(crate) fn command(args: &[String]) -> anyhow::Result<i32> {
    let started = Instant::now();
    let flags = match Flags::parse(args) {
        Ok(flags) => flags,
        Err(why) => {
            say::error(&why);
            return Ok(1);
        }
    };
    if flags.help {
        println!("{HELP}");
        return Ok(0);
    }
    let cwd = std::env::current_dir()?;
    let metadata = metadata::no_deps(flags.manifest_path.as_deref(), &flags.config);
    let packages = match selected(&flags, &metadata, &cwd) {
        Ok(packages) => packages,
        Err(why) => {
            say::error(&why);
            return Ok(101);
        }
    };
    let root = metadata.workspace_root.as_std_path().to_owned();
    let project = match &packages[..] {
        [one] => package_dir(one),
        _ => root.clone(),
    };
    let asked_in = flags.manifest_path.as_ref().map(|it| cwd.join(it));
    let selection = flags.selection(asked_in);
    let mut args = vec![project.display().to_string(), "--own-target".to_owned()];
    args.extend(selection.flags());
    if let Some(asked_in) = &selection.asked_in {
        args.extend(["--asked-in".to_owned(), asked_in.display().to_string()]);
    }
    args.extend(["--crate", "doc", "--out", "none"].map(str::to_owned));
    let args = crate::Args::parse(args.into_iter())?;
    crate::process_wide();
    let cargo_config = crate::cargo_config(&args);
    let (mut db, vfs, _checkouts) = crate::load(
        &[project.as_path()],
        &cargo_config,
        crate::load_config(),
        selection.clone(),
        None,
        None,
    )?;
    let loaded = started.elapsed();
    let asked = match asked_crates(&db, &vfs, &packages, &flags) {
        Ok(asked) => asked,
        Err(why) => {
            say::error(&why);
            return Ok(101);
        }
    };
    let test_only = test_only(&db, &vfs, &metadata);
    let crates = hir::attach_db(&db, || {
        blaze_doc::crates_to_document(&db, &asked, flags.no_deps, &test_only)
    });
    blaze_doc::set_cfg_doc(&mut db, &crates);
    for documented in &crates {
        say::status(flags.quiet, "Documenting", &describe(&db, &vfs, documented));
    }
    let first = crates.first().map(|it| it.name.clone()).context("nothing to document")?;
    let asked_names: Vec<String> =
        crates.iter().filter(|it| it.warn).map(|it| it.name.clone()).collect();
    let model = hir::attach_db(&db, || blaze_doc::model(&db, crates, test_only));
    let modelled = started.elapsed();
    let target_dir = match &flags.target_dir {
        Some(dir) => cwd.join(dir),
        None => metadata.target_directory.as_std_path().to_owned(),
    };
    let out = match &selection.target {
        Some(triple) => target_dir.join(triple).join("doc"),
        None => target_dir.join("doc"),
    };
    let written = blaze_doc::html::write(&model, &out)
        .with_context(|| format!("writing the documentation in {}", out.display()))?;
    warn(&model, &cwd);
    if flags.verbose {
        say::note(&format!(
            "loaded in {:.2}s, modelled in {:.2}s, wrote {} pages and {} source pages in {:.2}s",
            loaded.as_secs_f64(),
            (modelled - loaded).as_secs_f64(),
            written.pages,
            written.source_pages,
            (started.elapsed() - modelled).as_secs_f64(),
        ));
    }
    let profile = selection.profile.as_str();
    let shape = if profile == "release" { "optimized" } else { "unoptimized + debuginfo" };
    let took = started.elapsed().as_secs_f64();
    say::status(
        flags.quiet,
        "Finished",
        &format!("`{profile}` profile [{shape}] target(s) in {took:.2}s"),
    );
    let page = out.join(&first).join("index.html");
    let shown = page.strip_prefix(&cwd).unwrap_or(&page).display().to_string();
    match asked_names.len() {
        0 | 1 => say::status(flags.quiet, "Generated", &shown),
        n => {
            let others = n - 1;
            let files = if others == 1 { "file" } else { "files" };
            say::status(flags.quiet, "Generated", &format!("{shown} and {others} other {files}"));
        }
    }
    if flags.open {
        say::status(flags.quiet, "Opening", &shown);
        open(&page);
    }
    Ok(0)
}

/// The packages `cargo doc` documents with `flags`: those `-p` names, every member with
/// `--workspace`, else the one of the directory, or the default members at a virtual root.
fn selected<'a>(
    flags: &Flags,
    metadata: &'a Metadata,
    cwd: &Path,
) -> Result<Vec<&'a Package>, String> {
    let members = metadata.workspace_packages();
    if !flags.packages.is_empty() {
        let mut out = vec![];
        for named in &flags.packages {
            let name = named.split('@').next().unwrap_or(named);
            let package = members
                .iter()
                .find(|it| it.name.as_str() == name)
                .ok_or(format!("package ID specification `{named}` did not match any packages"))?;
            out.push(*package);
        }
        return Ok(out);
    }
    if flags.workspace {
        let excluded = |it: &&&Package| flags.excludes.iter().any(|name| name == it.name.as_str());
        return Ok(members.iter().filter(|it| !excluded(it)).copied().collect());
    }
    let start = match &flags.manifest_path {
        Some(manifest) => cwd.join(manifest).parent().map(Path::to_owned).unwrap_or_default(),
        None => cwd.to_owned(),
    };
    let here = start.ancestors().find_map(|dir| members.iter().find(|it| package_dir(it) == dir));
    if let Some(package) = here {
        return Ok(vec![*package]);
    }
    let defaults: Vec<&Package> = metadata.workspace_default_packages();
    if defaults.is_empty() {
        return Err("the workspace has no package to document".to_owned());
    }
    Ok(defaults)
}

/// The crates of `packages` to document: each one's library, and its binaries unless `--lib`
/// alone is asked, private items and all as cargo documents a binary. A binary named as its
/// package's library is left out, as cargo leaves it.
fn asked_crates(
    db: &RootDatabase,
    vfs: &vfs::Vfs,
    packages: &[&Package],
    flags: &Flags,
) -> Result<Vec<Asked>, String> {
    let by_root = crates_by_root(db, vfs);
    let picks_targets = flags.lib || flags.bins || !flags.bin.is_empty();
    let mut out = vec![];
    for package in packages {
        let lib = package.targets.iter().find(|it| {
            it.kind.iter().any(|kind| {
                matches!(
                    kind,
                    TargetKind::Lib | TargetKind::RLib | TargetKind::ProcMacro | TargetKind::DyLib
                )
            })
        });
        let lib_name = lib.map(|it| it.name.replace('-', "_"));
        if let Some(lib) = lib
            && (!picks_targets || flags.lib)
        {
            let krate = by_root
                .get(lib.src_path.as_std_path())
                .ok_or(format!("the library of {} was not loaded", package.name))?;
            out.push(Asked { krate: *krate, private: flags.private });
        }
        for bin in package.targets.iter().filter(|it| it.kind.contains(&TargetKind::Bin)) {
            let wanted = match picks_targets {
                false => true,
                true => flags.bins || flags.bin.contains(&bin.name),
            };
            if !wanted || lib_name.as_deref() == Some(&bin.name.replace('-', "_")) {
                continue;
            }
            let Some(krate) = by_root.get(bin.src_path.as_std_path()) else { continue };
            out.push(Asked { krate: *krate, private: true });
        }
    }
    if out.is_empty() {
        return Err("no library or binary to document in the packages asked for".to_owned());
    }
    Ok(out)
}

/// Each loaded crate by the path of its root file.
fn crates_by_root(db: &RootDatabase, vfs: &vfs::Vfs) -> FxHashMap<PathBuf, Crate> {
    let mut by_root: FxHashMap<PathBuf, Crate> = FxHashMap::default();
    for krate in Crate::all(db) {
        let path = vfs.file_path(krate.root_file(db));
        if let Some(path) = path.as_path() {
            by_root.entry(PathBuf::from(path.as_str())).or_insert(krate);
        }
    }
    by_root
}

/// The dependencies each workspace member's crates have for its tests alone: the packages its
/// manifest names only as dev-dependencies, which rust-analyzer's crate graph gives its library
/// and binaries too.
fn test_only(db: &RootDatabase, vfs: &vfs::Vfs, metadata: &Metadata) -> blaze_doc::TestOnly {
    use cargo_metadata::DependencyKind;
    let by_root = crates_by_root(db, vfs);
    let mut out = blaze_doc::TestOnly::default();
    for package in metadata.workspace_packages() {
        let named = |dev: bool| -> FxHashSet<&str> {
            package
                .dependencies
                .iter()
                .filter(|it| (it.kind == DependencyKind::Development) == dev)
                .map(|it| it.name.as_str())
                .collect()
        };
        let (dev, other) = (named(true), named(false));
        if dev.iter().all(|it| other.contains(it)) {
            continue;
        }
        for target in &package.targets {
            let Some(&krate) = by_root.get(target.src_path.as_std_path()) else { continue };
            for dep in krate.dependencies(db) {
                let name = package_name(db, dep.krate);
                if name.as_deref().is_some_and(|it| dev.contains(it) && !other.contains(it)) {
                    out.insert((krate, dep.krate));
                }
            }
        }
    }
    out
}

/// The name of a crate's package, as its manifest has it.
fn package_name(db: &RootDatabase, krate: Crate) -> Option<String> {
    use ide_db::base_db::CrateOrigin;
    match krate.origin(db) {
        CrateOrigin::Local { name: Some(name), .. } | CrateOrigin::Library { name, .. } => {
            Some(name.as_str().to_owned())
        }
        _ => None,
    }
}

/// `serde v1.0.219` as cargo names a package it documents, with the directory of one that is
/// not from a registry or a git checkout.
fn describe(db: &RootDatabase, vfs: &vfs::Vfs, documented: &Documented) -> String {
    let krate = documented.krate;
    let name = package_name(db, krate).unwrap_or_else(|| documented.name.clone());
    let version = krate.version(db).map(|it| format!(" v{it}")).unwrap_or_default();
    let root = vfs.file_path(krate.root_file(db));
    let dir = root
        .as_path()
        .map(|it| Path::new(it.as_str()))
        .and_then(|it| it.ancestors().skip(1).find(|dir| dir.join("Cargo.toml").is_file()));
    let fetched =
        crate::cargo::switch::cargo_home().map(|home| [home.join("registry"), home.join("git")]);
    match dir {
        Some(dir) if !fetched.iter().flatten().any(|it| dir.starts_with(it)) => {
            format!("{name}{version} ({})", dir.display())
        }
        _ => format!("{name}{version}"),
    }
}

/// rustdoc's warnings, with the count cargo says after them.
fn warn(model: &blaze_doc::model::Model, cwd: &Path) {
    let mut counts: Vec<(String, usize)> = vec![];
    for warning in &model.warnings {
        say::warning(&warning.message);
        if let Some(at) = &warning.at {
            let at = Path::new(at).strip_prefix(cwd).map(|it| it.display().to_string());
            let at = at.unwrap_or_else(|_| warning.at.clone().unwrap_or_default());
            eprintln!("  --> {at}");
        }
        eprintln!();
        match counts.iter_mut().find(|(name, _)| *name == warning.krate) {
            Some((_, count)) => *count += 1,
            None => counts.push((warning.krate.clone(), 1)),
        }
    }
    for (name, count) in counts {
        let plural = if count == 1 { "warning" } else { "warnings" };
        say::warning(&format!("`{name}` (lib doc) generated {count} {plural}"));
    }
}

/// Opens `page` as cargo doc --open does: with `$BROWSER`, else the system's opener.
fn open(page: &Path) {
    let browser = std::env::var_os("BROWSER").filter(|it| !it.is_empty());
    let opener = match &browser {
        Some(browser) => browser.clone(),
        None if cfg!(target_os = "macos") => "open".into(),
        None if cfg!(windows) => "explorer".into(),
        None => "xdg-open".into(),
    };
    let dir = page.parent().unwrap_or(page);
    let mut command =
        toolchain::command(&opener, dir, &FxHashMap::<String, Option<String>>::default());
    if let Err(why) = command.arg(page).status() {
        say::error(&format!(
            "could not open {} with {}: {why}",
            page.display(),
            opener.to_string_lossy()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::Flags;

    fn parse(text: &str) -> Result<Flags, String> {
        let typed: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
        Flags::parse(&typed)
    }

    #[test]
    fn reads_cargo_docs_flags() {
        let flags =
            parse("-p a --package=b --no-deps --open -Ffoo,bar --target wasm32-wasip1 -r -q")
                .unwrap();
        assert_eq!(flags.packages, ["a", "b"]);
        assert!(flags.no_deps && flags.open && flags.quiet);
        assert_eq!(flags.features, ["foo", "bar"]);
        assert_eq!(flags.target.as_deref(), Some("wasm32-wasip1"));
        assert_eq!(flags.profile.as_deref(), Some("release"));
        assert!(parse("--lib --bin x --document-private-items -j 8 --target-dir t").is_ok());
    }

    #[test]
    fn refuses_what_it_does_not_take() {
        assert!(parse("--message-format json").unwrap_err().contains("--message-format"));
        assert!(parse("stray").unwrap_err().contains("stray"));
        assert!(parse("--frobnicate").is_err());
    }
}
