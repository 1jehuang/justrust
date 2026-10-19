//! Non-callable associated member type schemas and their declaration-bound facts.
//! Evaluated values are graph outputs, never method providers or runtime getters.
use super::*;

#[derive(Clone, Debug)]
pub struct AssociatedConstHeader {
    pub declaration: DeclId,
    pub owner: DeclId,
    pub owner_ty: Option<Ty>,
    pub implemented_member: Option<DeclId>,
    pub env: Arc<GenericEnv>,
    pub ty: Ty,
    pub predicates: Arc<[Predicate]>,
    pub has_initializer: bool,
}

/// Logical closed member selection. It is not authority for a source occurrence
/// and contains no planning context, mutable budget, or runtime getter.
#[derive(Clone, Debug)]
pub struct ClosedAssociatedConstant {
    world: WorldId,
    member: DeclId,
    application: TraitRef,
    implementation: DeclId,
    declaration: DeclId,
    arguments: Arc<[Ty]>,
    ty: Ty,
    membership: ProofEvidence,
}
impl ClosedAssociatedConstant {
    pub fn member(&self) -> DeclId {
        self.member
    }
    pub fn declaration(&self) -> DeclId {
        self.declaration
    }
    pub fn arguments(&self) -> &[Ty] {
        &self.arguments
    }
    pub fn ty(&self) -> &Ty {
        &self.ty
    }
    pub fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        let world = facts.world();
        world.check_facts(facts, true)?;
        if self.world != world.id
            || self.membership.0.world != world.id
            || self.membership.0.environment != world.empty.index
            || self.membership.0.phase != FactsPhase::Closed
            || !world.eq_predicate(
                &self.membership.0.goal,
                &Predicate::Implements(self.application.clone()),
            )?
        {
            return Err(FactsError::invalid(
                &world.budget,
                "associated constant selection has foreign world or membership context",
            ));
        }
        let replay = world.select_associated_constant(self.member, &self.application, facts)?;
        if replay.implementation != self.implementation
            || replay.declaration != self.declaration
            || !world.eq_types(&replay.arguments, &self.arguments, 0)?
            || !world.eq_ty(&replay.ty, &self.ty, 0)?
        {
            return Err(FactsError::invalid(
                &world.budget,
                "associated constant selection changed its canonical implementation or value type",
            ));
        }
        Ok(())
    }
}

/// A declared member type under exact caller assumptions, not an evaluated value.
#[derive(Clone, Debug)]
pub struct AssociatedConstantType {
    world: WorldId,
    caller: DeclId,
    environment: usize,
    phase: FactsPhase,
    member: DeclId,
    application: TraitRef,
    ty: Ty,
    membership: ProofEvidence,
}
impl PartialEq for AssociatedConstantType {
    fn eq(&self, other: &Self) -> bool {
        self.world == other.world
            && self.caller == other.caller
            && self.environment == other.environment
            && self.phase == other.phase
            && self.member == other.member
            && self.application == other.application
            && self.ty == other.ty
    }
}
impl Eq for AssociatedConstantType {}
impl AssociatedConstantType {
    pub fn member(&self) -> DeclId {
        self.member
    }
    pub fn application(&self) -> &TraitRef {
        &self.application
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
            || self.membership.0.world != world.id
            || self.membership.0.environment != self.environment
            || self.membership.0.phase != self.phase
            || !world.eq_predicate(
                &self.membership.0.goal,
                &Predicate::Implements(self.application.clone()),
            )?
        {
            return Err(FactsError::invalid(
                &world.budget,
                "associated constant type has foreign caller predicate authority",
            ));
        }
        let replay = world.associated_constant_type(self.member, &self.application, facts)?;
        if !world.eq_ty(&replay.ty, &self.ty, 0)? {
            return Err(FactsError::invalid(
                &world.budget,
                "associated constant declared type replay changed",
            ));
        }
        Ok(())
    }
}

impl ValidatedTraitWorld {
    pub fn associated_constant_type(
        &self,
        member: DeclId,
        application: &TraitRef,
        facts: &ValidatedFacts,
    ) -> Result<AssociatedConstantType> {
        self.check_facts(facts, false)?;
        let header = self.associated_constant_header(member)?;
        if header.owner != application.declaration
            || self.info(header.owner)?.kind != DeclKind::Trait
        {
            return Err(FactsError::invalid(
                &self.budget,
                "associated constant type selected a foreign trait member",
            ));
        }
        let context = ProofContext {
            predicates: &facts.predicates,
            phase: facts.phase(),
        };
        let membership = match self.prove_in(
            &Predicate::Implements(application.clone()),
            context,
            &mut Vec::new(),
        )? {
            ProofOutcome::Proved(proof) => proof,
            ProofOutcome::NotProven => {
                return Err(FactsError::unproved(
                    &self.budget,
                    "associated constant declared type lacks trait membership",
                ));
            }
        };
        let arguments = self.trait_arguments(application)?;
        let raw =
            self.schema
                .instantiate_constant_type(member, &arguments, facts.env(), &self.budget)?;
        self.validate_rhs_substitution(&header.ty, &raw, &header.env, &arguments, 0)?;
        let ty = facts.normalize_substituted_projection_type(&raw, &self.budget)?;
        retain::<AssociatedConstantType>(&self.budget, 1)?;
        Ok(AssociatedConstantType {
            world: self.id,
            caller: facts.declaration(),
            environment: facts.predicates.index,
            phase: facts.phase(),
            member,
            application: application.clone(),
            ty,
            membership,
        })
    }

    pub fn select_associated_constant(
        &self,
        member: DeclId,
        application: &TraitRef,
        facts: &ValidatedFacts,
    ) -> Result<ClosedAssociatedConstant> {
        self.check_facts(facts, true)?;
        if facts.phase() != FactsPhase::Closed || !self.goal_is_closed(application, facts.env())? {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: associated constant evaluation requires closed caller and trait arguments",
            ));
        }
        let original = self.associated_constant_header(member)?;
        if original.owner != application.declaration
            || self.info(original.owner)?.kind != DeclKind::Trait
        {
            return Err(FactsError::invalid(
                &self.budget,
                "associated constant application selected a foreign trait member",
            ));
        }
        self.validate_ref_in(application, facts.env(), None)?;
        let context = ProofContext {
            predicates: &self.empty,
            phase: FactsPhase::Closed,
        };
        let membership = match self.prove_implementation_in(
            &Predicate::Implements(application.clone()),
            context,
            &mut Vec::new(),
        )? {
            ProofOutcome::Proved(proof) => proof,
            ProofOutcome::NotProven => {
                return Err(FactsError::unproved(
                    &self.budget,
                    "associated constant has no concrete implementation witness",
                ));
            }
        };
        let (implementation, implementation_arguments) = match &membership.0.origin {
            ProofOrigin::Implementation {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            _ => {
                return Err(FactsError::unproved(
                    &self.budget,
                    "associated constant requires concrete implementation evidence",
                ));
            }
        };
        let (declaration, arguments) = match self.constant_impl_index.get(&(implementation, member))
        {
            Some(declaration) => (*declaration, implementation_arguments),
            None if original.has_initializer => (member, self.trait_arguments(application)?.into()),
            None => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "E0046: selected implementation has no required associated constant",
                ));
            }
        };
        let header = self.associated_constant_header(declaration)?;
        let raw = self.schema.instantiate_constant_type(
            declaration,
            &arguments,
            facts.env(),
            &self.budget,
        )?;
        self.validate_rhs_substitution(&header.ty, &raw, &header.env, &arguments, 0)?;
        let ty = self.normalize_ty(&raw, facts)?;
        let trait_arguments = self.trait_arguments(application)?;
        let required = self.schema.instantiate_constant_type(
            member,
            &trait_arguments,
            facts.env(),
            &self.budget,
        )?;
        self.validate_rhs_substitution(
            &original.ty,
            &required,
            &original.env,
            &trait_arguments,
            0,
        )?;
        let required = self.normalize_ty(&required, facts)?;
        if !self.eq_ty(&ty, &required, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "associated constant selected value type differs from declared member type",
            ));
        }
        retain::<ClosedAssociatedConstant>(&self.budget, 1)?;
        Ok(ClosedAssociatedConstant {
            world: self.id,
            member,
            application: application.clone(),
            implementation,
            declaration,
            arguments,
            ty,
            membership,
        })
    }

    pub(super) fn build_constant_index(&mut self) -> Result<()> {
        for (index, header) in self.headers.constants.iter().enumerate() {
            self.info(header.declaration)?;
            if self
                .constant_header_index
                .get(&self.budget, header.declaration)?
                .is_some()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "duplicate associated constant header identity",
                ));
            }
            self.constant_header_index
                .note(&self.budget, header.declaration, index)?;
            if let Some(member) = header.implemented_member {
                charged(&self.budget, self.budget.charge_work(1))?;
                self.constant_impl_index.try_reserve(1).map_err(|_| {
                    FactsError::resource(&self.budget, "constant member index allocation")
                })?;
                if self
                    .constant_impl_index
                    .insert((header.owner, member), header.declaration)
                    .is_some()
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "duplicate canonical associated constant implementation",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn associated_constant_header(
        &self,
        declaration: DeclId,
    ) -> Result<&AssociatedConstHeader> {
        self.info(declaration)?;
        self.constant_header_index
            .get(&self.budget, declaration)?
            .and_then(|index| self.headers.constants.get(index))
            .ok_or_else(|| {
                FactsError::invalid(&self.budget, "missing associated constant type header")
            })
    }

    pub(super) fn validate_constant_headers(&self) -> Result<()> {
        for header in self.headers.constants.iter() {
            let info = self.info(header.declaration)?;
            if info.kind != DeclKind::AssociatedConst
                || info.owner != Some(header.owner)
                || !self.same_env(&info.env, &header.env)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated constant has foreign declaration/owner/binder authority",
                ));
            }
            let owner = self.info(header.owner)?;
            if !matches!(
                owner.kind,
                DeclKind::Trait | DeclKind::Impl | DeclKind::TraitImpl
            ) || !self.same_env(&owner.env, &header.env)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated constant has invalid owner environment",
                ));
            }
            let source = self.schema.associated_constant_schema(header.declaration)?;
            if source.declaration != header.declaration
                || source.owner != header.owner
                || source.has_initializer != header.has_initializer
                || source.implemented_member != header.implemented_member
                || source.owner_ty != header.owner_ty
                || !self.same_env(&source.env, &header.env)?
                || !self.eq_ty(&source.ty, &header.ty, 0)?
                || source.predicates.len() != header.predicates.len()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated constant header differs from retained source schema",
                ));
            }
            for (actual, original) in header.predicates.iter().zip(source.predicates.iter()) {
                if !self.eq_predicate(actual, original)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated constant predicate changed source authority",
                    ));
                }
            }
            self.validate_raw_associated_ty(&header.ty, &header.env, 0)?;
            self.validate_predicates(&header.predicates, &header.env, None)?;
            if owner.kind != DeclKind::Trait && !header.has_initializer {
                return Err(FactsError::invalid(
                    &self.budget,
                    "E0046: associated constant implementation has no initializer",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn build_constant_environments(&mut self) -> Result<()> {
        for header in self.headers.constants.iter() {
            let mut predicates = Vec::new();
            for predicate in header.predicates.iter() {
                self.append_predicate(&mut predicates, predicate)?;
            }
            self.implicit_sized(&header.env, &mut predicates)?;
            if self.info(header.owner)?.kind == DeclKind::Trait {
                self.append_predicate(
                    &mut predicates,
                    &Predicate::Implements(self.symbolic_trait_ref(header.owner)?),
                )?;
            }
            self.close_predicates(&mut predicates, &header.env)?;
            self.validate_declared_equality_domains(&predicates, &header.env)?;
            retain::<PredicateEnv>(&self.budget, 1)?;
            let env = Arc::new(PredicateEnv {
                world: self.id,
                index: self.environments.len(),
                declaration: Some(header.declaration),
                env: header.env.clone(),
                predicates: predicates.into(),
            });
            push_indexed_environment(
                &self.budget,
                &mut self.environments,
                &mut self.environment_index,
                env,
            )?;
        }
        Ok(())
    }

    pub(super) fn validate_constant_implementations(self: &Arc<Self>) -> Result<()> {
        let mut owners = DeclIndex::default();
        for (index, implementation) in self.headers.impls.iter().enumerate() {
            owners.note(&self.budget, implementation.declaration, index)?;
        }
        for header in self.headers.constants.iter() {
            let Some(member) = header.implemented_member else {
                continue;
            };
            let index = owners.get(&self.budget, header.owner)?.ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "constant implementation lost its exact trait impl owner",
                )
            })?;
            let implementation = &self.headers.impls[index];
            let original = self.associated_constant_header(member)?;
            if original.owner != implementation.trait_ref.declaration {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated constant implementation selected a foreign trait member",
                ));
            }
            let facts = self.associated_constant_facts(header.declaration)?;
            let arguments = self.trait_arguments(&implementation.trait_ref)?;
            let expected = self.schema.instantiate_constant_type(
                member,
                &arguments,
                facts.env(),
                &self.budget,
            )?;
            self.validate_rhs_substitution(&original.ty, &expected, &original.env, &arguments, 0)?;
            let expected = self.normalize_ty(&expected, &facts)?;
            let actual = self.normalize_ty(&header.ty, &facts)?;
            if !self.eq_ty(&expected, &actual, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "E0326: implemented associated constant type differs from its trait declaration",
                ));
            }
        }
        Ok(())
    }

    pub fn associated_constant_facts(
        self: &Arc<Self>,
        declaration: DeclId,
    ) -> Result<Arc<ValidatedFacts>> {
        self.associated_constant_header(declaration)?;
        let predicates = self.predicate_env(declaration)?;
        retain::<ValidatedFacts>(&self.budget, 1)?;
        Ok(Arc::new(ValidatedFacts {
            world: self.clone(),
            subject: FactsSubject::AssociatedConstant(declaration),
            predicates,
            requirements: Arc::from([]),
            closure: None,
            closure_mode: None,
        }))
    }
}
