//! `cargo tree`, made by blaze with no cargo: `src/ops/cargo_tree/` of cargo 1.101.0
//! (cad07cf8e5), on the workspace blaze-metadata resolves and blaze-resolve's feature
//! resolver. It prints what cargo prints, line for line; `harness/own_tree.py` holds it to
//! that.

mod format;
mod graph;

use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use blaze_manifest::Workspace;
use blaze_metadata::{Asked, Read, Resolved};
use blaze_resolve::resolver::{
    CompileKind, FeatureOpts, FeatureResolver, ForceAllTargets, HasDevUnits, PackageTargets,
    members_with_features,
};
use blaze_resolve::{Cfg, InternedString, PackageId, PackageIdSpec, Platform};
use rustc_hash::{FxHashMap, FxHashSet};

pub use blaze_resolve::DepKind;
pub use graph::EdgeKind;
use graph::{Graph, Node, NodeId};

/// What `cargo tree` is asked, as its flags say it.
pub struct Options {
    /// The packages to display the tree for.
    pub packages: Packages,
    /// The platform to filter for.
    pub target: Target,
    /// The dependency kinds to display.
    pub edge_kinds: FxHashSet<EdgeKind>,
    pub invert: Vec<String>,
    /// The packages to prune from the display of the dependency tree.
    pub pkgs_to_prune: Vec<String>,
    /// The style of prefix for each line.
    pub prefix: Prefix,
    /// Whether duplicates are repeated, rather than marked with `(*)` and their dependencies
    /// left out.
    pub no_dedupe: bool,
    /// Only the packages that are in the graph in more than one version. Implies `invert`.
    pub duplicates: bool,
    /// How each package is displayed.
    pub format: String,
    /// Features in the tree as nodes of their own.
    pub graph_features: bool,
    pub display_depth: DisplayDepth,
    /// Excludes proc-macro dependencies.
    pub no_proc_macro: bool,
    /// Include only public dependencies.
    pub public: bool,
    /// `│├└─` rather than `|`-`.
    pub unicode: bool,
    /// Whether the lines are colored, as they are on a terminal.
    pub color: bool,
}

/// `-p`, `--workspace` and `--exclude`: cargo's `Packages`.
pub enum Packages {
    /// None of them: the default members.
    Default,
    /// `--workspace`, with the `-p`s given too.
    All(Vec<String>),
    /// `--workspace --exclude`.
    OptOut(Vec<String>),
    /// `-p`.
    Packages(Vec<String>),
}

#[derive(PartialEq)]
pub enum Target {
    Host,
    Specific(Vec<String>),
    All,
}

impl Target {
    pub fn from_cli(targets: Vec<String>) -> Target {
        match targets.len() {
            0 => Target::Host,
            1 if targets[0] == "all" => Target::All,
            _ => Target::Specific(targets),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Prefix {
    None,
    Indent,
    Depth,
}

#[derive(Clone, Copy)]
pub enum DisplayDepth {
    MaxDisplayDepth(u32),
    Workspace,
}

/// A platform a package is built for: its triple and its `cfg`, as blaze answers
/// `rustc --print cfg` itself.
pub struct Platformed {
    pub triple: String,
    pub cfg: Vec<Cfg>,
}

/// The host, and the `--target`s asked (none: the host only).
pub struct Kinds {
    pub host: Platformed,
    pub targets: Vec<Platformed>,
}

impl Kinds {
    fn requested(&self) -> Vec<CompileKind> {
        match self.targets.is_empty() {
            true => vec![CompileKind::Host],
            false => self
                .targets
                .iter()
                .map(|it| CompileKind::Target(InternedString::new(&it.triple)))
                .collect(),
        }
    }

    fn activated(&self, platform: &Platform, kind: CompileKind) -> bool {
        let of = match kind {
            CompileKind::Host => Some(&self.host),
            CompileKind::Target(triple) => self.targets.iter().find(|it| it.triple == *triple),
        };
        of.is_some_and(|it| platform.matches(&it.triple, &it.cfg))
    }
}

struct Targets<'a> {
    packages: &'a FxHashMap<PackageId, Read>,
    kinds: &'a Kinds,
}

impl PackageTargets for Targets<'_> {
    fn has_any_proc_macro(&self, package_id: PackageId) -> bool {
        self.packages.get(&package_id).is_some_and(|it| it.any_proc_macro)
    }

    fn has_proc_macro_lib(&self, package_id: PackageId) -> bool {
        self.packages.get(&package_id).is_some_and(|it| it.proc_macro_lib)
    }

    fn platform_activated(&self, platform: &Platform, kind: CompileKind) -> bool {
        self.kinds.activated(platform, kind)
    }
}

struct Symbols {
    down: &'static str,
    tee: &'static str,
    ell: &'static str,
    right: &'static str,
}

static UTF8_SYMBOLS: Symbols = Symbols { down: "│", tee: "├", ell: "└", right: "─" };

static ASCII_SYMBOLS: Symbols = Symbols { down: "|", tee: "|", ell: "`", right: "-" };

const RESET: &str = "\x1b[0m";

/// The color of a line to a dependency of `kind`: cargo's `style::DEP_*`.
fn edge_line_color(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Dep(DepKind::Normal) => "\x1b[2m",
        EdgeKind::Dep(DepKind::Build) => "\x1b[1m\x1b[34m",
        EdgeKind::Dep(DepKind::Development) => "\x1b[1m\x1b[36m",
        EdgeKind::Feature => "\x1b[1m\x1b[35m",
    }
}

/// The tree `opts` asks of the workspace of `asked`, printed into `out`; what cargo warns of
/// goes to `warn`. `asked.features` and its siblings are the command line's.
pub fn print_tree(
    asked: &Asked<'_>,
    opts: &Options,
    kinds: &Kinds,
    out: &mut String,
    warn: &mut dyn FnMut(&str),
) -> Result<()> {
    let has_dev = match opts.edge_kinds.contains(&EdgeKind::Dep(DepKind::Development)) {
        true => HasDevUnits::Yes,
        false => HasDevUnits::No,
    };
    let force_all = match opts.target == Target::All {
        true => ForceAllTargets::Yes,
        false => ForceAllTargets::No,
    };
    let build = blaze_metadata::Build {
        packages: &[],
        workspace: false,
        excludes: &[],
        dev_units: has_dev == HasDevUnits::Yes,
    };
    let asked = Asked {
        build: Some(build),
        env: asked.env.clone(),
        platform: None,
        store: asked.store.clone(),
        ..*asked
    };
    let warned = std::cell::RefCell::new(Vec::new());
    let pick = |ws: &Workspace<'_>, member_ids: &[PackageId]| {
        package_id_specs(&opts.packages, ws, member_ids, &mut |it| {
            warned.borrow_mut().push(it.to_owned());
        })
    };
    let printed = blaze_metadata::resolved(&asked, Some(&pick), |resolved| {
        let Resolved { resolve, members, member_ids, cli_features, behavior, packages, .. } =
            resolved;
        let requested_kinds = kinds.requested();
        // One tree per feature resolve: per package under `resolver.feature-unification =
        // "package"`.
        for roots in resolved.feature_resolves()? {
            let feature_opts = FeatureOpts::new(behavior, has_dev, force_all);
            let targets = Targets { packages, kinds };
            let features = FeatureResolver::resolve(
                &targets,
                resolve,
                &roots.members,
                &requested_kinds,
                feature_opts,
            )?;
            let mut graph = graph::build(
                resolve,
                &features,
                members_with_features(&members, &roots.specs, cli_features)?,
                kinds,
                &requested_kinds,
                packages,
                opts,
            );

            let root_specs = match opts.invert.is_empty() {
                true => roots.specs,
                false => {
                    opts.invert.iter().map(|p| PackageIdSpec::parse(p)).collect::<Result<_>>()?
                }
            };
            let root_ids = graph::specs_to_ids(resolve, &root_specs)?;
            let mut root_indexes = graph.indexes_from_ids(&root_ids);
            if opts.duplicates {
                // `-d -p foo` only shows duplicates within foo's subtree.
                graph = graph.from_reachable(&root_indexes);
                root_indexes = graph.find_duplicates();
            }
            if !opts.invert.is_empty() || opts.duplicates {
                graph.invert();
            }

            let all_ids: Vec<PackageId> = resolve.iter().collect();
            let pkgs_to_prune = opts
                .pkgs_to_prune
                .iter()
                .map(|p| {
                    // Said if the spec is not of the resolved graph.
                    let spec = PackageIdSpec::parse(p)?;
                    spec.query(&all_ids)?;
                    Ok(spec)
                })
                .collect::<Result<Vec<PackageIdSpec>>>()?;

            if root_indexes.is_empty() {
                warn(
                    "nothing to print.\n\n\
                     To find dependencies that require specific target platforms, \
                     try to use option `--target all` first, and then narrow your search scope accordingly.",
                );
                continue;
            }
            let printer = Printer {
                graph: &graph,
                format: format::Pattern::new(&opts.format)
                    .with_context(|| format!("tree format `{}` not valid", opts.format))?,
                symbols: if opts.unicode { &UTF8_SYMBOLS } else { &ASCII_SYMBOLS },
                pkgs_to_prune: &pkgs_to_prune,
                member_ids,
                opts,
            };
            printer.print(root_indexes, out);
        }
        Ok(())
    });
    for it in warned.into_inner() {
        warn(&it);
    }
    printed
}

/// The specs of the packages `packages` selects: cargo's `Packages::to_package_id_specs`.
fn package_id_specs(
    packages: &Packages,
    ws: &Workspace<'_>,
    member_ids: &[PackageId],
    warn: &mut dyn FnMut(&str),
) -> Result<Vec<PackageIdSpec>> {
    let glob = |name: &str| name.contains(['*', '?', '[', ']']);
    let unread = |names: &[String]| match names.iter().find(|it| glob(it)) {
        Some(pattern) => bail!("blaze does not read package patterns like `{pattern}` yet"),
        None => Ok(()),
    };
    let every = || member_ids.iter().map(|id| id.to_spec()).collect();
    Ok(match packages {
        Packages::All(_) => every(),
        Packages::OptOut(opt_out) => {
            unread(opt_out)?;
            let ids: Vec<PackageIdSpec> =
                opt_out.iter().map(|it| PackageIdSpec::parse(it)).collect::<Result<_>>()?;
            let excluded = |id: &PackageId| ids.iter().any(|spec| spec.matches(*id));
            let not_found: Vec<String> = ids
                .iter()
                .filter(|spec| !member_ids.iter().any(|id| spec.matches(*id)))
                .map(|spec| spec.to_string())
                .collect();
            if !not_found.is_empty() {
                warn(&format!(
                    "excluded package(s) `{}` not found in workspace `{}`",
                    not_found.join(", "),
                    ws.root().display()
                ));
            }
            member_ids.iter().filter(|id| !excluded(id)).map(|id| id.to_spec()).collect()
        }
        Packages::Packages(opt_in) => {
            unread(opt_in)?;
            let mut specs: Vec<PackageIdSpec> =
                opt_in.iter().map(|it| PackageIdSpec::parse(it)).collect::<Result<_>>()?;
            specs.sort_by_key(|it| it.to_string());
            specs.dedup_by_key(|it| it.to_string());
            specs
        }
        Packages::Default => {
            let named = |name: &str| member_ids.iter().find(|id| id.name().as_str() == name);
            let defaults = ws.default_members().map(|(_, m)| named(&m.package_id.name));
            let defaults: Option<Vec<&PackageId>> = defaults.collect();
            defaults
                .context("a default member that is not a member")?
                .iter()
                .map(|id| id.to_spec())
                .collect()
        }
    })
}

struct Printer<'a> {
    graph: &'a Graph<'a>,
    format: format::Pattern,
    symbols: &'static Symbols,
    pkgs_to_prune: &'a [PackageIdSpec],
    member_ids: &'a [PackageId],
    opts: &'a Options,
}

/// Of each level printed, its line's color and whether a later sibling continues it.
type Levels = Vec<(&'static str, bool)>;

impl Printer<'_> {
    fn paint(&self, color: &str, text: &str) -> String {
        match self.opts.color {
            true => format!("{color}{text}{RESET}"),
            false => text.to_owned(),
        }
    }

    /// Prints a tree for each given root.
    fn print(&self, roots: Vec<NodeId>, out: &mut String) {
        // A dependency already printed is marked `(*)` (unless --no-dedupe).
        let mut visited_deps = FxHashSet::default();
        for (i, root_index) in roots.into_iter().enumerate() {
            if i != 0 {
                out.push('\n');
            }
            // A stack of levels used to determine where | symbols should appear when printing
            // a line.
            let mut levels_continue = vec![];
            // A Node for each level, to detect cycles when --no-dedupe is used.
            let mut print_stack = vec![];
            self.print_node(
                root_index,
                &mut visited_deps,
                &mut levels_continue,
                &mut print_stack,
                out,
            );
        }
    }

    /// Prints a package and all of its dependencies.
    fn print_node(
        &self,
        node_index: NodeId,
        visited_deps: &mut FxHashSet<NodeId>,
        levels_continue: &mut Levels,
        print_stack: &mut Vec<NodeId>,
        out: &mut String,
    ) {
        let new = self.opts.no_dedupe || visited_deps.insert(node_index);
        let symbols = self.symbols;
        match self.opts.prefix {
            Prefix::Depth => {
                let _ = write!(out, "{}", levels_continue.len());
            }
            Prefix::Indent => {
                if let Some(((last_style, last_continues), rest)) = levels_continue.split_last() {
                    for (style, continues) in rest {
                        let c = if *continues { symbols.down } else { " " };
                        out.push_str(&self.paint(style, c));
                        out.push_str("   ");
                    }
                    let c = if *last_continues { symbols.tee } else { symbols.ell };
                    out.push_str(&self.paint(last_style, &format!("{c}{0}{0}", symbols.right)));
                    out.push(' ');
                }
            }
            Prefix::None => {}
        }

        let in_cycle = print_stack.contains(&node_index);
        // A node with no dependencies has no (*): nothing was left out.
        let has_deps = self.graph.has_outgoing_edges(node_index);
        let star = match (new && !in_cycle) || !has_deps {
            true => String::new(),
            false => format!(" {}", self.paint("\x1b[2m\x1b[33m", "(*)")),
        };
        let _ = writeln!(out, "{}{star}", self.format.display(self.graph, node_index));

        if !new || in_cycle {
            return;
        }
        print_stack.push(node_index);
        for kind in &[
            EdgeKind::Dep(DepKind::Normal),
            EdgeKind::Dep(DepKind::Build),
            EdgeKind::Dep(DepKind::Development),
            EdgeKind::Feature,
        ] {
            self.print_dependencies(
                node_index,
                visited_deps,
                levels_continue,
                print_stack,
                kind,
                out,
            );
        }
        print_stack.pop();
    }

    /// Prints all the dependencies of a package for the given dependency kind.
    fn print_dependencies(
        &self,
        node_index: NodeId,
        visited_deps: &mut FxHashSet<NodeId>,
        levels_continue: &mut Levels,
        print_stack: &mut Vec<NodeId>,
        kind: &EdgeKind,
        out: &mut String,
    ) {
        let deps = self.graph.edges_of_kind(node_index, kind);
        if deps.is_empty() {
            return;
        }
        let name = match kind {
            EdgeKind::Dep(DepKind::Normal) | EdgeKind::Feature => None,
            EdgeKind::Dep(DepKind::Build) => Some(("\x1b[94m\x1b[1m", "[build-dependencies]")),
            EdgeKind::Dep(DepKind::Development) => Some(("\x1b[96m\x1b[1m", "[dev-dependencies]")),
        };
        if let (Prefix::Indent, Some((color, name))) = (self.opts.prefix, name) {
            for (style, continues) in &**levels_continue {
                let c = if *continues { self.symbols.down } else { " " };
                out.push_str(&self.paint(style, c));
                out.push_str("   ");
            }
            out.push_str(&self.paint(color, name));
            out.push('\n');
        }

        let (max_display_depth, filter_non_workspace_member) = match self.opts.display_depth {
            DisplayDepth::MaxDisplayDepth(max) => (max, false),
            DisplayDepth::Workspace => (u32::MAX, true),
        };
        // Current level exceeds maximum display depth. Skip.
        if levels_continue.len() + 1 > max_display_depth as usize {
            return;
        }

        let mut it = deps
            .iter()
            .filter(|dep| match self.graph.node(dep.node()) {
                Node::Package { package_id, .. } => {
                    if filter_non_workspace_member && !self.member_ids.contains(package_id) {
                        return false;
                    }
                    !self.pkgs_to_prune.iter().any(|spec| spec.matches(*package_id))
                }
                Node::Feature { .. } => true,
            })
            .peekable();
        while let Some(dependency) = it.next() {
            levels_continue.push((edge_line_color(dependency.kind()), it.peek().is_some()));
            self.print_node(dependency.node(), visited_deps, levels_continue, print_stack, out);
            levels_continue.pop();
        }
    }
}
