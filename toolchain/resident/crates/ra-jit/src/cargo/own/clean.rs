//! `cargo clean`: the target and build directories removed, or one profile's, or the
//! documentation, as cargo 1.101.0's `ops/cargo_clean.rs` removes them and counts what it
//! removed.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use blaze_manifest::metadata::root_manifest;
use blaze_manifest::profiles::{Profiles, requested_profile};
use blaze_manifest::{Config, Context, Workspace};
use rustc_hash::FxHashSet;

use super::args::{Args, switch, valued};
use super::manifest::{failed, read_or_refuse};
use crate::cargo::say;

const ABORT_NOTE: &str =
    "cleaning has been aborted to prevent accidental deletion of unrelated files";

pub(super) fn command(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [
        switch(&["--doc"]),
        valued(&["-p", "--package"]),
        switch(&["--workspace"]),
        switch(&["-r", "--release"]),
        valued(&["--profile"]),
        valued(&["--target"]),
        valued(&["--target-dir"]),
        valued(&["--manifest-path"]),
        switch(&["-n", "--dry-run"]),
    ];
    let usage = "cargo blaze clean [--doc] [--release] [--profile <NAME>] [--target <TRIPLE>] \
                 [--target-dir <DIR>] [--manifest-path <PATH>] [--dry-run]";
    let read = match read_or_refuse(&args, &flags, usage) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    let quiet = read.has("--quiet");
    let mut removal = Removal {
        dry_run: read.has("--dry-run"),
        verbose: read.has("--verbose"),
        quiet,
        ..Removal::default()
    };
    let cleaned = (|| {
        if read.has("--package") || read.has("--workspace") {
            anyhow::bail!(
                "blaze does not clean single packages yet (`-p`, `--workspace`); `cargo blaze clean` removes the whole target directory"
            );
        }
        let cwd = std::env::current_dir().context("couldn't get the current directory")?;
        let mut env: Vec<(String, String)> = std::env::vars().collect();
        let target_dir_arg = read.value("--target-dir");
        if let Some(dir) = &target_dir_arg {
            env.retain(|(name, _)| name != "CARGO_TARGET_DIR");
            env.push(("CARGO_TARGET_DIR".to_owned(), dir.clone()));
        }
        let mut config = Config::load(&cwd, env, &[])?;
        config.start("clean", &read.values("--config"))?;
        let ctx = Context::new(config);
        let manifest_path = read.value("--manifest-path").map(PathBuf::from);
        let ws = Workspace::new(&root_manifest(manifest_path.as_deref(), &ctx)?, &ctx)?;
        let mut target_dir = ws.target_dir();
        let mut build_dir = ws.build_dir();

        if let Ok(meta) = std::fs::symlink_metadata(&target_dir)
            && !meta.is_symlink()
            && !meta.is_dir()
        {
            return Ok(Err(format!("cannot clean `{}`: not a directory", target_dir.display())));
        }
        if target_dir_arg.is_some()
            && target_dir.exists()
            && let Err(err) = valid_tag(&target_dir)
        {
            return Ok(Err(format!("cannot clean `{}`: {err}", target_dir.display())));
        }

        if read.has("--doc") {
            let doc_dirs: Vec<PathBuf> = match read.values("--target")[..] {
                [] => vec![target_dir.join("doc")],
                ref triples => {
                    triples.iter().map(|it| target_dir.join(short_name(it)).join("doc")).collect()
                }
            };
            removal.remove_paths(&doc_dirs)?;
        } else {
            let release = read.has("--release");
            let profile = read.value("--profile");
            let requested = requested_profile("dev", release, profile.as_deref())?;
            let profiles =
                Profiles::new(ws.profiles(), ws.unstable_features(), &ctx.config, &requested, "")?;
            if release || profile.is_some() {
                let dir_name = profiles.get_dir_name();
                target_dir = target_dir.join(dir_name);
                build_dir = build_dir.join(dir_name);
            }
            let paths = match build_dir == target_dir {
                true => vec![target_dir],
                false => vec![target_dir, build_dir],
            };
            removal.remove_paths(&paths)?;
        }
        removal.summary();
        anyhow::Ok(Ok(()))
    })();
    match cleaned {
        Ok(Ok(())) => Ok(0),
        Ok(Err(title)) => {
            say::error(&title);
            eprintln!("  |\n  = note: {ABORT_NOTE}");
            Ok(super::super::FAILED)
        }
        Err(err) => Ok(failed(&err)),
    }
}

/// The name of a `--target`'s directory: a target given by its JSON file is named by the file.
fn short_name(target: &str) -> &str {
    match target.strip_suffix(".json") {
        Some(path) => Path::new(path).file_name().and_then(|it| it.to_str()).unwrap_or(target),
        None => target,
    }
}

/// Whether `dir` holds the `CACHEDIR.TAG` cargo writes in a target directory.
fn valid_tag(dir: &Path) -> anyhow::Result<()> {
    const TAG_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";
    let tag_path = dir.join("CACHEDIR.TAG");
    if tag_path.is_symlink() {
        anyhow::bail!("expect `CACHEDIR.TAG` to be a regular file, got a symlink");
    }
    if !tag_path.is_file() {
        anyhow::bail!("missing or invalid `CACHEDIR.TAG` file");
    }
    let text = std::fs::read(&tag_path)
        .map_err(|err| anyhow::anyhow!("failed to open `{}`: {err}", tag_path.display()))?;
    if !text.starts_with(TAG_SIGNATURE) {
        anyhow::bail!("invalid signature in `CACHEDIR.TAG` file");
    }
    Ok(())
}

/// What a clean removed, or would have with `--dry-run`, counted as cargo's `CleanContext`
/// counts it.
#[derive(Default)]
struct Removal {
    dry_run: bool,
    verbose: bool,
    quiet: bool,
    files: u64,
    dirs: u64,
    bytes: u64,
    /// The files of more than one link already counted, by device and inode.
    seen: FxHashSet<(u64, u64)>,
}

impl Removal {
    fn remove_paths(&mut self, paths: &[PathBuf]) -> anyhow::Result<()> {
        for path in paths {
            self.rm_rf(path)?;
        }
        Ok(())
    }

    fn rm_rf(&mut self, path: &Path) -> anyhow::Result<()> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    say::warning(&format!("cannot access {}: {e}", path.display()));
                }
                return Ok(());
            }
        };
        // A dry run prints the paths as it walks them instead.
        if !self.dry_run && self.verbose {
            say::status(self.quiet, "Removing", &path.display().to_string());
        }
        if !meta.is_dir() {
            return self.rm_file(path, &meta);
        }
        self.walk(path)
    }

    /// Removes the directory `dir` and what is in it, its contents first.
    fn walk(&mut self, dir: &Path) -> anyhow::Result<()> {
        let entries =
            std::fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                self.walk(&path)?;
            } else {
                if self.dry_run && self.verbose {
                    println!("{}", path.display());
                }
                self.rm_file(&path, &meta)?;
            }
        }
        if self.dry_run && self.verbose {
            println!("{}", dir.display());
        }
        self.dirs += 1;
        if !self.dry_run {
            std::fs::remove_dir_all(dir)
                .with_context(|| format!("failed to remove {}", dir.display()))?;
        }
        Ok(())
    }

    fn rm_file(&mut self, path: &Path, meta: &std::fs::Metadata) -> anyhow::Result<()> {
        self.bytes += self.count_bytes(meta);
        self.files += 1;
        if !self.dry_run {
            std::fs::remove_file(path)
                .with_context(|| format!("failed to remove file `{}`", path.display()))?;
        }
        Ok(())
    }

    /// The bytes a file frees: none for a hard link to a file already counted.
    fn count_bytes(&mut self, meta: &std::fs::Metadata) -> u64 {
        if !meta.is_file() {
            return meta.len();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if meta.nlink() > 1 && !self.seen.insert((meta.dev(), meta.ino())) {
                return 0;
            }
        }
        meta.len()
    }

    /// `     Removed 12 files, 3.4MiB total`, as cargo says it.
    fn summary(&self) {
        let status = if self.dry_run { "Summary" } else { "Removed" };
        let byte_count = match self.bytes {
            0 => String::new(),
            bytes => format!(", {} total", human_bytes(bytes)),
        };
        let file_count = match (self.files, self.dirs) {
            (0, 0) => "0 files".to_owned(),
            (0, 1) => "1 directory".to_owned(),
            (0, dirs) => format!("{dirs} directories"),
            (1, _) => "1 file".to_owned(),
            (files, _) => format!("{files} files"),
        };
        say::status(self.quiet, status, &format!("{file_count}{byte_count}"));
        if self.dry_run {
            say::warning("no files deleted due to --dry-run");
        }
    }
}

/// `bytes` as cargo's `HumanBytes` prints it with one decimal.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let bytes = bytes as f32;
    let i = ((bytes.log2() / 10.0) as usize).min(UNITS.len() - 1);
    let size = bytes / 1024_f32.powi(i as i32);
    match i {
        0 => format!("{size}{}", UNITS[i]),
        _ => format!("{size:.1}{}", UNITS[i]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_as_cargo_prints_them() {
        assert_eq!(human_bytes(512), "512B");
        assert_eq!(human_bytes(1536), "1.5KiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0GiB");
    }
}
