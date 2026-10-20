//! Type shape of a call whose canonical callee/signature was already validated.
use super::*;

fn exact(left: &Ty, right: &Ty, facts: FactsCtx<'_>, budget: &PlanBudget) -> Result<bool, String> {
    if let Some(authority) = facts.validated() {
        // Source and graph-realized nominal values can have different storage
        // identities in either phase. Validate both representations before
        // comparing their canonical declaration and ordered type arguments.
        authority
            .validate_ty(left)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        authority
            .validate_ty(right)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        return authority
            .world()
            .types_equal(std::slice::from_ref(left), std::slice::from_ref(right))
            .map_err(|e| e.into_legacy_diagnostic(budget));
    }
    if identical_type(left, right, budget)? {
        return Ok(true);
    }
    ref_types_equal(left, right, budget, 0)
}

impl CheckedFunction {
    pub(super) fn validate_call_value_types(
        &self,
        value: &TypedExpr,
        args: &[ExprId],
        signature: &Signature,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if !exact(&value.ty, &signature.result, facts, budget)? {
            return Err("call result type differs from canonical signature".into());
        }
        if args.len() != signature.params.len() {
            return Err("call value type arity mismatch".into());
        }
        for (id, formal) in args.iter().zip(signature.params.iter()) {
            budget.charge_work(1)?;
            let actual = self.exprs.get(id.0).ok_or("call argument outside arena")?;
            if actual.ty == Ty::Never && actual.completion != Completion::Never {
                return Err("call never argument may not complete".into());
            }
            if !exact(&actual.ty, formal, facts, budget)?
                && !Self::conditional_join_matches(&actual.ty, formal, formal, budget)?
            {
                return Err("call argument type differs from canonical signature".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend;

    #[test]
    fn closed_call_equality_preserves_nominal_and_reference_authority() {
        let source = "struct A;struct B;enum E{One(i32)}enum F{One(i32)}fn types(a:A,b:B,e:E,f:F){}fn main(){}";
        let program = frontend::parse(source).unwrap();
        let budget = PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let named = |name: &str| {
            (0..plan.callable_count())
                .map(|index| plan.callable_decl(index).unwrap())
                .find(|declaration| {
                    matches!(plan.persistent_decl_path(*declaration).unwrap().item(),
                        frontend::PersistentDeclItem::Name(found) if found == name)
                })
                .unwrap()
        };
        let parameters = plan
            .signature(named("types"))
            .unwrap()
            .signature
            .params
            .clone();
        let authority = plan
            .instance_facts(&CallTarget {
                declaration: named("main"),
                type_args: Arc::from([]),
            })
            .unwrap();
        let facts = FactsCtx::from_validated(&authority);
        assert!(exact(&parameters[0], &parameters[0], facts, &budget).unwrap());
        assert!(!exact(&parameters[0], &parameters[1], facts, &budget).unwrap());
        assert!(exact(&parameters[2], &parameters[2], facts, &budget).unwrap());
        assert!(!exact(&parameters[2], &parameters[3], facts, &budget).unwrap());
        let shared = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(parameters[0].clone()),
        };
        let mutable = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(parameters[0].clone()),
        };
        assert!(!exact(&shared, &mutable, facts, &budget).unwrap());
        let ordered = Ty::Tuple(Arc::from([parameters[0].clone(), parameters[1].clone()]));
        let reversed = Ty::Tuple(Arc::from([parameters[1].clone(), parameters[0].clone()]));
        assert!(!exact(&ordered, &reversed, facts, &budget).unwrap());
        let foreign_program = frontend::parse(source).unwrap();
        let foreign_budget = PlanBudget::default();
        let foreign_plan =
            frontend::PlanCtx::from_program(&foreign_program, &foreign_budget).unwrap();
        let foreign = (0..foreign_plan.callable_count())
            .map(|index| foreign_plan.callable_decl(index).unwrap())
            .find(|declaration| {
                matches!(foreign_plan.persistent_decl_path(*declaration).unwrap().item(),
                    frontend::PersistentDeclItem::Name(found) if found == "types")
            })
            .unwrap();
        let foreign_type = foreign_plan.signature(foreign).unwrap().signature.params[0].clone();
        assert!(exact(&parameters[0], &foreign_type, facts, &budget).is_err());
        let foreign_enum = foreign_plan.signature(foreign).unwrap().signature.params[2].clone();
        assert!(exact(&parameters[2], &foreign_enum, facts, &budget).is_err());
    }
}
