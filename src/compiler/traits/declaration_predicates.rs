//! Declaration bounds describe logical type applications, not executable storage.
use super::*;

impl ValidatedTraitWorld {
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
