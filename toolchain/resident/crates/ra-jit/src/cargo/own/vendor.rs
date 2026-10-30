//! `cargo vendor`: every package of the lock file copied into the workspace, as cargo 1.99.0's
//! `commands/vendor.rs` and `ops/vendor.rs` do: each registry package unpacked from its
//! `.crate` as it is, with a `.cargo-checksum.json` of its files, older copies removed, and
//! the `[source]` configuration that reads them printed. A git package is refused (blaze does
//! not yet write the manifest `cargo package` would), and path packages stay where they are.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use blaze_manifest::metadata::root_manifest;
use blaze_manifest::{Context, Workspace};
use blaze_resolve::PackageId;
use serde::Serialize;
use sha2::Digest as _;

use super::args::{Args, Read, switch, valued};
use super::manifest::{USAGE, config, failed};

const HELP: &str = "\
Vendor all dependencies for a project locally

Usage: cargo vendor [OPTIONS] [path]

Arguments:
  [path]  Where to vendor crates (`vendor` by default)

Options:
      --no-delete                Don't delete older crates in the vendor directory
  -s, --sync <TOML>              Additional `Cargo.toml` to sync and vendor
      --respect-source-config    Respect `[source]` config in `.cargo/config`
      --versioned-dirs           Always include version in subdir name
  -v, --verbose...               Use verbose output (-vv very verbose/build.rs output)
  -q, --quiet                    Do not print cargo log messages
      --color <WHEN>             Coloring [possible values: auto, always, never]
      --config <KEY=VALUE|PATH>  Override a configuration value
  -Z <FLAG>                      Unstable (nightly-only) flags to Cargo, see 'cargo -Z help' for
                                 details
  -h, --help                     Print help

Manifest Options:
  -m, --manifest-path <PATH>  Path to Cargo.toml
      --locked                Assert that `Cargo.lock` will remain unchanged
      --offline               Run without accessing the network
      --frozen                Equivalent to specifying both --locked and --offline

Run `cargo help vendor` for more detailed information.";

/// The flags of the crates.io `cargo vendor` that cargo's does not take.
const UNSUPPORTED: [&str; 4] =
    ["--no-merge-sources", "--relative-path", "--only-git-deps", "--disallow-duplicates"];

const COMMENT: &str = "This file only protects against accidental modifications. It is not a \
                       security mechanism and does not protect against malicious changes.";

/// `cargo vendor [OPTIONS] [path]`.
pub(super) fn command(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [
        switch(&["--no-delete"]),
        valued(&["-s", "--sync"]),
        switch(&["--respect-source-config"]),
        switch(&["--versioned-dirs"]),
        valued(&["-m", "--manifest-path"]),
        switch(&["--no-merge-sources"]),
        switch(&["--relative-path"]),
        switch(&["--only-git-deps"]),
        switch(&["--disallow-duplicates"]),
    ];
    let usage = |why: &str| {
        super::super::say::error(why);
        eprintln!("\nUsage: cargo vendor [OPTIONS] [path]\n\nFor more information, try '--help'.");
        Ok(USAGE)
    };
    let read = match args.read(&flags) {
        Ok(read) if read.has("--help") => {
            println!("{HELP}");
            return Ok(0);
        }
        Ok(read) => read,
        Err(why) => return usage(&why),
    };
    if let Some(flag) = UNSUPPORTED.iter().find(|it| read.has(it)) {
        return usage(&format!(
            "unexpected argument '{flag}' found\n\n  tip: the crates.io `cargo vendor` command has \
             been merged into Cargo\n  tip: and the flag `{flag}` isn't supported currently\n  \
             tip: to continue using the flag, execute `cargo-vendor vendor ...`\n  tip: to \
             suggest this flag supported in Cargo, file an issue at \
             <https://github.com/rust-lang/cargo/issues/new>"
        ));
    }
    if let Some(extra) = read.positional.get(1) {
        return usage(&format!("unexpected argument '{extra}' found"));
    }
    match vendor(&read) {
        Ok(()) => Ok(0),
        Err(err) => Ok(failed(&err)),
    }
}

fn vendor(read: &Read) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("couldn't get the current directory")?;
    let ctx = Context::new(config("vendor", read)?);
    let manifest_path = read.value("--manifest-path").map(|it| cwd.join(it));
    let ws = Workspace::new(&root_manifest(manifest_path.as_deref(), &ctx)?, &ctx)?;
    let mut workspaces = Vec::new();
    for extra in read.values("--sync") {
        let extra = cwd.join(extra);
        let it = Workspace::new(&extra, &ctx)?;
        workspaces.push((extra, it.root().to_owned()));
    }
    workspaces.push((ws.root_manifest().to_owned(), ws.root().to_owned()));
    for (_, root) in &workspaces {
        if !root.join("Cargo.lock").exists() {
            bail!(
                "{} has no lock file, and writing one chooses versions, which blaze's resolver \
                 does not yet (#321)",
                root.display()
            );
        }
    }
    let destination = PathBuf::from(read.positional.first().map_or("vendor", String::as_str));
    let config = sync(read, &cwd, &workspaces, &destination).context("failed to sync")?;
    if !read.has("--quiet") {
        if config.source.is_empty() {
            eprintln!("There is no dependency to vendor in this project.");
        } else {
            eprint!(
                "To use vendored sources, add this to your .cargo/config.toml for this project:\n\n"
            );
            print!("{}", toml::to_string_pretty(&config)?);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct VendorConfig {
    source: BTreeMap<String, VendorSource>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase", untagged)]
enum VendorSource {
    Directory {
        directory: String,
    },
    Registry {
        registry: Option<String>,
        #[serde(rename = "replace-with")]
        replace_with: String,
    },
}

/// A registry package to copy: its source unpacked, and its `.crate`.
struct Copied {
    root: PathBuf,
    tarball: PathBuf,
}

fn sync(
    read: &Read,
    cwd: &Path,
    workspaces: &[(PathBuf, PathBuf)],
    destination: &Path,
) -> anyhow::Result<VendorConfig> {
    let vendor_dir = std::fs::canonicalize(destination).unwrap_or_else(|_| destination.to_owned());
    let existed = vendor_dir.exists();
    std::fs::create_dir_all(&vendor_dir)
        .with_context(|| format!("failed to create directory `{}`", vendor_dir.display()))?;
    let mut to_remove = HashSet::new();
    if !read.has("--no-delete") {
        for entry in vendor_dir.read_dir()? {
            let entry = entry?;
            if !entry.file_name().to_str().is_some_and(|it| it.starts_with('.')) {
                to_remove.insert(entry.path());
            }
        }
    }

    let locked = match (read.has("--frozen"), read.has("--locked")) {
        (true, _) => Some("--frozen"),
        (_, true) => Some("--locked"),
        _ => None,
    };
    let configs = read.values("--config");
    let mut checksums = HashMap::new();
    let mut ids: BTreeMap<PackageId, Copied> = BTreeMap::new();
    for (manifest, root) in workspaces {
        let asked = blaze_metadata::Asked {
            cwd,
            env: std::env::vars().collect(),
            manifest_path: Some(manifest),
            config: &configs,
            features: &[],
            all_features: false,
            no_default_features: false,
            build: None,
            platform: None,
            offline: read.has("--offline") || read.has("--frozen"),
            store: None,
            rustc_version: Some(crate::cargo::toolchain::ours_release()),
        };
        let packages =
            blaze_metadata::vendored(&asked, read.has("--respect-source-config"), locked)
                .with_context(|| format!("failed to load lockfile for {}", root.display()))?;
        for package in packages {
            let Some(tarball) = package.tarball else {
                if let Ok(path) = std::fs::canonicalize(&package.root) {
                    to_remove.remove(&path);
                }
                continue;
            };
            checksums.insert(package.id, package.checksum);
            ids.insert(package.id, Copied { root: package.root, tarball });
        }
    }

    let mut versions: HashMap<&str, BTreeMap<&semver::Version, PackageId>> = HashMap::new();
    for id in ids.keys() {
        let map = versions.entry(id.name().as_str()).or_default();
        if let Some(prev) = map.get(id.version()) {
            bail!(
                "found duplicate version of package `{} v{}` vendored from two sources:\n\n\
                 \tsource 1: {}\n\tsource 2: {}",
                id.name(),
                id.version(),
                prev.source_id(),
                id.source_id()
            );
        }
        map.insert(id.version(), *id);
    }

    let mut config = BTreeMap::new();
    let quiet = read.has("--quiet");
    for (id, copied) in &ids {
        let newest = versions[id.name().as_str()].keys().next_back().copied();
        let suffixed = read.has("--versioned-dirs") || Some(id.version()) != newest;
        let unpacked = format!("{}-{}", id.name(), id.version());
        let dst_name = match suffixed {
            true => unpacked.clone(),
            false => id.name().to_string(),
        };
        let source = id.source_id();
        let (name, entry) = match source.is_crates_io() {
            true => ("crates-io".to_owned(), None),
            false => {
                (source.without_precise().as_url().to_string(), Some(source.url().to_string()))
            }
        };
        config.insert(
            name,
            VendorSource::Registry { registry: entry, replace_with: "vendored-sources".to_owned() },
        );
        let dst = vendor_dir.join(&dst_name);
        to_remove.remove(&dst);
        let cksum = dst.join(".cargo-checksum.json");
        if suffixed && cksum.exists() {
            continue;
        }
        super::super::say::status(
            quiet,
            "Vendoring",
            &format!("{id} ({}) to {}", copied.root.display(), dst.display()),
        );
        let _ = std::fs::remove_dir_all(&dst);
        let tarball = std::fs::read(&copied.tarball)
            .with_context(|| format!("failed to open {}", copied.tarball.display()))?;
        if suffixed {
            blaze_registry::download::unpack_package_in(&tarball, &dst, &vendor_this)?;
        } else {
            let staging = vendor_dir.join(format!(".vendor-staging{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging)?;
            let inside = staging.join(&unpacked);
            let moved =
                blaze_registry::download::unpack_package_in(&tarball, &inside, &vendor_this)
                    .and_then(|_| {
                        std::fs::rename(&inside, &dst).with_context(|| {
                            format!("failed to move {} to {}", inside.display(), dst.display())
                        })
                    });
            let _ = std::fs::remove_dir_all(&staging);
            moved?;
        }
        let mut files = BTreeMap::new();
        checksum_files(&dst, &dst, &mut files)?;
        let json = serde_json::json!({
            "$comment": COMMENT,
            "package": checksums.get(id),
            "files": files,
        });
        std::fs::write(&cksum, json.to_string())
            .with_context(|| format!("failed to write `{}`", cksum.display()))?;
    }

    for path in to_remove {
        let removed = match path.is_dir() {
            true => std::fs::remove_dir_all(&path),
            false => std::fs::remove_file(&path),
        };
        removed.with_context(|| format!("failed to remove `{}`", path.display()))?;
    }

    if !config.is_empty() {
        let directory = destination.to_string_lossy().replace('\\', "/");
        config.insert("vendored-sources".to_owned(), VendorSource::Directory { directory });
    } else if !existed {
        std::fs::remove_dir(&vendor_dir)
            .with_context(|| format!("failed to remove `{}`", vendor_dir.display()))?;
    }
    Ok(VendorConfig { source: config })
}

/// The sha256 of every file and link under `dir`, by its path from `root`.
fn checksum_files(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, String>,
) -> anyhow::Result<()> {
    for entry in dir.read_dir().with_context(|| format!("failed to read `{}`", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            checksum_files(root, &path, out)?;
        } else if kind.is_file() || kind.is_symlink() {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to checksum `{}`", path.display()))?;
            let relative = path.strip_prefix(root).expect("under the root");
            let relative = relative.to_str().context("a path that is not UTF-8")?;
            out.insert(relative.replace('\\', "/"), format!("{:x}", sha2::Sha256::digest(&bytes)));
        }
    }
    Ok(())
}

/// Whether a file of a package, by its path in it, is vendored: not git's files, nor the mark
/// cargo leaves in what it unpacked.
fn vendor_this(relative: &Path) -> bool {
    if relative.components().any(|it| it.as_os_str() == ".git") {
        return false;
    }
    if relative.file_name().is_some_and(|it| it == ".gitattributes" || it == ".gitignore") {
        return false;
    }
    relative != Path::new(".cargo-ok")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::vendor_this;

    #[test]
    fn what_is_vendored() {
        assert!(vendor_this(Path::new("src/lib.rs")));
        assert!(vendor_this(Path::new(".cargo_vcs_info.json")));
        assert!(!vendor_this(Path::new(".gitignore")));
        assert!(!vendor_this(Path::new("sub/.gitattributes")));
        assert!(!vendor_this(Path::new(".git/config")));
        assert!(!vendor_this(Path::new(".cargo-ok")));
    }
}
