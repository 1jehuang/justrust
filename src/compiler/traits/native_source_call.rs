//! Source-only native roles for a canonically selected implementation method.
//! Required trait item schemas do not themselves carry executable source bodies.
//! This bridge retains the caller's proof context and never grants dispatch.
use super::*;

impl ValidatedFacts {
    pub(in crate::compiler) fn native_sequence_element_for_call_with_scratch(
        &self,
        callee: &Callee,
        signature: &Signature,
        scratch: &mut TraitScratch,
    ) -> Result<Option<(DeclId, Arc<super::super::frontend::NativeElementSource>)>> {
        self.validate_source_flow_signature_with_scratch(
            SignatureSubject::Call(callee),
            signature,
            scratch,
        )?;
        let declaration = match callee {
            Callee::Direct(target) => target.declaration,
            Callee::TraitItem(call) => {
                let inputs =
                    self.parent_local_call_inputs_with_scratch(callee, signature, scratch)?;
                let validation = std::cell::RefCell::new(
                    nominal_normalization::ParentLocalValidation::default(),
                );
                let transport = ParentLocalTransport::new(&inputs).with_validation(&validation);
                transport.validate_with_scratch(&self.world, self, scratch)?;
                let context = ProofContext {
                    transport,
                    ..ProofContext::from_facts(self).for_storage(false)
                };
                let predicate = Predicate::Implements(call.trait_ref.clone());
                let evidence =
                    self.world
                        .require_in(&predicate, context, scratch.for_world(self.world.id))?;
                if evidence.0.world != self.world.id
                    || evidence.0.environment != context.predicates.index
                    || evidence.0.phase != context.phase
                    || evidence.0.executable_storage
                    || !context.matches_source(&evidence.0.source_metadata)?
                    || !self.world.eq_predicate(&evidence.0.goal, &predicate)?
                {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "native source role changed its original caller implementation proof",
                    ));
                }
                // A caller bound proves availability, not a particular body.
                // Only the canonical selected implementation can issue a role.
                if !matches!(&evidence.0.origin, ProofOrigin::Implementation { .. }) {
                    return Ok(None);
                }
                let target = self
                    .world
                    .selected_trait_target(call, self, context, &evidence, false)?;
                let selected = self.source_direct_call_signature_with_scratch(&target, scratch)?;
                if selected.params.len() != signature.params.len()
                    || !self.world.eq_ty(&selected.result, &signature.result, 0)?
                {
                    return Ok(None);
                }
                for (selected, original) in selected.params.iter().zip(&signature.params) {
                    if !self.world.eq_ty(selected, original, 0)? {
                        return Ok(None);
                    }
                }
                target.declaration
            }
            Callee::Virtual(_) | Callee::Untupled(_) => return Ok(None),
        };
        Ok(self
            .native_sequence_element_with_scratch(declaration, scratch)?
            .map(|role| (declaration, role)))
    }
}
