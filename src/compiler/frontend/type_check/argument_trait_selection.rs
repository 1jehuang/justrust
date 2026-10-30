//! `Owner::item(args)` where `Owner` implements one trait several times with
//! different trait arguments (`u32: From<u8>`, `u32: From<u16>`, ...). Stock
//! rustc picks the unique application whose instantiated parameter types the
//! argument types satisfy. This uses exactly known argument types or the
//! integer-only domain of an unsuffixed literal, never width fallback.
//! Ordinary selection then rediscovers it as an explicit
//! trait call and discharges every obligation; this grants no membership.
use super::*;

impl Checker<'_> {
    /// The unique trait application of `owner` whose `name` item accepts
    /// `args`, when ordinary selection would otherwise see several
    /// applications of one trait. `Ok(None)` leaves the ordinary path (and its
    /// ambiguity diagnostic) unchanged.
    pub(super) fn argument_selected_trait_application(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        type_args: &[Type],
        args: &[Expr],
    ) -> Result<Option<ir::TraitRef>, String> {
        if !type_args.is_empty() || args.is_empty() {
            return Ok(None);
        }
        // Trait-application disambiguation must not bypass a matching inherent
        // item, including an inherent method whose actuals remain unresolved.
        if matches!(
            self.inherent_method_actuals(owner, name, args, type_args, None, None)?,
            inherent_method_hint::InherentMethodInference::Applicable(_)
        ) {
            return Ok(None);
        }
        // A bare integer literal rules out non-integer formals, but never
        // chooses one integer width over another or applies integer fallback.
        self.infer_storage(args.len())?;
        let mut actual = Vec::with_capacity(args.len());
        for arg in args {
            let ty = self.scalar_hint(arg)?;
            if ty.is_none() && !matches!(arg, Expr::Integer(literal) if literal.suffix.is_none()) {
                return Ok(None);
            }
            actual.push(ty);
        }
        self.trait_application_matching_arguments(owner, name, args.len(), |formals| {
            let plan = self.signatures.plan()?;
            for (formal, actual) in formals.iter().zip(&actual) {
                match actual {
                    Some(actual) if metered_type_equal(formal, actual, &plan.budget, 0)? => {}
                    None if matches!(formal, ir::Ty::Int(_)) => {}
                    _ => return Ok(false),
                }
            }
            Ok(true)
        })
    }

    /// When `owner` has several applications of one trait whose `name` item
    /// all return the same concrete type, that result. This is only an
    /// inference hint; selection of the exact application happens later.
    pub(in crate::compiler::frontend) fn shared_trait_application_result(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        arity: usize,
    ) -> Result<Option<ir::Ty>, String> {
        let plan = self.signatures.plan()?;
        let candidates =
            plan.method_trait_candidates(owner, name, self.facts(), &self.module, &plan.budget)?;
        let candidates = candidates.as_slice();
        let Some((first, _)) = candidates.first() else {
            return Ok(None);
        };
        if candidates.len() < 2
            || candidates
                .iter()
                .any(|(application, _)| application.declaration != first.declaration)
        {
            return Ok(None);
        }
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Ok(None);
        };
        let mut result: Option<ir::Ty> = None;
        for (application, item) in candidates {
            plan.budget.charge_work(1)?;
            if plan.param_env(*item)?.arity() != 0 {
                return Ok(None);
            }
            let call = ir::TraitCall {
                trait_ref: application.clone(),
                item: *item,
                method_args: Arc::from([]),
            };
            let signature = facts
                .service()
                .instantiate_trait_signature(&call, facts)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            if signature.params.len() != arity {
                return Ok(None);
            }
            match &result {
                None => result = Some(signature.result),
                Some(previous) => {
                    if !metered_type_equal(previous, &signature.result, &plan.budget, 0)? {
                        return Ok(None);
                    }
                }
            }
        }
        Ok(result)
    }

    /// As above, from exactly known argument types.
    pub(in crate::compiler::frontend) fn trait_application_for_argument_types(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        actual: &[ir::Ty],
    ) -> Result<Option<ir::TraitRef>, String> {
        self.trait_application_matching_arguments(owner, name, actual.len(), |formals| {
            let plan = self.signatures.plan()?;
            for (formal, actual) in formals.iter().zip(actual) {
                if !metered_type_equal(formal, actual, &plan.budget, 0)? {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }

    fn trait_application_matching_arguments(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        arity: usize,
        mut fits: impl FnMut(&[ir::Ty]) -> Result<bool, String>,
    ) -> Result<Option<ir::TraitRef>, String> {
        if arity == 0 {
            return Ok(None);
        }
        let plan = self.signatures.plan()?;
        let candidates =
            plan.method_trait_candidates(owner, name, self.facts(), &self.module, &plan.budget)?;
        let candidates = candidates.as_slice();
        // Only the multiple-applications-of-one-trait case is changed.
        let Some((first, _)) = candidates.first() else {
            return Ok(None);
        };
        if candidates.len() < 2
            || candidates
                .iter()
                .any(|(application, _)| application.declaration != first.declaration)
        {
            return Ok(None);
        }
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Ok(None);
        };
        let mut selected: Option<ir::TraitRef> = None;
        for (application, item) in candidates {
            plan.budget.charge_work(1)?;
            if plan.param_env(*item)?.arity() != 0 {
                return Ok(None);
            }
            if matches!(
                facts
                    .service()
                    .prove_goal(&ir::Predicate::Implements(application.clone()), facts)
                    .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
                tr::ProofOutcome::NotProven
            ) {
                continue;
            }
            let call = ir::TraitCall {
                trait_ref: application.clone(),
                item: *item,
                method_args: Arc::from([]),
            };
            let signature = facts
                .service()
                .instantiate_trait_signature(&call, facts)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            if signature.params.len() != arity {
                continue;
            }
            if !fits(&signature.params)? {
                continue;
            }
            if selected.replace(application.clone()).is_some() {
                return Ok(None);
            }
        }
        Ok(selected)
    }
}
