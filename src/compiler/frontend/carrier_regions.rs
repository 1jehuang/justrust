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
        budget,
        rows: Vec::new(),
        next_region: original.region_count,
        empty_nominals: Vec::new(),
        empty_nominal_storage: None,
    };
    let offset = usize::from(receiver != ReceiverKind::None);
    if let Some(block) = implementation {
        let source = match receiver {
            ReceiverKind::Borrowed { kind } => Type::Ref {
                kind,
                region: regions.receiver,
                referent: Box::new(block.self_ty.clone()),
            },
            _ => block.self_ty.clone(),
        };
        if offset != 0 {
            collector.walk(
                &source,
                &signature.params[0],
                Some(0),
                &mut Vec::new(),
                0,
                0,
            )?;
        }
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
    collector.walk(result, &signature.result, None, &mut Vec::new(), 0, 0)?;
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

struct Collector<'a> {
    registry: &'a DeclRegistry,
    regions: &'a RegionDecls,
    implementation: Option<&'a ImplDecl>,
    signature: &'a ir::Signature,
    declaration: ir::DeclId,
    budget: &'a ir::PlanBudget,
    rows: Vec<ir::SourceCarrierRegion>,
    next_region: u16,
    // Completed, carrier-free source subtrees only. Depth remains part of the
    // key so a deeper reuse cannot bypass any source traversal depth check.
    empty_nominals: Vec<(ir::DeclId, usize, usize)>,
    empty_nominal_storage: Option<ir::PlanReservation>,
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
        if path.contains(&ir::CarrierStep::SliceElement)
            && matches!(region, Some(RegionName::Static))
        {
            return Err("unsupported: explicit 'static slice element references require static input authority".into());
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
        } else if self.implementation.is_some() {
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
                    (Type::Tuple(fields), ir::Ty::Tuple(actual)) => {
                        for (index, (field, actual)) in fields.iter().zip(actual.iter()).enumerate()
                        {
                            path.push(ir::CarrierStep::Field(index));
                            self.walk(field, actual, parameter, path, 0, depth + 1)?;
                            path.pop();
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
                        if matches!(element.as_ref(), ir::Ty::Ref { .. }) =>
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
        let (type_params, region_params, fields): (
            _,
            _,
            Vec<(ir::CarrierStep, &FieldDecl, &ir::Ty)>,
        ) = match semantic {
            ir::Ty::Struct(value) => {
                let declaration = &self.registry.program.structs[info.source_index];
                (
                    &declaration.type_params,
                    &declaration.region_params,
                    declaration
                        .fields
                        .iter()
                        .zip(value.def().fields.iter())
                        .enumerate()
                        .map(|(index, (source, actual))| {
                            (ir::CarrierStep::Field(index), source, &actual.ty)
                        })
                        .collect(),
                )
            }
            ir::Ty::Enum(value) => {
                let declaration = &self.registry.program.enums[info.source_index];
                (
                    &declaration.type_params,
                    &declaration.region_params,
                    declaration
                        .variants
                        .iter()
                        .zip(value.def().variants.iter())
                        .enumerate()
                        .flat_map(|(index, (source, actual))| {
                            source
                                .fields
                                .iter()
                                .zip(actual.fields.iter())
                                .enumerate()
                                .map(move |(field, (source, ty))| {
                                    (
                                        ir::CarrierStep::VariantField {
                                            variant: ir::VariantId {
                                                owner: value.def().id,
                                                index,
                                            },
                                            field,
                                        },
                                        source,
                                        &ty.ty,
                                    )
                                })
                        })
                        .collect(),
                )
            }
            _ => return Ok(()),
        };
        if !region_args.is_empty() && region_args.len() != region_params.len() {
            return Err("nominal lifetime argument arity mismatch".into());
        }
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
            self.walk(&field, actual, parameter, path, 0, depth + 1)?;
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
        Type::Tuple(fields) => {
            for field in fields {
                *field = substitute(registry, field, types, regions, budget, depth + 1)?;
            }
        }
        Type::Array(element, _) | Type::Slice(element) => {
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
