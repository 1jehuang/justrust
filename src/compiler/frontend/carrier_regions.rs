//! Declaration-owned lifetime rows for individual reference carriers.
//! Nominal lifetimes stay erased from runtime type identity, not from authority.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn attach(
    registry: &DeclRegistry,
    regions: &RegionDecls,
    params: &[(BindingName, Type)],
    result: &Type,
    implementation: Option<&ImplDecl>,
    signature: &ir::Signature,
    receiver: ReceiverKind,
    original: Arc<ir::SourceRegionEnv>,
    budget: &ir::PlanBudget,
) -> Result<Arc<ir::SourceRegionEnv>, String> {
    let mut collector = Collector {
        registry,
        regions,
        implementation,
        signature,
        declaration: original.declaration,
        receiver,
        budget,
        rows: Vec::new(),
        next_region: original.region_count,
        empty_nominals: Vec::new(),
        empty_nominal_storage: None,
        nominal_stack: Vec::new(),
    };
    let offset = usize::from(receiver != ReceiverKind::None);
    if offset != 0 {
        let self_ty = implementation
            .map(|block| block.self_ty.clone())
            .unwrap_or(Type::SelfType);
        let source = match receiver {
            ReceiverKind::Borrowed { kind } => Type::Ref {
                kind,
                region: regions.receiver,
                referent: Box::new(self_ty),
            },
            _ => self_ty,
        };
        collector.walk(
            &source,
            &signature.params[0],
            Some(0),
            &mut Vec::new(),
            0,
            0,
        )?;
    }
    for (index, (_, source)) in params.iter().enumerate() {
        collector.walk(
            source,
            &signature.params[index + offset],
            Some(index + offset),
            &mut Vec::new(),
            0,
            0,
        )?;
    }
    let env = &registry.declaration(original.declaration)?.env;
    budget.charge_work(env.total_arity())?;
    if env
        .ordered_params()
        .any(|parameter| env.outlives_static(parameter))
    {
        for (parameter, ty) in signature.params.iter().enumerate() {
            for location in ir::carrier_locations(ty, budget)?.iter() {
                budget.charge_work(collector.rows.len() + 1)?;
                if let ir::CarrierChannel::ParamTail {
                    parameter: generic, ..
                } = location.channel
                    && env.outlives_static(generic)
                    && !collector
                        .rows
                        .iter()
                        .any(|row| row.parameter == Some(parameter) && row.location == *location)
                {
                    budget.retain_bytes(std::mem::size_of::<ir::SourceCarrierRegion>())?;
                    collector.rows.push(ir::SourceCarrierRegion {
                        parameter: Some(parameter),
                        location: location.clone(),
                        region: ir::CarrierRegionTerm::Static,
                    });
                }
            }
        }
    }
    collector.walk(result, &signature.result, None, &mut Vec::new(), 0, 0)?;
    if let (Type::Opaque(source), ir::Ty::Opaque(opaque)) = (result, &signature.result) {
        // Each source-written outlives bound constrains the whole retained
        // capture family. The contract builder uses this row to select only
        // input occurrences which outlive the required result lifetime.
        // Multiple bounds require intersection rather than choosing one row.
        if source.regions.len() > 1 {
            return Err(
                "unsupported: multiple opaque outlives bounds require a region intersection".into(),
            );
        }
        if let Some(region) = source.regions.first().copied().or_else(|| {
            // Edition 2024 captures every in-scope lifetime. With a single
            // elidable input lifetime that is exactly the `+ '_` contract.
            // Several candidates keep no row: the body check then rejects any
            // hidden value carrying a borrow (fail closed, never widened).
            (!signature.ref_contract.input_regions.is_empty()).then_some(RegionName::Infer)
        }) {
            let implicit = source.regions.is_empty();
            let term = if matches!(region, RegionName::Infer) {
                // Apply ordinary output elision: a borrowed receiver wins,
                // otherwise exactly one input lifetime must be available.
                let candidate = if matches!(receiver, ReceiverKind::Borrowed { .. }) {
                    signature
                        .ref_contract
                        .input_regions
                        .iter()
                        .find(|row| row.input.param == 0 && row.input.position.depth == 0)
                        .map(|row| row.region)
                } else {
                    let mut candidates = signature
                        .ref_contract
                        .input_regions
                        .iter()
                        .map(|row| row.region);
                    let first = candidates.next();
                    if candidates.next().is_some() {
                        None
                    } else {
                        first
                    }
                };
                match candidate {
                    Some(candidate) => ir::CarrierRegionTerm::Fixed(candidate),
                    None if implicit => ir::CarrierRegionTerm::Static,
                    None => {
                        return Err("E0106: opaque inferred lifetime requires one input lifetime or a borrowed receiver".into());
                    }
                }
            } else {
                collector
                    .term(Some(region), None, &[], 0)?
                    .ok_or("opaque outlives bound has no source region")?
            };
            budget.retain_bytes(std::mem::size_of::<ir::SourceCarrierRegion>())?;
            collector.rows.push(ir::SourceCarrierRegion {
                parameter: None,
                location: ir::CarrierLocation {
                    path: ir::CarrierPath(Arc::from([])),
                    channel: ir::CarrierChannel::OpaqueTail {
                        opaque: opaque.clone(),
                        prefix: 0,
                    },
                },
                region: term,
            });
        }
    }
    budget.retain_bytes(
        std::mem::size_of::<ir::SourceRegionEnv>()
            + usize::from(collector.next_region) * std::mem::size_of::<Option<ir::RegionParamId>>(),
    )?;
    let mut normalized = Vec::new();
    normalized
        .try_reserve_exact(usize::from(collector.next_region))
        .map_err(|_| "source slice carrier region allocation failed")?;
    normalized.extend_from_slice(&original.normalized);
    normalized.resize(usize::from(collector.next_region), None);
    Ok(Arc::new(ir::SourceRegionEnv {
        carriers: collector.rows.into(),
        region_count: collector.next_region,
        normalized: normalized.into(),
        ..original.as_ref().clone()
    }))
}

fn explicit_array_reference(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    for location in ir::carrier_locations(ty, budget)?.iter() {
        budget.charge_work(1)?;
        match &location.channel {
            ir::CarrierChannel::Fixed(_) => return Ok(true),
            ir::CarrierChannel::ArrayTail { array, .. } => {
                let ir::Ty::DependentArray { element, .. } = array.as_ref() else {
                    return Err("dependent array carrier lost its canonical shape".into());
                };
                if explicit_array_reference(element, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

struct Collector<'a> {
    registry: &'a DeclRegistry,
    regions: &'a RegionDecls,
    implementation: Option<&'a ImplDecl>,
    signature: &'a ir::Signature,
    declaration: ir::DeclId,
    receiver: ReceiverKind,
    budget: &'a ir::PlanBudget,
    rows: Vec<ir::SourceCarrierRegion>,
    next_region: u16,
    // Completed, carrier-free source subtrees only. Depth remains part of the
    // key so a deeper reuse cannot bypass any source traversal depth check.
    empty_nominals: Vec<(ir::DeclId, usize, usize)>,
    empty_nominal_storage: Option<ir::PlanReservation>,
    // Canonical nominal applications on the current source traversal stack.
    // Local to this one signature computation.
    nominal_stack: Vec<ir::NominalAppId>,
}
impl Collector<'_> {
    fn lifetime_key(&self, name: AstIdent) -> Result<NameKey, String> {
        self.registry
            .names
            .key(name, LookupClass::LifetimeParameter)
            .map_err(native_adapter::name_diagnostic)
    }
    fn term(
        &mut self,
        region: Option<RegionName>,
        parameter: Option<usize>,
        path: &[ir::CarrierStep],
        prefix: u8,
    ) -> Result<Option<ir::CarrierRegionTerm>, String> {
        if let Some(RegionName::Named(name)) = region {
            let key = self.lifetime_key(name)?;
            for (index, binder) in self.regions.params.iter().enumerate() {
                self.budget.charge_work(1)?;
                if self.lifetime_key(*binder)? == key {
                    return Ok(Some(ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                        declaration: self.declaration,
                        index: index as u16,
                    })));
                }
            }
            return Err("undeclared source carrier lifetime".into());
        }
        if matches!(region, Some(RegionName::Static)) {
            return Ok(Some(ir::CarrierRegionTerm::Static));
        }
        if parameter.is_some() && path.contains(&ir::CarrierStep::SliceElement) {
            if self.next_region as usize >= ir::MAX_REGION_PARAMS {
                return Err("combined source carrier lifetime limit exceeded".into());
            }
            let index = self.next_region;
            self.next_region += 1;
            return Ok(Some(ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                declaration: self.declaration,
                index,
            })));
        }
        if path.is_empty() {
            let fixed = if let Some(parameter) = parameter {
                self.signature
                    .ref_contract
                    .input_regions
                    .iter()
                    .find(|row| row.input.param == parameter && row.input.position.depth == prefix)
                    .map(|row| row.region)
            } else {
                self.signature
                    .ref_contract
                    .results
                    .iter()
                    .find(|row| row.position.depth == prefix)
                    .map(|row| row.region)
            };
            return Ok(fixed.map(ir::CarrierRegionTerm::Fixed));
        }
        let fixed = if let Some(parameter) = parameter {
            if matches!(path.first(), Some(ir::CarrierStep::Referent)) {
                self.signature
                    .ref_contract
                    .input_regions
                    .iter()
                    .find(|row| row.input.param == parameter && row.input.position.depth == 0)
                    .map(|row| row.region)
            } else {
                None
            }
        } else if matches!(self.receiver, ReceiverKind::Borrowed { .. }) {
            self.signature
                .ref_contract
                .input_regions
                .iter()
                .find(|row| row.input.param == 0 && row.input.position.depth == 0)
                .map(|row| row.region)
        } else if self.signature.ref_contract.input_regions.len() == 1 {
            Some(self.signature.ref_contract.input_regions[0].region)
        } else {
            None
        };
        if let Some(fixed) = fixed {
            return Ok(Some(ir::CarrierRegionTerm::Fixed(fixed)));
        }
        Ok(None)
    }
    fn walk(
        &mut self,
        source: &Type,
        semantic: &ir::Ty,
        parameter: Option<usize>,
        path: &mut Vec<ir::CarrierStep>,
        prefix: u8,
        depth: usize,
    ) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        match source {
            Type::Param(_) | Type::Named(_) | Type::SelfType
                if matches!(semantic, ir::Ty::Param(_))
                    && path.last() == Some(&ir::CarrierStep::SliceElement)
                    && prefix == 0 =>
            {
                let ir::Ty::Param(element) = semantic else {
                    unreachable!()
                };
                if self.rows.len() >= ir::MAX_LOAN_SITES {
                    return Err("source carrier region resource limit exceeded".into());
                }
                self.budget.retain_bytes(
                    std::mem::size_of::<ir::SourceCarrierRegion>()
                        + path.len() * std::mem::size_of::<ir::CarrierStep>(),
                )?;
                self.rows.push(ir::SourceCarrierRegion {
                    parameter,
                    location: ir::CarrierLocation {
                        path: ir::CarrierPath(path.clone().into()),
                        channel: ir::CarrierChannel::ParamTail {
                            parameter: *element,
                            prefix: 0,
                        },
                    },
                    region: ir::CarrierRegionTerm::Family {
                        parameter: *element,
                    },
                });
            }
            Type::SelfType => {
                if let Some(block) = self.implementation {
                    self.walk(&block.self_ty, semantic, parameter, path, prefix, depth + 1)?;
                }
            }
            Type::Projection(projection) => {
                if let SourceProjection::SelfItem { item, .. } = projection.as_ref()
                    && let Some(block) = self.implementation
                {
                    let key = self
                        .registry
                        .names
                        .key(*item, LookupClass::Member)
                        .map_err(native_adapter::name_diagnostic)?;
                    for associated in &block.associated_types {
                        self.budget.charge_work(1)?;
                        if self
                            .registry
                            .names
                            .key(associated.name, LookupClass::Member)
                            .map_err(native_adapter::name_diagnostic)?
                            == key
                        {
                            self.walk(
                                &associated.value,
                                semantic,
                                parameter,
                                path,
                                prefix,
                                depth + 1,
                            )?;
                            break;
                        }
                    }
                }
            }
            Type::Ref {
                region, referent, ..
            } => {
                if let Some(region) = self.term(*region, parameter, path, prefix)? {
                    if self.rows.len() >= ir::MAX_LOAN_SITES {
                        return Err("source carrier region resource limit exceeded".into());
                    }
                    self.budget.retain_bytes(
                        std::mem::size_of::<ir::SourceCarrierRegion>()
                            + path.len() * std::mem::size_of::<ir::CarrierStep>(),
                    )?;
                    self.rows.push(ir::SourceCarrierRegion {
                        parameter,
                        location: ir::CarrierLocation {
                            path: ir::CarrierPath(path.clone().into()),
                            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                                depth: prefix,
                            }),
                        },
                        region,
                    });
                }
                let next = if let ir::Ty::Ref { referent, .. } = semantic {
                    referent.as_ref()
                } else {
                    semantic
                };
                self.walk(
                    referent,
                    next,
                    parameter,
                    path,
                    prefix
                        .checked_add(1)
                        .ok_or("source reference depth overflow")?,
                    depth + 1,
                )?;
            }
            _ => {
                let saved = path.len();
                path.extend((0..prefix).map(|_| ir::CarrierStep::Referent));
                match (source, semantic) {
                    (Type::ManuallyDrop(source), ir::Ty::ManuallyDrop(wrapper)) => {
                        source.require_kind(SourceCatalogKind::ManuallyDrop)?;
                        let argument = source
                            .argument
                            .as_ref()
                            .ok_or("missing ManuallyDrop carrier argument")?;
                        path.push(ir::CarrierStep::ManuallyDropPayload);
                        self.walk(argument, wrapper.argument(), parameter, path, 0, depth + 1)?;
                        path.pop();
                    }
                    (Type::Tuple(fields), ir::Ty::Tuple(actual)) => {
                        for (index, (field, actual)) in fields.iter().zip(actual.iter()).enumerate()
                        {
                            path.push(ir::CarrierStep::Field(index));
                            self.walk(field, actual, parameter, path, 0, depth + 1)?;
                            path.pop();
                        }
                    }
                    (Type::DependentArray(_, _), ir::Ty::DependentArray { element, .. }) => {
                        // A generic element family retains each caller component on
                        // substitution. Explicit element lifetimes instead need
                        // per-element source-region selectors, not one erased
                        // lifetime for the entire array family.
                        if explicit_array_reference(element, self.budget, depth + 1)? {
                            return Err("unsupported: explicit source reference regions in dependent array elements".into());
                        }
                    }
                    (Type::Array(field, _), ir::Ty::Array { element, len }) => {
                        for index in 0..*len {
                            path.push(ir::CarrierStep::ConstantIndex(index));
                            self.walk(field, element, parameter, path, 0, depth + 1)?;
                            path.pop();
                        }
                    }
                    (Type::Slice(field), ir::Ty::Slice { element })
                        if matches!(
                            element.as_ref(),
                            ir::Ty::Ref { .. }
                                | ir::Ty::Param(_)
                                | ir::Ty::Tuple(_)
                                | ir::Ty::Struct(_)
                        ) =>
                    {
                        path.push(ir::CarrierStep::SliceElement);
                        self.walk(field, element, parameter, path, 0, depth + 1)?;
                        path.pop();
                    }
                    (Type::Named(_) | Type::Apply(_), ir::Ty::Struct(_) | ir::Ty::Enum(_)) => {
                        self.nominal(source, semantic, parameter, path, depth + 1)?
                    }
                    _ => (),
                }
                path.truncate(saved);
            }
        }
        Ok(())
    }
    fn nominal(
        &mut self,
        source: &Type,
        semantic: &ir::Ty,
        parameter: Option<usize>,
        path: &mut Vec<ir::CarrierStep>,
        depth: usize,
    ) -> Result<(), String> {
        let (source_path, args, region_args) = match source {
            Type::Named(path) => (path.clone(), &[][..], &[][..]),
            Type::Apply(path) => {
                let segment = path.segments.last().ok_or("empty nominal source path")?;
                (
                    path.path(self.registry.names.as_ref())?,
                    segment.args.as_slice(),
                    segment.region_args.as_slice(),
                )
            }
            _ => return Ok(()),
        };
        let key =
            name_scope::path_key(self.registry.names.as_ref(), &source_path, Namespace::Type)?;
        let id = *self
            .registry
            .nominals
            .get(&key)
            .ok_or("unknown source nominal carrier")?;
        let info = self.registry.declaration(id)?;
        if let ir::Ty::Struct(value) = semantic
            && value.def().source_decl == id
            && value.native_sequence_binding().is_some_and(|binding| {
                binding.matches_original_template(value.def())
                    || binding.matches_source_substitution(value.def())
            })
            && let Some(element) = semantic.owned_sequence_element()
        {
            let [source_element] = args else {
                return Err("native source carrier type argument arity mismatch".into());
            };
            // Authority identifies the exact canonical source application, not
            // a lifetime. Walk its SOURCE argument so named regions, elision
            // and parameter families use the ordinary source traversal rules.
            // In particular, never recover a region from the substituted Ty.
            path.push(ir::CarrierStep::SliceElement);
            self.walk(source_element, element, parameter, path, 0, depth + 1)?;
            path.pop();
            return Ok(());
        }
        let (type_params, region_params, fields): (
            _,
            _,
            Vec<(ir::CarrierStep, &FieldDecl, ir::Ty)>,
        ) = match semantic {
            ir::Ty::Struct(value) => {
                let declaration = &self.registry.program.structs[info.source_index];
                (
                    &declaration.type_params,
                    &declaration.region_params,
                    declaration
                        .fields
                        .iter()
                        .zip(value.def().fields.read(self.budget)?.iter())
                        .enumerate()
                        .map(|(index, (source, actual))| {
                            (ir::CarrierStep::Field(index), source, actual.ty.clone())
                        })
                        .collect(),
                )
            }
            ir::Ty::Enum(value) => {
                let declaration = &self.registry.program.enums[info.source_index];
                let mut fields = Vec::new();
                for (index, (source, actual)) in declaration
                    .variants
                    .iter()
                    .zip(value.def().variants.iter())
                    .enumerate()
                {
                    for (field, (source, ty)) in source
                        .fields
                        .iter()
                        .zip(actual.fields.read(self.budget)?.iter())
                        .enumerate()
                    {
                        fields.push((
                            ir::CarrierStep::VariantField {
                                variant: ir::VariantId {
                                    owner: value.def().id,
                                    index,
                                },
                                field,
                            },
                            source,
                            ty.ty.clone(),
                        ));
                    }
                }
                (&declaration.type_params, &declaration.region_params, fields)
            }
            _ => return Ok(()),
        };
        if !region_args.is_empty() && region_args.len() != region_params.len() {
            return Err("nominal lifetime argument arity mismatch".into());
        }
        // Re-entering a canonical application already being traversed is a
        // finite recursive type behind indirection (`Vec<Self>`). Its fields
        // were or will be traversed by the ancestor frame, so a carrier-free
        // application contributes no new rows. A carrier-bearing one would
        // need unboundedly many rows and is rejected explicitly.
        let app = match semantic {
            ir::Ty::Struct(value) => value.id(),
            ir::Ty::Enum(value) => value.id(),
            _ => unreachable!(),
        };
        self.budget.charge_work(self.nominal_stack.len())?;
        if self.nominal_stack.contains(&app) {
            if ir::carrier_locations(semantic, self.budget)?.is_empty() {
                return Ok(());
            }
            return Err("unsupported: recursive type carrying references, type parameters or projections through indirection".into());
        }
        self.budget
            .retain_bytes(std::mem::size_of::<ir::NominalAppId>())?;
        self.nominal_stack
            .try_reserve(1)
            .map_err(|_| "source carrier stack allocation failed")?;
        self.nominal_stack.push(app);
        let result = self.nominal_fields(
            semantic,
            parameter,
            path,
            depth,
            NominalFields {
                id,
                type_params,
                region_params,
                args,
                region_args,
                fields,
            },
        );
        self.nominal_stack.pop();
        result
    }
    fn nominal_fields(
        &mut self,
        semantic: &ir::Ty,
        parameter: Option<usize>,
        path: &mut Vec<ir::CarrierStep>,
        depth: usize,
        nominal: NominalFields<'_, '_>,
    ) -> Result<(), String> {
        let NominalFields {
            id,
            type_params,
            region_params,
            args,
            region_args,
            fields,
        } = nominal;
        let pointer = match semantic {
            ir::Ty::Struct(value) => std::ptr::from_ref(value.def()) as usize,
            ir::Ty::Enum(value) => std::ptr::from_ref(value.def()) as usize,
            _ => unreachable!(),
        };
        let memo_key = (id, pointer, depth);
        let cacheable = type_params.is_empty()
            && region_params.is_empty()
            && args.is_empty()
            && region_args.is_empty();
        if cacheable {
            self.budget.charge_work(self.empty_nominals.len())?;
            if self.empty_nominals.contains(&memo_key) {
                return Ok(());
            }
        }
        let mut types = HashMap::new();
        for (binder, actual) in type_params.iter().zip(args) {
            types.insert(self.registry.parameter_key(*binder)?, actual);
        }
        let mut regions = HashMap::new();
        for (index, binder) in region_params.iter().enumerate() {
            regions.insert(
                self.lifetime_key(*binder)?,
                region_args.get(index).copied().unwrap_or(RegionName::Infer),
            );
        }
        for (step, field, actual) in fields {
            let field = substitute(
                self.registry,
                &field.ty,
                &types,
                &regions,
                self.budget,
                depth + 1,
            )?;
            path.push(step);
            self.walk(&field, &actual, parameter, path, 0, depth + 1)?;
            path.pop();
        }
        // Preserve first-visit source validation. Only a complete traversal
        // with no possible semantic carriers is independent of parameter,
        // prefix and path. Never memoize reference-bearing region mappings.
        if cacheable && ir::carrier_locations(semantic, self.budget)?.is_empty() {
            if self.empty_nominals.len() == self.empty_nominals.capacity() {
                let capacity = self
                    .empty_nominals
                    .len()
                    .checked_mul(2)
                    .ok_or("source carrier memo capacity overflow")?
                    .max(4);
                let storage = self.budget.reserve_bytes(
                    capacity
                        .checked_mul(std::mem::size_of::<(ir::DeclId, usize, usize)>())
                        .ok_or("source carrier memo allocation overflow")?,
                )?;
                let mut grown = Vec::new();
                grown
                    .try_reserve_exact(capacity)
                    .map_err(|_| "source carrier memo allocation failed")?;
                self.budget.charge_work(self.empty_nominals.len())?;
                grown.extend_from_slice(&self.empty_nominals);
                self.empty_nominals = grown;
                self.empty_nominal_storage = Some(storage);
            }
            self.empty_nominals.push(memo_key);
        }
        Ok(())
    }
}
struct NominalFields<'d, 's> {
    id: ir::DeclId,
    type_params: &'d [AstIdent],
    region_params: &'d [AstIdent],
    args: &'s [Type],
    region_args: &'s [RegionName],
    fields: Vec<(ir::CarrierStep, &'d FieldDecl, ir::Ty)>,
}
fn substitute(
    registry: &DeclRegistry,
    ty: &Type,
    types: &HashMap<GenericBindingKey, &Type>,
    regions: &HashMap<NameKey, RegionName>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Type, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let mut result = copy_source_type(ty, budget, depth).map_err(|error| error.to_string())?;
    fn region(
        registry: &DeclRegistry,
        value: &mut Option<RegionName>,
        regions: &HashMap<NameKey, RegionName>,
    ) -> Result<(), String> {
        if let Some(RegionName::Named(name)) = value {
            let key = registry
                .names
                .key(*name, LookupClass::LifetimeParameter)
                .map_err(native_adapter::name_diagnostic)?;
            if let Some(actual) = regions.get(&key) {
                *value = Some(*actual);
            }
        }
        Ok(())
    }
    match &mut result {
        Type::Param(name) => {
            if let Some(actual) = types.get(&registry.parameter_key(*name)?) {
                return copy_source_type(actual, budget, depth + 1)
                    .map_err(|error| error.to_string());
            }
        }
        Type::Ref {
            region: lifetime,
            referent,
            ..
        } => {
            region(registry, lifetime, regions)?;
            **referent = substitute(registry, referent, types, regions, budget, depth + 1)?;
        }
        Type::PhantomData(source) | Type::ManuallyDrop(source) | Type::Cell(source) => {
            if let Some(argument) = &mut source.argument {
                **argument = substitute(registry, argument, types, regions, budget, depth + 1)?;
            }
        }
        Type::FnPointer { inputs, output } => {
            for t in inputs.iter_mut().chain(std::iter::once(output.as_mut())) {
                *t = substitute(registry, t, types, regions, budget, depth + 1)?;
            }
        }
        Type::Tuple(fields) => {
            for field in fields {
                *field = substitute(registry, field, types, regions, budget, depth + 1)?;
            }
        }
        Type::DependentArray(element, _) | Type::Array(element, _) | Type::Slice(element) => {
            **element = substitute(registry, element, types, regions, budget, depth + 1)?
        }
        Type::Apply(path) => {
            for segment in &mut path.segments {
                for value in &mut segment.region_args {
                    let mut current = Some(*value);
                    region(registry, &mut current, regions)?;
                    *value = current.unwrap();
                }
                for arg in &mut segment.args {
                    *arg = substitute(registry, arg, types, regions, budget, depth + 1)?;
                }
                for binding in &mut segment.associated_types {
                    binding.value =
                        substitute(registry, &binding.value, types, regions, budget, depth + 1)?;
                }
            }
        }
        _ => (),
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trait_lifetime_binders_reach_methods_without_leaking_scope() {
        let source = parse("enum Maybe<T>{Some(T),None}trait Named<'a,T>{fn get<'b>(&'b self)->Maybe<&'a T>;fn keep(&self)->Maybe<&'a T>{loop{}}}trait Plain{fn plain(&self);}fn main(){}").unwrap();
        let inherited = source.traits[0].items[0].signature.regions.params[0];
        let key = |name| {
            source
                .names
                .key(name, LookupClass::LifetimeParameter)
                .unwrap()
        };
        assert_eq!(source.traits[0].items[0].signature.regions.params.len(), 2);
        assert_eq!(source.traits[0].items[1].signature.regions.params.len(), 1);
        assert_eq!(
            key(inherited),
            key(source.traits[0].items[1].signature.regions.params[0])
        );
        assert_eq!(
            key(inherited),
            key(source.traits[0].items[1]
                .default
                .as_ref()
                .unwrap()
                .regions
                .params[0])
        );
        assert!(
            source.traits[1].items[0]
                .signature
                .regions
                .params
                .is_empty()
        );
        let invalid = parse("enum Maybe<T>{Some(T),None}trait Missing<T>{fn get(&self)->Maybe<&'missing T>;}fn main(){}").unwrap();
        let error = match PlanCtx::from_program(&invalid, &ir::PlanBudget::default()) {
            Ok(_) => panic!("undeclared trait carrier lifetime was accepted"),
            Err(error) => error,
        };
        assert!(
            error.contains("undeclared source carrier lifetime"),
            "{error}"
        );
    }

    #[test]
    fn trait_self_slice_retains_declared_element_family() {
        let source =
            parse("trait Scan{fn scan(data:&[Self]) where Self:Sized;}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let declaration = plan.registry.trait_items[0][0];
        let regions = plan.source_region_env(declaration).unwrap();
        let family = regions
            .carriers
            .iter()
            .find(|row| matches!(row.region, ir::CarrierRegionTerm::Family { .. }))
            .expect("trait Self slice lost its original element family");
        assert_eq!(family.parameter, Some(0));
        assert_eq!(
            family.location.path.0.as_ref(),
            &[ir::CarrierStep::Referent, ir::CarrierStep::SliceElement]
        );
        let ir::CarrierRegionTerm::Family { parameter } = family.region else {
            unreachable!()
        };
        assert_eq!(
            family.location.channel,
            ir::CarrierChannel::ParamTail {
                parameter,
                prefix: 0
            }
        );
    }

    #[test]
    fn empty_nominal_memo_keeps_reference_paths_and_source_arity() {
        let source = parse("struct Empty{x:i32}struct Ref<'a>{r:&'a i32}fn f<'a>(a:Empty,b:Empty,c:Ref<'a>,d:Ref<'a>){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let regions = plan.source_region_env(declaration).unwrap();
        let paths = regions
            .carriers
            .iter()
            .map(|row| (row.parameter, row.location.path.0.to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                (Some(2), vec![ir::CarrierStep::Field(0)]),
                (Some(3), vec![ir::CarrierStep::Field(0)]),
            ]
        );
        let invalid =
            parse("struct Empty{x:i32}fn f(a:Empty,b:Empty<'static>){}fn main(){}").unwrap();
        assert!(PlanCtx::from_program(&invalid, &ir::PlanBudget::default()).is_err());
    }

    #[test]
    fn nominal_lifetime_order_survives_parse_copy_and_source_authority() {
        let source = parse("struct W<'a,'b>{a:&'a i32,b:&'b i32}impl<'a,'b> W<'a,'b>{fn first(&self)->&'a i32{self.a}}fn main(){}").unwrap();
        assert_eq!(source.structs[0].region_params.len(), 2);
        let Type::Apply(path) = &source.impls[0].self_ty else {
            panic!("lifetime-only application was erased");
        };
        let args = &path.segments.last().unwrap().region_args;
        assert_eq!(args.len(), 2);
        let budget = ir::PlanBudget::default();
        let copied = copy_source_type(&source.impls[0].self_ty, &budget, 0).unwrap();
        assert!(
            exact_source_type(
                source.names.as_ref(),
                &source.impls[0].self_ty,
                &copied,
                &budget,
                0
            )
            .unwrap()
        );
        let mut forged = copied;
        let Type::Apply(path) = &mut forged else {
            unreachable!()
        };
        path.segments.last_mut().unwrap().region_args.swap(0, 1);
        assert!(
            !exact_source_type(
                source.names.as_ref(),
                &source.impls[0].self_ty,
                &forged,
                &budget,
                0
            )
            .unwrap()
        );
    }
}
