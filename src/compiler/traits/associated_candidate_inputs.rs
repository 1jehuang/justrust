//! Original declaration obligations for demand-only associated-call frontiers.
use super::*;

impl ValidatedTraitWorld {
    /// Candidate enumeration publishes no membership, callable or layout proof.
    pub(in crate::compiler) fn original_implementation_self_for_discovery(
        &self,
        declaration: DeclId,
        arguments: &[Ty],
        caller: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Ty> {
        self.check_facts(caller, true)?;
        let position = self
            .impl_header_index
            .get(&self.budget, declaration)?
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "candidate implementation declaration is absent",
                )
            })?;
        let header = &self.headers.impls[position];
        if self.info(declaration)?.kind != DeclKind::TraitImpl || header.declaration != declaration
        {
            return Err(FactsError::invalid(
                &self.budget,
                "candidate implementation declaration changed kind",
            ));
        }
        self.validate_original_declaration_environment(declaration, &header.schema_env)?;
        charged(
            &self.budget,
            header
                .schema_env
                .validate_argument_kinds(arguments, Some(caller.env()), &self.budget),
        )?;
        let mut replays = NominalReplays::default();
        self.validate_ty_metadata_with(
            &header.trait_ref.self_ty,
            &header.schema_env,
            0,
            &mut replays,
        )?;
        for (parameter, argument) in header.schema_env.ordered_params().zip(arguments) {
            if matches!(
                header.schema_env.parameter_kind(parameter),
                Some(ir::GenericParamKind::Type)
            ) {
                self.validate_ty_metadata_with_facts(
                    argument,
                    caller.env(),
                    0,
                    &mut replays,
                    Some(caller),
                )?;
                charged(
                    &self.budget,
                    ir::validate_root_reference_actual(argument, &self.budget),
                )?;
            }
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        let schema = GenericSchema {
            env: header.schema_env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: header.trait_ref.self_ty.clone(),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            // Candidate predicates are not caller premises. Selection proves
            // the original obligations only after choosing an application.
            predicates: Arc::from([]),
        };
        let value = self.schema.instantiate_schema_with_facts_scratch(
            &schema,
            arguments,
            caller,
            &self.budget,
            scratch,
        )?;
        if !value.signature.params.is_empty()
            || !value.predicates.is_empty()
            || !self.eq_contract(&value.signature.ref_contract, &ir::RefContract::default())?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "candidate metadata issued callable authority",
            ));
        }
        self.validate_rhs_substitution(
            &header.trait_ref.self_ty,
            &value.signature.result,
            &header.schema_env,
            arguments,
            0,
        )?;
        self.validate_ty_metadata_with_facts(
            &value.signature.result,
            caller.env(),
            0,
            &mut replays,
            Some(caller),
        )?;
        caller.normalize_substituted_projection_metadata(
            &value.signature.result,
            &self.budget,
            scratch,
        )
    }

    /// Original obligations only. This grants no premise or executable type.
    pub(in crate::compiler) fn original_predicate_schema(
        &self,
        declaration: DeclId,
    ) -> Result<GenericSchema> {
        self.check_normalization_budget()?;
        let info = self.info(declaration)?;
        let (env, predicates) = match info.kind {
            DeclKind::TraitImpl => {
                let position = self
                    .impl_header_index
                    .get(&self.budget, declaration)?
                    .ok_or_else(|| {
                        FactsError::invalid(&self.budget, "missing original impl predicates")
                    })?;
                let header = &self.headers.impls[position];
                if header.declaration != declaration {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "original impl predicate owner changed",
                    ));
                }
                (header.schema_env.clone(), header.predicates.clone())
            }
            DeclKind::Struct | DeclKind::Enum => {
                let schema = self.schema.nominal_declaration_schema(declaration)?;
                if schema.declaration != declaration {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "original nominal predicate owner changed",
                    ));
                }
                (schema.env, schema.predicates)
            }
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "original predicate declaration kind changed",
                ));
            }
        };
        if !self.same_env(&info.env, &env)? {
            return Err(FactsError::invalid(
                &self.budget,
                "original predicate binder changed",
            ));
        }
        for predicate in predicates.iter() {
            self.validate_predicate_metadata(predicate, &env)?;
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        Ok(GenericSchema {
            env,
            signature: Signature {
                ref_contract: Arc::new(ir::RefContract::default()),
                params: Vec::new(),
                result: Ty::Unit,
            },
            predicates,
        })
    }

    /// Reprove source obligations in the original caller, never in the impl's
    /// assumed environment. Only a canonical NotProven is negative evidence.
    pub(in crate::compiler) fn prove_original_declaration_predicates(
        &self,
        declaration: DeclId,
        arguments: &[Ty],
        caller: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<bool> {
        self.check_facts(caller, true)?;
        let schema = self.original_predicate_schema(declaration)?;
        if schema.env.total_arity() != arguments.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "original predicate argument arity changed",
            ));
        }
        charged(
            &self.budget,
            schema
                .env
                .validate_argument_kinds(arguments, Some(caller.env()), &self.budget),
        )?;
        let mut replays = NominalReplays::default();
        for (parameter, argument) in schema.env.ordered_params().zip(arguments) {
            if matches!(
                schema.env.parameter_kind(parameter),
                Some(ir::GenericParamKind::Const(_))
            ) {
                continue;
            }
            self.validate_ty_metadata_with_facts(
                argument,
                caller.env(),
                0,
                &mut replays,
                Some(caller),
            )?;
        }
        let instantiated = self.schema.instantiate_schema_with_facts_scratch(
            &schema,
            arguments,
            caller,
            &self.budget,
            scratch,
        )?;
        self.validate_predicates_with_facts(
            &instantiated.predicates,
            caller.env(),
            None,
            &mut replays,
            Some(caller),
        )?;
        for predicate in instantiated.predicates.iter() {
            if matches!(
                self.prove_goal_with(predicate, caller, scratch)?,
                ProofOutcome::NotProven
            ) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
