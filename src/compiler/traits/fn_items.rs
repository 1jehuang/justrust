//! Builtin callable evidence for function items and safe function pointers. The item's exact
//! declaration and ordered actuals select its instantiated signature; the
//! callable tuple is the ordered parameter list and `Output` is the result.
use super::*;

impl ValidatedFacts {
    pub(in crate::compiler) fn function_pointer_item_type(&self, item: &CallTarget) -> Result<Ty> {
        self.world.check_facts(self, true)?;
        self.world
            .schema
            .validate_fn_pointer_item(item.declaration)?;
        let signature =
            self.source_direct_call_signature_with_scratch(item, &mut TraitScratch::default())?;
        let pointer = charged(
            self.budget(),
            ir::FnPointerTy::new(signature.params, signature.result, self.budget()),
        )?;
        self.validate_fn_pointer_reification(item, &pointer)?;
        Ok(Ty::FnPointer(pointer))
    }

    pub(in crate::compiler) fn validate_fn_pointer_reification(
        &self,
        item: &CallTarget,
        pointer: &ir::FnPointerTy,
    ) -> Result<()> {
        self.world.check_facts(self, true)?;
        self.world
            .schema
            .validate_fn_pointer_item(item.declaration)?;
        charged(self.budget(), pointer.validate(self.budget()))?;
        for ty in pointer.types() {
            self.validate_ty(ty)?;
        }
        let signature =
            self.source_direct_call_signature_with_scratch(item, &mut TraitScratch::default())?;
        if !self.world.eq_types(&signature.params, &pointer.inputs, 0)?
            || !self.world.eq_ty(&signature.result, &pointer.output, 0)?
        {
            return Err(FactsError::invalid(
                self.budget(),
                "E0308: function item does not have the exact function pointer signature",
            ));
        }
        Ok(())
    }
}

impl ValidatedTraitWorld {
    pub(super) fn builtin_callable_signature(
        &self,
        ty: &Ty,
        context: ProofContext<'_>,
    ) -> Result<Signature> {
        match ty {
            Ty::FnItem(item) => self.fn_item_signature(item, context),
            Ty::FnPointer(pointer) => {
                self.validate_ty(ty, context.env(), None, true, 0)?;
                retain::<Ty>(&self.budget, pointer.inputs.len())?;
                Ok(Signature {
                    params: pointer.inputs.to_vec(),
                    result: pointer.output.clone(),
                    ref_contract: charged(
                        &self.budget,
                        ir::normalize_ref_contract(&ir::RefContract::default(), &self.budget),
                    )?,
                })
            }
            _ => Err(FactsError::invalid(
                &self.budget,
                "builtin callable lost exact item or pointer type",
            )),
        }
    }

    pub(super) fn builtin_callable_origin(&self, goal: &TraitRef) -> Result<(DeclId, Arc<[Ty]>)> {
        match &goal.self_ty {
            Ty::FnItem(item) => Ok((item.declaration, item.type_args.clone())),
            Ty::FnPointer(_) => {
                retain::<Ty>(&self.budget, 1)?;
                Ok((goal.declaration, Arc::from([goal.self_ty.clone()])))
            }
            _ => Err(FactsError::invalid(
                &self.budget,
                "builtin callable origin lost exact item or pointer",
            )),
        }
    }

    pub(super) fn validated_pointer_untupled(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<Signature> {
        if !matches!(call.trait_ref.self_ty, Ty::FnPointer(_))
            || !call.method_args.is_empty()
            || self
                .schema
                .closure_trait_kind(call.trait_ref.declaration)?
                .is_none()
            || self.item_header(call.item)?.trait_decl != call.trait_ref.declaration
        {
            return Err(FactsError::invalid(
                &self.budget,
                "indirect untupled call lost canonical callable trait item",
            ));
        }
        let context = ProofContext::from_facts(facts);
        match self.prove_fn_item(&Predicate::Implements(call.trait_ref.clone()), context)? {
            Some(ProofOutcome::Proved(_)) => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "function pointer call has no exact signature proof",
                ));
            }
        }
        let direct = self.builtin_callable_signature(&call.trait_ref.self_ty, context)?;
        let signature = self.instantiate_trait_signature(call, facts)?;
        // Universal callable signatures retain their Output projection. Compare
        // its proved normal form, but retain the original typed call evidence.
        let result = self.normalize_ty(&signature.result, facts)?;
        if signature.params.len() != 2
            || !self.eq_ty(&signature.params[1], &self.fn_item_arguments(&direct)?, 0)?
            || !self.eq_ty(&result, &direct.result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function pointer call changed exact indirect signature",
            ));
        }
        Ok(signature)
    }

    /// The instantiated signature of a function item, with its reference
    /// contract. Reference-bearing parameters are admitted; a result that
    /// borrows from an input is still rejected downstream with an explicit
    /// diagnostic.
    pub(super) fn fn_item_signature(
        &self,
        item: &CallTarget,
        context: ProofContext<'_>,
    ) -> Result<Signature> {
        let proof = self.discharge_in(item, context, &mut ProofScratch::default())?;
        Ok(proof.signature().clone())
    }

    pub(super) fn fn_item_arguments(&self, signature: &Signature) -> Result<Ty> {
        retain::<Ty>(&self.budget, signature.params.len())?;
        Ok(if signature.params.is_empty() {
            Ty::Unit
        } else {
            Ty::Tuple(signature.params.clone().into())
        })
    }

    pub(super) fn prove_fn_item(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
    ) -> Result<Option<ProofOutcome>> {
        let Predicate::Implements(goal) = predicate else {
            return Ok(None);
        };
        if !matches!(goal.self_ty, Ty::FnItem(_) | Ty::FnPointer(_)) {
            return Ok(None);
        }
        let Some(kind) = self.schema.closure_trait_kind(goal.declaration)? else {
            return Ok(None);
        };
        let signature = self.builtin_callable_signature(&goal.self_ty, context)?;
        if goal.args.len() != 1
            || !self.eq_ty(&goal.args[0], &self.fn_item_arguments(&signature)?, 0)?
        {
            return Ok(Some(ProofOutcome::NotProven));
        }
        Ok(Some(self.evidence(
            predicate,
            context,
            ProofOrigin::FnItem { kind },
        )?))
    }
}

impl ValidatedTraitWorld {
    /// Builtin untupled dispatch: the exact `FnItem: Fn*<(A..)>` goal is
    /// proved, the trait item signature `(receiver, (A..)) -> R` is returned,
    /// and the executed item's signature must be exactly `(A..) -> R`.
    pub(super) fn validated_untupled(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<(Signature, ir::DirectCallProofView)> {
        let Ty::FnItem(item) = &call.trait_ref.self_ty else {
            return Err(FactsError::invalid(
                &self.budget,
                "untupled call receiver is not a function item",
            ));
        };
        if !call.method_args.is_empty()
            || self
                .schema
                .closure_trait_kind(call.trait_ref.declaration)?
                .is_none()
            || self.item_header(call.item)?.trait_decl != call.trait_ref.declaration
        {
            return Err(FactsError::invalid(
                &self.budget,
                "untupled call is not a builtin callable trait item",
            ));
        }
        let context = ProofContext::from_facts(facts);
        match self.prove_fn_item(&Predicate::Implements(call.trait_ref.clone()), context)? {
            Some(ProofOutcome::Proved(_)) => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "untupled call lacks function item callable evidence",
                ));
            }
        }
        let direct = self.discharge_in(item, context, &mut ProofScratch::default())?;
        let trait_signature = self.instantiate_trait_signature(call, facts)?;
        let arguments = self.fn_item_arguments(direct.signature())?;
        if trait_signature.params.len() != 2
            || !self.eq_ty(&trait_signature.params[1], &arguments, 0)?
            || !self.eq_ty(&trait_signature.result, &direct.signature().result, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "untupled call signature differs from its function item",
            ));
        }
        Ok((
            trait_signature,
            ir::DirectCallProofView {
                target: direct.target().clone(),
                signature: direct.signature().clone(),
            },
        ))
    }
}

impl ValidatedTraitWorld {
    /// The callable tuple input of a function item is exactly its ordered
    /// parameter tuple, references included.
    pub(super) fn closed_fn_item_tuple_input(
        &self,
        goal: &TraitRef,
        signature: &Signature,
        index: usize,
        context: ProofContext<'_>,
    ) -> Result<bool> {
        let direct = self.builtin_callable_signature(&goal.self_ty, context)?;
        let tuple = self.fn_item_arguments(&direct)?;
        if goal.args.len() != 1
            || signature.params.get(index).is_none()
            || !self.eq_ty(&signature.params[index], &tuple, 0)?
            || !self.eq_ty(&goal.args[0], &tuple, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "function item callable tuple differs from its signature",
            ));
        }
        Ok(true)
    }
}
