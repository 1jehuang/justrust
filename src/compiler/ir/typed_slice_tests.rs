//! Private authority controls for typed shared slices. These do not assert native layout.
use super::*;
use crate::compiler::frontend;

fn shared(referent: Ty) -> Ty {
    Ty::Ref {
        kind: BorrowKind::Shared,
        referent: Arc::new(referent),
    }
}

#[test]
fn slice_data_pointer_rejects_forged_type_operand_completion_and_origin() {
    use crate::compiler::cfg;
    let program = frontend::parse(
        "fn data(s:&mut[i32])->*mut i32{__justrust_slice_as_mut_ptr!(s)}fn main(){}",
    )
    .unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
    let function = frontend::type_function_with_plan(&program, 0, &plan, &budget).unwrap();
    function
        .validate_with_facts(FactsCtx::from_validated(&facts))
        .unwrap();
    let index = function
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ExprKind::SliceDataPointer { .. }))
        .unwrap();
    for mode in 0..4 {
        let mut forged = function.clone();
        match mode {
            0 => {
                forged.exprs[index].ty = Ty::RawPtr {
                    kind: BorrowKind::Shared,
                    pointee: Arc::new(Ty::Int(IntTy::I32)),
                }
            }
            1 => {
                forged.exprs[index].ty = Ty::RawPtr {
                    kind: BorrowKind::Mut,
                    pointee: Arc::new(Ty::Int(IntTy::U32)),
                }
            }
            2 => forged.exprs[index].completion = Completion::Never,
            _ => {
                forged.exprs[index].kind = ExprKind::SliceDataPointer {
                    slice: ExprId(usize::MAX),
                }
            }
        }
        assert!(
            forged
                .validate_with_facts(FactsCtx::from_validated(&facts))
                .is_err(),
            "typed mode {mode}"
        );
    }
    assert!(slice_data_pointer_ty(&Ty::shared_byte_slice_ty()).is_err());
    assert!(slice_data_pointer_ty(&Ty::Int(IntTy::Usize)).is_err());
    let element = Arc::new(Ty::Int(IntTy::I32));
    let slice = Ty::Ref {
        kind: BorrowKind::Mut,
        referent: Arc::new(Ty::Slice {
            element: Arc::clone(&element),
        }),
    };
    let Ty::RawPtr { pointee, .. } = slice_data_pointer_ty(&slice).unwrap() else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&element, &pointee));

    let template = cfg::lower_template(Arc::new(function), facts.clone(), &budget).unwrap();
    for mode in 0..3 {
        let mut forged = template.function().as_ref().clone();
        if mode == 0 {
            forged.lowering_proof = None;
        }
        let located = forged
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find(|s| {
                matches!(
                    s.value,
                    cfg::Statement::Assign {
                        value: cfg::Rvalue::SliceDataPointer { .. },
                        ..
                    }
                )
            })
            .unwrap();
        if mode == 1 {
            located.origin = None;
        }
        if mode == 2 {
            let cfg::Statement::Assign {
                value: cfg::Rvalue::SliceDataPointer { slice },
                ..
            } = &mut located.value
            else {
                unreachable!()
            };
            *slice = cfg::Operand::Constant(cfg::Constant::Int(IntValue::usize(0)));
        }
        assert!(
            forged
                .validate_with_facts(FactsCtx::from_validated(&facts))
                .is_err(),
            "lowered mode {mode}"
        );
    }
}

#[test]
fn typed_shapes_preserve_exact_element_and_byte_only_helpers() {
    let element = Ty::Tuple(vec![Ty::Int(IntTy::I128), Ty::Bool].into());
    let slice = shared(Ty::Slice {
        element: Arc::new(element.clone()),
    });
    let array = shared(Ty::Array {
        element: Arc::new(element.clone()),
        len: 3,
    });
    assert_eq!(slice.shared_slice_element(), Some(&element));
    assert_eq!(array.shared_array_element_len(), Some((&element, 3)));
    assert!(!slice.shared_byte_slice());
    assert_eq!(array.shared_byte_array_len(), None);
    assert!(Ty::shared_byte_slice_ty().shared_byte_slice());
    let mutable = Ty::Ref {
        kind: BorrowKind::Mut,
        referent: Arc::new(Ty::Slice {
            element: Arc::new(element),
        }),
    };
    assert!(mutable.shared_slice_element().is_none());
}

#[test]
fn shared_slice_carrier_domain_is_typed_and_cannot_be_forged_into_an_array() {
    let budget = PlanBudget::default();
    let element = shared(Ty::Int(IntTy::I32));
    let slice = shared(Ty::Slice {
        element: Arc::new(element.clone()),
    });
    let locations = carrier_locations(&slice, &budget).unwrap();
    assert_eq!(locations.len(), 2);
    assert!(locations[0].path.0.is_empty());
    assert_eq!(
        locations[1].path.0.as_ref(),
        &[CarrierStep::Referent, CarrierStep::SliceElement]
    );
    assert_eq!(
        carrier_at(&slice, &locations[1], &budget).unwrap(),
        &element
    );
    let array = shared(Ty::Array {
        element: Arc::new(element),
        len: 1,
    });
    assert!(carrier_at(&array, &locations[1], &budget).is_err());
    let mut forged = locations[1].clone();
    forged.path = CarrierPath(Arc::from([CarrierStep::SliceElement]));
    assert!(carrier_at(&slice, &forged, &budget).is_err());
    forged.path = locations[1].path.clone();
    forged.channel = CarrierChannel::Fixed(ReferencePosition { depth: 1 });
    assert!(carrier_at(&slice, &forged, &budget).is_err());
    let mutable = Ty::Ref {
        kind: BorrowKind::Mut,
        referent: Arc::new(Ty::Slice {
            element: Arc::new(shared(Ty::Bool)),
        }),
    };
    assert!(
        validate_slice_carrier_with_facts(&mutable, FactsCtx::Concrete)
            .unwrap_err()
            .contains("mutable slices")
    );
}

#[test]
fn metadata_errors_precede_the_reference_element_boundary() {
    let program = frontend::parse("fn opaque<T>(x:T){}fn main(){}").unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
    let parameter = facts.env().ordered_params().next().unwrap();
    let invalid = ParamId {
        owner: parameter.owner,
        index: u16::MAX,
    };
    let element = Ty::Tuple(vec![shared(Ty::Bool), Ty::Param(invalid)].into());
    let authority = FactsCtx::from_validated(&facts);
    let expected = authority
        .validate_value_ty_with_budget(&element, &budget)
        .unwrap_err();
    let actual = validate_shared_slice_element(&element, authority, &budget).unwrap_err();
    assert_eq!(actual, expected);
    assert!(!actual.contains("slice elements containing references"));
    assert!(!budget.has_resource_failure());
}

#[test]
fn universal_element_permission_is_rechecked_at_the_shared_reference_boundary() {
    let program = frontend::parse("fn opaque<T>(x:T){}fn main(){}").unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
    let parameter = Ty::Param(facts.env().ordered_params().next().unwrap());
    validate_shared_slice_element(&parameter, FactsCtx::from_validated(&facts), &budget).unwrap();
    assert!(validate_shared_slice_element(&parameter, FactsCtx::Concrete, &budget).is_err());
    validate_shared_slice_element(&shared(Ty::Bool), FactsCtx::Concrete, &budget).unwrap();
    for concrete in [
        Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(Ty::Bool),
        },
        shared(shared(Ty::Bool)),
        Ty::Tuple(vec![shared(Ty::Bool)].into()),
    ] {
        let error =
            validate_shared_slice_element(&concrete, FactsCtx::Concrete, &budget).unwrap_err();
        assert!(
            error.contains("slice elements containing references"),
            "{error}"
        );
    }
    let concrete = Ty::Tuple(vec![Ty::Int(IntTy::I128), Ty::Bool].into());
    validate_shared_slice_element(&concrete, FactsCtx::Concrete, &budget).unwrap();
    assert!(!budget.has_resource_failure());
}

#[test]
fn both_element_helper_entries_preserve_the_original_failure_latch() {
    let program = frontend::parse("fn main(){}").unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
    assert!(budget.charge_work(usize::MAX).is_err());
    let first = budget.first_resource_failure().unwrap();
    assert!(
        validate_shared_slice_element(&Ty::Unit, FactsCtx::from_validated(&facts), &budget)
            .is_err()
    );
    assert!(
        validate_shared_slice_element_with_facts(&Ty::Unit, FactsCtx::from_validated(&facts))
            .is_err()
    );
    assert_eq!(budget.first_resource_failure(), Some(first));
}

#[test]
fn actual_array_unsizing_requires_exact_elements_not_equal_width() {
    let program =
        frontend::parse("fn count(a:&[i32;2])->usize{let s:&[i32]=a;s.len()}fn main(){}").unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
    let mut function = frontend::type_function_with_plan(&program, 0, &plan, &budget).unwrap();
    function
        .validate_with_facts(FactsCtx::from_validated(&facts))
        .unwrap();
    let operation = function
        .exprs
        .iter_mut()
        .find(|expression| matches!(expression.kind, ExprKind::ArrayToSlice { .. }))
        .unwrap();
    operation.ty = shared(Ty::Slice {
        element: Arc::new(Ty::Int(IntTy::U32)),
    });
    let error = function
        .validate_with_facts(FactsCtx::from_validated(&facts))
        .unwrap_err();
    assert!(
        error.contains("array-to-slice element type mismatch"),
        "{error}"
    );
}

#[test]
fn unused_slice_reads_require_real_copy_evidence_and_exact_result_type() {
    for copy in [false, true] {
        let source = if copy {
            "fn inspect<T:Copy>(s:&[T])->usize{s.len()}fn main(){}"
        } else {
            "fn inspect<T>(s:&[T])->usize{s.len()}fn main(){}"
        };
        let program = frontend::parse(source).unwrap();
        let budget = PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let authority = FactsCtx::from_validated(&facts);
        let element = Ty::Param(facts.env().ordered_params().next().unwrap());
        assert_eq!(authority.must_copy(&element).unwrap(), copy);
        let mut function = frontend::type_function_with_plan(&program, 0, &plan, &budget).unwrap();
        function.validate_with_facts(authority).unwrap();
        let slice = function
            .exprs
            .iter()
            .find_map(|expression| match expression.kind {
                ExprKind::SliceLen { slice } => Some(slice),
                _ => None,
            })
            .unwrap();
        let index = ExprId(function.exprs.len());
        function.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::Usize),
            completion: Completion::Returns,
            kind: ExprKind::Int(IntValue::usize(0)),
        });
        function.exprs.push(TypedExpr {
            ty: element,
            completion: Completion::Returns,
            kind: ExprKind::SliceIndex { slice, index },
        });
        let result = function.validate_with_facts(authority);
        if copy {
            result.unwrap();
            function.exprs.last_mut().unwrap().ty = Ty::Bool;
            let error = function.validate_with_facts(authority).unwrap_err();
            assert!(
                error.contains("slice index result element type mismatch"),
                "{error}"
            );
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("non-Copy element"), "{error}");
        }
        assert!(!budget.has_resource_failure());
    }
}
