//! The graph `cargo tree` prints: `src/ops/cargo_tree/graph.rs` of cargo 1.101.0 (cad07cf8e5),
//! on blaze-resolve's resolve and features. blaze reads no artifact dependencies, so a package
//! is built for the host or for the target asked, never for an artifact's.

use std::collections::BTreeSet;

use blaze_metadata::Read;
use blaze_resolve::resolver::{CliFeatures, CompileKind, FeaturesFor, ResolvedFeatures};
use blaze_resolve::{
    DepKind, Dependency, FeatureMap, FeatureValue, InternedString, PackageId, PackageIdSpec,
    Resolve, Summary,
};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{Kinds, Options, Target};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NodeId(usize);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub(crate) enum Node {
    Package {
        package_id: PackageId,
        /// Features that are enabled on this package.
        features: Vec<InternedString>,
        kind: CompileKind,
    },
    Feature {
        /// Index of the package node this feature is for.
        node_index: NodeId,
        /// Name of the feature.
        name: InternedString,
    },
}

#[derive(Debug, Copy, Hash, Eq, Clone, PartialEq)]
pub(crate) struct Edge {
    kind: EdgeKind,
    node: NodeId,
    public: bool,
}

impl Edge {
    pub(crate) fn kind(&self) -> EdgeKind {
        self.kind
    }

    pub(crate) fn node(&self) -> NodeId {
        self.node
    }
}

/// The kind of edge, for separating dependencies into different sections.
#[derive(Debug, Copy, Hash, Eq, Clone, PartialEq)]
pub enum EdgeKind {
    Dep(DepKind),
    Feature,
}

/// Set of outgoing edges for a single node, by kind, so the output can show sections like
/// `[build-dependencies]`.
#[derive(Clone, Debug, Default)]
struct Edges(FxHashMap<EdgeKind, Vec<Edge>>);

impl Edges {
    /// Adds an edge pointing to the given node.
    fn add_edge(&mut self, edge: Edge) {
        let indexes = self.0.entry(edge.kind()).or_default();
        if !indexes.contains(&edge) {
            indexes.push(edge)
        }
    }

    fn all(&self) -> impl Iterator<Item = &Edge> + '_ {
        self.0.values().flatten()
    }

    fn of_kind(&self, kind: &EdgeKind) -> &[Edge] {
        self.0.get(kind).map(Vec::as_slice).unwrap_or_default()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A graph of dependencies.
pub(crate) struct Graph<'a> {
    nodes: Vec<Node>,
    /// `edges[i]` is the set of outgoing edges of `nodes[i]`.
    edges: Vec<Edges>,
    index: FxHashMap<Node, NodeId>,
    packages: &'a FxHashMap<PackageId, Read>,
    /// The feature nodes `--features` added.
    cli_features: FxHashSet<NodeId>,
    /// Of each package node, its dependencies by the name it gives them, each `(the
    /// dependency's node, whether it is optional)`, for `dep_name/feat_name` features.
    dep_name_map: FxHashMap<NodeId, FxHashMap<InternedString, BTreeSet<(NodeId, bool)>>>,
}

impl<'a> Graph<'a> {
    fn new(packages: &'a FxHashMap<PackageId, Read>) -> Graph<'a> {
        Graph {
            nodes: Vec::new(),
            edges: Vec::new(),
            index: FxHashMap::default(),
            packages,
            cli_features: FxHashSet::default(),
            dep_name_map: FxHashMap::default(),
        }
    }

    /// Adds a new node to the graph, returning its new index.
    fn add_node(&mut self, node: Node) -> NodeId {
        let from_index = NodeId(self.nodes.len());
        self.nodes.push(node.clone());
        self.edges.push(Edges::default());
        self.index.insert(node, from_index);
        from_index
    }

    /// The nodes `from` points to by edges of `kind`, sorted for consistent output.
    pub(crate) fn edges_of_kind(&self, from: NodeId, kind: &EdgeKind) -> Vec<Edge> {
        let mut edges = self.edges[from.0].of_kind(kind).to_owned();
        edges.sort_unstable_by(|a, b| self.node(a.node()).cmp(self.node(b.node())));
        edges
    }

    fn edges_mut(&mut self, from: NodeId) -> &mut Edges {
        &mut self.edges[from.0]
    }

    pub(crate) fn has_outgoing_edges(&self, index: NodeId) -> bool {
        !self.edges[index.0].is_empty()
    }

    pub(crate) fn node(&self, index: NodeId) -> &Node {
        &self.nodes[index.0]
    }

    /// The package nodes of `package_ids`, sorted for consistent output.
    pub(crate) fn indexes_from_ids(&self, package_ids: &[PackageId]) -> Vec<NodeId> {
        let mut result: Vec<(&Node, NodeId)> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| match node {
                Node::Package { package_id, .. } => package_ids.contains(package_id),
                Node::Feature { .. } => false,
            })
            .map(|(i, node)| (node, NodeId(i)))
            .collect();
        result.sort_unstable();
        result.into_iter().map(|(_, i)| i).collect()
    }

    pub(crate) fn package(&self, id: PackageId) -> &'a Read {
        &self.packages[&id]
    }

    fn package_id_for_index(&self, index: NodeId) -> PackageId {
        match self.node(index) {
            Node::Package { package_id, .. } => *package_id,
            Node::Feature { .. } => panic!("unexpected feature node"),
        }
    }

    pub(crate) fn is_cli_feature(&self, index: NodeId) -> bool {
        self.cli_features.contains(&index)
    }

    /// A new graph of the nodes reachable from `roots`.
    pub(crate) fn from_reachable(&self, roots: &[NodeId]) -> Graph<'a> {
        // Graph built with features does not (yet) support --duplicates.
        assert!(self.dep_name_map.is_empty());
        let mut new_graph = Graph::new(self.packages);
        // Maps old index to new index. None if not yet visited.
        let mut remap: Vec<Option<NodeId>> = vec![None; self.nodes.len()];

        fn visit(
            graph: &Graph<'_>,
            new_graph: &mut Graph<'_>,
            remap: &mut Vec<Option<NodeId>>,
            index: NodeId,
        ) -> NodeId {
            if let Some(new_index) = remap[index.0] {
                return new_index;
            }
            let new_from = new_graph.add_node(graph.node(index).clone());
            remap[index.0] = Some(new_from);
            for edge in graph.edges[index.0].all() {
                let new_to_index = visit(graph, new_graph, remap, edge.node());
                let new_edge = Edge { kind: edge.kind(), node: new_to_index, public: edge.public };
                new_graph.edges_mut(new_from).add_edge(new_edge);
            }
            new_from
        }

        for root in roots {
            visit(self, &mut new_graph, &mut remap, *root);
        }
        new_graph
    }

    /// Inverts the direction of all edges.
    pub(crate) fn invert(&mut self) {
        let mut new_edges = vec![Edges::default(); self.edges.len()];
        for (from_idx, node_edges) in self.edges.iter().enumerate() {
            for edge in node_edges.all() {
                let new_edge =
                    Edge { kind: edge.kind(), node: NodeId(from_idx), public: edge.public };
                new_edges[edge.node().0].add_edge(new_edge);
            }
        }
        self.edges = new_edges;
    }

    /// The nodes of packages that are in the graph more than once (same name, with other
    /// versions, features or sources), not counting one on both the host and the target.
    pub(crate) fn find_duplicates(&self) -> Vec<NodeId> {
        assert!(self.dep_name_map.is_empty());
        let mut packages: FxHashMap<InternedString, Vec<(&Node, NodeId)>> = FxHashMap::default();
        for (i, node) in self.nodes.iter().enumerate() {
            if let Node::Package { package_id, .. } = node {
                packages.entry(package_id.name()).or_default().push((node, NodeId(i)));
            }
        }
        let mut dupes: Vec<(&Node, NodeId)> = packages
            .into_values()
            .filter(|indexes| {
                let distinct: FxHashSet<Node> = indexes
                    .iter()
                    .map(|(node, _)| match node {
                        Node::Package { package_id, features, .. } => Node::Package {
                            package_id: *package_id,
                            features: features.clone(),
                            kind: CompileKind::Host,
                        },
                        Node::Feature { .. } => unreachable!(),
                    })
                    .collect();
                distinct.len() > 1
            })
            .flatten()
            .collect();
        dupes.sort_unstable();
        dupes.into_iter().map(|(_, i)| i).collect()
    }
}

/// Builds the graph of `members_with_features`, the members and the features the command line
/// gives each.
pub(crate) fn build<'a>(
    resolve: &Resolve,
    resolved_features: &ResolvedFeatures,
    mut members_with_features: Vec<(&Summary, CliFeatures)>,
    kinds: &Kinds,
    requested_kinds: &[CompileKind],
    packages: &'a FxHashMap<PackageId, Read>,
    opts: &Options,
) -> Graph<'a> {
    let mut graph = Graph::new(packages);
    members_with_features.sort_unstable_by_key(|(member, _)| member.package_id());
    for (member, cli_features) in members_with_features {
        let member_id = member.package_id();
        let proc_macro = packages.get(&member_id).is_some_and(|it| it.any_proc_macro);
        let features_for = match proc_macro {
            true => FeaturesFor::HostDep,
            false => FeaturesFor::NormalOrDev,
        };
        for kind in requested_kinds {
            let member_index = add_pkg(
                &mut graph,
                resolve,
                resolved_features,
                member_id,
                features_for,
                kinds,
                *kind,
                opts,
            );
            if opts.graph_features {
                let fmap = resolve.summary(member_id).features();
                add_cli_features(&mut graph, member_index, &cli_features, fmap);
            }
        }
    }
    if opts.graph_features {
        add_internal_features(&mut graph, resolve);
    }
    graph
}

/// Adds a single package node (if it does not already exist), and recursively all of its
/// dependencies. Returns the index to the package node.
fn add_pkg(
    graph: &mut Graph<'_>,
    resolve: &Resolve,
    resolved_features: &ResolvedFeatures,
    package_id: PackageId,
    features_for: FeaturesFor,
    kinds: &Kinds,
    requested_kind: CompileKind,
    opts: &Options,
) -> NodeId {
    let node_features = resolved_features.activated_features(package_id, features_for);
    let node_kind = match features_for {
        FeaturesFor::HostDep => CompileKind::Host,
        FeaturesFor::NormalOrDev => requested_kind,
    };
    let node = Node::Package { package_id, features: node_features, kind: node_kind };
    if let Some(idx) = graph.index.get(&node) {
        return *idx;
    }
    let from_index = graph.add_node(node);
    // The dep name map, later used for foo/bar feature lookups.
    let mut dep_name_map: FxHashMap<InternedString, BTreeSet<(NodeId, bool)>> =
        FxHashMap::default();
    let mut deps: Vec<_> = resolve.deps(package_id).collect();
    deps.sort_unstable_by_key(|(dep_id, _)| *dep_id);
    let show_all_targets = opts.target == Target::All;
    for (dep_id, deps) in deps {
        let mut deps: Vec<&Dependency> = deps
            .iter()
            // This filter is *similar* to the one found in `unit_dependencies::compute_deps`.
            .filter(|dep| {
                let kind = match (node_kind, dep.kind()) {
                    (CompileKind::Host, _) => CompileKind::Host,
                    (_, DepKind::Build) => CompileKind::Host,
                    (_, DepKind::Normal) => node_kind,
                    (_, DepKind::Development) => node_kind,
                };
                // Filter out inactivated targets.
                if !show_all_targets
                    && !dep.platform().is_none_or(|platform| kinds.activated(platform, kind))
                {
                    return false;
                }
                if !opts.edge_kinds.contains(&EdgeKind::Dep(dep.kind())) {
                    return false;
                }
                if opts.no_proc_macro && graph.package(dep_id).any_proc_macro {
                    return false;
                }
                if opts.public && !dep.is_public() {
                    return false;
                }
                // An optional dependency the feature resolver does not enable is not used.
                !dep.is_optional()
                    || resolved_features.is_dep_activated(
                        package_id,
                        features_for,
                        dep.name_in_toml(),
                    )
            })
            .collect();

        // This dependency is eliminated from the dependency tree under the current target and
        // feature set.
        if deps.is_empty() {
            continue;
        }

        deps.sort_unstable_by_key(|dep| (dep.kind(), dep.name_in_toml()));
        let dep_proc_macro = graph.package(dep_id).any_proc_macro;

        for dep in deps {
            // A host dependency's dependencies, build dependencies too, are built for the host;
            // a normal one's are built as cargo's old rules have them: normal and dev
            // dependencies for the dependent's platform, build dependencies for the host.
            let dep_features_for = match features_for {
                FeaturesFor::HostDep => FeaturesFor::HostDep,
                FeaturesFor::NormalOrDev if dep.is_build() || dep_proc_macro => {
                    FeaturesFor::HostDep
                }
                FeaturesFor::NormalOrDev => FeaturesFor::NormalOrDev,
            };
            let dep_index = add_pkg(
                graph,
                resolve,
                resolved_features,
                dep_id,
                dep_features_for,
                kinds,
                requested_kind,
                opts,
            );
            let new_edge =
                Edge { kind: EdgeKind::Dep(dep.kind()), node: dep_index, public: dep.is_public() };
            if opts.graph_features {
                // Add the dependency node with feature nodes in-between.
                dep_name_map
                    .entry(dep.name_in_toml())
                    .or_default()
                    .insert((dep_index, dep.is_optional()));
                if dep.uses_default_features() {
                    add_feature(graph, InternedString::new("default"), Some(from_index), new_edge);
                }
                for feature in dep.features().iter() {
                    add_feature(graph, *feature, Some(from_index), new_edge);
                }
                if !dep.uses_default_features() && dep.features().is_empty() {
                    // No features, use a direct connection.
                    graph.edges_mut(from_index).add_edge(new_edge);
                }
            } else {
                graph.edges_mut(from_index).add_edge(new_edge);
            }
        }
    }
    if opts.graph_features {
        assert!(graph.dep_name_map.insert(from_index, dep_name_map).is_none());
    }

    from_index
}

/// Adds a feature node between two nodes, `from -Edge-> featname -Edge::Feature-> to`.
///
/// Returns `(missing, index)`: whether the feature node is new, and its index.
fn add_feature(
    graph: &mut Graph<'_>,
    name: InternedString,
    from: Option<NodeId>,
    to: Edge,
) -> (bool, NodeId) {
    // `to` *must* point to a package node.
    assert!(matches! {graph.node(to.node()), Node::Package{..}});
    let node = Node::Feature { node_index: to.node(), name };
    let (missing, node_index) = match graph.index.get(&node) {
        Some(idx) => (false, *idx),
        None => (true, graph.add_node(node)),
    };
    if let Some(from) = from {
        let from_edge = Edge { kind: to.kind(), node: node_index, public: to.public };
        graph.edges_mut(from).add_edge(from_edge);
    }
    let to_edge = Edge { kind: EdgeKind::Feature, node: to.node(), public: true };
    graph.edges_mut(node_index).add_edge(to_edge);
    (missing, node_index)
}

/// Adds nodes for the features the command line asks of the member, as roots: they come from
/// the outside world, and usually only appear with `--invert`.
fn add_cli_features(
    graph: &mut Graph<'_>,
    package_index: NodeId,
    cli_features: &CliFeatures,
    feature_map: &FeatureMap,
) {
    // Recursive enabling of features is handled by add_internal_features.
    let mut to_add: BTreeSet<FeatureValue> = BTreeSet::new();
    if cli_features.all_features {
        to_add.extend(feature_map.keys().map(|feat| FeatureValue::Feature(*feat)));
    }
    if cli_features.uses_default_features {
        to_add.insert(FeatureValue::Feature(InternedString::new("default")));
    }
    to_add.extend(cli_features.features.iter().cloned());

    for fv in to_add {
        match fv {
            FeatureValue::Feature(feature) => {
                let feature_edge =
                    Edge { kind: EdgeKind::Feature, node: package_index, public: true };
                let index = add_feature(graph, feature, None, feature_edge).1;
                graph.cli_features.insert(index);
            }
            // This is enforced by CliFeatures.
            FeatureValue::Dep { .. } => panic!("unexpected cli dep feature {fv}"),
            FeatureValue::DepFeature { dep_name, dep_feature, weak: _ } => {
                let dep_connections = match graph.dep_name_map[&package_index].get(&dep_name) {
                    Some(dep_connections) => dep_connections.clone(),
                    // `--features bar?/feat` where `bar` is not activated is ignored; one that
                    // is not weak was refused before.
                    None => continue,
                };
                for (dep_index, is_optional) in dep_connections {
                    if is_optional {
                        // Activate the optional dep on self.
                        let feature_edge =
                            Edge { kind: EdgeKind::Feature, node: package_index, public: true };
                        let index = add_feature(graph, dep_name, None, feature_edge).1;
                        graph.cli_features.insert(index);
                    }
                    let dep_edge = Edge { kind: EdgeKind::Feature, node: dep_index, public: true };
                    let index = add_feature(graph, dep_feature, None, dep_edge).1;
                    graph.cli_features.insert(index);
                }
            }
        }
    }
}

/// Recursively adds connections between features in the `[features]` table of every package.
fn add_internal_features(graph: &mut Graph<'_>, resolve: &Resolve) {
    // Features already activated by dependencies or command-line.
    let feature_nodes: Vec<(PackageId, NodeId, NodeId, InternedString)> = graph
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, node)| match node {
            Node::Package { .. } => None,
            Node::Feature { node_index, name } => {
                let package_id = graph.package_id_for_index(*node_index);
                Some((package_id, *node_index, NodeId(i), *name))
            }
        })
        .collect();

    for (package_id, package_index, feature_index, feature_name) in feature_nodes {
        add_feature_rec(graph, resolve, feature_name, package_id, feature_index, package_index);
    }
}

/// Recursively add feature nodes for all features enabled by the given feature. `from` is the
/// node that enables it, `package_index` the package node it is of.
fn add_feature_rec(
    graph: &mut Graph<'_>,
    resolve: &Resolve,
    feature_name: InternedString,
    package_id: PackageId,
    from: NodeId,
    package_index: NodeId,
) {
    let feature_map = resolve.summary(package_id).features();
    let Some(fvs) = feature_map.get(&feature_name) else {
        return;
    };
    for fv in fvs {
        match fv {
            FeatureValue::Feature(dep_name) => {
                let feature_edge =
                    Edge { kind: EdgeKind::Feature, node: package_index, public: true };
                let (missing, feat_index) = add_feature(graph, *dep_name, Some(from), feature_edge);
                // Don't recurse if the edge already exists, to deal with cycles.
                if missing {
                    add_feature_rec(
                        graph,
                        resolve,
                        *dep_name,
                        package_id,
                        feat_index,
                        package_index,
                    );
                }
            }
            // Dependencies are already shown in the graph as dep edges.
            FeatureValue::Dep { .. } => {}
            FeatureValue::DepFeature { dep_name, dep_feature, weak } => {
                // `weak` is mostly handled where the graph is built, in `is_dep_activated`;
                // here it only says whether the feature of the dependency's name is
                // activated on self.
                let Some(dep_indexes) = graph.dep_name_map[&package_index].get(dep_name).cloned()
                else {
                    continue;
                };
                for (dep_index, is_optional) in dep_indexes {
                    let dep_pkg_id = graph.package_id_for_index(dep_index);
                    if is_optional && !weak {
                        // Activate the optional dep on self.
                        let feature_edge =
                            Edge { kind: EdgeKind::Feature, node: package_index, public: true };
                        add_feature(graph, *dep_name, Some(from), feature_edge);
                    }
                    let dep_edge = Edge { kind: EdgeKind::Feature, node: dep_index, public: true };
                    let (missing, feat_index) =
                        add_feature(graph, *dep_feature, Some(from), dep_edge);
                    if missing {
                        add_feature_rec(
                            graph,
                            resolve,
                            *dep_feature,
                            dep_pkg_id,
                            feat_index,
                            dep_index,
                        );
                    }
                }
            }
        }
    }
}

/// The ids of the packages of `resolve` that `specs` name, each spec one: cargo's
/// `Resolve::specs_to_ids`.
pub(crate) fn specs_to_ids(
    resolve: &Resolve,
    specs: &[PackageIdSpec],
) -> anyhow::Result<Vec<PackageId>> {
    let all_ids: Vec<PackageId> = resolve.iter().collect();
    specs.iter().map(|spec| spec.query(&all_ids)).collect()
}
