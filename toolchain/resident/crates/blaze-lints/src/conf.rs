//! clippy's configuration of a package: its `clippy.toml` (or `.clippy.toml`), found as
//! clippy finds it, and the Rust version it is to build on, which some lints keep to.
//!
//! Follows clippy's `clippy_config/src/conf.rs` (`load_conf_file`, `Conf::load_inner`) at
//! rust-lang/rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A Rust version, as `rust-version` and `msrv` give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// `1.70` or `1.70.0`, as clippy's `parse_version` reads it.
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |it| it.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |it| it.parse().ok())?;
        parts.next().is_none().then_some(Version(major, minor, patch))
    }
}

/// A package's clippy configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Conf {
    /// The file it was read from.
    pub path: Option<PathBuf>,
    /// Its keys, as written.
    #[serde(default)]
    pub table: toml::Table,
    /// `msrv`, else the package's `rust-version`.
    pub msrv: Option<Version>,
}

/// The names clippy looks for, in the order it takes them.
const NAMES: [&str; 2] = [".clippy.toml", "clippy.toml"];

/// The configuration file clippy reads for a package: in `CLIPPY_CONF_DIR` when that is set,
/// else in the package's directory, else in the nearest directory above with one.
pub fn find(conf_dir: Option<&Path>, package_dir: &Path) -> Option<PathBuf> {
    let mut current = conf_dir.unwrap_or(package_dir).canonicalize().ok()?;
    loop {
        if let Some(found) = NAMES.iter().map(|it| current.join(it)).find(|it| it.is_file()) {
            return Some(found);
        }
        if !current.pop() {
            return None;
        }
    }
}

impl Conf {
    /// The configuration of a package whose file is `path`, if it has one, and whose
    /// `rust-version` is `rust_version`.
    pub fn load(path: Option<PathBuf>, rust_version: Option<&str>) -> Result<Conf, String> {
        let table = match &path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|error| format!("error reading `{}`: {error}", path.display()))?;
                text.parse::<toml::Table>()
                    .map_err(|error| format!("error parsing `clippy.toml`: {}", error.message()))?
            }
            None => toml::Table::new(),
        };
        let msrv = match table.get("msrv").or_else(|| table.get("min-rust-version")) {
            Some(toml::Value::String(it)) => Version::parse(it),
            _ => rust_version.and_then(Version::parse),
        };
        Ok(Conf { path, table, msrv })
    }

    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.table.get(key).and_then(toml::Value::as_bool).unwrap_or(default)
    }

    pub fn int(&self, key: &str, default: i64) -> i64 {
        self.table.get(key).and_then(toml::Value::as_integer).unwrap_or(default)
    }

    pub fn strings(&self, key: &str) -> Option<Vec<String>> {
        let list = self.table.get(key)?.as_array()?;
        Some(list.iter().filter_map(|it| it.as_str().map(str::to_owned)).collect())
    }

    pub fn value(&self, key: &str) -> Option<&toml::Value> {
        self.table.get(key)
    }

    /// Whether the Rust version kept to has what came in `version`: clippy's `Msrv::meets`,
    /// which with no version set keeps to none.
    pub fn meets(&self, version: Version) -> bool {
        self.msrv.is_none_or(|it| it >= version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_as_clippy_reads_them() {
        assert_eq!(Version::parse("1.70"), Some(Version(1, 70, 0)));
        assert_eq!(Version::parse("1.74.1"), Some(Version(1, 74, 1)));
        assert_eq!(Version::parse("one"), None);
    }

    #[test]
    fn msrv_from_the_file_before_the_manifest() {
        let dir = std::env::temp_dir().join(format!("blaze-lints-conf-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("clippy.toml"), "msrv = \"1.60\"\nallow-dbg-in-tests = true\n")
            .unwrap();
        let found = find(None, &dir.join("sub")).unwrap();
        let conf = Conf::load(Some(found), Some("1.80")).unwrap();
        assert_eq!(conf.msrv, Some(Version(1, 60, 0)));
        assert!(conf.bool("allow-dbg-in-tests", false));
        assert!(!conf.meets(Version(1, 70, 0)));
        let conf = Conf::load(None, Some("1.80")).unwrap();
        assert!(conf.meets(Version(1, 70, 0)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
