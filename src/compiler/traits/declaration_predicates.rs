//! Declaration bounds describe logical type applications, not executable storage.
use super::*;

#[derive(Clone, Debug)]
pub struct DeclarationPredicateDomain {
    pub declaration: DeclId,
    pub env: Arc<GenericEnv>,
    pub predicates: Arc<[Predicate]>,
}

impl ValidatedTraitWorld {
    pub(super) fn build_declaration_predicate_domains(&mut self) -> Result<()> {
        for index in 0..self.headers.traits.len() {
            let header = &self.headers.traits[index];
            let mut predicates = Vec::new();
            for bound in header.supertraits.iter() {
                self.append_predicate(&mut predicates, &Predicate::Implements(bound.clone()))?;
            }
            for equality in header.supertrait_equalities.iter() {
                self.append_predicate(&mut predicates, equality)?;
            }
            // A trait declaration's Self implements that exact trait. This is
            // its original owner premise, not the nominal application's goal.
            self.append_predicate(
                &mut predicates,
                &Predicate::Implements(self.symbolic_trait_ref(header.declaration)?),
            )?;
            self.retain_declaration_predicate_domain(DeclarationPredicateDomain {
                declaration: header.declaration,
                env: header.env.clone(),
                predicates: predicates.into(),
            })?;
        }
        for domain in self.schema.declaration_predicate_domains()? {
            self.retain_declaration_predicate_domain(domain)?;
        }
        Ok(())
    }

    fn retain_declaration_predicate_domain(
        &mut self,
        domain: DeclarationPredicateDomain,
    ) -> Result<()> {
        let info = self.info(domain.declaration)?;
        if !self.same_env(&info.env, &domain.env)? {
            return Err(FactsError::invalid(
                &self.budget,
                "declaration predicate domain changed its original binder",
            ));
        }
        self.validate_predicates(&domain.predicates, &domain.env, None)?;
        let mut predicates = Vec::new();
        for predicate in domain.predicates.iter() {
            self.append_predicate(&mut predicates, predicate)?;
        }
        self.implicit_sized(&domain.env, &mut predicates)?;
        self.close_predicates(&mut predicates, &domain.env)?;
        self.validate_declared_equality_domains(&predicates, &domain.env)?;
        retain::<PredicateEnv>(&self.budget, 1)?;
        let env = Arc::new(PredicateEnv {
            world: self.id,
            index: self.environments.len(),
            declaration: Some(domain.declaration),
            env: domain.env,
            predicates: predicates.into(),
        });
        push_indexed_environment(
            &self.budget,
            &mut self.environments,
            &mut self.environment_index,
            env,
        )
    }

    // A well-formed logical application still owes its source declaration's
    // bounds. These are proved from the retained original caller rows, never
    // appended as assumptions and never obtained by inspecting physical fields.
    pub(super) fn validate_declaration_nominal_membership(
        &self,
        predicates: &PredicateEnv,
    ) -> Result<()> {
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates,
            executable_storage: false,
            phase: FactsPhase::Universal,
        };
        self.check_context(context)?;
        let mut active = ProofScratch::default();
        for predicate in predicates.predicates.iter() {
            for ty in predicate.metadata_types() {
                self.validate_declaration_nominal_type(ty, context, &mut active, 0)?;
            }
        }
        Ok(())
    }

    fn validate_declaration_nominal_type(
        &self,
        ty: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if ty.is_nominal_back_edge() {
            return Ok(());
        }
        let nominal = match ty {
            Ty::Struct(owner) => Some((owner.def().source_decl, &owner.def().type_args)),
            Ty::Enum(owner) => Some((owner.def().source_decl, &owner.def().type_args)),
            _ => None,
        };
        if let Some((owner, actuals)) = nominal {
            let info = self.info(owner)?;
            let declared = self.schema.nominal_declaration_schema(owner)?;
            if declared.declaration != owner
                || !matches!(info.kind, DeclKind::Struct | DeclKind::Enum)
                || !self.same_env(&info.env, &declared.env)?
                || declared.env.total_arity() != actuals.len()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "logical nominal bound source declaration or binder changed",
                ));
            }
            for predicate in declared.predicates.iter() {
                self.validate_predicate_metadata(predicate, &declared.env)?;
            }
            retain::<ir::RefContract>(&self.budget, 1)?;
            let schema = GenericSchema {
                env: declared.env,
                signature: Signature {
                    ref_contract: Arc::new(ir::RefContract::default()),
                    params: Vec::new(),
                    result: Ty::Unit,
                },
                predicates: declared.predicates,
            };
            let instantiated =
                self.schema
                    .instantiate_schema(&schema, actuals, context.env(), &self.budget)?;
            for obligation in instantiated.predicates.iter() {
                self.require_in(obligation, context, active)?;
            }
        }
        let mut child =
            |ty: &Ty| self.validate_declaration_nominal_type(ty, context, active, depth + 1);
        match ty {
            Ty::Struct(owner) => {
                for ty in owner.def().type_args.iter() {
                    child(ty)?;
                }
            }
            Ty::Enum(owner) => {
                for ty in owner.def().type_args.iter() {
                    child(ty)?;
                }
            }
            Ty::Ref { referent, .. } => child(referent)?,
            Ty::RawPtr { pointee, .. } => child(pointee)?,
            Ty::Array { element, .. }
            | Ty::DependentArray { element, .. }
            | Ty::Slice { element } => child(element)?,
            Ty::Tuple(parts) => {
                for ty in parts.iter() {
                    child(ty)?;
                }
            }
            Ty::AssociatedType(projection) => {
                child(&projection.trait_ref.self_ty)?;
                for ty in projection.trait_ref.args.iter() {
                    child(ty)?;
                }
            }
            Ty::FnItem(target) => {
                for ty in target.type_args.iter() {
                    child(ty)?;
                }
            }
            Ty::FnPointer(pointer) => {
                for ty in pointer.types() {
                    child(ty)?;
                }
            }
            Ty::Closure(closure) => {
                for ty in closure.types() {
                    child(ty)?;
                }
            }
            Ty::Opaque(opaque) => {
                for ty in opaque.types() {
                    child(ty)?;
                }
            }
            Ty::Dyn(object) => {
                for ty in object.types() {
                    child(ty)?;
                }
            }
            Ty::Cell(wrapper) => child(wrapper.argument())?,
            Ty::ManuallyDrop(wrapper) => child(wrapper.argument())?,
            Ty::PhantomData(wrapper) => child(wrapper.argument())?,
            Ty::Param(_)
            | Ty::Int(_)
            | Ty::Char
            | Ty::Str
            | Ty::Float(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Never
            | Ty::Const(_) => {}
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_declaration_predicate_formation(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        usage: TypeUse,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        // Keep the exact raw source identity, including its nominal schema and
        // ordered actuals. A declaration such as Holder<Source>: Trait does not
        // execute Holder's fields before their associated projections normalize.
        // The existing logical policy still validates every actual and direct
        // projection. Its successes cannot authorize physical storage formation.
        // Final declared-context checks repeat this policy after normalization,
        // while signatures and executable fields retain their physical checks.
        replays
            .raw_formations
            .check_for_use(self, ty, env, usage, maybe_unsized, 0, false)
    }
}
