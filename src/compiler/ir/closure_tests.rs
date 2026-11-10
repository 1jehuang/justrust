use super::*;

fn closure(captures: Vec<Ty>) -> ClosureTy {
    let snapshot = SnapshotId::standalone().unwrap();
    ClosureTy::new(
        DeclId { snapshot, index: 0 },
        Arc::from([Ty::Int(IntTy::I32)]),
        2,
        DeclId { snapshot, index: 1 },
        Arc::from([Ty::Int(IntTy::I32)]),
        Ty::Bool,
        captures.into(),
        ClosureKind::Fn,
    )
}

#[test]
fn canonical_identity_retains_parent_instantiation_and_site() {
    let original = closure(vec![]);
    for changed in [
        ClosureTy {
            site: original.site + 1,
            ..original.clone()
        },
        ClosureTy {
            owner_args: Arc::from([Ty::Bool]),
            ..original.clone()
        },
        ClosureTy {
            params: Arc::from([Ty::Bool]),
            ..original.clone()
        },
        ClosureTy {
            result: Arc::new(Ty::Unit),
            ..original.clone()
        },
        ClosureTy {
            captures: Arc::from([Ty::Bool]),
            ..original.clone()
        },
        ClosureTy {
            kind: ClosureKind::FnOnce,
            ..original.clone()
        },
    ] {
        assert_ne!(original, changed);
        assert!(
            !original
                .try_eq(&changed, |a, b| Ok::<_, String>(a == b))
                .unwrap()
        );
    }
    assert!(
        original
            .try_eq(&original.clone(), |a, b| Ok::<_, String>(a == b))
            .unwrap()
    );
}

#[test]
fn callable_capabilities_are_not_interchangeable() {
    let mut value = closure(vec![]);
    assert!(value.supports(ClosureKind::Fn));
    assert!(value.supports(ClosureKind::FnMut));
    assert!(value.supports(ClosureKind::FnOnce));
    value.kind = ClosureKind::FnMut;
    assert!(!value.supports(ClosureKind::Fn));
    assert!(value.supports(ClosureKind::FnMut));
    assert!(value.supports(ClosureKind::FnOnce));
    value.kind = ClosureKind::FnOnce;
    assert!(!value.supports(ClosureKind::Fn));
    assert!(!value.supports(ClosureKind::FnMut));
    assert!(value.supports(ClosureKind::FnOnce));
}

#[test]
fn substitution_separates_ordered_owner_actuals_from_storage_children() {
    struct Roles {
        owners: Vec<Ty>,
    }
    impl SubstitutionCx for Roles {
        fn nominal_application(&self, _: NominalAppId) -> Result<NominalAppInfo, String> {
            Err("unexpected nominal identity lookup".into())
        }
        fn intern_nominal(
            &mut self,
            _: DeclId,
            _: &[Ty],
            _: FactsCtx<'_>,
            _: &PlanBudget,
        ) -> Result<Ty, String> {
            Err("unexpected nominal issuer".into())
        }
        fn instantiate_signature(
            &mut self,
            _: &CallTarget,
            _: FactsCtx<'_>,
            _: &PlanBudget,
        ) -> Result<Signature, String> {
            Err("unexpected callable issuer".into())
        }
        fn substitute_closure_owner_actual(
            &mut self,
            ty: &Ty,
            env: &GenericEnv,
            args: &[Ty],
            caller: FactsCtx<'_>,
            budget: &PlanBudget,
            depth: usize,
        ) -> Result<Ty, String> {
            self.owners.push(ty.clone());
            substitute_closure_owner_actual_with_depth(ty, env, args, caller, self, budget, depth)
        }
    }
    let budget = PlanBudget::default();
    let env = GenericEnv::new(&[], &budget).unwrap();
    let mut original = closure(vec![Ty::Char]);
    original.params = Arc::from([Ty::Int(IntTy::U8)]);
    let mut cx = Roles { owners: Vec::new() };
    let mapped = substitute_ty_walk(
        &Ty::Closure(Arc::new(original.clone())),
        &env,
        &[],
        FactsCtx::Concrete,
        &mut cx,
        &budget,
        0,
    )
    .unwrap();
    assert_eq!(cx.owners, original.owner_args());
    assert_eq!(mapped, Ty::Closure(Arc::new(original)));
    assert!(
        realize_parent_local_substitution(&Ty::Bool, FactsCtx::Concrete, &mut cx, &budget,)
            .is_err()
    );
    assert!(
        substitute_closure_owner_actual_with_depth(
            &Ty::Bool,
            &env,
            &[],
            FactsCtx::Concrete,
            &mut cx,
            &budget,
            budget.limits().depth + 1,
        )
        .is_err()
    );
}

#[test]
fn only_capture_fields_are_runtime_carriers() {
    let shared = Ty::Ref {
        kind: BorrowKind::Shared,
        referent: Arc::new(Ty::Bool),
    };
    let mut value = closure(vec![Ty::Int(IntTy::I32), shared.clone()]);
    // A reference in the signature is not a reference stored in the environment.
    value.params = Arc::from([shared.clone()]);
    value.result = Arc::new(shared);
    let ty = Ty::Closure(Arc::new(value));
    let budget = PlanBudget::default();
    let locations = carrier_locations(&ty, &budget).unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].path.0.as_ref(), &[CarrierStep::Field(1)]);
    assert_eq!(ty.field_count(), Some(2));
    assert!(ty.is_copy());
}

#[test]
fn mutable_capture_is_not_copy_even_inside_a_product() {
    let value = closure(vec![Ty::Ref {
        kind: BorrowKind::Mut,
        referent: Arc::new(Ty::Bool),
    }]);
    let ty = Ty::Closure(Arc::new(value));
    assert!(!ty.is_copy());
    assert!(!Ty::Tuple(Arc::from([ty])).is_copy());
}

#[test]
fn metadata_mapping_is_immutable_and_visits_signature_and_owner_arguments() {
    let original = closure(vec![Ty::Int(IntTy::I32)]);
    let mapped = original
        .try_map(|ty| {
            Ok::<_, String>(if ty == &Ty::Int(IntTy::I32) {
                Ty::Bool
            } else {
                ty.clone()
            })
        })
        .unwrap();
    assert_eq!(original.owner_args(), &[Ty::Int(IntTy::I32)]);
    assert_eq!(mapped.owner_args(), &[Ty::Bool]);
    assert_eq!(mapped.params(), &[Ty::Bool]);
    assert_eq!(mapped.captures(), &[Ty::Bool]);
    assert_eq!(mapped.owner(), original.owner());
    assert_eq!(mapped.provider(), original.provider());
    assert_eq!(mapped.site(), original.site());
}
