//! `cargo fmt` and `cargo clippy` made blaze's by `enable`. They are not built into cargo: it
//! runs `cargo-fmt` and `cargo-clippy`, looked for in its home's `bin` ahead of `PATH`, so a
//! link there named so, to `cargo-blaze`, has cargo run blaze's. The file that was there,
//! rustup's proxy, is kept beside it and `disable` puts it back. An `[alias]` would do the same,
//! but cargo warns on every run that it shadows an external command and means to refuse it
//! (rust-lang/cargo#10049).

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::cargo::own::{self, Status};

/// The commands `enable` links, those of them that are blaze's own.
const COMMANDS: [&str; 2] = ["fmt", "clippy"];

/// The directory of cargo's home the files the links replace are kept in.
const KEPT: &str = "blaze-kept";

/// A link `enable` made.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    /// `cargo-<command>` in cargo's home.
    pub(crate) path: PathBuf,
    /// Where the file that was there is kept, when there was one.
    kept: Option<PathBuf>,
    /// Whether `enable` made the directory the link is in, which `disable` then removes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    made_dir: bool,
}

/// `cargo-blaze`, installed next to this ra-jit.
fn target() -> anyhow::Result<PathBuf> {
    let this = std::env::current_exe().context("blaze does not know where it is installed")?;
    Ok(this.with_file_name(format!("cargo-blaze{}", std::env::consts::EXE_SUFFIX)))
}

/// Whether `path` is a link to a `cargo-blaze`, this one's or one installed elsewhere since.
fn ours(path: &Path) -> bool {
    std::fs::read_link(path).is_ok_and(|it| it.file_stem().is_some_and(|it| it == "cargo-blaze"))
}

/// Links each command that is blaze's own and not in `have` to `cargo-blaze`, in cargo's home
/// `home`, adding each to `made` as it is made. Returns what to tell the user.
pub(crate) fn make(
    home: &Path,
    have: &[Link],
    made: &mut Vec<Link>,
) -> anyhow::Result<Vec<String>> {
    let names: Vec<_> = COMMANDS
        .into_iter()
        .filter(|it| own::named(it).is_some_and(|it| it.status == Status::Own))
        .collect();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    if !cfg!(unix) {
        return Ok(vec![format!(
            "`cargo {}` stay as they are: blaze links them on macOS and Linux",
            names.join("` and `cargo ")
        )]);
    }
    let target = target()?;
    anyhow::ensure!(
        target.exists(),
        "{} is not installed next to blaze; `cargo {}` are not changed",
        target.display(),
        names.join("` and `cargo ")
    );
    make_in(home, &target, &names, have, made)
}

fn make_in(
    home: &Path,
    target: &Path,
    names: &[&str],
    have: &[Link],
    made: &mut Vec<Link>,
) -> anyhow::Result<Vec<String>> {
    let bin = home.join("bin");
    let mut lines = Vec::new();
    for name in names {
        let path = bin.join(format!("cargo-{name}{}", std::env::consts::EXE_SUFFIX));
        if ours(&path) && have.iter().any(|it| it.path == path) {
            continue;
        }
        let kept = match std::fs::symlink_metadata(&path) {
            Ok(_) if ours(&path) => None,
            Ok(_) => {
                let kept = home.join(KEPT).join(path.file_name().context("no file name")?);
                anyhow::ensure!(
                    std::fs::symlink_metadata(&kept).is_err(),
                    "{} is kept from an earlier `enable` and would be lost; move it back to {} \
                     or remove it",
                    kept.display(),
                    path.display()
                );
                std::fs::create_dir_all(home.join(KEPT))?;
                std::fs::rename(&path, &kept)
                    .with_context(|| format!("moving {} aside", path.display()))?;
                Some(kept)
            }
            Err(_) => None,
        };
        let made_dir = !bin.exists();
        std::fs::create_dir_all(&bin)?;
        if !ours(&path)
            && let Err(error) = link(target, &path)
        {
            if let Some(kept) = &kept {
                std::fs::rename(kept, &path)?;
            }
            return Err(error).with_context(|| format!("linking {}", path.display()));
        }
        lines.push(match &kept {
            Some(kept) => format!(
                "`cargo {name}` is blaze's: {} links to it, and what was there is kept in {}",
                path.display(),
                kept.display()
            ),
            None => format!("`cargo {name}` is blaze's: {} links to it", path.display()),
        });
        made.push(Link { path, kept, made_dir });
    }
    Ok(lines)
}

#[cfg(unix)]
fn link(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(not(unix))]
fn link(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Puts back what `make` replaced. A link replaced since, as `rustup update` puts its proxy
/// back, is left as it is, and the file kept for it goes.
pub(crate) fn undo(links: &[Link]) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    for Link { path, kept, .. } in links {
        let name = command(path);
        if ours(path) {
            std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
            match kept {
                Some(kept) => {
                    std::fs::rename(kept, path)
                        .with_context(|| format!("moving {} back", kept.display()))?;
                    lines.push(format!("`cargo {name}` is as it was: {} is back", path.display()));
                }
                None => lines.push(format!("`cargo {name}` is as it was: the link is removed")),
            }
        } else if let Some(kept) = kept {
            if std::fs::symlink_metadata(path).is_ok() {
                std::fs::remove_file(kept)
                    .with_context(|| format!("removing {}", kept.display()))?;
            } else {
                std::fs::rename(kept, path)
                    .with_context(|| format!("moving {} back", kept.display()))?;
            }
        }
    }
    if let Some(dir) = links.iter().find_map(|it| it.kept.as_deref()?.parent()) {
        let _ = std::fs::remove_dir(dir);
    }
    // The directory `enable` made for its links goes too, when nothing else was put in it.
    if let Some(dir) = links.iter().find(|it| it.made_dir).and_then(|it| it.path.parent()) {
        let _ = std::fs::remove_dir(dir);
    }
    Ok(lines)
}

/// The lines of `status` about the commands blaze links: whose each one is.
pub(crate) fn status(home: &Path, links: &[Link]) -> Vec<String> {
    let mut lines = Vec::new();
    for name in COMMANDS {
        if !own::named(name).is_some_and(|it| it.status == Status::Own) {
            continue;
        }
        let path = home.join("bin").join(format!("cargo-{name}{}", std::env::consts::EXE_SUFFIX));
        let linked = links.iter().any(|it| it.path == path);
        lines.push(match (linked, ours(&path)) {
            (_, true) => format!("`cargo {name}`: blaze's, by the link {}", path.display()),
            (true, false) => format!(
                "`cargo {name}`: not blaze's, {} was replaced since `enable`, which links it again",
                path.display()
            ),
            (false, false) => format!("`cargo {name}`: not blaze's (`enable` links it)"),
        });
    }
    lines
}

/// The command `cargo-<command>` at `path` is.
fn command(path: &Path) -> String {
    let stem = path.file_stem().map(|it| it.to_string_lossy().into_owned()).unwrap_or_default();
    stem.strip_prefix("cargo-").unwrap_or(&stem).to_owned()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ra-jit-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        dir
    }

    #[test]
    fn a_proxy_is_kept_and_comes_back() {
        let home = scratch("proxy");
        let target = home.join("install").join("cargo-blaze");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "blaze").unwrap();
        let proxy = home.join("bin").join("cargo-fmt");
        std::fs::write(&proxy, "rustup's proxy").unwrap();
        let mut made = Vec::new();
        make_in(&home, &target, &["fmt", "clippy"], &[], &mut made).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(std::fs::read_link(&proxy).unwrap(), target);
        assert_eq!(std::fs::read_link(home.join("bin/cargo-clippy")).unwrap(), target);
        let mut again = Vec::new();
        make_in(&home, &target, &["fmt", "clippy"], &made, &mut again).unwrap();
        assert!(again.is_empty());
        undo(&made).unwrap();
        assert_eq!(std::fs::read_to_string(&proxy).unwrap(), "rustup's proxy");
        assert!(std::fs::symlink_metadata(home.join("bin/cargo-clippy")).is_err());
        assert!(!home.join(KEPT).exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_link_replaced_since_is_left_alone() {
        let home = scratch("replaced");
        let target = home.join("cargo-blaze");
        std::fs::write(&target, "blaze").unwrap();
        let proxy = home.join("bin").join("cargo-fmt");
        std::fs::write(&proxy, "old proxy").unwrap();
        let mut made = Vec::new();
        make_in(&home, &target, &["fmt"], &[], &mut made).unwrap();
        std::fs::remove_file(&proxy).unwrap();
        std::fs::write(&proxy, "new proxy").unwrap();
        undo(&made).unwrap();
        assert_eq!(std::fs::read_to_string(&proxy).unwrap(), "new proxy");
        assert!(!home.join(KEPT).exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_kept_file_is_never_overwritten() {
        let home = scratch("kept");
        let target = home.join("cargo-blaze");
        std::fs::write(&target, "blaze").unwrap();
        std::fs::write(home.join("bin/cargo-fmt"), "proxy").unwrap();
        std::fs::create_dir_all(home.join(KEPT)).unwrap();
        std::fs::write(home.join(KEPT).join("cargo-fmt"), "kept before").unwrap();
        let mut made = Vec::new();
        assert!(make_in(&home, &target, &["fmt"], &[], &mut made).is_err());
        assert!(made.is_empty());
        assert_eq!(std::fs::read_to_string(home.join("bin/cargo-fmt")).unwrap(), "proxy");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
