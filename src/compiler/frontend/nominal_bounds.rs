//! Bounds declared on structs and enums (`struct S<T: Ord>`, `where T: Ord`).
//! Constructors and function signatures prove instantiated obligations against
//! the caller's authoritative predicates, including nested nominal actuals and
//! reference pointees. Missing implementations remain E0277, even in unused
//! function signatures.
use super::*;

impl type_check::Checker<'_> {
    /// Prove the declared bounds of `ty`'s nominal declaration at its type args.
    pub(super) fn prove_nominal_bounds(&self, ty: &ir::Ty) -> Result<(), String> {
        self.prove_nominal_bounds_at_depth(ty, 0)
    }
    fn prove_nominal_bounds_at_depth(&self, ty: &ir::Ty, depth: usize) -> Result<(), String> {
        if let Some(plan) = &self.signatures.plan {
            plan.budget.check_depth(depth)?;
            plan.budget.charge_work(1)?;
        }
        if ty.is_nominal_back_edge() {
            return Ok(());
        }
        match ty {
            ir::Ty::Ref { referent, .. }
            | ir::Ty::RawPtr {
                pointee: referent, ..
            }
            | ir::Ty::Array {
                element: referent, ..
            }
            | ir::Ty::Slice { element: referent } => {
                return self.prove_nominal_bounds_at_depth(referent, depth + 1);
            }
            ir::Ty::Tuple(parts) => {
                for part in parts.iter() {
                    self.prove_nominal_bounds_at_depth(part, depth + 1)?;
                }
                return Ok(());
            }
            _ => {}
        }
        let (source_decl, args) = match ty {
            ir::Ty::Struct(t) => (t.def().source_decl, t.def().type_args.clone()),
            ir::Ty::Enum(t) => (t.def().source_decl, t.def().type_args.clone()),
            _ => return Ok(()),
        };
        for actual in args.iter() {
            self.prove_nominal_bounds_at_depth(actual, depth + 1)?;
        }
        if args.is_empty() {
            return Ok(());
        }
        let schema = self.nominal_source(source_decl)?;
        let (params, bounds) = match schema.as_ref() {
            NominalSchema::Struct(d) => (&d.type_params, &d.bounds),
            NominalSchema::Enum(d) => (&d.type_params, &d.bounds),
        };
        // Implicit `Sized` on every parameter not declared `?Sized`. Concrete
        // unsized actuals are never Sized; rigid parameters must be proven.
        let plan = self.signatures.plan()?;
        let facts = self
            .signatures
            .facts
            .as_deref()
            .ok_or("nominal bounds require validated facts")?;
        let binder = plan.declaration(source_decl)?.param_env.clone();
        let sized = *plan
            .registry
            .traits
            .get(&TraitLookupKey::Builtin(BuiltinTrait::Sized))
            .ok_or("missing canonical Sized identity")?;
        for (index, actual) in args.iter().enumerate() {
            plan.budget.charge_work(1)?;
            let parameter = ir::ParamId {
                owner: binder.binder(),
                index: u16::try_from(index).map_err(|_| "nominal arity overflow")?,
            };
            if binder.relaxed_sized(parameter)
                || !matches!(
                    actual,
                    ir::Ty::Param(_) | ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_)
                )
            {
                continue;
            }
            let goal = ir::Predicate::Implements(ir::TraitRef {
                declaration: sized,
                self_ty: actual.clone(),
                args: Arc::from([]),
            });
            let proved = matches!(actual, ir::Ty::Param(_))
                && matches!(
                    facts
                        .service()
                        .prove_goal(&goal, facts)
                        .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?,
                    super::super::traits::ProofOutcome::Proved(_)
                );
            if !proved {
                return Err(format!(
                    "E0277: the size for values of type `{actual:?}` cannot be known at compilation time (required by {})",
                    plan.declaration(source_decl)?.name,
                ));
            }
        }
        if bounds.is_empty() {
            return Ok(());
        }
        if params.len() != args.len() {
            return Err("contract: nominal bound arity mismatch".into());
        }
        let mut env = self.signatures.clone();
        for (param, actual) in params.iter().zip(args.iter()) {
            env.params
                .insert(plan.registry.parameter_key(*param)?, actual.clone());
        }
        for bound in bounds {
            let mut inspect = |node: &DepthNode<'_>, _| {
                if matches!(node, DepthNode::Type(Type::Projection(_))) {
                    return Err(
                        "unsupported: projected subjects in nominal declaration bounds".into(),
                    );
                }
                Ok(())
            };
            walk_depth_inspecting(
                DepthNode::Type(&bound.subject),
                Some(&plan.budget),
                None,
                Some(&mut inspect),
            )?;
            let owner = type_check::checked_ty(&bound.subject, &env)?;
            for path in &bound.traits {
                plan.budget.charge_work(1)?;
                if matches!(path, TraitPath::Builtin(BuiltinTrait::Sized)) {
                    continue;
                }
                let trait_ref =
                    plan.resolve_trait_use(&owner, path, &env, self.facts(), &plan.budget)?;
                let goal = ir::Predicate::Implements(trait_ref);
                match facts
                    .service()
                    .prove_goal(&goal, facts)
                    .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
                {
                    super::super::traits::ProofOutcome::Proved(_) => {}
                    super::super::traits::ProofOutcome::NotProven => {
                        let ir::Predicate::Implements(trait_ref) = &goal else {
                            unreachable!()
                        };
                        return Err(format!(
                            "E0277: the trait bound `{owner:?}: {}` is not satisfied (required by a bound on {})",
                            plan.declaration(trait_ref.declaration)?.name,
                            plan.declaration(source_decl)?.name,
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
