//! Impl parameters determined by associated-type equality obligations.
use super::*;

impl ValidatedTraitWorld {
    fn impl_parameters_in(
        &self,
        ty: &Ty,
        projections: bool,
        out: &mut Vec<ParamId>,
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::Param(p) => {
                if !out.contains(p) {
                    push(&self.budget, out, *p)?;
                }
            }
            Ty::AssociatedType(p) if projections => {
                self.impl_parameters_in(&p.trait_ref.self_ty, true, out, depth + 1)?;
                for ty in p.trait_ref.args.iter() {
                    self.impl_parameters_in(ty, true, out, depth + 1)?;
                }
            }
            Ty::AssociatedType(_) => {}
            Ty::Ref {
                referent: element, ..
            }
            | Ty::RawPtr {
                pointee: element, ..
            }
            | Ty::Array { element, .. }
            | Ty::Slice { element } => {
                self.impl_parameters_in(element, projections, out, depth + 1)?
            }
            _ => {
                for ty in type_arguments(ty) {
                    self.impl_parameters_in(ty, projections, out, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    fn impl_projection_inputs(
        &self,
        projection: &ir::AssociatedTypeProjection,
    ) -> Result<Vec<ParamId>> {
        let mut inputs = Vec::new();
        self.impl_parameters_in(&projection.trait_ref.self_ty, true, &mut inputs, 0)?;
        for ty in projection.trait_ref.args.iter() {
            self.impl_parameters_in(ty, true, &mut inputs, 0)?;
        }
        Ok(inputs)
    }

    /// Equality output parameters are constrained only after every projection
    /// input is itself determined. Never infer backwards through a projection.
    pub(super) fn validate_impl_parameter_constraints(&self, header: &ImplHeader) -> Result<()> {
        let mut known = Vec::new();
        self.impl_parameters_in(&header.trait_ref.self_ty, false, &mut known, 0)?;
        for ty in header.trait_ref.args.iter() {
            self.impl_parameters_in(ty, false, &mut known, 0)?;
        }
        loop {
            let before = known.len();
            for predicate in header.predicates.iter() {
                let Predicate::ProjectionEq { projection, ty } = predicate else {
                    continue;
                };
                let inputs = self.impl_projection_inputs(projection)?;
                if inputs.iter().all(|p| known.contains(p)) {
                    self.impl_parameters_in(ty, false, &mut known, 0)?;
                }
            }
            if before == known.len() {
                break;
            }
        }
        for parameter in header.schema_env.ordered_params() {
            if !known.contains(&parameter) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "unconstrained trait impl parameter",
                ));
            }
        }
        Ok(())
    }

    /// Derive missing impl actuals from canonical associated-type facts. The
    /// resulting complete argument list still goes through ordinary predicate
    /// discharge and exact head substitution before a candidate is selected.
    pub(super) fn infer_impl_equality_arguments(
        &self,
        header: &ImplHeader,
        slots: &mut [Option<Ty>],
        facts: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<bool> {
        loop {
            let before = slots.iter().filter(|slot| slot.is_some()).count();
            for predicate in header.predicates.iter() {
                let Predicate::ProjectionEq { projection, ty } = predicate else {
                    continue;
                };
                let mut outputs = Vec::new();
                self.impl_parameters_in(ty, false, &mut outputs, 0)?;
                if !outputs.iter().any(|p| {
                    header
                        .schema_env
                        .ordered_params()
                        .position(|parameter| parameter == *p)
                        .is_some_and(|i| slots[i].is_none())
                }) {
                    continue;
                }
                let inputs = self.impl_projection_inputs(projection)?;
                if inputs.iter().any(|p| {
                    header
                        .schema_env
                        .ordered_params()
                        .position(|parameter| parameter == *p)
                        .is_some_and(|i| slots[i].is_none())
                }) {
                    continue;
                }
                // Unused argument positions cannot occur in this projection.
                // Never is only a private substitution filler, not a fact or
                // inferred actual, and cannot escape this input-only schema.
                let mut args = Vec::new();
                for slot in slots.iter() {
                    push(&self.budget, &mut args, slot.clone().unwrap_or(Ty::Never))?;
                }
                let raw = GenericSchema {
                    env: header.schema_env.clone(),
                    signature: Signature {
                        params: Vec::new(),
                        result: Ty::AssociatedType(Arc::new(projection.clone())),
                        ref_contract: Arc::new(ir::RefContract::default()),
                    },
                    predicates: Arc::from([]),
                };
                let substituted =
                    self.schema
                        .instantiate_schema(&raw, &args, facts.env(), &self.budget)?;
                self.validate_rhs_substitution(
                    &raw.signature.result,
                    &substituted.signature.result,
                    &header.schema_env,
                    &args,
                    0,
                )?;
                if !substituted.signature.params.is_empty() || !substituted.predicates.is_empty() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "impl input substitution invented schema obligations",
                    ));
                }
                let Ty::AssociatedType(projection) = substituted.signature.result else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "raw impl equality substitution changed projection shape",
                    ));
                };
                if !self.raw_associated_ty_is_closed(&Ty::AssociatedType(projection.clone()), 0)?
                    && !self.checked_closure_projection(&projection, facts.env())?
                {
                    // A symbolic input (`<I as Iterator>::Item` for a caller
                    // parameter `I`): the output is the caller's declared
                    // equality when one exists, else the rigid projection
                    // itself. Ordinary predicate discharge still proves the
                    // completed candidate's bounds afterwards.
                    let symbolic = match self.declared_equality_rhs(
                        &projection,
                        &facts.predicates.predicates,
                        facts.env(),
                    )? {
                        Some(normal) => normal,
                        None => Ty::AssociatedType(projection.clone()),
                    };
                    if !self.match_head(ty, &symbolic, &header.schema_env, slots, 0)? {
                        return Ok(false);
                    }
                    continue;
                }
                let normalized =
                    match self.normalize_closed_equality_projection(&projection, facts, active) {
                        Ok(value) => value,
                        Err(error) if error.kind() == FactsErrorKind::Unproved => continue,
                        Err(error) => return Err(error),
                    };
                if !self.match_head(ty, normalized.ty(), &header.schema_env, slots, 0)? {
                    return Ok(false);
                }
            }
            let after = slots.iter().filter(|slot| slot.is_some()).count();
            if before == after {
                return Ok(slots.iter().all(|slot| slot.is_some()));
            }
        }
    }
}
