//! Existing inherent inference, with explicit absence versus unresolved evidence.
use super::*;

pub(super) enum InherentMethodInference {
    Missing,
    Applicable(Option<Vec<ir::Ty>>),
}

impl Checker<'_> {
    pub(super) fn inherent_method_actuals(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        args: &[Expr],
        explicit: &[Type],
        expected: Option<&ir::Ty>,
        filter: Option<ReceiverKind>,
    ) -> Result<InherentMethodInference, String> {
        let plan = self.signatures.plan()?;
        let mut root = receiver;
        if filter.is_none() {
            // Compatibility for associated owner inference. Dot inference is
            // already inside the canonical receiver search and must not peel
            // a reference or skip a candidate depth independently.
            while let ir::Ty::Ref { referent, .. } = root {
                root = referent;
            }
        }
        let Some((nominal, _)) = nominal_parts(root) else {
            return Ok(InherentMethodInference::Missing);
        };
        let mut candidate = None;
        for id in &plan.registry.function_ids {
            plan.budget.charge_work(1)?;
            let info = plan.declaration(*id)?;
            let Some(associated) = &info.path.associated else {
                continue;
            };
            if filter.is_some_and(|mode| receiver_mode(mode) != receiver_mode(associated.receiver))
                || associated.trait_decl.is_some()
                || associated.nominal != nominal
                || !plan.registry.source_member_equal(info.path.item, *name)?
            {
                continue;
            }
            let impl_env = plan.param_env(associated.impl_decl)?;
            let mut bindings = HashMap::new();
            if !match_impl_type(
                &associated.self_pattern,
                root,
                &impl_env,
                &mut bindings,
                &plan.budget,
                0,
            )? {
                continue;
            }
            if candidate.replace((*id, bindings)).is_some() {
                return Ok(InherentMethodInference::Applicable(None));
            }
        }
        let Some((id, owner_bindings)) = candidate else {
            return Ok(InherentMethodInference::Missing);
        };
        let info = plan.declaration(id)?;
        let schema = plan.function_schema(id)?;
        if info.param_env.arity() == 0 {
            return Ok(InherentMethodInference::Applicable(Some(Vec::new())));
        }
        let params = &schema.signature.params;
        let formals = match params.len().checked_sub(args.len()) {
            Some(0) => &params[..],
            Some(1) => &params[1..],
            _ => return Ok(InherentMethodInference::Applicable(None)),
        };
        let mut solver = Inference::new(self, info.param_env.clone())?;
        if !explicit.is_empty() {
            let anonymous = plan
                .registry
                .source_callable(id)
                .map_or(0, |f| f.impl_params);
            if explicit.len() + anonymous != info.param_env.arity() {
                return Err("E0107: wrong number of method generic arguments".into());
            }
            for (slot, ty) in explicit.iter().enumerate() {
                let formal = self.infer_node(InferKind::Slot(slot))?;
                let actual = self.partial_type_term(ty, 0)?;
                solver.constrain(&formal, &actual, 0)?;
            }
        }
        if let Some(expected) = expected {
            let formal = self.method_owner_term(
                &solver.schema(&schema.signature.result, 0)?,
                &owner_bindings,
                0,
            )?;
            let actual = self.infer_type_term(expected, None, 0)?;
            solver.constrain(&formal, &actual, 0)?;
        }
        for (formal, expr) in formals.iter().zip(args) {
            // Arguments that do not mention a method parameter carry no
            // evidence; skip them (they are checked normally afterwards).
            if !inferred_owner::mentions_param(formal) {
                continue;
            }
            let Ok(evidence) = self.inference_evidence(expr, 0) else {
                return Ok(InherentMethodInference::Applicable(None));
            };
            let Ok(formal) = solver
                .schema(formal, 0)
                .and_then(|term| self.method_owner_term(&term, &owner_bindings, 0))
            else {
                return Ok(InherentMethodInference::Applicable(None));
            };
            if solver.constrain(&formal, &evidence, 0).is_err() {
                return Ok(InherentMethodInference::Applicable(None));
            }
        }
        // The closure argument can determine a method parameter that occurs
        // only in its callable Output equality, e.g. Result::map_err's G.
        // As for trait methods, normalize the original predicate after binding
        // the receiver's impl parameters. Final method selection still proves
        // the complete instantiated signature and its obligations.
        for predicate in schema.predicates.predicates() {
            plan.budget.charge_work(1)?;
            if let ir::Predicate::ProjectionEq { projection, ty } = predicate
                && !matches!(ty, ir::Ty::AssociatedType(_))
                && inferred_owner::mentions_param(ty)
            {
                let projection = super::trait_method_hint::bind_owner(
                    &ir::Ty::AssociatedType(Arc::new(projection.clone())),
                    &owner_bindings,
                );
                let ty = super::trait_method_hint::bind_owner(ty, &owner_bindings);
                solver.constrain_projection_equality(&projection, &ty, true)?;
            }
        }
        Ok(InherentMethodInference::Applicable(
            solver.finish(true).ok().flatten(),
        ))
    }
}
