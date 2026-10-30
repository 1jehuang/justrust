//! The graph itself: cargo's root units (`unit_generator.rs`), their dependencies
//! (`unit_dependencies.rs`), the second pass that shares the host's units and settles the
//! deferred profile settings (`rebuild_unit_graph_shared`), and the JSON (`unit_graph.rs`).

use std::cmp::Ordering;
use std::fmt;

use anyhow::{Context as _, bail};
use blaze_manifest::config::CompileKind as ProfileKind;
use blaze_manifest::profiles::{Profile, ProfilePackage, UnitFor};
use blaze_manifest::source::SourceId as ManifestSourceId;
use blaze_manifest::targets::{CrateType, Target, TargetKind};
use blaze_resolve::resolver::FeaturesFor;
use blaze_resolve::{DepKind, Dependency, InternedString, PackageId};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{Filter, Intent, LibRule, Package, Platformed, Request, Rule, order, schema};

/// What is done with a unit's target (cargo's `CompileMode`), in cargo's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Mode {
    Test,
    Build,
    Check { test: bool },
    Doctest,
    RunCustomBuild,
}

impl Mode {
    fn is_any_test(self) -> bool {
        matches!(self, Mode::Test | Mode::Check { test: true } | Mode::Doctest)
    }

    fn is_check(self) -> bool {
        matches!(self, Mode::Check { .. })
    }

    fn schema(self) -> schema::Mode {
        match self {
            Mode::Test => schema::Mode::Test,
            Mode::Build => schema::Mode::Build,
            Mode::Check { .. } => schema::Mode::Check,
            Mode::Doctest => schema::Mode::Doctest,
            Mode::RunCustomBuild => schema::Mode::RunCustomBuild,
        }
    }
}

/// What a unit is compiled for (cargo's `CompileKind`): `Host` for what runs while building,
/// or the triple of a target, the host's own named when no `--target` is given until the
/// graph is shared.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Kind {
    Host,
    Target(String),
}

impl Kind {
    fn for_target(&self, target: &Target) -> Kind {
        match self {
            Kind::Target(_) if target.for_host => Kind::Host,
            kind => kind.clone(),
        }
    }
}

type UnitId = usize;

/// A unit (cargo's `UnitInner`), less what is the same for every unit of a build blaze makes
/// (flags, `links` overrides, artifacts, the standard library).
#[derive(Clone)]
struct Unit {
    pkg: PackageId,
    /// The target, by its index among its package's.
    target: usize,
    profile: Profile,
    kind: Kind,
    mode: Mode,
    features: Vec<InternedString>,
    /// cargo's `dep_hash`: the units it depends on, once the graph is shared; none before.
    dep_hash: Vec<UnitId>,
}

/// cargo's `UnitInterner`: a unit equal to one made before is that one. Profiles compare as
/// cargo's do, without whether a setting is deferred, so the first one made is kept.
#[derive(Default)]
struct Interner {
    units: Vec<Unit>,
    by_key:
        FxHashMap<(PackageId, usize, Kind, Mode, Vec<InternedString>, Vec<UnitId>), Vec<UnitId>>,
}

impl Interner {
    fn intern(&mut self, unit: Unit) -> UnitId {
        let key = (
            unit.pkg,
            unit.target,
            unit.kind.clone(),
            unit.mode,
            unit.features.clone(),
            unit.dep_hash.clone(),
        );
        let same = self.by_key.entry(key).or_default();
        if let Some(&id) = same.iter().find(|&&id| self.units[id].profile == unit.profile) {
            return id;
        }
        let id = self.units.len();
        same.push(id);
        self.units.push(unit);
        id
    }

    fn find(&self, unit: &Unit) -> Option<UnitId> {
        let key = (
            unit.pkg,
            unit.target,
            unit.kind.clone(),
            unit.mode,
            unit.features.clone(),
            unit.dep_hash.clone(),
        );
        let same = self.by_key.get(&key)?;
        same.iter().copied().find(|&id| self.units[id].profile == unit.profile)
    }
}

/// An edge (cargo's `UnitDep`).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Dep {
    unit: UnitId,
    unit_for: UnitFor,
    extern_crate_name: InternedString,
    dep_name: Option<InternedString>,
    public: bool,
}

/// A package as a profile override matches it.
struct ProfilePkg<'a>(&'a blaze_manifest::manifest::PackageId);

impl fmt::Display for ProfilePkg<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} v{}", self.0.name, self.0.version)
    }
}

impl ProfilePackage for ProfilePkg<'_> {
    fn name(&self) -> &str {
        &self.0.name
    }
    fn version(&self) -> &semver::Version {
        &self.0.version
    }
    fn source_id(&self) -> &ManifestSourceId {
        &self.0.source_id
    }
}

fn is_example(t: &Target) -> bool {
    matches!(t.kind, TargetKind::ExampleBin | TargetKind::ExampleLib(_))
}

fn is_test(t: &Target) -> bool {
    t.kind == TargetKind::Test
}

fn is_bench(t: &Target) -> bool {
    t.kind == TargetKind::Bench
}

fn is_linkable(t: &Target) -> bool {
    match &t.kind {
        TargetKind::Lib(kinds) => kinds.iter().any(|k| {
            matches!(k, CrateType::Lib | CrateType::Rlib | CrateType::Dylib | CrateType::ProcMacro)
        }),
        _ => false,
    }
}

fn crate_name(t: &Target) -> String {
    t.name.replace('-', "_")
}

struct State<'r, 'a> {
    request: &'r Request<'a>,
    old: Interner,
    /// The dependencies of each unit, by unit (cargo's `unit_dependencies`), in the order
    /// they were computed.
    graph: FxHashMap<UnitId, Vec<Dep>>,
    order: Vec<UnitId>,
    /// The edges made only for a dev-dependency (cargo's `dev_dependency_edges`).
    dev_edges: FxHashSet<(UnitId, UnitId)>,
    /// The packages each package depends on publicly (cargo's `public_dependencies`).
    public: FxHashMap<PackageId, FxHashSet<PackageId>>,
}

pub(crate) fn build(request: &Request<'_>) -> anyhow::Result<schema::UnitGraph> {
    let mut public: FxHashMap<PackageId, FxHashSet<PackageId>> = FxHashMap::default();
    for pkg in request.resolve.iter() {
        let deps = request.resolve.deps_not_replaced(pkg);
        let deps = deps
            .filter(|(_, deps)| deps.iter().any(|d| d.kind() == DepKind::Normal && d.is_public()));
        public.insert(pkg, deps.map(|(id, _)| id).collect());
    }
    for (id, package) in request.packages {
        if package.manifest.metabuild.is_some() {
            bail!("`{}` has a `metabuild`, which blaze does not build", id.name());
        }
        let unstable = &package.manifest.normalized_toml;
        if let Some(pkg) = unstable.package.as_ref()
            && (pkg.forced_target.is_some() || pkg.default_target.is_some())
        {
            bail!(
                "`{}` names a `forced-target` or `default-target` (`-Zper-package-target`), which \
                 blaze's unit graph does not read",
                id.name()
            );
        }
    }
    let mut state = State {
        request,
        old: Interner::default(),
        graph: FxHashMap::default(),
        order: Vec::new(),
        dev_edges: FxHashSet::default(),
        public,
    };
    let roots = state.generate_root_units()?;
    if roots.is_empty() {
        return Ok(schema::UnitGraph {
            version: schema::VERSION,
            units: Vec::new(),
            roots: Vec::new(),
        });
    }
    for &root in &roots {
        let unit = state.old.units[root].clone();
        let target = state.target(&unit);
        let unit_for = if unit.mode.is_any_test() {
            if target.proc_macro { UnitFor::new_host_test() } else { UnitFor::new_test() }
        } else if target.is_custom_build() {
            UnitFor::new_host(false)
        } else if target.proc_macro {
            UnitFor::new_host(true)
        } else if target.for_host {
            UnitFor::new_compiler()
        } else {
            UnitFor::new_normal()
        };
        state.deps_of(root, unit_for)?;
    }
    state.connect_run_custom_build_deps();
    let mut graph = std::mem::take(&mut state.graph);
    for deps in graph.values_mut() {
        deps.sort_by(|a, b| state.cmp_dep(a, b));
    }
    state.graph = graph;
    state.share_and_emit(&roots)
}

impl<'a> State<'_, 'a> {
    fn package(&self, id: PackageId) -> &Package<'a> {
        &self.request.packages[&id]
    }

    fn target(&self, unit: &Unit) -> &Target {
        &self.package(unit.pkg).manifest.targets[unit.target]
    }

    fn targets(&self, id: PackageId) -> &[Target] {
        &self.package(id).manifest.targets
    }

    /// The platform of `kind`, whose `cfg` decides a `[target.'cfg(..)'.dependencies]`.
    fn platformed(&self, kind: &Kind) -> &Platformed {
        match kind {
            Kind::Host => self.request.host,
            Kind::Target(triple) => match self.request.target {
                Some(target) if target.triple == *triple => target,
                _ => self.request.host,
            },
        }
    }

    fn dep_platform_activated(&self, dep: &Dependency, kind: &Kind) -> bool {
        dep.platform().is_none_or(|platform| self.platformed(kind).activates(platform))
    }

    /// The kinds the user asked for: the host's, by its triple, or `--target`'s.
    fn explicit_kind(&self) -> Kind {
        let platformed = self.request.target.unwrap_or(self.request.host);
        Kind::Target(platformed.triple.clone())
    }

    fn profile(&self, pkg: PackageId, unit_for: UnitFor, kind: &Kind) -> Profile {
        let package = self.package(pkg);
        let is_local = pkg.source_id().is_path();
        let kind = match kind {
            Kind::Host => ProfileKind::Host,
            Kind::Target(triple) => ProfileKind::Target(triple.clone()),
        };
        let profile_pkg = ProfilePkg(&package.manifest.package_id);
        self.request.profiles.get_profile(
            &profile_pkg,
            package.is_member,
            is_local,
            unit_for,
            &kind,
        )
    }

    fn activated_features(&self, pkg: PackageId, features_for: FeaturesFor) -> Vec<InternedString> {
        self.request.features.activated_features(pkg, features_for)
    }

    fn features_for(unit_for: UnitFor) -> FeaturesFor {
        FeaturesFor::from_for_host(unit_for.is_for_host_features())
    }

    /// cargo's `PackageSet::filter_deps`.
    fn filter_deps(
        &self,
        pkg: PackageId,
        has_dev_units: bool,
        kinds: &[Kind],
    ) -> Vec<(PackageId, Vec<&'a Dependency>)> {
        let resolve: &'a blaze_resolve::Resolve = self.request.resolve;
        let mut out = Vec::new();
        for (dep_id, deps) in resolve.deps(pkg) {
            let deps: Vec<&Dependency> = deps
                .iter()
                .filter(|dep| has_dev_units || dep.kind() != DepKind::Development)
                .filter(|dep| {
                    kinds
                        .iter()
                        .chain([&Kind::Host])
                        .any(|kind| self.dep_platform_activated(dep, kind))
                })
                .collect();
            if !deps.is_empty() {
                out.push((dep_id, deps));
            }
        }
        out
    }

    /// cargo's `resolve_all_features`: the package's features, and each of its dependencies'
    /// as `dep/feature`, what `required-features` is checked against.
    fn resolve_all_features(
        &self,
        pkg: PackageId,
        has_dev_units: bool,
        kinds: &[Kind],
    ) -> FxHashSet<String> {
        let mut features: FxHashSet<String> = self
            .activated_features(pkg, FeaturesFor::NormalOrDev)
            .iter()
            .map(|it| it.to_string())
            .collect();
        for (dep_id, deps) in self.filter_deps(pkg, has_dev_units, kinds) {
            let is_proc_macro = self.targets(dep_id).iter().any(|t| t.proc_macro);
            for dep in deps {
                let features_for = FeaturesFor::from_for_host(is_proc_macro || dep.is_build());
                let on = self.request.features.activated_features_unverified(dep_id, features_for);
                for feature in on.unwrap_or_default() {
                    features.insert(format!("{}/{}", dep.name_in_toml(), feature));
                }
            }
        }
        features
    }

    // ---- root units (`unit_generator.rs`) ----

    fn requested_kinds(&self) -> Vec<Kind> {
        match self.request.target {
            Some(target) => vec![Kind::Target(target.triple.clone())],
            None => vec![Kind::Host],
        }
    }

    fn to_compile_mode(&self) -> Mode {
        match self.request.intent {
            Intent::Test | Intent::Bench => Mode::Test,
            Intent::Build => Mode::Build,
            Intent::Check { test } => Mode::Check { test },
        }
    }

    fn new_units(&mut self, pkg: PackageId, target_at: usize, initial: Mode) -> Vec<UnitId> {
        let target = &self.targets(pkg)[target_at];
        let mode = match initial {
            Mode::Test
                if is_example(target) && !self.request.filter.is_specific() && !target.tested =>
            {
                Mode::Build
            }
            mode => mode,
        };
        let features = self.activated_features(pkg, FeaturesFor::from_for_host(target.proc_macro));
        let kind = self.explicit_kind();
        let unit_for = if initial.is_any_test() {
            UnitFor::new_test()
        } else if target.for_host {
            UnitFor::new_compiler()
        } else {
            UnitFor::new_normal()
        };
        let profile = self.profile(pkg, unit_for, &kind);
        let kind = kind.for_target(target);
        let unit =
            Unit { pkg, target: target_at, profile, kind, mode, features, dep_hash: Vec::new() };
        vec![self.old.intern(unit)]
    }

    fn default_targets(&self, pkg: PackageId) -> Vec<usize> {
        let targets = self.targets(pkg).iter().enumerate();
        let picked = targets.filter(|(_, t)| match self.request.intent {
            Intent::Bench => t.benched,
            Intent::Test => t.tested || is_example(t),
            Intent::Build | Intent::Check { .. } => t.is_bin() || t.is_lib(),
        });
        picked.map(|(at, _)| at).collect()
    }

    fn filter_targets(
        &self,
        packages: &[PackageId],
        predicate: impl Fn(&Target) -> bool,
        requires_features: bool,
        mode: Mode,
    ) -> Vec<Proposal> {
        let mut out = Vec::new();
        for &pkg in packages {
            for (at, target) in self.targets(pkg).iter().enumerate() {
                if predicate(target) {
                    out.push(Proposal { pkg, target: at, requires_features, mode });
                }
            }
        }
        out
    }

    fn list_rule_targets(
        &self,
        packages: &[PackageId],
        rule: &Rule,
        description: &str,
        is_expected_kind: fn(&Target) -> bool,
        mode: Mode,
    ) -> anyhow::Result<Vec<Proposal>> {
        match rule {
            Rule::All => Ok(self.filter_targets(packages, is_expected_kind, false, mode)),
            Rule::Just(names) => {
                let mut out = Vec::new();
                for name in names {
                    if name.contains(['*', '?', '[']) {
                        bail!(
                            "the target pattern `{name}`: blaze does not match glob patterns yet"
                        );
                    }
                    let named = |t: &Target| is_expected_kind(t) && t.name == *name;
                    let found = self.filter_targets(packages, named, true, mode);
                    if found.is_empty() {
                        bail!("no {description} target named `{name}`");
                    }
                    out.extend(found);
                }
                Ok(out)
            }
        }
    }

    fn create_proposals(&self, packages: &[PackageId]) -> anyhow::Result<Vec<Proposal>> {
        let mut proposals = Vec::new();
        match self.request.filter {
            Filter::Default { required_features_filterable } => {
                for &pkg in packages {
                    for target in self.default_targets(pkg) {
                        proposals.push(Proposal {
                            pkg,
                            target,
                            requires_features: !required_features_filterable,
                            mode: self.to_compile_mode(),
                        });
                    }
                    if self.request.intent == Intent::Test {
                        let targets = self.targets(pkg).iter();
                        let doctested = targets
                            .clone()
                            .position(|t| t.is_lib() && t.doctest && t.doctestable());
                        if let Some(target) = doctested {
                            let mode = Mode::Doctest;
                            proposals.push(Proposal {
                                pkg,
                                target,
                                requires_features: false,
                                mode,
                            });
                        }
                    }
                }
            }
            Filter::Only { all_targets, lib, bins, examples, tests, benches } => {
                if *lib != LibRule::False {
                    let mode = self.to_compile_mode();
                    let libs = self.filter_targets(packages, Target::is_lib, false, mode);
                    if !all_targets && libs.is_empty() && *lib == LibRule::True {
                        let names: Vec<String> =
                            packages.iter().map(|it| it.name().to_string()).collect();
                        if names.len() == 1 {
                            bail!("no library targets found in package `{}`", names[0]);
                        }
                        bail!("no library targets found in packages: {}", names.join(", "));
                    }
                    proposals.extend(libs);
                }
                let default_mode = self.to_compile_mode();
                let test_filter: fn(&Target) -> bool = match tests {
                    Rule::All => |t| t.tested,
                    Rule::Just(_) => is_test,
                };
                let test_mode = match self.request.intent {
                    Intent::Build => Mode::Test,
                    Intent::Check { .. } => Mode::Check { test: true },
                    _ => default_mode,
                };
                let bench_filter: fn(&Target) -> bool = match benches {
                    Rule::All => |t| t.benched,
                    Rule::Just(_) => is_bench,
                };
                proposals.extend(self.list_rule_targets(
                    packages,
                    bins,
                    "bin",
                    Target::is_bin,
                    default_mode,
                )?);
                proposals.extend(self.list_rule_targets(
                    packages,
                    examples,
                    "example",
                    is_example,
                    default_mode,
                )?);
                proposals.extend(self.list_rule_targets(
                    packages,
                    tests,
                    "test",
                    test_filter,
                    test_mode,
                )?);
                proposals.extend(self.list_rule_targets(
                    packages,
                    benches,
                    "bench",
                    bench_filter,
                    test_mode,
                )?);
            }
        }
        Ok(proposals)
    }

    fn generate_root_units(&mut self) -> anyhow::Result<Vec<UnitId>> {
        let mut packages = self.request.roots.to_vec();
        packages.sort();
        packages.dedup();
        let proposals = self.create_proposals(&packages)?;
        let kinds = self.requested_kinds();
        let mut all_features: FxHashMap<PackageId, FxHashSet<String>> = FxHashMap::default();
        let mut units = FxHashSet::default();
        for Proposal { pkg, target, requires_features, mode } in proposals {
            let t = &self.targets(pkg)[target];
            let unavailable: Vec<String> = match &t.required_features {
                Some(required) => {
                    let features = all_features.entry(pkg).or_insert_with(|| {
                        self.resolve_all_features(pkg, self.request.has_dev_units, &kinds)
                    });
                    required.iter().filter(|f| !features.contains(*f)).cloned().collect()
                }
                None => Vec::new(),
            };
            if t.is_lib() || unavailable.is_empty() {
                units.extend(self.new_units(pkg, target, mode));
            } else if requires_features {
                let required = t.required_features.clone().unwrap_or_default();
                let quoted: Vec<String> = required.iter().map(|it| format!("`{it}`")).collect();
                bail!(
                    "target `{}` in package `{}` requires the features: {}\nConsider enabling \
                     them by passing, e.g., `--features=\"{}\"`",
                    t.name,
                    pkg.name(),
                    quoted.join(", "),
                    required.join(" ")
                );
            }
        }
        let mut units: Vec<UnitId> = units.into_iter().collect();
        units.sort_by(|&a, &b| self.cmp_units(&self.old, a, b));
        Ok(units)
    }

    // ---- dependencies (`unit_dependencies.rs`) ----

    fn deps_of(&mut self, unit: UnitId, unit_for: UnitFor) -> anyhow::Result<()> {
        if self.graph.contains_key(&unit) {
            return Ok(());
        }
        let deps = self.compute_deps(unit, unit_for)?;
        self.graph.insert(unit, deps.clone());
        self.order.push(unit);
        for dep in deps {
            self.deps_of(dep.unit, dep.unit_for)?;
        }
        Ok(())
    }

    /// cargo's `State::deps`: the package's dependencies the unit builds with.
    fn deps(&self, unit: &Unit, unit_for: UnitFor) -> Vec<(PackageId, Vec<&'a Dependency>)> {
        let resolve: &'a blaze_resolve::Resolve = self.request.resolve;
        let target = self.target(unit);
        let mut out = Vec::new();
        for (id, deps) in resolve.deps(unit.pkg) {
            let deps: Vec<&Dependency> = deps
                .iter()
                .filter(|dep| {
                    if target.is_custom_build() != dep.is_build() {
                        return false;
                    }
                    if !dep.is_transitive()
                        && !is_test(target)
                        && !is_example(target)
                        && !unit.mode.is_any_test()
                    {
                        return false;
                    }
                    if !self.dep_platform_activated(dep, &unit.kind) {
                        return false;
                    }
                    if dep.is_optional() {
                        let features_for = Self::features_for(unit_for);
                        let on = self.request.features.is_dep_activated(
                            unit.pkg,
                            features_for,
                            dep.name_in_toml(),
                        );
                        if !on {
                            return false;
                        }
                    }
                    true
                })
                .collect();
            if !deps.is_empty() {
                out.push((id, deps));
            }
        }
        out
    }

    fn compute_deps(&mut self, id: UnitId, unit_for: UnitFor) -> anyhow::Result<Vec<Dep>> {
        let unit = self.old.units[id].clone();
        if unit.mode == Mode::RunCustomBuild {
            return self.compute_deps_custom_build(id, unit_for);
        }
        let target = self.target(&unit).clone();
        let mut ret = Vec::new();
        let mut dev_deps = Vec::new();
        for (dep_pkg, deps) in self.deps(&unit, unit_for) {
            let Some(dep_lib) = self.targets(dep_pkg).iter().position(Target::is_lib) else {
                continue;
            };
            let lib = &self.targets(dep_pkg)[dep_lib];
            let mode = check_or_build_mode(unit.mode, lib);
            let dep_unit_for =
                unit_for.with_dependency(target.is_custom_build(), lib.for_host, lib.proc_macro);
            let kind = unit.kind.for_target(lib);
            let start = ret.len();
            ret.push(self.new_unit_dep(id, dep_pkg, dep_lib, dep_unit_for, kind, mode, None)?);
            if deps.iter().all(|d| !d.is_transitive()) {
                for dep in &ret[start..] {
                    dev_deps.push((id, dep.unit));
                }
            }
        }
        self.dev_edges.extend(dev_deps);

        if target.is_custom_build() {
            return Ok(ret);
        }
        ret.extend(self.dep_build_script(id, unit_for)?);

        if target.is_lib() && unit.mode != Mode::Doctest {
            return Ok(ret);
        }
        ret.extend(self.maybe_lib(id, unit_for)?);

        if !unit.mode.is_check()
            && unit.mode.is_any_test()
            && (is_test(&target) || is_bench(&target))
        {
            let bins: Vec<usize> = self
                .targets(unit.pkg)
                .iter()
                .enumerate()
                .filter(|(_, t)| t.is_bin())
                .filter(|(_, t)| match &t.required_features {
                    Some(required) => {
                        let features = self.resolve_all_features(
                            unit.pkg,
                            false,
                            std::slice::from_ref(&unit.kind),
                        );
                        required.iter().all(|f| features.contains(f))
                    }
                    None => true,
                })
                .map(|(at, _)| at)
                .collect();
            for at in bins {
                let kind = unit.kind.for_target(&self.targets(unit.pkg)[at]);
                let dep = self.new_unit_dep(
                    id,
                    unit.pkg,
                    at,
                    UnitFor::new_normal(),
                    kind,
                    Mode::Build,
                    None,
                )?;
                ret.push(dep);
            }
        }
        Ok(ret)
    }

    fn compute_deps_custom_build(
        &mut self,
        id: UnitId,
        unit_for: UnitFor,
    ) -> anyhow::Result<Vec<Dep>> {
        let unit = self.old.units[id].clone();
        // `links` overrides in the configuration are read where the request is made: the
        // request has none.
        let script_unit_for = unit_for.for_custom_build();
        let compile = self.new_unit_dep(
            id,
            unit.pkg,
            unit.target,
            script_unit_for,
            Kind::Host,
            Mode::Build,
            None,
        )?;
        Ok(vec![compile])
    }

    fn dep_build_script(&mut self, id: UnitId, unit_for: UnitFor) -> anyhow::Result<Vec<Dep>> {
        let unit = self.old.units[id].clone();
        let scripts: Vec<usize> = self
            .targets(unit.pkg)
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_custom_build())
            .map(|(at, _)| at)
            .collect();
        let mut out = Vec::new();
        for at in scripts {
            let profile = self.request.profiles.get_profile_run_custom_build(&unit.profile);
            let script_unit_for = unit_for.for_custom_build();
            out.push(self.new_unit_dep(
                id,
                unit.pkg,
                at,
                script_unit_for,
                unit.kind.clone(),
                Mode::RunCustomBuild,
                Some(profile),
            )?);
        }
        Ok(out)
    }

    fn maybe_lib(&mut self, id: UnitId, unit_for: UnitFor) -> anyhow::Result<Option<Dep>> {
        let unit = self.old.units[id].clone();
        let Some(at) = self.targets(unit.pkg).iter().position(is_linkable) else {
            return Ok(None);
        };
        let lib = &self.targets(unit.pkg)[at];
        let mode = check_or_build_mode(unit.mode, lib);
        let parent_custom_build = self.target(&unit).is_custom_build();
        let dep_unit_for =
            unit_for.with_dependency(parent_custom_build, lib.for_host, lib.proc_macro);
        let kind = unit.kind.for_target(lib);
        self.new_unit_dep(id, unit.pkg, at, dep_unit_for, kind, mode, None).map(Some)
    }

    /// cargo's `new_unit_dep` and `new_unit_dep_with_profile`: `profile` none for the one
    /// the package's profile settings give.
    #[allow(clippy::too_many_arguments)]
    fn new_unit_dep(
        &mut self,
        parent: UnitId,
        pkg: PackageId,
        target: usize,
        unit_for: UnitFor,
        kind: Kind,
        mode: Mode,
        profile: Option<Profile>,
    ) -> anyhow::Result<Dep> {
        let profile = match profile {
            Some(profile) => profile,
            None => self.profile(pkg, unit_for, &kind),
        };
        let parent_pkg = self.old.units[parent].pkg;
        let crate_name = crate_name(&self.targets(pkg)[target]);
        let (extern_crate_name, dep_name) =
            self.request.resolve.extern_crate_name_and_dep_name(parent_pkg, pkg, &crate_name)?;
        let public = self.public.get(&parent_pkg).is_some_and(|it| it.contains(&pkg));
        let features = self.activated_features(pkg, Self::features_for(unit_for));
        let unit = Unit { pkg, target, profile, kind, mode, features, dep_hash: Vec::new() };
        let unit = self.old.intern(unit);
        Ok(Dep { unit, unit_for, extern_crate_name, dep_name, public })
    }

    /// cargo's `connect_run_custom_build_deps`: the run of a build script waits for the runs
    /// of the build scripts of the `links` packages its package depends on.
    fn connect_run_custom_build_deps(&mut self) {
        let mut reverse: FxHashMap<UnitId, FxHashSet<UnitId>> = FxHashMap::default();
        for (&unit, deps) in &self.graph {
            for dep in deps {
                if self.old.units[dep.unit].mode == Mode::RunCustomBuild {
                    reverse.entry(dep.unit).or_default().insert(unit);
                }
            }
        }
        let mut new_deps = Vec::new();
        for &unit in &self.order {
            if self.old.units[unit].mode != Mode::RunCustomBuild {
                continue;
            }
            let Some(reverse_deps) = reverse.get(&unit) else { continue };
            let pkg = self.old.units[unit].pkg;
            let mut to_add: Vec<Dep> = Vec::new();
            for &parent in reverse_deps {
                for other in &self.graph[&parent] {
                    let other_unit = &self.old.units[other.unit];
                    if other_unit.pkg == pkg {
                        continue;
                    }
                    let links = self.package(other_unit.pkg).manifest.metadata.links.is_some();
                    if !(is_linkable(self.target(other_unit)) && links) {
                        continue;
                    }
                    if self.dev_edges.contains(&(parent, other.unit)) {
                        continue;
                    }
                    let run = self.graph[&other.unit]
                        .iter()
                        .find(|it| self.old.units[it.unit].mode == Mode::RunCustomBuild);
                    if let Some(run) = run {
                        let mut dep = run.clone();
                        dep.dep_name = Some(other.dep_name.unwrap_or(other_unit.pkg.name()));
                        if !to_add.contains(&dep) {
                            to_add.push(dep);
                        }
                    }
                }
            }
            if !to_add.is_empty() {
                new_deps.push((unit, to_add));
            }
        }
        for (unit, deps) in new_deps {
            self.graph.get_mut(&unit).expect("a unit of the graph").extend(deps);
        }
    }

    // ---- order ----

    /// cargo's `Ord` of units: package, target, profile, kind, mode, features, then the
    /// dependencies' hash, which only a shared graph has.
    fn cmp_units(&self, interner: &Interner, a: UnitId, b: UnitId) -> Ordering {
        let (x, y) = (&interner.units[a], &interner.units[b]);
        x.pkg
            .cmp(&y.pkg)
            .then_with(|| order::target(self.target(x), self.target(y)))
            .then_with(|| order::profile(&x.profile, &y.profile))
            .then_with(|| x.kind.cmp(&y.kind))
            .then_with(|| x.mode.cmp(&y.mode))
            .then_with(|| x.features.cmp(&y.features))
    }

    /// cargo's `Ord` of the shared graph's units. Two units that differ only in what they
    /// depend on are ordered by cargo's `dep_hash`, which hashes where its interned
    /// dependencies happen to be in memory, so cargo prints them in either order from one run
    /// to the next. blaze orders them by their dependencies, compared the same way, which is
    /// one of cargo's orders; `harness/resolve.py units` accepts either.
    fn cmp_shared(&self, shared: &Shared, a: UnitId, b: UnitId) -> Ordering {
        if a == b {
            return Ordering::Equal;
        }
        self.cmp_units(&shared.new, a, b).then_with(|| {
            let (x, y) = (&shared.graph[&a], &shared.graph[&b]);
            for (p, q) in x.iter().zip(y) {
                let order = self
                    .cmp_shared(shared, p.unit, q.unit)
                    .then_with(|| p.unit_for.cmp(&q.unit_for))
                    .then_with(|| p.extern_crate_name.cmp(&q.extern_crate_name))
                    .then_with(|| p.dep_name.cmp(&q.dep_name))
                    .then_with(|| p.public.cmp(&q.public));
                if order != Ordering::Equal {
                    return order;
                }
            }
            x.len().cmp(&y.len())
        })
    }

    /// cargo's `Ord` of `UnitDep`s before the graph is shared: the unit, what it is for, then
    /// the names.
    fn cmp_dep(&self, a: &Dep, b: &Dep) -> Ordering {
        self.cmp_units(&self.old, a.unit, b.unit)
            .then_with(|| a.unit_for.cmp(&b.unit_for))
            .then_with(|| a.extern_crate_name.cmp(&b.extern_crate_name))
            .then_with(|| a.dep_name.cmp(&b.dep_name))
            .then_with(|| a.public.cmp(&b.public))
    }

    // ---- sharing (`rebuild_unit_graph_shared`) and the JSON (`unit_graph.rs`) ----

    fn share_and_emit(&self, roots: &[UnitId]) -> anyhow::Result<schema::UnitGraph> {
        let to_host = match self.request.target {
            None => Some(self.explicit_kind()),
            Some(_) => None,
        };
        let mut shared = Shared {
            new: Interner::default(),
            graph: FxHashMap::default(),
            memo: FxHashMap::default(),
            to_host,
        };
        let mut new_roots = Vec::new();
        for &root in roots {
            new_roots.push(self.traverse_and_share(&mut shared, root, false));
        }
        let mut units: Vec<UnitId> = shared.graph.keys().copied().collect();
        units.sort_by(|&a, &b| self.cmp_shared(&shared, a, b));
        let index: FxHashMap<UnitId, usize> =
            units.iter().enumerate().map(|(at, &id)| (id, at)).collect();
        let mut out = Vec::with_capacity(units.len());
        for &id in &units {
            let unit = &shared.new.units[id];
            let target = self.target(unit);
            let mut profile: schema::Profile =
                serde_json::from_value(serde_json::to_value(&unit.profile)?)
                    .context("a profile in cargo's form")?;
            // `trim-paths` is stable on the pinned cargo, which prints a profile that sets none
            // with its default.
            profile.trim_paths.get_or_insert_with(|| serde_json::Value::from("none"));
            let dependencies = shared.graph[&id]
                .iter()
                .map(|dep| schema::UnitDep {
                    index: index[&dep.unit],
                    extern_crate_name: dep.extern_crate_name.to_string(),
                    public: Some(dep.public),
                    noprelude: Some(false),
                    nounused: Some(false),
                })
                .collect();
            out.push(schema::Unit {
                pkg_id: unit.pkg.to_spec().to_string(),
                target: schema_target(unit.pkg, target)?,
                profile,
                platform: match &unit.kind {
                    Kind::Host => None,
                    Kind::Target(triple) => Some(triple.clone()),
                },
                mode: unit.mode.schema(),
                checked_as_test: unit.mode == Mode::Check { test: true },
                features: unit.features.iter().map(|it| it.to_string()).collect(),
                is_std: false,
                dependencies,
            });
        }
        let roots = new_roots.iter().map(|it| index[it]).collect();
        Ok(schema::UnitGraph { version: schema::VERSION, units: out, roots })
    }

    /// cargo's `traverse_and_share`: the unit as the shared graph has it, the host's own
    /// triple made `Host`, its dependencies' identity part of it, `strip` lowered to
    /// `debuginfo` where nothing has debuginfo, and a host unit's deferred debuginfo dropped
    /// unless the target builds the same unit.
    fn traverse_and_share(&self, shared: &mut Shared, id: UnitId, for_host: bool) -> UnitId {
        if let Some(&new) = shared.memo.get(&id) {
            return new;
        }
        let mut new_deps = Vec::new();
        for dep in &self.graph[&id] {
            let unit = self.traverse_and_share(shared, dep.unit, dep.unit_for.is_for_host());
            new_deps.push(Dep { unit, ..dep.clone() });
        }
        let unit = &self.old.units[id];
        let kind = match &shared.to_host {
            Some(to_host) if *to_host == unit.kind => Kind::Host,
            _ => unit.kind.clone(),
        };
        let mut profile = unit.profile.clone();
        if profile.strip.is_deferred()
            && !profile.debuginfo.is_turned_on()
            && new_deps.iter().all(|d| !shared.new.units[d.unit].profile.debuginfo.is_turned_on())
        {
            profile.strip = profile.strip.strip_debuginfo();
        }
        if for_host
            && let Some(to_host) = &shared.to_host
            && profile.debuginfo.is_deferred()
        {
            let canonical = profile.debuginfo.finalize();
            let mut probe = unit.clone();
            probe.profile.debuginfo = canonical;
            probe.kind = to_host.clone();
            let exists = self.old.find(&probe).is_some_and(|it| self.graph.contains_key(&it));
            profile.debuginfo = if exists { canonical } else { canonical.weaken() };
        }
        let new = shared.new.intern(Unit {
            pkg: unit.pkg,
            target: unit.target,
            profile,
            kind,
            mode: unit.mode,
            features: unit.features.clone(),
            dep_hash: new_deps.iter().map(|d| d.unit).collect(),
        });
        shared.memo.insert(id, new);
        shared.graph.entry(new).or_insert(new_deps);
        new
    }
}

struct Shared {
    new: Interner,
    graph: FxHashMap<UnitId, Vec<Dep>>,
    memo: FxHashMap<UnitId, UnitId>,
    to_host: Option<Kind>,
}

struct Proposal {
    pkg: PackageId,
    target: usize,
    requires_features: bool,
    mode: Mode,
}

/// cargo's `check_or_build_mode`: what a dependency of a unit in `mode` is built as.
fn check_or_build_mode(mode: Mode, target: &Target) -> Mode {
    match mode {
        Mode::Check { .. } if !target.for_host => Mode::Check { test: false },
        _ => Mode::Build,
    }
}

fn schema_target(pkg: PackageId, target: &Target) -> anyhow::Result<schema::Target> {
    let serialized = target
        .serialized()
        .with_context(|| format!("`{}`'s target `{}` has no source file", pkg, target.name))?;
    serde_json::from_value(serde_json::to_value(serialized)?).context("a target in cargo's form")
}
