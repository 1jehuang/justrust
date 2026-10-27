use super::*;
use std::fmt::Write as _;

/// Node-local syntax receipts. Only fields of the retained immutable schema
/// enter this cache, never caller-owned syntax or IR type payloads.
#[derive(Debug, Default)]
pub(super) struct SourceMetadataCache {
    registry: Option<Arc<DeclRegistry>>,
    parameters: HashMap<ir::DeclId, Arc<[GenericBindingKey]>>,
    fields: HashMap<(ir::DeclId, usize), Arc<SourceMetadata>>,
    active: Option<Arc<SourceMetadata>>,
}

#[derive(Debug)]
struct SourceMetadata {
    declaration: ir::DeclId,
    _schema: Arc<NominalSchema>,
    nodes: HashMap<usize, SourceNodeMetadata>,
    bindings: Vec<GenericBindingKey>,
}

/// Resolved syntax coordinates only, never a substituted IR type or proof.
#[derive(Clone, Copy, Debug)]
enum SourceNodeMetadata {
    Other,
    Parameter(GenericBindingKey),
    Nominal(ir::DeclId),
}

/// Stack-local evidence for this exact immutable binding map, caller and meter.
/// This is deliberately not retained in the syntax cache. In particular,
/// `validate_argument` is weaker than metadata authentication for some carriers
/// and cannot mint this evidence.
struct SourceBindingMetadata<'a> {
    bindings: &'a HashMap<GenericBindingKey, ir::Ty>,
    caller: ir::FactsCtx<'a>,
    budget: &'a ir::PlanBudget,
}

impl<'a> SourceBindingMetadata<'a> {
    fn validate(
        bindings: &'a HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'a>,
        budget: &'a ir::PlanBudget,
    ) -> Result<Self, String> {
        for actual in bindings.values() {
            budget.charge_work(1)?;
            caller.validate_ty_metadata_with_budget(actual, budget)?;
        }
        Ok(Self {
            bindings,
            caller,
            budget,
        })
    }
}

impl SourceMetadataCache {
    pub(super) fn new(registry: Arc<DeclRegistry>) -> Self {
        Self {
            registry: Some(registry),
            parameters: HashMap::new(),
            fields: HashMap::new(),
            active: None,
        }
    }

    fn covers(
        &self,
        registry: &DeclRegistry,
        context: Option<SourceTypeContext>,
        ty: &Type,
    ) -> bool {
        self.node(registry, context, ty).is_some()
    }

    fn node(
        &self,
        registry: &DeclRegistry,
        context: Option<SourceTypeContext>,
        ty: &Type,
    ) -> Option<SourceNodeMetadata> {
        if !self
            .registry
            .as_ref()
            .is_some_and(|cached| std::ptr::eq(cached.as_ref(), registry))
        {
            return None;
        }
        let metadata = self.active.as_ref()?;
        if !context.is_some_and(|context| context.declaration == metadata.declaration) {
            return None;
        }
        metadata.nodes.get(&(ty as *const Type as usize)).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SchemaFieldKey {
    Named(NameKey),
    Tuple(usize),
}

fn schema_field_key(names: &NameStore, name: FieldName) -> Result<SchemaFieldKey, String> {
    Ok(match name {
        FieldName::Named(name) => SchemaFieldKey::Named(
            names
                .key(name, LookupClass::Member)
                .map_err(native_adapter::name_diagnostic)?,
        ),
        FieldName::Tuple(index) => SchemaFieldKey::Tuple(index),
    })
}

impl PlanState {
    fn schema_param_bindings(
        &mut self,
        registry: &DeclRegistry,
        info: &DeclInfo,
        args: &[ir::Ty],
        budget: &ir::PlanBudget,
    ) -> Result<
        (
            HashMap<GenericBindingKey, ir::Ty>,
            Option<ir::PlanReservation>,
        ),
        String,
    > {
        if !self
            .source_metadata
            .registry
            .as_ref()
            .is_some_and(|cached| std::ptr::eq(cached.as_ref(), registry))
        {
            return Ok((param_bindings(registry, info, args)?, None));
        }
        if info.type_params.len() != args.len() {
            return Err(format!(
                "wrong generic arity for {}; omitted generic inference is unsupported",
                info.name
            ));
        }
        let keys = if let Some(keys) = self.source_metadata.parameters.get(&info.id) {
            keys.clone()
        } else {
            self.retain(
                budget,
                info.type_params
                    .len()
                    .checked_mul(4 * std::mem::size_of::<GenericBindingKey>())
                    .and_then(|bytes| bytes.checked_add(256))
                    .ok_or("source parameter allocation overflow")?,
            )?;
            let mut keys = Vec::new();
            keys.try_reserve_exact(info.type_params.len())
                .map_err(|_| "source parameter allocation failed")?;
            for name in info.type_params.iter() {
                budget.charge_work(1)?;
                keys.push(registry.parameter_key(*name)?);
            }
            let keys: Arc<[GenericBindingKey]> = keys.into();
            self.source_metadata
                .parameters
                .try_reserve(1)
                .map_err(|_| "source parameter cache allocation failed")?;
            self.source_metadata
                .parameters
                .insert(info.id, keys.clone());
            keys
        };
        let storage = budget.reserve_bytes(
            args.len()
                .checked_mul(4 * std::mem::size_of::<(GenericBindingKey, ir::Ty)>())
                .and_then(|bytes| bytes.checked_add(128))
                .ok_or("source binding allocation overflow")?,
        )?;
        let mut bindings = HashMap::new();
        bindings
            .try_reserve(args.len())
            .map_err(|_| "source binding allocation failed")?;
        for (key, actual) in keys.iter().zip(args) {
            budget.charge_work(1)?;
            bindings.insert(*key, actual.clone());
        }
        Ok((bindings, Some(storage)))
    }

    /// The caller supplies a field borrowed directly from `self.schemas`.
    /// Its allocation remains live for this scratch's entire lifetime. The
    /// receipt authenticates syntax only. The stack-local binding evidence
    /// separately checks current actuals against their exact current caller.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn source_field_metadata(
        &mut self,
        registry: &DeclRegistry,
        declaration: ir::DeclId,
        schema: &Arc<NominalSchema>,
        ty: &Type,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<SourceMetadata>, String> {
        let bindings = SourceBindingMetadata::validate(bindings, caller, budget)?;
        self.source_field_metadata_with_bindings(registry, declaration, schema, ty, &bindings)
    }

    fn source_field_metadata_with_bindings(
        &mut self,
        registry: &DeclRegistry,
        declaration: ir::DeclId,
        schema: &Arc<NominalSchema>,
        ty: &Type,
        authenticated: &SourceBindingMetadata<'_>,
    ) -> Result<Arc<SourceMetadata>, String> {
        let SourceBindingMetadata {
            bindings,
            caller,
            budget,
        } = authenticated;
        if !self
            .source_metadata
            .registry
            .as_ref()
            .is_some_and(|cached| std::ptr::eq(cached.as_ref(), registry))
        {
            return Err("nominal source metadata belongs to a different registry".into());
        }
        let key = (declaration, ty as *const Type as usize);
        budget.charge_work(1)?;
        if let Some(metadata) = self.source_metadata.fields.get(&key) {
            for key in &metadata.bindings {
                budget.charge_work(1)?;
                // Every value in this exact map was authenticated by the
                // evidence constructor. Existence is still syntax authority.
                bindings.get(key).ok_or("missing source type binding")?;
            }
            return Ok(metadata.clone());
        }
        self.validate_source_type_metadata(registry, ty, bindings, *caller, budget)?;
        self.retain(budget, 256)?;
        let mut metadata = SourceMetadata {
            declaration,
            _schema: schema.clone(),
            nodes: HashMap::new(),
            bindings: Vec::new(),
        };
        let mut inspect = |node: &DepthNode<'_>, depth| {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            if let DepthNode::Type(ty) = node {
                self.retain(budget, 128)?;
                metadata
                    .nodes
                    .try_reserve(1)
                    .map_err(|_| "source metadata allocation failed")?;
                let key = match ty {
                    Type::Param(parameter) => Some(registry.parameter_key(*parameter)?),
                    Type::SelfType => Some(GenericBindingKey::SelfType),
                    _ => None,
                };
                let coordinate = if let Some(key) = key {
                    SourceNodeMetadata::Parameter(key)
                } else {
                    let name = match ty {
                        Type::Named(path) => Some(name_scope::path_key(
                            registry.names.as_ref(),
                            path,
                            Namespace::Type,
                        )?),
                        Type::Apply(path) => Some(name_scope::path_key(
                            registry.names.as_ref(),
                            &path.path(registry.names.as_ref())?,
                            Namespace::Type,
                        )?),
                        _ => None,
                    };
                    if let Some(name) = name {
                        SourceNodeMetadata::Nominal(
                            *registry.nominals.get(&name).ok_or("unknown nominal type")?,
                        )
                    } else {
                        SourceNodeMetadata::Other
                    }
                };
                metadata
                    .nodes
                    .insert(*ty as *const Type as usize, coordinate);
                if let Some(key) = key {
                    // Preserve occurrences rather than performing an unmetered
                    // search. Repeated actual authentication is conservative.
                    self.retain(budget, 64)?;
                    metadata
                        .bindings
                        .try_reserve(1)
                        .map_err(|_| "source binding allocation failed")?;
                    metadata.bindings.push(key);
                }
            }
            Ok(())
        };
        walk_depth_inspecting(DepthNode::Type(ty), None, None, Some(&mut inspect))?;
        let metadata = Arc::new(metadata);
        self.source_metadata
            .fields
            .try_reserve(1)
            .map_err(|_| "source metadata cache allocation failed")?;
        self.source_metadata.fields.insert(key, metadata.clone());
        Ok(metadata)
    }

    pub(super) fn retain(&mut self, budget: &ir::PlanBudget, bytes: usize) -> Result<(), String> {
        let reservation = budget.reserve_bytes(
            bytes
                .checked_add(4 * std::mem::size_of::<ir::PlanReservation>() + 64)
                .ok_or("planning reservation allocation overflow")?,
        )?;
        self.reservations
            .try_reserve(1)
            .map_err(|_| "planning reservation allocation failed")?;
        self.reservations.push(reservation);
        Ok(())
    }

    // These copies are diagnostic IR payload only. Source lookup always uses
    // the checked source schema and its structural keys, never this String.
    fn schema_text(&mut self, text: &str, budget: &ir::PlanBudget) -> Result<String, String> {
        budget.charge_work(text.len())?;
        self.retain(budget, text.len())?;
        let mut result = String::new();
        result
            .try_reserve_exact(text.len())
            .map_err(|_| "schema diagnostic allocation failed")?;
        if result.capacity() > text.len() {
            self.retain(budget, result.capacity() - text.len())?;
        }
        result.push_str(text);
        Ok(result)
    }

    /// The declaration's display path, charged to this computation's plan
    /// budget. Re-interning an application in a fresh per-call scratch must
    /// not spend the shared name store's diagnostic budget each time.
    fn schema_path_text(
        &mut self,
        registry: &DeclRegistry,
        path: &AstPath,
        budget: &ir::PlanBudget,
    ) -> Result<String, String> {
        let names = registry.names.as_ref();
        let mut text = String::new();
        match path.anchor() {
            PathAnchor::Relative | PathAnchor::Super(0) => {}
            PathAnchor::Crate => text.push_str("crate"),
            PathAnchor::SelfModule => text.push_str("self"),
            PathAnchor::SelfType => text.push_str("Self"),
            PathAnchor::Super(count) => {
                for index in 0..count {
                    if index > 0 {
                        text.push_str("::");
                    }
                    text.push_str("super");
                }
            }
        }
        for ident in path.segments() {
            budget.charge_work(1)?;
            if !text.is_empty() {
                text.push_str("::");
            }
            text.push_str(
                names
                    .spelling(*ident)
                    .map_err(native_adapter::name_diagnostic)?,
            );
        }
        self.schema_text(&text, budget)
    }

    fn schema_field_text(
        &mut self,
        names: &NameStore,
        name: FieldName,
        budget: &ir::PlanBudget,
    ) -> Result<String, String> {
        match name {
            FieldName::Named(name) => self.schema_text(
                names
                    .spelling(name)
                    .map_err(native_adapter::name_diagnostic)?,
                budget,
            ),
            FieldName::Tuple(index) => {
                let digits = index.checked_ilog10().map_or(1, |log| log as usize + 1);
                budget.charge_work(digits)?;
                self.retain(budget, digits)?;
                let mut text = String::new();
                text.try_reserve_exact(digits)
                    .map_err(|_| "tuple field diagnostic allocation failed")?;
                if text.capacity() > digits {
                    self.retain(budget, text.capacity() - digits)?;
                }
                write!(&mut text, "{index}")
                    .map_err(|_| "tuple field diagnostic formatting failed")?;
                Ok(text)
            }
        }
    }

    fn schema_identity(
        &self,
        registry: &DeclRegistry,
        decl: ir::DeclId,
        path: &AstPath,
        module: ModuleId,
        visibility: &ir::Visibility,
    ) -> Result<(), String> {
        registry
            .modules
            .check_names(registry.names.as_ref())
            .map_err(module_diagnostic)?;
        visibility
            .validate_for(&registry.modules, module)
            .map_err(module_diagnostic)?;
        if registry.declaration(decl)?.path.module != module
            || registry.nominals.get(&name_scope::path_key(
                registry.names.as_ref(),
                path,
                Namespace::Type,
            )?) != Some(&decl)
        {
            return Err("nominal schema belongs to a different source declaration".into());
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn source_ty(
        &mut self,
        registry: &DeclRegistry,
        ty: &Type,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
        allow_ref: bool,
    ) -> Result<ir::Ty, String> {
        self.source_ty_with_transport(
            registry, ty, bindings, caller, budget, depth, allow_ref, None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn source_ty_with_transport(
        &mut self,
        registry: &DeclRegistry,
        ty: &Type,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
        allow_ref: bool,
        transport: Option<&ir::NominalSubstitutionRequest<'_>>,
    ) -> Result<ir::Ty, String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        budget.charge_nodes(1)?;
        Ok(match ty {
            Type::Opaque(opaque) => {
                self.source_opaque_type(registry, opaque, bindings, caller, budget, depth + 1)?
            }
            Type::Projection(projection) => {
                let context = self
                    .source_context
                    .ok_or("projection without retained source declaration context")?;
                registry.validate_source_projection(context, projection, budget)?;
                let (trait_ref, item) = match projection.as_ref() {
                    SourceProjection::Qualified {
                        self_ty,
                        trait_path,
                        item,
                    } => {
                        registry
                            .names
                            .check(*item)
                            .map_err(native_adapter::name_diagnostic)?;
                        let self_ty = self.source_ty_with_transport(
                            registry,
                            self_ty,
                            bindings,
                            caller,
                            budget,
                            depth + 1,
                            false,
                            transport,
                        )?;
                        (
                            self.source_trait_application(
                                registry, trait_path, &self_ty, bindings, caller, budget,
                            )?
                            .reject_equalities()?,
                            *item,
                        )
                    }
                    SourceProjection::SelfItem { item } => {
                        registry
                            .names
                            .check(*item)
                            .map_err(native_adapter::name_diagnostic)?;
                        let context = self
                            .source_context
                            .ok_or("Self associated type without retained source owner")?;
                        registry
                            .modules
                            .check(context.module)
                            .map_err(module_diagnostic)?;
                        let owner = registry.declaration(
                            context
                                .owner
                                .ok_or("Self associated type outside trait owner")?,
                        )?;
                        if owner.path.module != context.module {
                            return Err("source associated owner module changed".into());
                        }
                        let self_ty = bindings
                            .get(&GenericBindingKey::SelfType)
                            .cloned()
                            .ok_or("Self projection has no source Self type")?;
                        let trait_ref = match owner.kind {
                            ir::DeclKind::Trait => {
                                let mut args = symbolic_args(&owner.param_env, budget)?;
                                if args.is_empty() {
                                    return Err("trait Self environment is empty".into());
                                }
                                args.remove(0);
                                ir::TraitRef {
                                    declaration: owner.id,
                                    self_ty,
                                    args: args.into(),
                                }
                            }
                            ir::DeclKind::TraitImpl => {
                                let block = registry
                                    .program
                                    .impls
                                    .get(owner.source_index)
                                    .ok_or("missing retained projection impl")?;
                                if block.module != context.module {
                                    return Err("retained impl module changed".into());
                                }
                                self.source_trait_application(
                                    registry,
                                    block
                                        .trait_path
                                        .as_ref()
                                        .ok_or("Self projection requires trait impl")?,
                                    &self_ty,
                                    bindings,
                                    caller,
                                    budget,
                                )?
                                .reject_equalities()?
                            }
                            _ => return Err("Self projection requires retained trait owner".into()),
                        };
                        (trait_ref, *item)
                    }
                };
                let (trait_ref, item) = self
                    .inherited_associated_owner(
                        registry,
                        trait_ref,
                        item,
                        caller,
                        budget,
                        &mut Vec::new(),
                    )?
                    .ok_or("no associated type in trait or its supertraits")?;
                budget.retain_bytes(
                    std::mem::size_of::<ir::AssociatedTypeProjection>()
                        + 2 * std::mem::size_of::<usize>(),
                )?;
                ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection { trait_ref, item }))
            }
            Type::Param(p) => {
                let key = match self.source_metadata.node(registry, self.source_context, ty) {
                    Some(SourceNodeMetadata::Parameter(key)) => key,
                    _ => registry.parameter_key(*p)?,
                };
                bindings
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| "unknown generic parameter".to_owned())?
            }
            Type::Apply(path) => {
                if !self
                    .source_metadata
                    .covers(registry, self.source_context, ty)
                {
                    self.validate_source_type_metadata(registry, ty, bindings, caller, budget)?;
                }
                if path
                    .segments
                    .iter()
                    .any(|segment| !segment.associated_types.is_empty())
                {
                    return Err(
                        "unsupported: associated equalities in nominal type applications".into(),
                    );
                }
                let (at, segment) = path
                    .segments
                    .iter()
                    .enumerate()
                    .find(|(_, s)| !s.region_args.is_empty() || !s.args.is_empty())
                    .ok_or("missing type arguments")?;
                if at + 1 != path.segments.len() {
                    return Err("generic associated types are unsupported".into());
                }
                let decl = match self.source_metadata.node(registry, self.source_context, ty) {
                    Some(SourceNodeMetadata::Nominal(declaration)) => declaration,
                    _ => {
                        let name = name_scope::path_key(
                            registry.names.as_ref(),
                            &path.path(registry.names.as_ref())?,
                            Namespace::Type,
                        )?;
                        *registry
                            .nominals
                            .get(&name)
                            .ok_or_else(|| "unknown nominal type".to_owned())?
                    }
                };
                let args = segment
                    .args
                    .iter()
                    .map(|arg| {
                        // A trait object as a direct nominal actual (`Box<dyn P>`);
                        // `?Sized` admission is the parameter's Sized row.
                        if let Type::Dyn(trait_path) = arg {
                            return self.source_dynamic_type(
                                registry, trait_path, bindings, caller, budget,
                            );
                        }
                        self.source_ty_with_transport(
                            registry,
                            arg,
                            bindings,
                            caller,
                            budget,
                            depth + 1,
                            false,
                            transport,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.intern_with_transport(
                    registry,
                    decl,
                    &args,
                    caller,
                    budget,
                    depth + 1,
                    transport,
                )?
            }
            Type::SelfType => bindings
                .get(&GenericBindingKey::SelfType)
                .cloned()
                .ok_or("Self is not in an impl scope")?,
            Type::Named(name) => {
                let decl = match self.source_metadata.node(registry, self.source_context, ty) {
                    Some(SourceNodeMetadata::Nominal(declaration)) => declaration,
                    _ => *registry
                        .nominals
                        .get(&name_scope::path_key(
                            registry.names.as_ref(),
                            name,
                            Namespace::Type,
                        )?)
                        .ok_or_else(|| "unknown nominal type".to_owned())?,
                };
                self.intern_with_transport(
                    registry,
                    decl,
                    &[],
                    caller,
                    budget,
                    depth + 1,
                    transport,
                )?
            }
            Type::Ref {
                kind,
                referent: slice,
                ..
            } if matches!(slice.as_ref(), Type::Slice(_)) => {
                admit_shared_slice_kind(*kind, slice)?;
                let Type::Slice(element) = slice.as_ref() else {
                    return Err("invalid shared slice source shape".into());
                };
                if matches!(
                    element.as_ref(),
                    Type::Ref {
                        region: Some(RegionName::Static),
                        ..
                    }
                ) {
                    return Err("unsupported: explicit 'static slice element references require static input authority".into());
                }
                let element = self.source_ty_with_transport(
                    registry,
                    element,
                    bindings,
                    caller,
                    budget,
                    depth + 1,
                    false,
                    transport,
                )?;
                if matches!(caller, ir::FactsCtx::Universal(_))
                    && matches!(&element, ir::Ty::AssociatedType(_))
                {
                    // Raw trait headers retain binder-owned projections before
                    // a world exists. This proves metadata only, not a Sized
                    // element or executable carrier. Validated admission must
                    // normalize and recheck the actual element later.
                    caller.validate_ty_metadata_with_budget(&element, budget)?;
                } else {
                    ir::validate_shared_slice_element(&element, caller, budget)?;
                }
                ir::Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(ir::Ty::Slice {
                        element: Arc::new(element),
                    }),
                }
            }
            Type::Ref {
                kind: ir::BorrowKind::Shared,
                region: Some(RegionName::Static),
                referent,
            } if matches!(referent.as_ref(), Type::Str) => {
                // `&'static str` carries no borrowed origin, so it is admitted
                // in aggregate positions such as struct fields.
                ir::Ty::shared_str_ty()
            }
            Type::Ref {
                kind,
                referent: object,
                ..
            } if matches!(object.as_ref(), Type::Dyn(_)) => {
                let Type::Dyn(trait_path) = object.as_ref() else {
                    return Err("invalid trait object source shape".into());
                };
                ir::Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(
                        self.source_dynamic_type(registry, trait_path, bindings, caller, budget)?,
                    ),
                }
            }
            Type::Ref { kind, referent, .. } => {
                let _ = allow_ref;
                let layers = source_reference_layers(ty)?;
                let inner = self.source_ty_with_transport(
                    registry,
                    referent,
                    bindings,
                    caller,
                    budget,
                    depth + 1,
                    layers > 1,
                    transport,
                )?;
                ir::Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(inner),
                }
            }
            // Bare `[T]` as an impl Self type (`impl Tr for [T]`); value
            // positions are rejected by the slice admission checks.
            Type::Slice(element) if depth == 0 => {
                let element = self.source_ty_with_transport(
                    registry,
                    element,
                    bindings,
                    caller,
                    budget,
                    depth + 1,
                    false,
                    transport,
                )?;
                ir::Ty::Slice {
                    element: Arc::new(element),
                }
            }
            Type::Slice(_) => {
                return Err(BARE_SLICE_UNSUPPORTED.into());
            }
            Type::RawPtr { kind, pointee }
                if matches!(
                    pointee.as_ref(),
                    Type::Ref { kind: ir::BorrowKind::Shared, referent, .. }
                        if matches!(referent.as_ref(), Type::Str)
                ) =>
            {
                // A raw pointer to a `&str` word pair carries no loan; its
                // pointee is the origin-free string descriptor.
                ir::Ty::RawPtr {
                    kind: *kind,
                    pointee: Arc::new(ir::Ty::shared_str_ty()),
                }
            }
            Type::RawPtr { kind, pointee } if matches!(pointee.as_ref(), Type::Dyn(_)) => {
                // `*const dyn P` / `*mut dyn P`: a two-word (data, vtable)
                // pointer with the same canonical object identity as `&dyn P`.
                let Type::Dyn(trait_path) = pointee.as_ref() else {
                    return Err("invalid trait object source shape".into());
                };
                ir::Ty::RawPtr {
                    kind: *kind,
                    pointee: Arc::new(
                        self.source_dynamic_type(registry, trait_path, bindings, caller, budget)?,
                    ),
                }
            }
            Type::RawPtr { kind, pointee } => ir::Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(self.source_ty_with_transport(
                    registry,
                    pointee,
                    bindings,
                    caller,
                    budget,
                    depth + 1,
                    false,
                    transport,
                )?),
            },
            Type::Dyn(_) => {
                return Err(
                    "unsupported: trait object types are only admitted directly behind a reference"
                        .into(),
                );
            }
            Type::Array(element, len) => {
                if *len > ir::MAX_ARRAY_TYPE_LEN {
                    return Err("array length exceeds supported limit".into());
                }
                ir::Ty::Array {
                    element: Arc::new(self.source_ty_with_transport(
                        registry,
                        element,
                        bindings,
                        caller,
                        budget,
                        depth + 1,
                        false,
                        transport,
                    )?),
                    len: *len,
                }
            }
            Type::Tuple(fields) => {
                if fields.is_empty() {
                    return Err("empty tuple types must use ()".into());
                }
                ir::Ty::Tuple(
                    fields
                        .iter()
                        .map(|t| {
                            self.source_ty_with_transport(
                                registry,
                                t,
                                bindings,
                                caller,
                                budget,
                                depth + 1,
                                false,
                                transport,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            Type::Int(i) => ir::Ty::Int(*i),
            Type::Float(f) => ir::Ty::Float(*f),
            Type::Char => ir::Ty::Char,
            Type::Str => ir::Ty::Str,
            Type::Bool => ir::Ty::Bool,
            Type::Unit => ir::Ty::Unit,
            Type::Never => ir::Ty::Never,
            Type::Infer => {
                return Err(
                    "signature types cannot be inferred (E0121): `_` is not allowed in item signatures"
                        .into(),
                );
            }
        })
    }
    pub(super) fn intern(
        &mut self,
        registry: &DeclRegistry,
        decl: ir::DeclId,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<ir::Ty, String> {
        self.intern_with_transport(registry, decl, args, caller, budget, depth, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn intern_with_transport(
        &mut self,
        registry: &DeclRegistry,
        decl: ir::DeclId,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        depth: usize,
        transport: Option<&ir::NominalSubstitutionRequest<'_>>,
    ) -> Result<ir::Ty, String> {
        budget.check_depth(depth)?;
        budget.charge_work(args.len().checked_add(1).ok_or("nominal work overflow")?)?;
        let info = registry.declaration(decl)?;
        // Arity is authority even on a cache hit. Parameter-name bindings are
        // only needed when a fresh schema application must be constructed.
        if info.type_params.len() != args.len() {
            return Err(format!(
                "wrong generic arity for {}; omitted generic inference is unsupported",
                info.name
            ));
        }
        for arg in args {
            if transport.is_some() {
                // Only IR's unforgeable structural request can reach this path.
                // The produced application is not an executable body proof.
                let structural = match caller {
                    ir::FactsCtx::Validated(facts) => ir::FactsCtx::Universal(facts.env()),
                    facts => facts,
                };
                structural.validate_ty_metadata_with_budget(arg, budget)?;
                if !raw_type_has_projection(arg, budget, 0)? {
                    ir::carrier_locations(arg, budget)?;
                }
            } else {
                validate_argument(arg, caller, budget, depth + 1)?;
            }
        }
        let key_bytes = args
            .len()
            .checked_mul(std::mem::size_of::<ir::Ty>())
            .ok_or("generic key allocation overflow")?;
        let _key_scratch = budget.reserve_bytes(key_bytes)?;
        let mut key_args = Vec::new();
        key_args
            .try_reserve_exact(args.len())
            .map_err(|_| "generic key allocation failed")?;
        key_args.extend_from_slice(args);
        let application_key = (decl, key_args);
        if let Some(ty) = self
            .nominals
            .base
            .applications
            .get(&application_key)
            .or_else(|| self.nominals.applications.get(&application_key))
        {
            return Ok(ty.clone());
        }
        if let Some((_, back)) = self
            .nominals
            .building
            .iter()
            .find(|(key, _)| *key == application_key)
        {
            // Recursive reference to an application under construction. A
            // by-value cycle is rejected once the outer definition completes.
            return Ok(back.clone());
        }
        let (bindings, _bindings_storage) =
            self.schema_param_bindings(registry, info, args, budget)?;
        if self.nominals.active.contains(&decl) {
            return Err(format!(
                "unsupported: polymorphically recursive nominal {}",
                info.name
            ));
        }
        // Distinct applications are bounded per interner: discarded per-call
        // scratch interners re-derive the same canonical applications, so a
        // node-wide count would grow with occurrences, not distinct types.
        // Node-wide cost stays bounded by the work and byte ledgers.
        if self.nominals.applications.len() + self.nominals.building.len()
            >= budget.limits().applications
        {
            budget.charge_application()?;
        }
        self.retain(
            budget,
            512usize
                .checked_add(std::mem::size_of_val(args))
                .ok_or("nominal allocation overflow")?,
        )?;
        let active_len = self.nominals.active.len();
        self.nominals
            .active
            .try_reserve(1)
            .map_err(|_| "nominal active allocation failed")?;
        self.nominals.active.push(decl);
        let result = (|| -> Result<ir::Ty, String> {
            let source_bindings = self
                .source_metadata
                .registry
                .as_ref()
                .map(|_| SourceBindingMetadata::validate(&bindings, caller, budget))
                .transpose()?;
            let schema = self
                .schemas
                .get(&decl)
                .cloned()
                .ok_or("declaration is not a nominal schema")?;
            let make_fields = |state: &mut Self,
                               fields: &[FieldDecl],
                               module: ModuleId|
             -> Result<Arc<[ir::FieldDef]>, String> {
                if fields.len() > 4096 {
                    return Err("nominal field resource limit exceeded".into());
                }
                state.retain(
                    budget,
                    fields
                        .len()
                        .checked_mul(4 * std::mem::size_of::<ir::FieldDef>() + 128)
                        .ok_or("nominal field allocation overflow")?,
                )?;
                let mut names = std::collections::HashSet::new();
                let mut result = Vec::new();
                names
                    .try_reserve(fields.len())
                    .map_err(|_| "nominal field key allocation failed")?;
                result
                    .try_reserve_exact(fields.len())
                    .map_err(|_| "nominal field allocation failed")?;
                for field in fields {
                    budget.charge_work(1)?;
                    clone_visibility(&field.visibility, budget)?;
                    field
                        .visibility
                        .validate_for(&registry.modules, module)
                        .map_err(module_diagnostic)?;
                    let key = schema_field_key(registry.names.as_ref(), field.name)?;
                    if !names.insert(key) {
                        return Err("duplicate field".into());
                    }
                    let name =
                        state.schema_field_text(registry.names.as_ref(), field.name, budget)?;
                    if source_elides_lifetime(&field.ty) {
                        return Err("E0106: missing lifetime specifier in a field type; declare a lifetime parameter".into());
                    }
                    let previous_context = state.source_context;
                    state.source_context = Some(registry.source_type_context(decl)?);
                    let previous_metadata = state.source_metadata.active.take();
                    let ty = (|| {
                        // Construction scratch has a mutable, not-yet-sealed
                        // registry and deliberately cannot mint reusable receipts.
                        if let Some(source_bindings) = &source_bindings {
                            state.source_metadata.active =
                                Some(state.source_field_metadata_with_bindings(
                                    registry,
                                    decl,
                                    &schema,
                                    &field.ty,
                                    source_bindings,
                                )?);
                        }
                        state.source_ty_with_transport(
                            registry,
                            &field.ty,
                            &bindings,
                            caller,
                            budget,
                            depth + 1,
                            false,
                            transport,
                        )
                    })();
                    state.source_metadata.active = previous_metadata;
                    state.source_context = previous_context;
                    let ty = ty?;
                    result.push(ir::FieldDef {
                        name,
                        ty,
                        visibility: field.visibility.clone(),
                        lifetime_bound: source_names_lifetime(&field.ty),
                    });
                }
                Ok(result.into())
            };
            // Descendant applications are completed first, so every published Arc is acyclic.
            self.retain(
                budget,
                info.name
                    .len()
                    .checked_mul(2)
                    .ok_or("nominal name allocation overflow")?,
            )?;
            // The slot is a canonical digest of the resolved declaration and
            // its ordered actuals, never an interning-order ordinal, so every
            // node computes the same identity for the same application.
            let index = canonical_slot(&application_key);
            if let Some(existing) = self
                .nominals
                .base
                .by_slot
                .get(&index)
                .or_else(|| self.nominals.by_slot.get(&index))
            {
                let same = match existing {
                    ir::Ty::Struct(t) => t.def().source_decl == decl && *t.def().type_args == *args,
                    ir::Ty::Enum(t) => t.def().source_decl == decl && *t.def().type_args == *args,
                    _ => false,
                };
                if !same {
                    return Err("unsupported: nominal application identity collision".into());
                }
            }
            let id = ir::NominalAppId {
                snapshot: decl.snapshot,
                index: ir::NominalIndex::SourceSlot(index),
            };
            let is_struct = matches!(schema.as_ref(), NominalSchema::Struct(_));
            let mut build = |back: ir::Ty| -> Result<ir::Ty, String> {
                self.nominals
                    .building
                    .try_reserve(1)
                    .map_err(|_| "nominal building allocation failed")?;
                self.nominals.building.push((application_key.clone(), back));
                let built = (|| -> Result<ir::Ty, String> {
                    Ok(match schema.as_ref() {
                        NominalSchema::Struct(d) => {
                            self.schema_identity(registry, decl, &d.name, d.module, &d.visibility)?;
                            clone_visibility(&d.visibility, budget)?;
                            match d.shape {
                                StructShape::Unit => {
                                    if !d.fields.is_empty() {
                                        return Err("unit struct source schema has fields".into());
                                    }
                                    if !d.type_params.is_empty() || !args.is_empty() {
                                        return Err(
                                            "unsupported: generic unit structs are unsupported"
                                                .into(),
                                        );
                                    }
                                }
                                StructShape::Tuple => {
                                    if d.fields.is_empty() {
                                        return Err(
                                            "unsupported: zero-field tuple structs are unsupported"
                                                .into(),
                                        );
                                    }
                                    // The application key, parameter bindings and argument
                                    // evidence were validated above for this exact DeclId.
                                    // Keep tuple fields on the same substitution path as
                                    // record fields, preserving actual nominal arguments.
                                    for (index, field) in d.fields.iter().enumerate() {
                                        budget.charge_work(1)?;
                                        if !matches!(field.name, FieldName::Tuple(actual) if actual == index)
                                        {
                                            return Err("tuple struct fields must have contiguous numeric identities".into());
                                        }
                                    }
                                }
                                StructShape::Record => {
                                    // With no fields, no declared lifetime can
                                    // constrain owned data. Bounds alone do not
                                    // make an empty record's lifetime used.
                                    if d.fields.is_empty() && !d.region_params.is_empty() {
                                        return Err(
                                            "unused lifetime parameter in empty record struct"
                                                .into(),
                                        );
                                    }
                                }
                            }
                            let fields = make_fields(self, &d.fields, d.module)?;
                            let name = self.schema_path_text(registry, &d.name, budget)?;
                            let definition = ir::StructDef {
                                id,
                                source_decl: decl,
                                type_args: args.to_vec().into(),
                                name,
                                module: d.module,
                                visibility: d.visibility.clone(),
                                fields: fields.into(),
                            };
                            ir::Ty::Struct(ir::StructTy::new(definition))
                        }
                        NominalSchema::Enum(d) => {
                            self.schema_identity(registry, decl, &d.name, d.module, &d.visibility)?;
                            clone_visibility(&d.visibility, budget)?;
                            if d.variants.is_empty() || d.variants.len() > 4096 {
                                return Err("enum variant count exceeds supported limits".into());
                            }
                            self.retain(
                                budget,
                                d.variants
                                    .len()
                                    .checked_mul(std::mem::size_of::<ir::VariantDef>() * 4 + 128)
                                    .ok_or("enum allocation overflow")?,
                            )?;
                            let mut names = std::collections::HashSet::new();
                            let mut variants = Vec::new();
                            names
                                .try_reserve(d.variants.len())
                                .map_err(|_| "enum variant key allocation failed")?;
                            variants
                                .try_reserve_exact(d.variants.len())
                                .map_err(|_| "enum variant allocation failed")?;
                            for v in &d.variants {
                                budget.charge_work(1)?;
                                let key = registry
                                    .names
                                    .key(v.name, LookupClass::Member)
                                    .map_err(native_adapter::name_diagnostic)?;
                                if !names.insert(key) {
                                    return Err("duplicate enum variant".into());
                                }
                                if v.shape == ir::VariantShape::Unit && !v.fields.is_empty() {
                                    return Err("unit variant cannot have fields".into());
                                }
                                if v.shape == ir::VariantShape::Tuple
                            && v.fields.iter().enumerate().any(
                                |(i, f)| !matches!(f.name, FieldName::Tuple(index) if index == i),
                            )
                        {
                            return Err("tuple variant fields require positional names".into());
                        }
                                variants.push(ir::VariantDef {
                                    name: self.schema_text(
                                        registry
                                            .names
                                            .spelling(v.name)
                                            .map_err(native_adapter::name_diagnostic)?,
                                        budget,
                                    )?,
                                    shape: v.shape,
                                    fields: make_fields(self, &v.fields, d.module)?.into(),
                                });
                            }
                            let name = self.schema_path_text(registry, &d.name, budget)?;
                            ir::Ty::Enum(ir::EnumTy::new(ir::EnumDef {
                                id,
                                source_decl: decl,
                                type_args: args.to_vec().into(),
                                name,
                                module: d.module,
                                visibility: d.visibility.clone(),
                                variants: variants.into(),
                            }))
                        }
                    })
                })();
                self.nominals.building.pop();
                built
            };
            // Recursive applications get a cycle-closing back edge into their
            // own allocation; definitions with back edges are leaked so the
            // edge stays valid for the rest of the compilation.
            let mut failure = None;
            let ty = if is_struct {
                let arc = std::sync::Arc::new_cyclic(|weak| {
                    let back = ir::Ty::Struct(ir::StructTy::from_ref(ir::NominalRef::Back(
                        weak.clone(),
                        id,
                    )));
                    match build(back).and_then(|ty| match ty {
                        ir::Ty::Struct(t) => t.into_def(),
                        _ => Err("nominal kind changed during construction".into()),
                    }) {
                        Ok(def) => def,
                        Err(error) => {
                            failure = Some(error);
                            ir::StructDef::placeholder(id, decl, info.path.module)
                        }
                    }
                });
                if std::sync::Arc::weak_count(&arc) > 0 {
                    std::mem::forget(arc.clone());
                }
                ir::Ty::Struct(ir::StructTy::from_ref(ir::NominalRef::Strong(arc)))
            } else {
                let arc = std::sync::Arc::new_cyclic(|weak| {
                    let back =
                        ir::Ty::Enum(ir::EnumTy::from_ref(ir::NominalRef::Back(weak.clone(), id)));
                    match build(back).and_then(|ty| match ty {
                        ir::Ty::Enum(t) => t.into_def(),
                        _ => Err("nominal kind changed during construction".into()),
                    }) {
                        Ok(def) => def,
                        Err(error) => {
                            failure = Some(error);
                            ir::EnumDef::placeholder(id, decl, info.path.module)
                        }
                    }
                });
                if std::sync::Arc::weak_count(&arc) > 0 {
                    std::mem::forget(arc.clone());
                }
                ir::Ty::Enum(ir::EnumTy::from_ref(ir::NominalRef::Strong(arc)))
            };
            if let Some(error) = failure {
                return Err(error);
            }
            let ty = match (ty, schema.as_ref()) {
                (ir::Ty::Struct(value), NominalSchema::Struct(source)) => {
                    let binding = native_sequence::issue(registry, decl, source, &value, budget)?;
                    ir::Ty::Struct(value.with_native_sequence_binding(binding))
                }
                (ty, _) => ty,
            };
            if ir::by_value_contains(&ty, id, 0, budget)? {
                return Err(format!(
                    "recursive type `{}` has infinite size (insert some indirection, e.g. Box)",
                    info.name
                ));
            }
            if transport.is_some() {
                if !raw_type_has_projection(&ty, budget, 0)? {
                    ir::carrier_locations(&ty, budget)?;
                }
            } else {
                validate_argument(&ty, caller, budget, 0)?;
            }
            // A projection-free native application was rebuilt here from the
            // authenticated source schema, not copied from structural fields.
            // Re-run ordinary argument admission before retaining this exact
            // allocation's logical source coordinate. This is not a graph read
            // or executable body proof. Unresolved projection transport stays
            // unsealed until its existing normalization boundary.
            let native_transport = transport.is_some()
                && matches!(&ty, ir::Ty::Struct(value) if value.native_sequence_binding().is_some())
                && !raw_type_has_projection(&ty, budget, 0)?;
            if native_transport {
                validate_argument(&ty, caller, budget, 0)?;
            }
            let ty = if transport.is_none() || native_transport {
                native_sequence::bind_source_substitution(ty, caller, budget)?
            } else {
                ty
            };
            self.nominals
                .by_slot
                .try_reserve(1)
                .map_err(|_| "nominal ordering allocation failed")?;
            self.nominals
                .applications
                .try_reserve(1)
                .map_err(|_| "nominal application allocation failed")?;
            self.nominals.active.pop();
            self.nominals.by_slot.insert(index, ty.clone());
            self.nominals
                .applications
                .insert(application_key, ty.clone());
            Ok(ty)
        })();
        self.nominals.active.truncate(active_len);
        result
    }
}

/// Canonical slot of one nominal application: a deterministic digest of the
/// resolved declaration and ordered type actuals. Nested nominals hash by
/// their own canonical identity, so the digest is independent of interning
/// order and of which node interned the application first.
pub(super) fn canonical_slot(application: &(ir::DeclId, Vec<ir::Ty>)) -> usize {
    use std::hash::{Hash, Hasher};
    // DefaultHasher::new uses fixed keys: deterministic across nodes.
    let mut hasher = std::hash::DefaultHasher::new();
    0x4a52_4e4f_4d49_4e41u64.hash(&mut hasher);
    application.hash(&mut hasher);
    hasher.finish() as usize
}

/// Whether a source field type mentions a declared (non-`'static`) lifetime.
pub(super) fn source_names_lifetime(ty: &Type) -> bool {
    match ty {
        Type::Ref {
            region, referent, ..
        } => matches!(region, Some(RegionName::Named(_))) || source_names_lifetime(referent),
        Type::Array(inner, _) | Type::Slice(inner) => source_names_lifetime(inner),
        Type::RawPtr { pointee, .. } => source_names_lifetime(pointee),
        Type::Tuple(items) => items.iter().any(source_names_lifetime),
        Type::Apply(path) => path
            .segments
            .iter()
            .any(|segment| segment.args.iter().any(source_names_lifetime)),
        _ => false,
    }
}

/// Whether a source field type contains a reference with an elided lifetime.
fn source_elides_lifetime(ty: &Type) -> bool {
    match ty {
        Type::Ref {
            region, referent, ..
        } => region.is_none() || source_elides_lifetime(referent),
        Type::Array(inner, _) | Type::Slice(inner) => source_elides_lifetime(inner),
        Type::RawPtr { pointee, .. } => source_elides_lifetime(pointee),
        Type::Tuple(items) => items.iter().any(source_elides_lifetime),
        Type::Apply(path) => path
            .segments
            .iter()
            .any(|segment| segment.args.iter().any(source_elides_lifetime)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_keys_use_checked_member_identity_not_occurrence_spans() {
        let program = parse("struct A{x:i32}struct B{x:i32}fn main(){}").unwrap();
        let first = program.structs[0].fields[0].name;
        let second = program.structs[1].fields[0].name;
        assert_eq!(
            schema_field_key(program.names.as_ref(), first).unwrap(),
            schema_field_key(program.names.as_ref(), second).unwrap()
        );
    }

    #[test]
    fn field_keys_reject_foreign_source_occurrences() {
        let first = parse("struct A{x:i32}fn main(){}").unwrap();
        let second = parse("struct A{x:i32}fn main(){}").unwrap();
        assert!(schema_field_key(first.names.as_ref(), second.structs[0].fields[0].name).is_err());
    }

    #[test]
    fn tuple_field_positions_are_not_identifier_spelling() {
        let program = parse("struct A{x:i32}fn main(){}").unwrap();
        let named =
            schema_field_key(program.names.as_ref(), program.structs[0].fields[0].name).unwrap();
        let zero = schema_field_key(program.names.as_ref(), FieldName::Tuple(0)).unwrap();
        let one = schema_field_key(program.names.as_ref(), FieldName::Tuple(1)).unwrap();
        assert_ne!(zero, one);
        assert_ne!(zero, named);
    }

    #[test]
    fn cached_nominal_application_does_not_rebuild_parameter_name_bindings() {
        let program = parse("struct Pair<T,U>(T,U);fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let key = name_scope::path_key(
            program.names.as_ref(),
            &program.structs[0].name,
            Namespace::Type,
        )
        .unwrap();
        let declaration = *plan.registry.nominals.get(&key).unwrap();
        let args = [ir::Ty::Bool, ir::Ty::Unit];
        let mut state = plan.scratch();
        let first = state
            .intern(
                &plan.registry,
                declaration,
                &args,
                ir::FactsCtx::Concrete,
                &budget,
                0,
            )
            .unwrap();
        let name_work = || {
            program.names.budget().snapshot().counters
                [crate::compiler::expansion_budget::Counter::Work as usize]
        };
        let before = name_work();
        let again = state
            .intern(
                &plan.registry,
                declaration,
                &args,
                ir::FactsCtx::Concrete,
                &budget,
                0,
            )
            .unwrap();
        assert_eq!(first, again);
        assert_eq!(
            name_work(),
            before,
            "a hit must not rebuild parameter names"
        );
        assert!(
            state
                .intern(
                    &plan.registry,
                    declaration,
                    &args[..1],
                    ir::FactsCtx::Concrete,
                    &budget,
                    0
                )
                .unwrap_err()
                .contains("wrong generic arity")
        );
        assert_eq!(
            name_work(),
            before,
            "arity rejection needs no parameter lookup"
        );
    }

    #[test]
    fn source_metadata_reuses_only_retained_syntax_and_rechecks_actuals() {
        let program =
            parse("struct Leaf<T>{value:T} struct Wrap<T>{value:Leaf<T>} fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let key = name_scope::path_key(
            program.names.as_ref(),
            &program.structs[1].name,
            Namespace::Type,
        )
        .unwrap();
        let declaration = plan.registry.nominals[&key];
        let info = plan.registry.declaration(declaration).unwrap();
        let mut state = plan.scratch();
        let retained = state.schemas[&declaration].clone();
        let NominalSchema::Struct(schema) = retained.as_ref() else {
            panic!("expected struct")
        };
        let ty = &schema.fields[0].ty;
        state.source_context = Some(plan.registry.source_type_context(declaration).unwrap());
        let bindings = param_bindings(&plan.registry, info, &[ir::Ty::Bool]).unwrap();
        let before = budget.usage_snapshot().work;
        let metadata = state
            .source_field_metadata(
                &plan.registry,
                declaration,
                &retained,
                ty,
                &bindings,
                ir::FactsCtx::Concrete,
                &budget,
            )
            .unwrap();
        let cold = budget.usage_snapshot().work - before;
        let name_work = || {
            program.names.budget().snapshot().counters
                [crate::compiler::expansion_budget::Counter::Work as usize]
        };
        let before_names = name_work();
        let before = budget.usage_snapshot().work;
        let again = state
            .source_field_metadata(
                &plan.registry,
                declaration,
                &retained,
                ty,
                &bindings,
                ir::FactsCtx::Concrete,
                &budget,
            )
            .unwrap();
        let warm = budget.usage_snapshot().work - before;
        assert!(Arc::ptr_eq(&metadata, &again));
        assert!(warm < cold, "syntax was not reused: {warm}/{cold}");
        assert_eq!(
            name_work(),
            before_names,
            "a hit must not rebuild source names"
        );

        let authenticated =
            SourceBindingMetadata::validate(&bindings, ir::FactsCtx::Concrete, &budget).unwrap();
        let before = budget.usage_snapshot().work;
        state
            .source_field_metadata_with_bindings(
                &plan.registry,
                declaration,
                &retained,
                ty,
                &authenticated,
            )
            .unwrap();
        let shared_work = budget.usage_snapshot().work - before;
        assert!(
            shared_work < warm,
            "same-application binding authentication was repeated: {shared_work}/{warm}"
        );

        state.source_metadata.active = Some(metadata);
        assert!(
            state
                .source_metadata
                .covers(&plan.registry, state.source_context, ty)
        );
        assert!(!state.source_metadata.covers(&plan.registry, None, ty));
        let copied_syntax = (*ty).clone();
        assert!(!state.source_metadata.covers(
            &plan.registry,
            state.source_context,
            &copied_syntax
        ));

        let lowered = state
            .source_ty(
                &plan.registry,
                ty,
                &bindings,
                ir::FactsCtx::Concrete,
                &budget,
                0,
                false,
            )
            .unwrap();
        let before_names = name_work();
        let repeated = state
            .source_ty(
                &plan.registry,
                ty,
                &bindings,
                ir::FactsCtx::Concrete,
                &budget,
                0,
                false,
            )
            .unwrap();
        assert_eq!(lowered, repeated);
        assert_eq!(
            name_work(),
            before_names,
            "warm source conversion rebuilt resolved paths or parameter names"
        );

        // Same retained syntax, different actual: a syntax receipt does not
        // authorize an out-of-scope parameter or missing binding.
        let symbolic = symbolic_args(&info.param_env, &budget).unwrap();
        let forged = param_bindings(&plan.registry, info, &symbolic).unwrap();
        assert!(
            state
                .source_field_metadata(
                    &plan.registry,
                    declaration,
                    &retained,
                    ty,
                    &forged,
                    ir::FactsCtx::Concrete,
                    &budget,
                )
                .is_err()
        );
        assert!(
            state
                .source_field_metadata(
                    &plan.registry,
                    declaration,
                    &retained,
                    ty,
                    &HashMap::new(),
                    ir::FactsCtx::Concrete,
                    &budget,
                )
                .is_err()
        );

        let hidden_foreign = ir::Ty::RawPtr {
            kind: ir::BorrowKind::Shared,
            pointee: Arc::new(symbolic[0].clone()),
        };
        assert!(validate_argument(&hidden_foreign, ir::FactsCtx::Concrete, &budget, 0).is_ok());
        let hidden_bindings = param_bindings(&plan.registry, info, &[hidden_foreign]).unwrap();
        assert!(
            SourceBindingMetadata::validate(&hidden_bindings, ir::FactsCtx::Concrete, &budget)
                .is_err()
        );

        let foreign_program =
            parse("struct Leaf<T>{value:T} struct Wrap<T>{value:Leaf<T>} fn main(){}").unwrap();
        let foreign = PlanCtx::from_program(&foreign_program, &budget).unwrap();
        assert!(
            !state
                .source_metadata
                .covers(&foreign.registry, state.source_context, ty)
        );
        assert!(
            state
                .source_field_metadata(
                    &foreign.registry,
                    declaration,
                    &retained,
                    ty,
                    &bindings,
                    ir::FactsCtx::Concrete,
                    &budget,
                )
                .unwrap_err()
                .contains("different registry")
        );
    }

    #[test]
    fn source_metadata_parameter_keys_reuse_names_not_actuals() {
        let program = parse("struct Pair<T,U>(T,U);fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let name = name_scope::path_key(
            program.names.as_ref(),
            &program.structs[0].name,
            Namespace::Type,
        )
        .unwrap();
        let declaration = plan.registry.nominals[&name];
        let info = plan.registry.declaration(declaration).unwrap();
        let first_key = plan.registry.parameter_key(info.type_params[0]).unwrap();
        let second_key = plan.registry.parameter_key(info.type_params[1]).unwrap();
        let mut state = plan.scratch();
        let (first, first_storage) = state
            .schema_param_bindings(&plan.registry, info, &[ir::Ty::Bool, ir::Ty::Unit], &budget)
            .unwrap();
        assert_eq!(first[&first_key], ir::Ty::Bool);
        assert_eq!(first[&second_key], ir::Ty::Unit);
        drop(first);
        drop(first_storage);
        let name_work = || {
            program.names.budget().snapshot().counters
                [crate::compiler::expansion_budget::Counter::Work as usize]
        };
        let before = name_work();
        let (second, _) = state
            .schema_param_bindings(&plan.registry, info, &[ir::Ty::Unit, ir::Ty::Bool], &budget)
            .unwrap();
        assert_eq!(
            name_work(),
            before,
            "ordered parameter keys were reconstructed"
        );
        assert_eq!(second[&first_key], ir::Ty::Unit);
        assert_eq!(second[&second_key], ir::Ty::Bool);
        assert!(
            state
                .schema_param_bindings(&plan.registry, info, &[ir::Ty::Bool], &budget)
                .is_err()
        );
    }

    #[test]
    fn source_metadata_scope_is_restored_after_nested_nominal_lowering() {
        let program =
            parse("struct Leaf<T>{value:T} struct Wrap<T>{value:Leaf<T>} fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let key = name_scope::path_key(
            program.names.as_ref(),
            &program.structs[1].name,
            Namespace::Type,
        )
        .unwrap();
        let declaration = plan.registry.nominals[&key];
        let mut state = plan.scratch();
        state
            .intern(
                &plan.registry,
                declaration,
                &[ir::Ty::Bool],
                ir::FactsCtx::Concrete,
                &budget,
                0,
            )
            .unwrap();
        assert!(state.source_metadata.active.is_none());
        assert!(state.source_context.is_none());
        assert!(!state.source_metadata.fields.is_empty());
        state
            .intern(
                &plan.registry,
                declaration,
                &[ir::Ty::Unit],
                ir::FactsCtx::Concrete,
                &budget,
                0,
            )
            .unwrap();
        assert!(state.source_metadata.active.is_none());
        assert!(state.source_context.is_none());
    }

    #[test]
    fn generic_tuple_application_keeps_exact_declaration_actuals_and_substituted_fields() {
        let program =
            parse("struct Pair<T,U>(T,(U,T));struct Other<T,U>(T,(U,T));fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declarations = program
            .structs
            .iter()
            .map(|declaration| {
                let key = name_scope::path_key(
                    program.names.as_ref(),
                    &declaration.name,
                    Namespace::Type,
                )
                .unwrap();
                *plan.registry.nominals.get(&key).unwrap()
            })
            .collect::<Vec<_>>();
        let args = [ir::Ty::Bool, ir::Ty::Unit];
        let first = plan
            .intern_nominal(declarations[0], &args, ir::FactsCtx::Concrete, &budget)
            .unwrap();
        let again = plan
            .intern_nominal(declarations[0], &args, ir::FactsCtx::Concrete, &budget)
            .unwrap();
        let other = plan
            .intern_nominal(declarations[1], &args, ir::FactsCtx::Concrete, &budget)
            .unwrap();
        let (ir::Ty::Struct(first), ir::Ty::Struct(again), ir::Ty::Struct(other)) =
            (first, again, other)
        else {
            panic!("generic tuple application lost nominal struct carrier");
        };
        assert_eq!(first.def().source_decl, declarations[0]);
        assert_eq!(first.def().type_args.as_ref(), &args);
        assert_eq!(first.def().id, again.def().id);
        assert_ne!(first.def().id, other.def().id);
        assert_eq!(
            first.def().fields.read(&budget).unwrap()[0].ty,
            ir::Ty::Bool
        );
        assert_eq!(
            first.def().fields.read(&budget).unwrap()[1].ty,
            ir::Ty::Tuple(Arc::from([ir::Ty::Unit, ir::Ty::Bool]))
        );
        assert!(
            plan.intern_nominal(declarations[0], &args[..1], ir::FactsCtx::Concrete, &budget)
                .unwrap_err()
                .contains("wrong generic arity")
        );
    }
}
