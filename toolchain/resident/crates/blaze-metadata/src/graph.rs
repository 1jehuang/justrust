//! The resolve graph of one build: what `cargo build` with a selection of packages and features
//! compiles, as cargo's `unit_dependencies` walks it, folded into `cargo metadata`'s shape, a
//! node a package. `cargo metadata` itself unifies features over every member.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use blaze_resolve::metadata::{Dep, DepKindInfo, MetadataResolve, MetadataResolveNode};
use blaze_resolve::resolver::{FeaturesFor, ResolvedFeatures};
use blaze_resolve::{DepKind, InternedString, PackageId, Platform, Resolve};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::summary::Read;

#[derive(Default)]
struct Node {
    features: BTreeSet<InternedString>,
    deps: BTreeMap<PackageId, (InternedString, BTreeSet<DepKindInfo>)>,
}

/// Whether a `cargo build` of the member `pkg` builds anything: its library, or a binary
/// whose `required-features` it is built with. One with only tests, examples or benchmarks,
/// or binaries that need features it lacks, has no units.
fn builds_anything(
    pkg: PackageId,
    features: &ResolvedFeatures,
    reads: &FxHashMap<PackageId, Read>,
) -> bool {
    let Some(read) = reads.get(&pkg) else { return false };
    let on =
        features.activated_features_unverified(pkg, FeaturesFor::NormalOrDev).unwrap_or_default();
    // A feature of a dependency (`dep/feature`) is taken to be on.
    let has =
        |required: &String| required.contains('/') || on.iter().any(|it| it.as_str() == required);
    read.lib.is_some() || read.bins.iter().any(|it| it.iter().all(has))
}

/// From each of `roots`, the packages the build reaches, each with the features it is built
/// with (both builds of one built for the target and for the host) and the dependencies it
/// is built against. `dev_units` is whether the roots' tests, examples and benchmarks are
/// built; `activated` whether a `[target.'cfg(..)']` platform holds.
pub(crate) fn of_build(
    resolve: &Resolve,
    features: &ResolvedFeatures,
    roots: &[PackageId],
    dev_units: bool,
    reads: &FxHashMap<PackageId, Read>,
    activated: &dyn Fn(&Platform) -> bool,
    root: Option<PackageId>,
) -> Result<MetadataResolve> {
    let mut nodes: BTreeMap<PackageId, Node> = BTreeMap::new();
    let mut seen = FxHashSet::default();
    let mut queue = Vec::new();
    for &member in roots {
        if !dev_units && !builds_anything(member, features, reads) {
            continue;
        }
        queue.push((member, FeaturesFor::NormalOrDev));
        if reads.get(&member).is_some_and(|it| it.any_proc_macro) {
            queue.push((member, FeaturesFor::HostDep));
        }
    }
    while let Some((pkg, fk)) = queue.pop() {
        if !seen.insert((pkg, fk)) {
            continue;
        }
        let node = nodes.entry(pkg).or_default();
        node.features.extend(features.activated_features_unverified(pkg, fk).unwrap_or_default());
        let is_root = roots.contains(&pkg);
        let build_script = reads.get(&pkg).is_some_and(|it| it.build_script);
        for (dep_id, deps) in resolve.deps(pkg) {
            let Some(read) = reads.get(&dep_id) else { continue };
            let Some(lib) = read.lib else { continue };
            let passing: Vec<_> = deps
                .iter()
                .filter(|dep| {
                    let kind_built = match dep.kind() {
                        DepKind::Development => is_root && dev_units,
                        DepKind::Normal => true,
                        DepKind::Build => build_script,
                    };
                    let name = dep.name_in_toml();
                    let enabled = !dep.is_optional()
                        || features.is_dep_activated(pkg, fk, name)
                        || (dep.kind() == DepKind::Build
                            && features.is_dep_activated(pkg, FeaturesFor::HostDep, name));
                    kind_built && dep.platform().is_none_or(activated) && enabled
                })
                .collect();
            if passing.is_empty() {
                continue;
            }
            let (name, _) = resolve.extern_crate_name_and_dep_name(pkg, dep_id, lib.as_str())?;
            let entry = node.deps.entry(dep_id).or_insert_with(|| (name, BTreeSet::new()));
            for dep in &passing {
                entry.1.insert(DepKindInfo { kind: dep.kind(), target: dep.platform().cloned() });
                let host = dep.kind() == DepKind::Build || read.proc_macro_lib;
                queue.push((dep_id, if host { FeaturesFor::HostDep } else { fk }));
            }
        }
    }
    let nodes = nodes.into_iter().map(|(id, node)| {
        let deps: Vec<Dep> = node
            .deps
            .into_iter()
            .map(|(pkg_id, (name, kinds))| Dep {
                name,
                pkg: pkg_id.to_spec(),
                pkg_id,
                dep_kinds: kinds.into_iter().collect(),
            })
            .collect();
        MetadataResolveNode {
            id: id.to_spec(),
            dependencies: deps.iter().map(|it| it.pkg_id.to_spec()).collect(),
            deps,
            features: node.features.into_iter().collect(),
        }
    });
    Ok(MetadataResolve { nodes: nodes.collect(), root: root.map(|it| it.to_spec()) })
}
