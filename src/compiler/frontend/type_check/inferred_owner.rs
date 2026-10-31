//! `Vec::new()` / `W::new(x)`: an associated call whose generic owner was
//! written without arguments. The resolver leaves `Type::Infer` actuals; here
//! they are inferred from the call arguments and expected type against the
//! unique inherent candidate's schema, exactly like a generic function call.
use super::*;

/// True when the associated owner carries inferred actuals.
pub(super) fn has_inferred_owner(owner: &Type) -> bool {
    matches!(owner, Type::Apply(_)) && partial_annotation::has_nested_infer(owner)
}

impl Checker<'_> {
    /// Concrete owner type for an associated call whose owner actuals are
    /// inferred. Returns None when evidence is insufficient.
    pub(super) fn infer_associated_owner(
        &self,
        owner: &Type,
        name: &AstIdent,
        method_args: usize,
        args: &[Expr],
        expected: Option<&ir::Ty>,
    ) -> Result<ir::Ty, String> {
        let Type::Apply(path) = owner else {
            return Err("inferred owner requires a generic path".into());
        };
        let plan = self.signatures.plan()?;
        let names = self.names()?;
        let nominal_path = path.path(names)?;
        let nominal = self.nominal_decl(&nominal_path)?;
        let mut candidate = None;
        for id in &plan.registry.function_ids {
            plan.budget.charge_work(1)?;
            let info = plan.declaration(*id)?;
            let Some(associated) = &info.path.associated else {
                continue;
            };
            if associated.trait_decl.is_some()
                || associated.nominal != nominal
                || !plan.registry.source_member_equal(info.path.item, *name)?
            {
                continue;
            }
            if candidate.replace(*id).is_some() {
                return Err(
                    "unsupported: ambiguous inherent associated item; write explicit owner type arguments"
                        .into(),
                );
            }
        }
        let id = candidate.ok_or_else(|| {
            format!(
                "no applicable associated item {}",
                names.spelling(*name).unwrap_or_default()
            )
        })?;
        let info = plan.declaration(id)?;
        let associated = info
            .path
            .associated
            .as_ref()
            .ok_or("lost associated owner")?;
        if info.param_env.arity() != method_args {
            return Err("unsupported omitted method type arguments on inferred owner".into());
        }
        let schema = plan.function_schema(id)?;
        if schema.signature.params.len() != args.len() {
            return Err("associated function argument count mismatch".into());
        }
        let impl_env = plan.param_env(associated.impl_decl)?;
        let mut solver = Inference::new(self, impl_env.clone())?;
        let formal_owner = solver.schema(&associated.self_pattern, 0)?;
        let written_owner = self.partial_type_term(owner, 0)?;
        solver.constrain(&formal_owner, &written_owner, 0)?;
        if let Some(expected) = expected {
            let formal = solver.schema(&schema.signature.result, 0)?;
            let evidence = self.infer_type_term(expected, None, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        for (formal, expr) in schema.signature.params.iter().zip(args) {
            let evidence = self.inference_evidence(expr, 0)?;
            let formal = solver.schema(formal, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        let actuals = solver.finish(true)?.ok_or(
            "unsupported generic inference: insufficient evidence for the owner type; write explicit type arguments",
        )?;
        let pattern = &associated.self_pattern;
        let mut bindings = HashMap::new();
        for (index, actual) in actuals.into_iter().enumerate() {
            bindings.insert(
                ir::ParamId {
                    owner: impl_env.binder(),
                    index: index as u16,
                },
                actual,
            );
        }
        let Some((decl, pattern_args)) = nominal_parts(pattern) else {
            return Err("unsupported: inferred owner impl must be on a nominal type".into());
        };
        let mut concrete = Vec::with_capacity(pattern_args.len());
        for arg in pattern_args {
            match arg {
                ir::Ty::Param(param) => concrete.push(
                    bindings
                        .get(param)
                        .cloned()
                        .ok_or("unconstrained impl parameter")?,
                ),
                other => concrete.push(other.clone()),
            }
        }
        plan.intern_nominal(decl, &concrete, self.facts(), &plan.budget)
    }
}

impl Checker<'_> {
    /// `recv.method(args)` on a generic inherent method with omitted method
    /// type arguments: infer them from the arguments against the unique
    /// candidate's schema. None when there is no such unique candidate or the
    /// evidence is insufficient (the caller then reports its usual error).
    pub(super) fn infer_method_type_args(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        args: &[Expr],
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        self.infer_method_type_args_partial(receiver, name, args, &[], None)
    }
    pub(super) fn infer_method_type_args_partial(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        args: &[Expr],
        explicit: &[Type],
        expected: Option<&ir::Ty>,
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        Ok(
            match self.inherent_method_actuals(receiver, name, args, explicit, expected, None)? {
                inherent_method_hint::InherentMethodInference::Missing => None,
                inherent_method_hint::InherentMethodInference::Applicable(actuals) => actuals,
            },
        )
    }
}

pub(super) fn mentions_param(ty: &ir::Ty) -> bool {
    match ty {
        ir::Ty::Param(_) => true,
        ir::Ty::Ref { referent, .. } => mentions_param(referent),
        ir::Ty::RawPtr { pointee, .. } => mentions_param(pointee),
        ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => mentions_param(element),
        ir::Ty::PhantomData(marker) => mentions_param(marker.argument()),
        ir::Ty::FnPointer(pointer) => pointer.types().any(mentions_param),
        ir::Ty::Tuple(fields) => fields.iter().any(mentions_param),
        ir::Ty::Struct(t) => t.def().type_args.iter().any(mentions_param),
        ir::Ty::Enum(t) => t.def().type_args.iter().any(mentions_param),
        ir::Ty::Int(_)
        | ir::Ty::Float(_)
        | ir::Ty::Bool
        | ir::Ty::Char
        | ir::Ty::Str
        | ir::Ty::Unit
        | ir::Ty::Never => false,
        _ => true,
    }
}
