//! A package's `[lints]`, as cargo reads them from its manifest: its own, or the workspace's
//! `[workspace.lints]` when it has `workspace = true`.

use std::path::Path;

use crate::Level;

/// One lint of a `[lints]` table: its tool (`rust`, `clippy`), its name, level and priority.
pub type ManifestLint = (String, String, Level, i8);

/// The `[lints]` of the package whose manifest is `manifest`, in the workspace whose root
/// manifest is `workspace`.
pub fn lints(manifest: &Path, workspace: &Path) -> Result<Vec<ManifestLint>, String> {
    let table = read(manifest)?;
    let Some(lints) = table.get("lints").and_then(|it| it.as_table()) else {
        return Ok(Vec::new());
    };
    if lints.get("workspace").and_then(|it| it.as_bool()) == Some(true) {
        let root = read(workspace)?;
        let inherited =
            root.get("workspace").and_then(|it| it.get("lints")).and_then(|it| it.as_table());
        return Ok(inherited.map(of_table).unwrap_or_default());
    }
    Ok(of_table(lints))
}

/// The flags cargo gives rustc for `lints`, in its order: `--warn=clippy::all`.
pub fn flags(lints: &[ManifestLint]) -> Vec<String> {
    let command_line = crate::level::of_manifest(lints);
    let flags = command_line.opts.into_iter();
    flags.map(|(name, level)| format!("{}={name}", crate::level::flag_of(level))).collect()
}

fn read(path: &Path) -> Result<toml::Table, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    text.parse::<toml::Table>()
        .map_err(|error| format!("reading {}: {}", path.display(), error.message()))
}

fn of_table(lints: &toml::Table) -> Vec<ManifestLint> {
    let mut found = Vec::new();
    for (tool, of_tool) in lints {
        let Some(of_tool) = of_tool.as_table() else { continue };
        for (name, value) in of_tool {
            let (level, priority) = match value {
                toml::Value::String(level) => (level.as_str(), 0),
                toml::Value::Table(it) => {
                    let level = it.get("level").and_then(|it| it.as_str()).unwrap_or("warn");
                    let priority = it.get("priority").and_then(|it| it.as_integer()).unwrap_or(0);
                    (level, priority)
                }
                _ => continue,
            };
            let level = match level {
                "forbid" => Level::Forbid,
                "deny" => Level::Deny,
                "warn" => Level::Warn,
                "allow" => Level::Allow,
                _ => continue,
            };
            let priority = i8::try_from(priority).unwrap_or(0);
            found.push((tool.clone(), name.clone(), level, priority));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_from_the_workspace() {
        let dir = std::env::temp_dir().join(format!("blaze-lints-manifest-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("member")).unwrap();
        let root = dir.join("Cargo.toml");
        std::fs::write(
            &root,
            "[workspace]\n[workspace.lints.clippy]\nall = { level = \"warn\", priority = -1 }\nempty_enums = \"warn\"\n",
        )
        .unwrap();
        let member = dir.join("member/Cargo.toml");
        std::fs::write(&member, "[package]\nname = \"m\"\n[lints]\nworkspace = true\n").unwrap();
        let mut found = lints(&member, &root).unwrap();
        found.sort();
        assert_eq!(
            found,
            vec![
                ("clippy".to_owned(), "all".to_owned(), Level::Warn, -1),
                ("clippy".to_owned(), "empty_enums".to_owned(), Level::Warn, 0),
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
