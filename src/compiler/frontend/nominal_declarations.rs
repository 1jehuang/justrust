//! Exact nominal declaration inputs for the per-declaration graph producer.
use super::*;

impl SchemaAdapter {
    pub(super) fn nominal_source_schema(
        &self,
        declaration: ir::DeclId,
    ) -> Result<tr::NominalDeclarationSchema, String> {
        let info = self.registry.declaration(declaration)?;
        let field_count = match info.kind {
            ir::DeclKind::Struct => self
                .program
                .structs
                .get(info.source_index)
                .ok_or("missing nominal struct source")?
                .fields
                .len(),
            ir::DeclKind::Enum => self
                .program
                .enums
                .get(info.source_index)
                .ok_or("missing nominal enum source")?
                .variants
                .iter()
                .try_fold(0usize, |count, variant| {
                    self.budget.charge_work(1)?;
                    count
                        .checked_add(variant.fields.len())
                        .ok_or_else(|| "nominal field inventory overflow".to_string())
                })?,
            _ => return Err("nominal declaration proof requires a struct or enum".into()),
        };
        self.budget.retain_bytes(
            field_count
                .checked_mul(std::mem::size_of::<&Type>())
                .ok_or("nominal field inventory overflow")?,
        )?;
        let (bounds, fields): (&[TypeBound], Vec<&Type>) = match info.kind {
            ir::DeclKind::Struct => {
                let source = self
                    .program
                    .structs
                    .get(info.source_index)
                    .ok_or("missing nominal struct source")?;
                if source.module != info.path.module {
                    return Err("nominal source module changed".into());
                }
                (
                    source.bounds.as_slice(),
                    source.fields.iter().map(|field| &field.ty).collect(),
                )
            }
            ir::DeclKind::Enum => {
                let source = self
                    .program
                    .enums
                    .get(info.source_index)
                    .ok_or("missing nominal enum source")?;
                if source.module != info.path.module {
                    return Err("nominal source module changed".into());
                }
                (
                    source.bounds.as_slice(),
                    source
                        .variants
                        .iter()
                        .flat_map(|variant| variant.fields.iter().map(|field| &field.ty))
                        .collect(),
                )
            }
            _ => return Err("nominal declaration proof requires a struct or enum".into()),
        };
        let args = symbolic_args(&info.param_env, &self.budget)?;
        let env = Arc::new(ir::GenericEnv::new(
            std::slice::from_ref(&info.param_env),
            &self.budget,
        )?);
        let bindings = param_bindings(&self.registry, info, &args)?;
        let mut state = self.scratch();
        let previous = state.source_context;
        state.source_context = Some(self.registry.source_type_context(declaration)?);
        let result = (|| {
            let mut predicates = Vec::new();
            append_implicit_sized(&self.registry, &env, &mut predicates, &self.budget)?;
            append_source_bounds(
                &mut state,
                &self.registry,
                &bindings,
                bounds,
                &mut predicates,
                ir::FactsCtx::Universal(&env),
                &self.budget,
            )?;
            let mut projections = Vec::new();
            for field in fields {
                // This walk only inspects field types and never clones them, so
                // it charges per-node work rather than retained clone storage
                // (repeated per-proof replays otherwise exhaust the task bytes).
                let mut inspect = |node: &DepthNode<'_>, _| {
                    self.budget.charge_work(1)?;
                    if let DepthNode::Type(ty @ Type::Projection(_)) = node {
                        let lowered = state.source_ty(
                            &self.registry,
                            ty,
                            &bindings,
                            ir::FactsCtx::Universal(&env),
                            &self.budget,
                            0,
                            false,
                        )?;
                        let ir::Ty::AssociatedType(projection) = lowered else {
                            return Err("nominal projection source lost its identity".into());
                        };
                        self.budget
                            .retain_bytes(std::mem::size_of::<ir::AssociatedTypeProjection>())?;
                        projections
                            .try_reserve_exact(1)
                            .map_err(|_| "nominal projection allocation failed")?;
                        projections.push(projection.as_ref().clone());
                    }
                    Ok(())
                };
                walk_depth_inspecting(DepthNode::Type(field), None, None, Some(&mut inspect))?;
            }
            Ok(tr::NominalDeclarationSchema {
                declaration,
                env,
                predicates: predicates.into(),
                projections: projections.into(),
            })
        })();
        state.source_context = previous;
        result
    }
}

impl PlanCtx {
    pub(crate) fn nominal_declaration_count(&self) -> usize {
        self.program.structs.len() + self.program.enums.len()
    }
    pub(crate) fn nominal_declaration(&self, index: usize) -> Result<ir::DeclId, String> {
        self.budget.charge_work(1)?;
        let name = if let Some(source) = self.program.structs.get(index) {
            &source.name
        } else {
            &self
                .program
                .enums
                .get(
                    index
                        .checked_sub(self.program.structs.len())
                        .ok_or("invalid nominal declaration index")?,
                )
                .ok_or("invalid nominal declaration index")?
                .name
        };
        let key = name_scope::path_key(self.program.names.as_ref(), name, Namespace::Type)?;
        self.registry
            .nominals
            .get(&key)
            .copied()
            .ok_or_else(|| "missing canonical nominal declaration".into())
    }
    pub(crate) fn publish_nominal_declaration(
        &self,
        declaration: ir::DeclId,
    ) -> Result<tr::NominalDeclarationProof, String> {
        let world = self.trait_world()?;
        world
            .nominal_declaration_wellformed(declaration)
            .map_err(|error| error.into_legacy_diagnostic(&self.budget))
    }
}
