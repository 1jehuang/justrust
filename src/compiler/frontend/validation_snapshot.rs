//! Capability for one exact, immutable, fully name-validated AST allocation.
//!
//! There is deliberately no caller-provided token, owner-only comparison,
//! mutable accessor, or owned extraction. Cloning an external Arc and mutating
//! it with copy-on-write creates a different allocation and needs validation.
use super::*;

#[derive(Debug)]
pub(in crate::compiler) struct ValidatedProgram {
    shared: Arc<Program>,
}
impl crate::engine::Fact for ValidatedProgram {}

impl ValidatedProgram {
    pub(in crate::compiler) fn new(shared: Arc<Program>) -> Result<Self, String> {
        shared.validate_name_handles()?;
        Ok(Self { shared })
    }

    pub(in crate::compiler) fn program(&self) -> &Program {
        self.shared.as_ref()
    }

    // Only the frontend constructor can borrow the retained Arc. It cannot
    // replace or mutate the snapshot. Its registry keeps an additional strong
    // reference, so outside Arc::make_mut calls cannot change that allocation.
    pub(super) fn shared(&self) -> &Arc<Program> {
        &self.shared
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_snapshot_does_not_authorize_same_owner_mutated_clone() {
        let original = Arc::new(parse("fn unused(){}fn main(){}").unwrap());
        let sealed = ValidatedProgram::new(original.clone()).unwrap();
        for site in 0..2 {
            let mut shared = original.clone();
            let changed = Arc::make_mut(&mut shared);
            if site == 0 {
                changed.functions[0].module = changed.modules.core_root();
            } else {
                changed.functions[0].name = changed.functions[1].name.clone();
            }
            assert!(Arc::ptr_eq(&sealed.program().names, &shared.names));
            assert!(Arc::ptr_eq(&sealed.program().modules, &shared.modules));
            assert!(!Arc::ptr_eq(sealed.shared(), &shared));
            if site == 0 {
                assert!(ValidatedProgram::new(shared.clone()).is_err());
            } else {
                // This same-store path is name-valid. Duplicate declarations
                // remain a semantic error, not a name-handle contract failure.
                let changed_seal = ValidatedProgram::new(shared.clone()).unwrap();
                assert!(Arc::ptr_eq(changed_seal.shared(), &shared));
                assert!(!Arc::ptr_eq(changed_seal.shared(), sealed.shared()));
            }
            assert!(PlanCtx::from_program(shared.as_ref(), &ir::PlanBudget::default()).is_err());
            assert!(PlanCtx::from_shared_program(shared, &ir::PlanBudget::default()).is_err());
            sealed.program().validate_name_handles().unwrap();
        }
        assert!(Arc::ptr_eq(&original, sealed.shared()));
    }

    #[test]
    fn resolver_seal_retains_the_exact_final_snapshot_for_signatures() {
        let parsed = Arc::new(parse_unresolved("fn unused(){}fn main(){}").unwrap());
        let namespace = prepare_resolution_owned(parsed).unwrap();
        let items = (0..resolution_modules(&namespace).len())
            .map(|ordinal| resolve_module_items(&namespace, ordinal).unwrap())
            .collect();
        let (program, sealed) = finish_resolution_sealed(&namespace, items).unwrap();
        assert!(Arc::ptr_eq(&program, sealed.shared()));
        let budget = ir::PlanBudget::default();
        let owner = |domain| {
            crate::compiler::owner_path::standalone_issued(
                &crate::compiler::expansion_budget::ExpansionBudget::default(),
                domain,
            )
            .unwrap()
        };
        let result = signature_graph::immediate(PlanCtx::from_validated_program_with_signatures(
            &sealed,
            &budget,
            false,
            owner(crate::compiler::owner_path::OwnerDomain::DeclarationSnapshot),
            owner(crate::compiler::owner_path::OwnerDomain::TraitWorld),
            |inputs| std::future::ready(signature_graph::compute_standalone(inputs, &budget)),
        ));
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn malformed_input_precedes_deferred_equality_boundary() {
        let source = "trait Call<A>{type Out;}trait Apply:Call<i32,Out=i32>{}fn main(){}";
        let boundary = parse(source).unwrap_err();
        assert!(boundary.contains("associated equalities"), "{boundary}");
        let mut parsed = parse_unresolved(source).unwrap();
        parsed.program.functions[0].module = parsed.program.modules.core_root();
        let error = prepare_resolution_owned(Arc::new(parsed)).unwrap_err();
        assert!(!error.starts_with("unsupported:"), "{error}");
    }

    #[test]
    fn public_resolution_revalidates_same_owner_mutated_ast() {
        let mut parsed = parse_unresolved("fn unused(){}fn main(){}").unwrap();
        let original = Arc::new(parsed.program.clone());
        let sealed = ValidatedProgram::new(original).unwrap();
        parsed.program.functions[0].module = parsed.program.modules.core_root();
        assert!(prepare_resolution_owned(Arc::new(parsed)).is_err());
        sealed.program().validate_name_handles().unwrap();
    }
}
