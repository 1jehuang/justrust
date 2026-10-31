//! Return-position `impl Trait` retains exact bounds and a per-callable site.
use super::*;

impl PlanCtx {
    /// Candidate discovery is not trait evidence. Validate the exact retained
    /// opaque descriptor first, then let select_trait/prove_goal authorize each
    /// application in the original caller context. No hidden body is read.
    pub(super) fn opaque_callable_bound_candidates(
        &self,
        receiver: &ir::Ty,
        caller: ir::FactsCtx<'_>,
    ) -> Result<Vec<ir::Predicate>, String> {
        let ir::Ty::Opaque(opaque) = receiver else {
            return Ok(Vec::new());
        };
        caller.validate_ty_with_budget(receiver, &self.budget)?;
        self.budget.retain_bytes(
            opaque
                .bounds()
                .len()
                .saturating_mul(std::mem::size_of::<ir::Predicate>()),
        )?;
        let mut candidates = Vec::with_capacity(opaque.bounds().len());
        for bound in opaque.bounds() {
            self.budget.charge_work(1)?;
            candidates.push(ir::Predicate::Implements(
                bound.application(receiver.clone()),
            ));
        }
        Ok(candidates)
    }

    /// This reads declaration schemas only. In particular, inspecting a call's
    /// opaque result never demands that callee's body or hidden representation.
    pub(super) fn source_inventory_uses_opaque(
        &self,
        declaration: ir::DeclId,
        inventory: &SourceInventory,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        fn contains(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<bool, String> {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            match ty {
                ir::Ty::Opaque(_) => Ok(true),
                ir::Ty::Ref { referent, .. } => contains(referent, budget, depth + 1),
                ir::Ty::RawPtr { pointee, .. } => contains(pointee, budget, depth + 1),
                ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                    contains(element, budget, depth + 1)
                }
                ir::Ty::Tuple(fields) => {
                    for field in fields.iter() {
                        if contains(field, budget, depth + 1)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                ir::Ty::Struct(value) => {
                    for actual in value.def().type_args.iter() {
                        if contains(actual, budget, depth + 1)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                ir::Ty::Enum(value) => {
                    for actual in value.def().type_args.iter() {
                        if contains(actual, budget, depth + 1)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                _ => Ok(false),
            }
        }
        let signature_uses = |owner| -> Result<bool, String> {
            let signature = self.signature(owner)?;
            if contains(&signature.signature.result, budget, 0)? {
                return Ok(true);
            }
            for parameter in &signature.signature.params {
                if contains(parameter, budget, 0)? {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        if signature_uses(declaration)? {
            return Ok(true);
        }
        for call in &inventory.calls {
            budget.charge_work(1)?;
            if let Some(path) = &call.ordinary {
                let key =
                    name_scope::path_key(self.program.names.as_ref(), path, Namespace::Value)?;
                if let Some(target) = self.registry.functions.get(&key)
                    && signature_uses(*target)?
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[derive(Clone, Debug)]
pub struct SourceOpaqueType {
    pub site: u32,
    pub bounds: Vec<TraitPath>,
    pub regions: Vec<RegionName>,
}

impl SourceOpaqueType {
    pub(super) fn validate_names(&self, names: &impl NameAccess) -> Result<(), String> {
        let region = |region: &RegionName| -> Result<(), String> {
            if let RegionName::Named(name) = region {
                names
                    .key(*name, LookupClass::LifetimeParameter)
                    .map_err(native_adapter::name_diagnostic)?;
            }
            Ok(())
        };
        for lifetime in &self.regions {
            region(lifetime)?;
        }
        for bound in &self.bounds {
            if let TraitPath::Source(path) = bound {
                path.path(names)?;
                for segment in &path.segments {
                    names
                        .budget()
                        .charge(super::super::expansion_budget::Counter::Work, 1)
                        .map_err(|error| error.to_string())?;
                    names
                        .key(segment.name, LookupClass::Item(Namespace::Type))
                        .map_err(native_adapter::name_diagnostic)?;
                    for lifetime in &segment.region_args {
                        region(lifetime)?;
                    }
                    for binding in &segment.associated_types {
                        names
                            .key(binding.name, LookupClass::Member)
                            .map_err(native_adapter::name_diagnostic)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn semantic_compare<'a>(
        &'a self,
        other: &'a Self,
        names: &impl NameAccess,
        mut child: impl FnMut(&'a Type, &'a Type) -> Result<(), String>,
    ) -> Result<bool, String> {
        let same_region = |a: &RegionName, b: &RegionName| -> Result<bool, String> {
            Ok(match (a, b) {
                (RegionName::Named(a), RegionName::Named(b)) => {
                    names
                        .key(*a, LookupClass::LifetimeParameter)
                        .map_err(native_adapter::name_diagnostic)?
                        == names
                            .key(*b, LookupClass::LifetimeParameter)
                            .map_err(native_adapter::name_diagnostic)?
                }
                (RegionName::Static, RegionName::Static)
                | (RegionName::Infer, RegionName::Infer) => true,
                _ => false,
            })
        };
        let mut equal = self.site == other.site
            && self.bounds.len() == other.bounds.len()
            && self.regions.len() == other.regions.len();
        for (a, b) in self.regions.iter().zip(&other.regions) {
            equal &= same_region(a, b)?;
        }
        for (a, b) in self.bounds.iter().zip(&other.bounds) {
            match (a, b) {
                (TraitPath::Builtin(a), TraitPath::Builtin(b)) => equal &= a == b,
                (TraitPath::Source(a), TraitPath::Source(b)) => {
                    equal &= a.anchor == b.anchor && a.segments.len() == b.segments.len();
                    for (a, b) in a.segments.iter().zip(&b.segments) {
                        equal &= names
                            .key(a.name, LookupClass::Item(Namespace::Type))
                            .map_err(native_adapter::name_diagnostic)?
                            == names
                                .key(b.name, LookupClass::Item(Namespace::Type))
                                .map_err(native_adapter::name_diagnostic)?;
                        equal &= a.args.len() == b.args.len()
                            && a.associated_types.len() == b.associated_types.len()
                            && a.region_args.len() == b.region_args.len();
                        for (a, b) in a.region_args.iter().zip(&b.region_args) {
                            equal &= same_region(a, b)?;
                        }
                        for (a, b) in a.args.iter().zip(&b.args) {
                            child(a, b)?;
                        }
                        for (a, b) in a.associated_types.iter().zip(&b.associated_types) {
                            equal &= names
                                .key(a.name, LookupClass::Member)
                                .map_err(native_adapter::name_diagnostic)?
                                == names
                                    .key(b.name, LookupClass::Member)
                                    .map_err(native_adapter::name_diagnostic)?;
                            child(&a.value, &b.value)?;
                        }
                    }
                }
                _ => equal = false,
            }
        }
        Ok(equal)
    }

    pub(super) fn copy(&self, budget: &ir::PlanBudget, depth: usize) -> Result<Self, FactsError> {
        sr::depth(budget, depth)?;
        sr::work(budget, self.bounds.len().saturating_add(self.regions.len()))?;
        let mut bounds = Vec::new();
        bounds
            .try_reserve_exact(self.bounds.len())
            .map_err(|_| FactsError::resource(budget, "opaque bounds allocation"))?;
        for bound in &self.bounds {
            bounds.push(match bound {
                TraitPath::Builtin(kind) => TraitPath::Builtin(*kind),
                TraitPath::Source(path) => {
                    TraitPath::Source(copy_source_path(path, budget, depth + 1)?)
                }
            });
        }
        let mut regions = Vec::new();
        regions
            .try_reserve_exact(self.regions.len())
            .map_err(|_| FactsError::resource(budget, "opaque regions allocation"))?;
        regions.extend_from_slice(&self.regions);
        Ok(Self {
            site: self.site,
            bounds,
            regions,
        })
    }

    pub(super) fn retain(&self, budget: &ir::PlanBudget, depth: usize) -> Result<(), FactsError> {
        sr::depth(budget, depth)?;
        sr::retain(
            budget,
            std::mem::size_of::<Self>()
                + self.bounds.len() * std::mem::size_of::<TraitPath>()
                + self.regions.len() * std::mem::size_of::<RegionName>(),
        )?;
        for bound in &self.bounds {
            sr::work(budget, 1)?;
            if let TraitPath::Source(path) = bound {
                retain_source_path(path, budget, depth + 1)?;
            }
        }
        Ok(())
    }
    pub(super) fn types(&self) -> impl Iterator<Item = &Type> {
        self.bounds
            .iter()
            .filter_map(|bound| match bound {
                TraitPath::Source(path) => Some(path),
                TraitPath::Builtin(_) => None,
            })
            .flat_map(|path| &path.segments)
            .flat_map(|segment| {
                segment.args.iter().chain(
                    segment
                        .associated_types
                        .iter()
                        .map(|binding| &binding.value),
                )
            })
    }

    pub(super) fn exact(
        &self,
        other: &Self,
        names: &NameStore,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<bool, String> {
        budget.check_depth(depth)?;
        budget.charge_work(
            self.bounds
                .len()
                .saturating_add(self.regions.len())
                .saturating_add(1),
        )?;
        let mut equal = self.site == other.site
            && self.bounds.len() == other.bounds.len()
            && self.regions.len() == other.regions.len();
        for (left, right) in self.bounds.iter().zip(&other.bounds) {
            equal &= match (left, right) {
                (TraitPath::Builtin(left), TraitPath::Builtin(right)) => left == right,
                (TraitPath::Source(left), TraitPath::Source(right)) => {
                    exact_source_generic_path(names, left, right, budget, depth + 1)?
                }
                _ => false,
            };
        }
        for (left, right) in self.regions.iter().zip(&other.regions) {
            equal &= match (left, right) {
                (RegionName::Named(left), RegionName::Named(right)) => {
                    exact_source_ident(names, *left, *right)?
                }
                (RegionName::Static, RegionName::Static)
                | (RegionName::Infer, RegionName::Infer) => true,
                _ => false,
            };
        }
        Ok(equal)
    }
}

impl DeclRegistry {
    pub(super) fn source_opaque_return(
        &self,
        declaration: ir::DeclId,
        site: u32,
        budget: &ir::PlanBudget,
    ) -> Result<&SourceOpaqueType, String> {
        fn find<'a>(
            ty: &'a Type,
            site: u32,
            budget: &ir::PlanBudget,
            depth: usize,
        ) -> Result<Option<&'a SourceOpaqueType>, String> {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            match ty {
                Type::Opaque(opaque) if opaque.site == site => return Ok(Some(opaque)),
                Type::Ref {
                    referent: inner, ..
                }
                | Type::RawPtr { pointee: inner, .. }
                | Type::Array(inner, _)
                | Type::Slice(inner) => {
                    return find(inner, site, budget, depth + 1);
                }
                Type::PhantomData(source) | Type::ManuallyDrop(source) | Type::Cell(source) => {
                    if let Some(argument) = &source.argument {
                        return find(argument, site, budget, depth + 1);
                    }
                }
                Type::FnPointer { inputs, output } => {
                    for field in inputs.iter().chain(std::iter::once(output.as_ref())) {
                        if let Some(found) = find(field, site, budget, depth + 1)? {
                            return Ok(Some(found));
                        }
                    }
                }
                Type::Tuple(fields) => {
                    for field in fields {
                        if let Some(found) = find(field, site, budget, depth + 1)? {
                            return Ok(Some(found));
                        }
                    }
                }
                Type::Opaque(opaque) => {
                    for child in opaque.types() {
                        if let Some(found) = find(child, site, budget, depth + 1)? {
                            return Ok(Some(found));
                        }
                    }
                }
                Type::Apply(path) => {
                    for segment in &path.segments {
                        for arg in segment.args.iter().chain(
                            segment
                                .associated_types
                                .iter()
                                .map(|binding| &binding.value),
                        ) {
                            if let Some(found) = find(arg, site, budget, depth + 1)? {
                                return Ok(Some(found));
                            }
                        }
                    }
                }
                _ => {}
            }
            Ok(None)
        }
        self.declaration(declaration)?;
        let function = self
            .source_callable(declaration)
            .ok_or("unsupported: opaque returns require a retained function body")?;
        find(&function.result, site, budget, 0)?
            .ok_or("opaque return changed its defining declaration or occurrence".into())
    }
}

impl PlanState {
    pub(super) fn source_opaque_type(
        &mut self,
        registry: &DeclRegistry,
        source: &SourceOpaqueType,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<ir::Ty, String> {
        budget.check_depth(depth)?;
        let context = self
            .source_context
            .ok_or("opaque return has no defining source declaration")?;
        let original = registry.source_opaque_return(context.declaration, source.site, budget)?;
        if !source.exact(original, registry.names.as_ref(), budget, depth + 1)? {
            return Err("opaque return changed its original bounds or source occurrence".into());
        }
        if !source.regions.is_empty() {
            return Err(
                "unsupported: opaque outlives bounds require declared carrier transport".into(),
            );
        }
        let info = registry.declaration(context.declaration)?;
        budget.retain_bytes(
            info.env
                .total_arity()
                .saturating_mul(std::mem::size_of::<ir::Ty>()),
        )?;
        let mut owner_args = Vec::with_capacity(info.env.total_arity());
        for scope in info.env.scopes() {
            let declaration = registry.declaration(scope.binder().declaration)?;
            if declaration.kind == ir::DeclKind::Trait {
                owner_args.push(
                    bindings
                        .get(&GenericBindingKey::SelfType)
                        .cloned()
                        .ok_or("opaque return lost enclosing Self parameter")?,
                );
            }
            for name in declaration.type_params.iter() {
                owner_args.push(
                    bindings
                        .get(&registry.parameter_key(*name)?)
                        .cloned()
                        .ok_or("opaque return lost an enclosing generic parameter")?,
                );
            }
        }
        if owner_args.len() != info.env.total_arity() {
            return Err("opaque owner argument environment changed arity".into());
        }
        budget.retain_bytes(
            source
                .bounds
                .len()
                .saturating_mul(std::mem::size_of::<ir::OpaqueBound>()),
        )?;
        let mut bounds = Vec::with_capacity(source.bounds.len());
        for path in &source.bounds {
            let written = match path {
                TraitPath::Builtin(_) => 0,
                TraitPath::Source(path) => {
                    path.segments.iter().map(|segment| segment.args.len()).sum()
                }
            };
            // Only this nonexecuting temporary uses Unit for Self. Any default
            // argument referring to Self is converted to a typed SelfType marker
            // before publishing the descriptor, so no fabricated Self escapes.
            let application = self.source_trait_application(
                registry,
                path,
                &ir::Ty::Unit,
                bindings,
                caller,
                budget,
            )?;
            let args = application
                .trait_ref
                .args
                .iter()
                .enumerate()
                .map(|(index, actual)| {
                    if index >= written {
                        ir::OpaqueBoundArg::SelfType
                    } else {
                        ir::OpaqueBoundArg::Type(actual.clone())
                    }
                })
                .collect::<Vec<_>>();
            let equalities = application
                .equalities
                .into_iter()
                .map(|equality| (equality.projection.item, equality.ty))
                .collect::<Vec<_>>();
            bounds.push(ir::OpaqueBound::new(
                application.trait_ref.declaration,
                args.into(),
                equalities.into(),
            ));
        }
        let function = registry
            .source_callable(context.declaration)
            .ok_or("opaque owner has no retained function")?;
        budget.retain_bytes(
            function
                .regions
                .params
                .len()
                .saturating_mul(std::mem::size_of::<ir::SourceRegionId>()),
        )?;
        let captured_regions = (0..function.regions.params.len())
            .map(|index| {
                Ok(ir::SourceRegionId {
                    declaration: context.declaration,
                    index: u16::try_from(index).map_err(|_| "opaque region index overflow")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(ir::Ty::Opaque(Arc::new(ir::OpaqueTy::new(
            context.declaration,
            owner_args.into(),
            source.site,
            bounds.into(),
            captured_regions.into(),
        ))))
    }
}

impl Parser<'_, '_> {
    pub(super) fn opaque_function_result(&mut self) -> Result<Type, ParseError> {
        if !self.eat("->") {
            return Ok(Type::Unit);
        }
        let enclosing = self.opaque_return_sites.replace(0);
        let result = self.ty();
        self.opaque_return_sites = enclosing;
        result
    }

    /// Only function result parsing enables this context. Closure annotations,
    /// item fields and local annotations retain the ordinary E0562 rejection.
    pub(super) fn opaque_return_type(&mut self) -> Result<Type, ParseError> {
        if self.names.edition() != super::super::token::Edition::E2024 {
            return Err(self.unsupported(
                "pre-2024 `impl Trait` return lifetime capture rules are unsupported; the native compiler implements only edition 2024 capture",
            ));
        }
        let site = self.opaque_return_sites.ok_or_else(|| {
            self.error("E0562: `impl Trait` is not allowed in this type position")
        })?;
        self.opaque_return_sites = Some(
            site.checked_add(1)
                .ok_or_else(|| self.limit("opaque return type site limit exceeded"))?,
        );
        self.expect("impl")?;
        let mut bounds = Vec::new();
        let mut regions = Vec::new();
        loop {
            if bounds.len().saturating_add(regions.len()) >= 256 {
                return Err(self.limit("predicate limit exceeded"));
            }
            if self.token().is_lifetime() {
                let region = self.lifetime()?;
                self.reserve_parser_items(&mut regions, 1)?;
                regions.push(region);
            } else {
                if self.is("use") {
                    return Err(self.unsupported("precise opaque capture bounds are unsupported"));
                }
                let bound = self.trait_path()?;
                self.reserve_parser_items(&mut bounds, 1)?;
                bounds.push(bound);
            }
            if !self.eat("+") {
                break;
            }
        }
        if bounds.is_empty() {
            return Err(self.error("E0224: at least one trait is required for an opaque type"));
        }
        Ok(Type::Opaque(Box::new(SourceOpaqueType {
            site,
            bounds,
            regions,
        })))
    }
}
