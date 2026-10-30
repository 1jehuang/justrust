//! Where a package comes from, as cargo's `SourceId` names it in `cargo metadata`.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use cargo_util_schemas::core::{GitReference, SourceKind};
use url::Url;

use crate::config::Config;

pub const CRATES_IO_INDEX: &str = "https://github.com/rust-lang/crates.io-index";
pub const CRATES_IO_REGISTRY: &str = "crates-io";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceId {
    pub kind: SourceKind,
    pub url: Url,
    /// The name of the registry in `[registries]`, for one named there.
    pub registry_key: Option<String>,
}

impl SourceId {
    pub fn for_path(path: &Path) -> anyhow::Result<SourceId> {
        let url = Url::from_file_path(path)
            .map_err(|()| anyhow!("invalid path url `{}`", path.display()))?;
        Ok(SourceId { kind: SourceKind::Path, url, registry_key: None })
    }

    /// The source of the package whose manifest is at `manifest_path`: its directory.
    pub fn for_manifest_path(manifest_path: &Path) -> anyhow::Result<SourceId> {
        SourceId::for_path(manifest_path.parent().unwrap())
    }

    pub fn for_git(url: &Url, reference: GitReference) -> SourceId {
        SourceId { kind: SourceKind::Git(reference), url: url.clone(), registry_key: None }
    }

    fn remote_kind(url: &Url) -> SourceKind {
        if url.as_str().starts_with("sparse+") {
            SourceKind::SparseRegistry
        } else {
            SourceKind::Registry
        }
    }

    pub fn for_registry(url: &Url) -> SourceId {
        SourceId { kind: Self::remote_kind(url), url: url.clone(), registry_key: None }
    }

    pub fn for_alt_registry(url: &Url, key: &str) -> SourceId {
        SourceId { kind: Self::remote_kind(url), url: url.clone(), registry_key: Some(key.into()) }
    }

    /// crates.io as cargo names it in metadata: its git index, whatever protocol fetches it.
    pub fn crates_io(config: &Config) -> anyhow::Result<SourceId> {
        config.check_registry_index_not_set()?;
        Ok(SourceId::for_alt_registry(&CRATES_IO_INDEX.parse().unwrap(), CRATES_IO_REGISTRY))
    }

    /// A registry by its name in `[registries]`.
    pub fn alt_registry(config: &Config, key: &str) -> anyhow::Result<SourceId> {
        if key == CRATES_IO_REGISTRY {
            return SourceId::crates_io(config);
        }
        let url = config.registry_index(key)?;
        Ok(SourceId::for_alt_registry(&url, key))
    }

    pub fn is_path(&self) -> bool {
        self.kind == SourceKind::Path
    }

    pub fn local_path(&self) -> Option<PathBuf> {
        self.is_path().then(|| self.url.to_file_path().unwrap())
    }

    /// The URL with its protocol and, for git, its reference: what metadata prints.
    pub fn as_url(&self) -> SourceIdAsUrl<'_> {
        SourceIdAsUrl { id: self, encoded: false }
    }
}

pub struct SourceIdAsUrl<'a> {
    id: &'a SourceId,
    encoded: bool,
}

impl fmt::Display for SourceIdAsUrl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(protocol) = self.id.kind.protocol() {
            write!(f, "{protocol}+")?;
        }
        write!(f, "{}", self.id.url)?;
        if let SourceKind::Git(reference) = &self.id.kind
            && let Some(pretty) = reference.pretty_ref(self.encoded)
        {
            write!(f, "?{pretty}")?;
        }
        Ok(())
    }
}

impl serde::Serialize for SourceId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.is_path() { s.serialize_none() } else { s.collect_str(&self.as_url()) }
    }
}

/// A URL as cargo's `IntoUrl` parses one, with its hint for scp-like git addresses.
pub(crate) fn into_url(s: &str) -> anyhow::Result<Url> {
    Url::parse(s).map_err(|e| {
        if s.starts_with("git@") {
            anyhow!(
                "invalid url `{s}`: {e}; try using `{}` instead",
                format_args!("ssh://{}", s.replacen(':', "/", 1))
            )
        } else {
            anyhow!("invalid url `{s}`: {e}")
        }
    })
}
