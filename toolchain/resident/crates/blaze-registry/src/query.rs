//! blaze-resolve's [`Registry`] over the registries' indexes, as cargo 1.99 answers the
//! resolver: each line of an index file made a [`Summary`] as `index_package_to_summary` and
//! `registry_dependency_into_dep` make it (`src/sources/registry/index/mod.rs`), the versions a
//! dependency is offered as `RegistrySource::query` and `RegistryIndex::query_inner` offer them
//! (`src/sources/registry/mod.rs`), behind `[source]` replacement as `SourceConfigMap` and
//! `ReplacedSource` put it (`src/sources/{config.rs,replaced.rs}`). Sources that are not
//! registries (paths, git checkouts, vendored directories) are answered by another registry.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use blaze_resolve::{
    DepKind, Dependency, IndexSummary, InternedString, OptVersionReq, PackageId, QueryKind,
    Registry as Resolver, SourceId, SourceKind, Summary,
};

use crate::{
    Registry, Sources,
    index::{self, IndexDependency, IndexEntry, IndexLine, Status},
};

/// A line of an index of `source` as the resolver takes it, as cargo's `IndexSummary::parse`:
/// a line whose summary cannot be made is [`IndexSummary::Invalid`], with its name, version and
/// `rust-version` alone. None for a name or version no package id has.
pub fn index_summary(line: &IndexLine, source: SourceId) -> Option<IndexSummary> {
    let made = match line.status {
        Status::Invalid => None,
        _ => summary(&line.entry, source).ok(),
    };
    let Some(summary) = made else {
        let IndexEntry { name, version, rust_version, .. } = &line.entry;
        let id = PackageId::new(InternedString::new(name), version.clone(), source);
        let rust_version = rust_version.and_then(|it| it.to_string().parse().ok());
        let none = BTreeMap::new();
        return Summary::new(id, Vec::new(), &none, None::<&str>, rust_version)
            .ok()
            .map(IndexSummary::Invalid);
    };
    Some(match line.status {
        Status::Candidate => IndexSummary::Candidate(summary),
        Status::Yanked => IndexSummary::Yanked(summary),
        Status::Unsupported(v) => IndexSummary::Unsupported(summary, v),
        Status::Invalid => unreachable!("an invalid line has no summary made"),
    })
}

/// cargo's `index_package_to_summary`.
fn summary(entry: &IndexEntry, source: SourceId) -> Result<Summary> {
    let id = PackageId::new(InternedString::new(&entry.name), entry.version.clone(), source);
    let deps = entry.deps.iter().map(|it| dependency(it, source)).collect::<Result<Vec<_>>>()?;
    let features: BTreeMap<InternedString, Vec<InternedString>> = entry
        .features
        .iter()
        .map(|(name, values)| (name.into(), values.iter().map(InternedString::from).collect()))
        .collect();
    let rust_version = entry.rust_version.map(|it| it.to_string().parse()).transpose()?;
    let mut summary = Summary::new(id, deps, &features, entry.links.as_deref(), rust_version)?;
    summary.set_checksum(entry.checksum.clone());
    if let Some(pubtime) = &entry.pubtime {
        summary.set_pubtime(pubtime.clone());
    }
    Ok(summary)
}

/// cargo's `registry_dependency_into_dep`: a dependency of a package of the registry
/// `default`.
fn dependency(dep: &IndexDependency, default: SourceId) -> Result<Dependency> {
    if dep.artifact.is_some() {
        bail!(
            "`{}` is an artifact dependency (`-Zbindeps`), which blaze does not read",
            dep.package
        );
    }
    if dep.public && dep.kind != index::DepKind::Normal {
        bail!("`{}` is public but not a normal dependency", dep.package);
    }
    let id = dep.registry.unwrap_or(default);
    let mut it = Dependency::parse(dep.package.as_str(), Some(&dep.req.to_string()), id)?;
    if dep.name_in_toml != dep.package {
        it.set_explicit_name_in_toml(dep.name_in_toml.as_str());
    }
    if !id.is_crates_io() {
        it.set_registry_id(id);
    }
    it.set_optional(dep.optional)
        .set_default_features(dep.default_features)
        .set_features(dep.features.iter().map(String::as_str))
        .set_platform(dep.platform.clone())
        .set_kind(match dep.kind {
            index::DepKind::Normal => DepKind::Normal,
            index::DepKind::Development => DepKind::Development,
            index::DepKind::Build => DepKind::Build,
        })
        .set_public(dep.public);
    Ok(it)
}

impl Registry {
    /// cargo's `RegistryIndex::query_inner`: each version of `name` that `req` matches, with
    /// `source` as its source. Offline, only those whose `.crate` is on disk, when there are
    /// any; when there are none, all of them, so that a dependency nothing builds still
    /// resolves and one that is built fails to download.
    async fn query_index(
        &self,
        name: &str,
        req: &OptVersionReq,
        source: SourceId,
        f: &mut dyn FnMut(IndexSummary),
    ) -> Result<()> {
        let summaries = self.summaries(name).await?;
        let lines: Vec<IndexLine> = summaries.select(|version| req.matches(version)).collect();
        if self.context().net.offline {
            let mut called = false;
            for line in &lines {
                let IndexEntry { name, version, checksum, .. } = &line.entry;
                if self.has_crate(name, version, checksum)
                    && let Some(summary) = index_summary(line, source)
                {
                    called = true;
                    f(summary);
                }
            }
            if called {
                return Ok(());
            }
        }
        lines.iter().filter_map(|line| index_summary(line, source)).for_each(f);
        Ok(())
    }

    /// cargo's `RegistrySource::query`: the versions of the registry's package `dep` asks
    /// for, as `kind` matches them, with `source` (the source `dep` names, which this registry
    /// may replace) as their source.
    pub async fn query_dependency(
        &self,
        dep: &Dependency,
        kind: QueryKind,
        source: SourceId,
        f: &mut dyn FnMut(IndexSummary),
    ) -> Result<()> {
        let name = dep.package_name();
        let mut req = dep.version_req().clone();
        if let Some((_, to)) =
            source.precise_registry_version(&name).filter(|(from, _)| req.matches(from))
        {
            req.precise_to(to);
        }
        let mut called = false;
        // A dependency of the lock file is looked for in the index as it is on disk first,
        // and the index is updated only when it is not there.
        if kind == QueryKind::Exact && req.is_locked() && !self.is_updated() {
            self.query_index(&name, &req, source, &mut |s| {
                if matches!(s, IndexSummary::Candidate(_) | IndexSummary::Yanked(_))
                    && dep.matches(s.summary())
                {
                    called = true;
                    f(s);
                }
            })
            .await?;
            if called {
                return Ok(());
            }
            self.invalidate();
        }
        self.query_index(&name, &req, source, &mut |s| {
            let matched = match kind {
                QueryKind::Exact | QueryKind::RejectedVersions => dep.matches(s.summary()),
                QueryKind::AlternativeNames | QueryKind::Normalized => true,
            };
            if !matched {
                return;
            }
            match s {
                s if kind == QueryKind::RejectedVersions => {
                    called = true;
                    f(s);
                }
                s @ (IndexSummary::Candidate(_) | IndexSummary::Yanked(_)) => {
                    called = true;
                    f(s);
                }
                IndexSummary::Unsupported(..)
                | IndexSummary::Invalid(_)
                | IndexSummary::Offline(_) => {}
            }
        })
        .await?;
        if called || !matches!(kind, QueryKind::AlternativeNames | QueryKind::Normalized) {
            return Ok(());
        }
        // A name spelled with `-` for `_` or `_` for `-`, which the resolver turns into "a
        // similar crate exists".
        for other in [name.replace('-', "_"), name.replace('_', "-")] {
            if other == *name {
                continue;
            }
            self.query_index(&other, &req, source, &mut |s| {
                if !s.is_yanked() || kind == QueryKind::AlternativeNames {
                    f(s);
                }
            })
            .await?;
        }
        Ok(())
    }
}

/// blaze-resolve's [`Registry`](Resolver) over the registries of `sources`, each after its
/// `[source]` replacement, and over `others` for what is not a registry or is replaced by a
/// vendored directory: cargo's `PackageRegistry` over its `SourceMap`, without `[patch]` and
/// the lock file's locks, which blaze-resolve's `PackageRegistry` puts around it.
pub struct IndexRegistry<'a, R> {
    sources: &'a Sources,
    others: R,
}

impl<'a, R: Resolver> IndexRegistry<'a, R> {
    pub fn new(sources: &'a Sources, others: R) -> IndexRegistry<'a, R> {
        IndexRegistry { sources, others }
    }

    /// What answers for the sources that are not registries.
    pub fn others(&self) -> &R {
        &self.others
    }

    /// The source `id`'s packages come from, when it is a registry's index.
    pub fn registry_of(&self, id: SourceId) -> Result<Option<SourceId>> {
        if !id.is_registry() {
            return Ok(None);
        }
        let from = self.sources.map().replacement(id)?;
        Ok(from.is_registry().then_some(from))
    }
}

impl<R: Resolver> Resolver for IndexRegistry<'_, R> {
    async fn query(
        &self,
        dep: &Dependency,
        kind: QueryKind,
        f: &mut dyn FnMut(IndexSummary),
    ) -> Result<()> {
        let id = dep.source_id();
        let Some(from) = self.registry_of(id)? else {
            return self.others.query(dep, kind, f).await;
        };
        let registry = self.sources.registry(from)?;
        // A source asked for at no version the lock file pins is updated, once, as cargo's
        // `PackageRegistry::load` invalidates it.
        if !id.has_locked_precise() && !registry.is_updated() {
            registry.invalidate();
        }
        registry.query_dependency(dep, kind, id, f).await
    }

    /// As cargo's `ReplacedSource::describe`: the source read, and the one it replaces unless
    /// that is crates.io read over its sparse index.
    fn describe_source(&self, source: SourceId) -> String {
        let from = self.sources.map().replacement(source).unwrap_or(source);
        if from == source || (from.is_crates_io() && source.is_crates_io()) {
            describe(from)
        } else {
            format!("{} (which is replacing {source})", describe(from))
        }
    }

    fn is_replaced(&self, source: SourceId) -> bool {
        let from = self.sources.map().replacement(source).unwrap_or(source);
        from != source && !(from.is_crates_io() && source.is_crates_io())
    }
}

/// How cargo's sources describe themselves (each `Source::describe`).
fn describe(id: SourceId) -> String {
    match id.kind() {
        SourceKind::Registry | SourceKind::SparseRegistry | SourceKind::LocalRegistry => {
            id.display_index()
        }
        SourceKind::Directory => match id.url().to_file_path() {
            Ok(path) => format!("directory source `{}`", path.display()),
            Err(()) => id.to_string(),
        },
        SourceKind::Git(_) => format!("Git repository {id}"),
        SourceKind::Path => match id.url().to_file_path() {
            Ok(path) => path.display().to_string(),
            Err(()) => id.to_string(),
        },
    }
}
