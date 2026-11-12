//! Struct and enum field well-formedness inputs (stock E0277 at the
//! declaration, even when the type is never used). Every field type must be
//! well formed in its declaration's own predicate environment: nested nominal
//! applications satisfy their declared bounds and implicit `Sized` rows, array
//! and slice elements are `Sized`, and every field except a struct's last is
//! `Sized`. The trait world proves these rows; this module only derives them
//! from the canonical symbolic template, never from display names.
use super::*;

impl PlanCtx {
    pub(super) fn nominal_field_obligations_source(
        &self,
        declaration: ir::DeclId,
    ) -> Result<Arc<[ir::Predicate]>, String> {
        let info = self.registry.declaration(declaration)?;
        if !matches!(info.kind, ir::DeclKind::Struct | ir::DeclKind::Enum) {
            return Err("nominal field well-formedness requires a struct or enum".into());
        }
        let args = symbolic_args(&info.param_env, &self.budget)?;
        let template = self
            .nominals
            .applications
            .get(&(declaration, args))
            .cloned()
            .ok_or("missing canonical nominal schema")?;
        let env = Arc::new(ir::GenericEnv::new(
            std::slice::from_ref(&info.param_env),
            &self.budget,
        )?);
        let sized = *self
            .registry
            .traits
            .get(&TraitLookupKey::Builtin(BuiltinTrait::Sized))
            .ok_or("missing canonical Sized identity")?;
        let mut state = self.scratch();
        let mut out = Vec::new();
        let mut walk = FieldWalk {
            registry: &self.registry,
            caller: ir::FactsCtx::Universal(&env),
            budget: &self.budget,
            sized,
            out: &mut out,
        };
        match &template {
            ir::Ty::Struct(t) => {
                let fields = t.def().fields.read(&self.budget)?;
                let last = fields.len().saturating_sub(1);
                for (index, field) in fields.iter().enumerate() {
                    walk.ty(&mut state, &field.ty, index != last, 0)?;
                }
            }
            ir::Ty::Enum(t) => {
                for variant in t.def().variants.iter() {
                    for field in variant.fields.read(&self.budget)?.iter() {
                        walk.ty(&mut state, &field.ty, true, 0)?;
                    }
                }
            }
            _ => return Err("canonical nominal template is not a struct or enum".into()),
        }
        Ok(out.into())
    }
}

struct FieldWalk<'a> {
    registry: &'a DeclRegistry,
    caller: ir::FactsCtx<'a>,
    budget: &'a ir::PlanBudget,
    sized: ir::DeclId,
    out: &'a mut Vec<ir::Predicate>,
}

impl FieldWalk<'_> {
    fn push(&mut self, predicate: ir::Predicate) -> Result<(), String> {
        self.budget.charge_work(self.out.len() + 1)?;
        if self.out.contains(&predicate) {
            return Ok(());
        }
        self.budget
            .retain_bytes(std::mem::size_of::<ir::Predicate>())?;
        self.out
            .try_reserve_exact(1)
            .map_err(|_| "infrastructure: field obligation allocation failed")?;
        self.out.push(predicate);
        Ok(())
    }
    fn push_sized(&mut self, ty: &ir::Ty) -> Result<(), String> {
        self.push(ir::Predicate::Implements(ir::TraitRef {
            declaration: self.sized,
            self_ty: ty.clone(),
            args: Arc::from([]),
        }))
    }
    fn ty(
        &mut self,
        state: &mut PlanState,
        ty: &ir::Ty,
        sized: bool,
        depth: usize,
    ) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        if matches!(ty, ir::Ty::Const(_)) {
            return Ok(());
        }
        // Only shapes that can be unsized need a row. Tuples pass the
        // requirement to their tail; nominals only through `?Sized` actuals.
        if sized
            && matches!(
                ty,
                ir::Ty::Param(_)
                    | ir::Ty::Str
                    | ir::Ty::Slice { .. }
                    | ir::Ty::Dyn(_)
                    | ir::Ty::AssociatedType(_)
                    | ir::Ty::Opaque(_)
            )
        {
            self.push_sized(ty)?;
        }
        match ty {
            ir::Ty::Ref { referent, .. } => self.ty(state, referent, false, depth + 1),
            ir::Ty::RawPtr { pointee, .. } => self.ty(state, pointee, false, depth + 1),
            ir::Ty::Array { element, .. }
            | ir::Ty::DependentArray { element, .. }
            | ir::Ty::Slice { element } => self.ty(state, element, true, depth + 1),
            ir::Ty::Tuple(parts) => {
                let last = parts.len().saturating_sub(1);
                for (index, part) in parts.iter().enumerate() {
                    self.ty(state, part, sized || index != last, depth + 1)?;
                }
                Ok(())
            }
            ir::Ty::PhantomData(marker) => self.ty(state, marker.argument(), false, depth + 1),
            ir::Ty::Cell(cell) => self.ty(state, cell.argument(), false, depth + 1),
            ir::Ty::ManuallyDrop(wrapper) => self.ty(state, wrapper.argument(), false, depth + 1),
            ir::Ty::Struct(value) => self.nominal(
                state,
                ty,
                sized,
                value.def().source_decl,
                &value.def().type_args,
                depth,
            ),
            ir::Ty::Enum(value) => self.nominal(
                state,
                ty,
                sized,
                value.def().source_decl,
                &value.def().type_args,
                depth,
            ),
            _ => Ok(()),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn nominal(
        &mut self,
        state: &mut PlanState,
        ty: &ir::Ty,
        sized: bool,
        declaration: ir::DeclId,
        actuals: &[ir::Ty],
        depth: usize,
    ) -> Result<(), String> {
        let info = self.registry.declaration(declaration)?;
        for (index, actual) in actuals.iter().enumerate() {
            self.budget.charge_work(1)?;
            let parameter = ir::ParamId {
                owner: info.param_env.binder(),
                index: u16::try_from(index).map_err(|_| "nominal arity overflow")?,
            };
            if matches!(actual, ir::Ty::Const(_))
                || matches!(
                    info.param_env.parameter_kind(parameter),
                    Some(ir::GenericParamKind::Const(_))
                )
            {
                continue;
            }
            let relaxed = info.param_env.relaxed_sized(parameter);
            // `S<str>` with `struct S<T: ?Sized>` may itself be unsized: a
            // sized position proves the whole application structurally.
            if relaxed
                && sized
                && matches!(actual, ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_))
            {
                self.push_sized(ty)?;
            }
            self.ty(state, actual, !relaxed, depth + 1)?;
        }
        if actuals.is_empty() {
            return Ok(());
        }
        let schema = state
            .schemas
            .get(&declaration)
            .cloned()
            .ok_or("missing nested nominal field schema")?;
        let bounds = match schema.as_ref() {
            NominalSchema::Struct(source) => &source.bounds,
            NominalSchema::Enum(source) => &source.bounds,
        };
        if bounds.is_empty() {
            return Ok(());
        }
        let (bindings, _storage) =
            state.schema_param_bindings(self.registry, info, actuals, self.budget)?;
        let previous = state.source_context;
        state.source_context = Some(self.registry.source_type_context(declaration)?);
        let mut rows = Vec::new();
        let result = append_source_bounds(
            state,
            self.registry,
            &bindings,
            bounds,
            &mut rows,
            self.caller,
            self.budget,
        );
        state.source_context = previous;
        result?;
        for row in rows {
            self.push(row)?;
        }
        Ok(())
    }
}
