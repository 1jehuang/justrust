//! Inherent constant owner/type obligations, distinct from trait membership.
use super::*;

#[derive(Clone, Debug)]
pub struct InherentConstantType {
    world: WorldId,
    caller: DeclId,
    environment: usize,
    phase: FactsPhase,
    member: DeclId,
    owner: Ty,
    arguments: Arc<[Ty]>,
    ty: Ty,
    obligations: Arc<[ProofEvidence]>,
}
impl PartialEq for InherentConstantType {
    fn eq(&self, other: &Self) -> bool {
        self.world == other.world
            && self.caller == other.caller
            && self.environment == other.environment
            && self.phase == other.phase
            && self.member == other.member
            && self.owner == other.owner
            && self.arguments == other.arguments
            && self.ty == other.ty
    }
}
impl Eq for InherentConstantType {}
impl InherentConstantType {
    pub fn member(&self) -> DeclId {
        self.member
    }
    pub fn arguments(&self) -> &[Ty] {
        &self.arguments
    }
    pub fn ty(&self) -> &Ty {
        &self.ty
    }
    pub fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        let world = facts.world();
        world.check_facts(facts, false)?;
        if self.world != world.id
            || self.caller != facts.declaration()
            || self.environment != facts.predicates.index
            || self.phase != facts.phase()
        {
            return Err(FactsError::invalid(
                &world.budget,
                "inherent constant has foreign caller/world/predicate authority",
            ));
        }
        let replay =
            world.inherent_constant_type(self.member, &self.owner, &self.arguments, facts)?;
        if !world.eq_ty(&replay.ty, &self.ty, 0)?
            || replay.obligations.len() != self.obligations.len()
        {
            return Err(FactsError::invalid(
                &world.budget,
                "inherent constant type or obligation coverage changed",
            ));
        }
        for (actual, expected) in self.obligations.iter().zip(replay.obligations.iter()) {
            if actual.0.world != world.id
                || actual.0.environment != self.environment
                || actual.0.phase != self.phase
                || !world.eq_predicate(&actual.0.goal, &expected.0.goal)?
            {
                return Err(FactsError::invalid(
                    &world.budget,
                    "inherent constant obligation evidence changed",
                ));
            }
        }
        Ok(())
    }
}
impl ValidatedTraitWorld {
    pub fn inherent_constant_type(
        &self,
        member: DeclId,
        owner: &Ty,
        arguments: &[Ty],
        facts: &ValidatedFacts,
    ) -> Result<InherentConstantType> {
        self.check_facts(facts, false)?;
        let header = self.associated_constant_header(member)?;
        if self.info(header.owner)?.kind != DeclKind::Impl
            || header.implemented_member.is_some()
            || arguments.len() != header.env.total_arity()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "inherent constant has foreign owner or binder arity",
            ));
        }
        self.validate_ty_metadata_in(owner, facts.env(), 0)?;
        for argument in arguments {
            self.validate_executable_actual(argument, facts.env())?;
        }
        let (actual_owner, raw_ty, predicates) = self.schema.instantiate_constant_metadata(
            member,
            arguments,
            facts.env(),
            &self.budget,
        )?;
        let raw_owner = header.owner_ty.as_ref().ok_or_else(|| {
            FactsError::invalid(
                &self.budget,
                "inherent constant lost its original impl Self pattern",
            )
        })?;
        let actual_owner = actual_owner.ok_or_else(|| {
            FactsError::invalid(&self.budget, "inherent constant substitution lost owner")
        })?;
        self.validate_rhs_substitution(raw_owner, &actual_owner, &header.env, arguments, 0)?;
        self.validate_rhs_substitution(&header.ty, &raw_ty, &header.env, arguments, 0)?;
        let actual_owner =
            facts.normalize_substituted_projection_type(&actual_owner, &self.budget)?;
        if !self.eq_ty(&actual_owner, owner, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "inherent constant substituted owner differs from original receiver",
            ));
        }
        if header.predicates.len() != predicates.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "inherent constant predicate substitution coverage changed",
            ));
        }
        self.validate_predicates(&predicates, facts.env(), None)?;
        let mut obligations = Vec::new();
        for (original, predicate) in header.predicates.iter().zip(predicates.iter()) {
            self.validate_predicate_substitution(original, predicate, &header.env, arguments)?;
            match self.prove_in(predicate, ProofContext::from_facts(facts), &mut Vec::new())? {
                ProofOutcome::Proved(proof) => push(&self.budget, &mut obligations, proof)?,
                ProofOutcome::NotProven => {
                    return Err(FactsError::unproved(
                        &self.budget,
                        "inherent constant owner bounds are not satisfied",
                    ));
                }
            }
        }
        let ty = facts.normalize_substituted_projection_type(&raw_ty, &self.budget)?;
        retain::<InherentConstantType>(&self.budget, 1)?;
        retain::<Ty>(&self.budget, arguments.len())?;
        Ok(InherentConstantType {
            world: self.id,
            caller: facts.declaration(),
            environment: facts.predicates.index,
            phase: facts.phase(),
            member,
            owner: owner.clone(),
            arguments: arguments.into(),
            ty,
            obligations: obligations.into(),
        })
    }
}
