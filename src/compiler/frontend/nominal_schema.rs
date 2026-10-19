use super::*;
use std::fmt::Write as _;

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
            Type::Param(p) => bindings
                .get(&registry.parameter_key(*p)?)
                .cloned()
                .ok_or_else(|| "unknown generic parameter".to_owned())?,
            Type::Apply(path) => {
                self.validate_source_type_metadata(registry, ty, bindings, caller, budget)?;
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
                let name = name_scope::path_key(
                    registry.names.as_ref(),
                    &path.path(registry.names.as_ref())?,
                    Namespace::Type,
                )?;
                let decl = *registry
                    .nominals
                    .get(&name)
                    .ok_or_else(|| "unknown nominal type".to_owned())?;
                let args = segment
                    .args
                    .iter()
                    .map(|arg| {
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
                let decl = *registry
                    .nominals
                    .get(&name_scope::path_key(
                        registry.names.as_ref(),
                        name,
                        Namespace::Type,
                    )?)
                    .ok_or_else(|| "unknown nominal type".to_owned())?;
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
                ir::validate_shared_slice_element(&element, caller, budget)?;
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
                    referent: Arc::new(registry.trait_object_ty(trait_path, budget)?),
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
                if *len > ir::MAX_ARRAY_LEN {
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
        let bindings = param_bindings(registry, info, args)?;
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
        if let Some(ty) = self.nominals.applications.get(&application_key) {
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
        if self.nominals.active.contains(&decl) {
            return Err(format!(
                "unsupported: polymorphically recursive nominal {}",
                info.name
            ));
        }
        budget.charge_application()?;
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
                    let ty = state.source_ty_with_transport(
                        registry,
                        &field.ty,
                        &bindings,
                        caller,
                        budget,
                        depth + 1,
                        false,
                        transport,
                    )?;
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
            // Reserve this application's index before building fields, which
            // may intern other applications (e.g. `Box<Self>`).
            let index = self.nominals.ordered.len();
            self.nominals
                .ordered
                .try_reserve(1)
                .map_err(|_| "nominal ordering allocation failed")?;
            self.nominals.ordered.push(ir::Ty::Unit);
            let id = ir::NominalAppId {
                snapshot: decl.snapshot,
                index,
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
                                StructShape::Record if d.fields.is_empty() => {
                                    return Err(
                                        "unsupported: empty record structs are unsupported".into(),
                                    );
                                }
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
                                StructShape::Record => {}
                            }
                            let fields = make_fields(self, &d.fields, d.module)?;
                            let display = d
                                .name
                                .display(registry.names.as_ref())
                                .map_err(native_adapter::name_diagnostic)?;
                            let name = self.schema_text(display.as_str(), budget)?;
                            ir::Ty::Struct(ir::StructTy::new(ir::StructDef {
                                id,
                                source_decl: decl,
                                type_args: args.to_vec().into(),
                                name,
                                module: d.module,
                                visibility: d.visibility.clone(),
                                fields,
                            }))
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
                                    fields: make_fields(self, &v.fields, d.module)?,
                                });
                            }
                            let display = d
                                .name
                                .display(registry.names.as_ref())
                                .map_err(native_adapter::name_diagnostic)?;
                            let name = self.schema_text(display.as_str(), budget)?;
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
            if ir::by_value_contains(&ty, id, 0)? {
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
            self.nominals
                .ordered
                .try_reserve(1)
                .map_err(|_| "nominal ordering allocation failed")?;
            self.nominals
                .applications
                .try_reserve(1)
                .map_err(|_| "nominal application allocation failed")?;
            self.nominals.active.pop();
            self.nominals.ordered[index] = ty.clone();
            self.nominals
                .applications
                .insert(application_key, ty.clone());
            Ok(ty)
        })();
        self.nominals.active.truncate(active_len);
        result
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
        assert_eq!(first.def().fields[0].ty, ir::Ty::Bool);
        assert_eq!(
            first.def().fields[1].ty,
            ir::Ty::Tuple(Arc::from([ir::Ty::Unit, ir::Ty::Bool]))
        );
        assert!(
            plan.intern_nominal(declarations[0], &args[..1], ir::FactsCtx::Concrete, &budget)
                .unwrap_err()
                .contains("wrong generic arity")
        );
    }
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
