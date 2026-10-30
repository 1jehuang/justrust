//! `experimental` in `[workspace.metadata.blaze]` of a workspace's root manifest: the features
//! of blaze that are off until a workspace opts into them, by name.
//!
//! ```toml
//! [workspace.metadata.blaze]
//! experimental = ["sandboxed-tests"]
//! ```

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::Context as _;

/// The key, in `[workspace.metadata.blaze]`, that lists the experiments a workspace opts into.
pub(crate) const KEY: &str = "experimental";

/// Every test of the workspace runs in a sandbox, and under `RA_JIT_TESTS=affected` a pass is
/// kept by what the test read ([`super::sandboxed`]).
pub(crate) const SANDBOXED_TESTS: &str = "sandboxed-tests";

/// The experiments there are.
pub(crate) const KNOWN: &[&str] = &[SANDBOXED_TESTS];

/// The experiments a workspace's root manifest opts into.
#[derive(Default, Debug)]
pub(crate) struct Experiments {
    pub(crate) manifest: PathBuf,
    pub(crate) on: BTreeSet<String>,
}

impl Experiments {
    pub(crate) fn has(&self, name: &str) -> bool {
        self.on.contains(name)
    }

    /// The line that opts the workspace into `name`, and where it goes.
    pub(crate) fn how_to(&self, name: &str) -> String {
        format!(
            "`{KEY} = [\"{name}\"]` under `[workspace.metadata.blaze]` in {}",
            self.manifest.display()
        )
    }
}

/// What the root manifest of the workspace at `root` lists in [`KEY`]: nothing when it has no
/// manifest or does not say. A name not [`KNOWN`], or a value that is not a list of strings, is
/// an error.
pub(crate) fn of(root: &Path) -> anyhow::Result<Experiments> {
    let manifest = root.join("Cargo.toml");
    let mut out = Experiments { manifest: manifest.clone(), on: BTreeSet::new() };
    let Ok(text) = std::fs::read_to_string(&manifest) else { return Ok(out) };
    let value: toml::Table =
        toml::from_str(&text).with_context(|| format!("reading {}", manifest.display()))?;
    let listed = ["workspace", "metadata", "blaze"]
        .iter()
        .try_fold(&value, |table, key| table.get(*key)?.as_table())
        .and_then(|it| it.get(KEY));
    let Some(listed) = listed else { return Ok(out) };
    let wrong = |what: &str| {
        anyhow::anyhow!("`{KEY}` in [workspace.metadata.blaze] of {}: {what}", manifest.display())
    };
    let known = || KNOWN.iter().map(|it| format!("`{it}`")).collect::<Vec<_>>().join(", ");
    let names = listed
        .as_array()
        .ok_or_else(|| wrong(&format!("a list of experiments' names (known: {})", known())))?;
    for name in names {
        let name = name.as_str().ok_or_else(|| {
            wrong(&format!("a list of experiments' names, as strings (known: {})", known()))
        })?;
        if !KNOWN.contains(&name) {
            return Err(wrong(&format!("`{name}` is not known (known: {})", known())));
        }
        out.on.insert(name.to_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_opts_into_experiments_by_its_root_manifest() {
        let dir = std::env::temp_dir().join(format!("ra-jit-experimental-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |text: &str| std::fs::write(dir.join("Cargo.toml"), text).unwrap();
        assert!(!of(&dir.join("none")).unwrap().has(SANDBOXED_TESTS));
        write("[workspace]\n");
        assert!(!of(&dir).unwrap().has(SANDBOXED_TESTS));
        write("[workspace]\n[workspace.metadata.blaze]\nnative-proc-macros = []\n");
        assert!(!of(&dir).unwrap().has(SANDBOXED_TESTS));
        write("[workspace]\n[workspace.metadata.blaze]\nexperimental = [\"sandboxed-tests\"]\n");
        let out = of(&dir).unwrap();
        assert!(out.has(SANDBOXED_TESTS));
        assert!(out.how_to(SANDBOXED_TESTS).contains("experimental = [\"sandboxed-tests\"]"));
        write("[workspace]\n[workspace.metadata.blaze]\nexperimental = [\"sandboxed-test\"]\n");
        let error = of(&dir).unwrap_err().to_string();
        assert!(
            error.contains("`sandboxed-test` is not known (known: `sandboxed-tests`)"),
            "{error}"
        );
        write("[workspace]\n[workspace.metadata.blaze]\nexperimental = \"sandboxed-tests\"\n");
        let error = of(&dir).unwrap_err().to_string();
        assert!(error.contains("Cargo.toml: a list of experiments' names"), "{error}");
        write("[workspace]\n[workspace.metadata.blaze]\nexperimental = [true]\n");
        let error = of(&dir).unwrap_err().to_string();
        assert!(error.contains("as strings"), "{error}");
        _ = std::fs::remove_dir_all(&dir);
    }
}
