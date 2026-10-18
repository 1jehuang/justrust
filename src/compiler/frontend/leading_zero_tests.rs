//! Exact integer-method typing and authoritative source replay controls.
use super::*;

fn publication(
    source: &str,
    index: usize,
) -> Result<(ir::PlanBudget, Arc<sr::CanonicalSourcePublication>), String> {
    let program = parse(&format!("{source}\nfn main() {{}}"))?;
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget)?;
    let declaration = plan.function_decl(index)?;
    match plan.publish_callable_body(declaration, &budget) {
        Ok(PublishedCallableBody::Source(body)) => Ok((budget, body)),
        Ok(_) => Err("expected original source".into()),
        Err(error) => Err(error.into_legacy_diagnostic(&budget)),
    }
}

fn invalid<T>(result: Result<T, FactsError>) {
    match result {
        Err(error) => assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata),
        Ok(_) => panic!("forged leading-zero replay accepted"),
    }
}

fn replay(
    original: &OriginalSource,
    body: &sr::CanonicalSourcePublication,
    budget: &ir::PlanBudget,
) -> Result<Vec<sr::NarrowValidatedCall>, FactsError> {
    replay_source_body(original, body.source(), body.facts(), budget)
}

#[test]
fn exact_integer_receivers_return_u32() {
    for ty in [
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
    ] {
        let source = format!("fn f(x: {ty}) -> u32 {{ x.leading_zeros() }}");
        let (budget, body) = publication(&source, 0).unwrap();
        let original = body.original().source_for_test();
        assert_eq!(original.builtin_leading_zeros_mapping.len(), 1, "{ty}");
        assert!(original.call_mapping.is_empty());
        let row = original.builtin_leading_zeros_mapping[0];
        let result = &body.source().exprs[row.expr.0];
        let receiver = &body.source().exprs[row.receiver.0];
        assert_eq!(result.ty, ir::Ty::Int(ir::IntTy::U32));
        assert_eq!(result.completion, receiver.completion);
        assert!(matches!(receiver.ty, ir::Ty::Int(_)));
        assert!(
            matches!(result.kind, ir::ExprKind::LeadingZeros { value } if value == row.receiver)
        );
        replay(&original, &body, &budget).unwrap();
    }
}

#[test]
fn estimator_binary_receiver_and_nested_methods() {
    for source in [
        "fn f(mant: u64) -> u32 { (mant - 1).leading_zeros() }",
        "fn f(x: u64) -> u32 { x.leading_zeros().leading_zeros() }",
        "fn f(x: u64) -> u32 { (x.leading_zeros() + x.leading_zeros()).leading_zeros() }",
        "fn f(x: u64) -> u32 { (x.leading_zeros() + 1).leading_zeros().leading_zeros() }",
    ] {
        let (budget, body) = publication(source, 0).unwrap();
        replay(&body.original().source_for_test(), &body, &budget).unwrap();
    }
}

#[test]
fn receiver_is_lowered_once() {
    let (budget, body) = publication(
        "fn get() -> u64 { 7 } fn f() -> u32 { get().leading_zeros() }",
        1,
    )
    .unwrap();
    let original = body.original().source_for_test();
    assert_eq!(original.call_mapping.len(), 1);
    assert_eq!(original.builtin_leading_zeros_mapping.len(), 1);
    assert_eq!(
        body.source()
            .exprs
            .iter()
            .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
            .count(),
        1
    );
    let row = original.builtin_leading_zeros_mapping[0];
    assert_eq!(row.receiver, original.call_mapping[0].1);
    replay(&original, &body, &budget).unwrap();
}

#[test]
fn divergent_integer_receiver_keeps_actual_type_and_completion() {
    let (budget, body) = publication(
        "fn f() -> u32 { ({ return 7; 1u64 } as u64).leading_zeros() }",
        0,
    )
    .unwrap();
    let original = body.original().source_for_test();
    let row = original.builtin_leading_zeros_mapping[0];
    assert_eq!(
        body.source().exprs[row.receiver.0].ty,
        ir::Ty::Int(ir::IntTy::U64)
    );
    assert_eq!(
        body.source().exprs[row.receiver.0].completion,
        ir::Completion::Never
    );
    assert_eq!(
        body.source().exprs[row.expr.0].ty,
        ir::Ty::Int(ir::IntTy::U32)
    );
    assert_eq!(
        body.source().exprs[row.expr.0].completion,
        ir::Completion::Never
    );
    replay(&original, &body, &budget).unwrap();
}

#[test]
fn mixed_slice_length_and_integer_intrinsic_keep_separate_authority() {
    let (budget, body) =
        publication("fn f(x: &[u8]) -> u32 { x.len().leading_zeros() }", 0).unwrap();
    let original = body.original().source_for_test();
    assert_eq!(original.builtin_len_mapping.len(), 1);
    assert_eq!(original.builtin_leading_zeros_mapping.len(), 1);
    assert!(original.call_mapping.is_empty());
    replay(&original, &body, &budget).unwrap();
}

#[test]
fn constant_and_dynamic_intrinsic_shift_counts() {
    for source in [
        "fn f() -> u8 { 1u8 << 32u8.leading_zeros() }",
        "fn f(x: u8) -> u8 { 1u8 << x.leading_zeros() }",
    ] {
        publication(source, 0).unwrap();
    }
    // Eight is outside a u8 shift width, including through the new unary node.
    assert!(publication("fn f() -> u8 { 1u8 << 0u8.leading_zeros() }", 0).is_err());
}

#[test]
fn wrong_receivers_and_arguments_do_not_acquire_builtin_authority() {
    for source in [
        "fn f(x: &u32) -> u32 { x.leading_zeros() }",
        "fn f(x: &mut u32) -> u32 { x.leading_zeros() }",
        "fn f<T>(x: T) -> u32 { x.leading_zeros() }",
        "fn f(x: f32) -> u32 { x.leading_zeros() }",
        "fn f(x: f64) -> u32 { x.leading_zeros() }",
        "fn f(x: bool) -> u32 { x.leading_zeros() }",
        "fn f(x: char) -> u32 { x.leading_zeros() }",
        "fn f(x: u32) -> u32 { x.leading_zeros(1) }",
        "fn f(x: u32) -> u32 { x.leading_zeros::<u8>() }",
        "fn f() -> u32 { 1.leading_zeros() }",
        "fn f(x: u32) -> u64 { x.leading_zeros() }",
        "fn f(x: f32) { return; x.leading_zeros(); }",
        "fn f(x: u32) { return; let y: bool = x.leading_zeros(); }",
    ] {
        assert!(
            publication(source, 0).is_err(),
            "unexpected admission: {source}"
        );
    }
}

#[test]
fn ordinary_nominal_method_is_not_hijacked() {
    let source = "struct S { v: u64 } impl S { fn leading_zeros(&self) -> u64 { self.v } }
        fn f(s: &S) -> u64 { s.leading_zeros() }";
    let (budget, body) = publication(source, 0).unwrap();
    let original = body.original().source_for_test();
    assert!(original.builtin_leading_zeros_mapping.is_empty());
    assert_eq!(original.call_mapping.len(), 1);
    replay(&original, &body, &budget).unwrap();
    let mut forged = original.clone();
    let (source_index, expr, _) = &original.call_mapping[0];
    forged
        .builtin_leading_zeros_mapping
        .push(BuiltinLeadingZerosOccurrence {
            source_index: *source_index,
            expr: *expr,
            receiver: *expr,
        });
    invalid(replay(&forged, &body, &budget));
}

#[test]
fn ordinary_owned_and_generic_methods_keep_their_own_signature() {
    for source in [
        "struct S { v: u64 } impl S { fn leading_zeros(self) -> u64 { self.v } }
            fn f(s: S) -> u64 { s.leading_zeros() }",
        "struct S { v: u64 } impl S { fn leading_zeros<T>(self, x: T) -> T { x } }
            fn f(s: S) -> bool { s.leading_zeros::<bool>(true) }",
    ] {
        let (budget, body) = publication(source, 0).unwrap();
        let original = body.original().source_for_test();
        assert!(original.builtin_leading_zeros_mapping.is_empty());
        assert_eq!(original.call_mapping.len(), 1);
        replay(&original, &body, &budget).unwrap();
    }
}

#[test]
fn replay_rejects_missing_duplicate_reordered_and_forged_source_rows() {
    for source in [
        "fn f(a: u64) -> u32 { a.leading_zeros() }",
        "fn f(a: u64) { return; let n = a.leading_zeros(); }",
    ] {
        let (budget, body) = publication(source, 0).unwrap();
        let mut original = body.original().source_for_test();
        original.builtin_leading_zeros_mapping.clear();
        invalid(replay(&original, &body, &budget));
    }
    let (budget, body) = publication(
        "fn f(a: u64, b: u64) -> u32 { a.leading_zeros() + b.leading_zeros() }",
        0,
    )
    .unwrap();
    let original = body.original().source_for_test();
    for mutation in 0..6 {
        let mut forged = original.clone();
        match mutation {
            0 => forged
                .builtin_leading_zeros_mapping
                .push(forged.builtin_leading_zeros_mapping[0]),
            1 => forged.builtin_leading_zeros_mapping.swap(0, 1),
            2 => {
                let first = forged.builtin_leading_zeros_mapping[0].source_index;
                forged.builtin_leading_zeros_mapping[0].source_index =
                    forged.builtin_leading_zeros_mapping[1].source_index;
                forged.builtin_leading_zeros_mapping[1].source_index = first;
            }
            3 => forged.builtin_leading_zeros_mapping[0].source_index = usize::MAX,
            4 => {
                let index = forged.builtin_leading_zeros_mapping[0].source_index;
                forged.calls[index].leading_zeros_candidate = false;
            }
            _ => {
                forged.builtin_leading_zeros_mapping[0].receiver =
                    forged.builtin_leading_zeros_mapping[1].receiver
            }
        }
        invalid(replay(&forged, &body, &budget));
    }
}

#[test]
fn replay_rejects_forged_operation_receiver_result_completion_and_orphans() {
    let (budget, body) = publication(
        "fn f(a: u64, b: u64) -> u32 { a.leading_zeros() + b.leading_zeros() }",
        0,
    )
    .unwrap();
    let original = body.original().source_for_test();
    let row = original.builtin_leading_zeros_mapping[0];
    for mutation in 0..7 {
        let mut payload = body.source().as_ref().clone();
        match mutation {
            0 => {
                payload.exprs[row.expr.0].kind = ir::ExprKind::Int(ir::IntValue {
                    ty: ir::IntTy::U32,
                    bits: 0,
                })
            }
            1 => {
                payload.exprs[row.expr.0].kind = ir::ExprKind::LeadingZeros {
                    value: original.builtin_leading_zeros_mapping[1].receiver,
                }
            }
            2 => payload.exprs[row.expr.0].ty = ir::Ty::Int(ir::IntTy::U64),
            3 => payload.exprs[row.receiver.0].ty = ir::Ty::Bool,
            4 => payload.exprs[row.expr.0].completion = ir::Completion::Never,
            5 => payload.exprs.push(payload.exprs[row.expr.0].clone()),
            _ => {
                payload.exprs[row.expr.0].kind = ir::ExprKind::LeadingZeros {
                    value: ir::ExprId(usize::MAX),
                }
            }
        }
        invalid(replay_source_body(
            &original,
            &Arc::new(payload),
            body.facts(),
            &budget,
        ));
    }
}

#[test]
fn replay_budget_authority_and_exhaustion() {
    let (budget, body) = publication("fn f(x: u64) -> u32 { x.leading_zeros() }", 0).unwrap();
    let original = body.original().source_for_test();
    invalid(replay(&original, &body, &ir::PlanBudget::default()));
    for bit in (0..usize::BITS).rev() {
        let _ = budget.charge_work(1usize << bit);
    }
    let error = match replay(&original, &body, &budget) {
        Err(error) => error,
        Ok(_) => panic!("exhausted replay admitted"),
    };
    assert_eq!(error.kind(), tr::FactsErrorKind::Resource);
}

#[test]
fn divergent_literal_block_receiver_matches_cli_regression() {
    let source = "fn f()->u32{({return 7u32;1u64}).leading_zeros()} fn main(){if f()==7u32{println!(\"ok\");}}";
    let program = parse(source).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for index in 0..program.functions.len() {
        let declaration = plan.function_decl(index).unwrap();
        let body = match plan.publish_callable_body(declaration, &budget).unwrap() {
            PublishedCallableBody::Source(body) => body,
            _ => panic!("expected original source"),
        };
        body.validate_source(body.source(), body.facts(), &budget)
            .unwrap();
        if index == 0 {
            let original = body.original().source_for_test();
            let row = original.builtin_leading_zeros_mapping[0];
            assert_eq!(
                body.source().exprs[row.receiver.0].ty,
                ir::Ty::Int(ir::IntTy::U64)
            );
            assert_eq!(
                body.source().exprs[row.expr.0].completion,
                ir::Completion::Never
            );
            replay(&original, &body, &budget).unwrap();
        }
    }
}

#[test]
fn sealed_publication_rejects_in_place_receiver_mutation_and_sibling_swap() {
    let (budget, body) = publication(
        "fn f(a: u64, b: u64) -> u32 { a.leading_zeros() + b.leading_zeros() }",
        0,
    )
    .unwrap();
    let original = body.original().source_for_test();
    let a = original.builtin_leading_zeros_mapping[0].receiver;
    let b = original.builtin_leading_zeros_mapping[1].receiver;
    body.validate_source(body.source(), body.facts(), &budget)
        .unwrap();

    for swap_both in [false, true] {
        // Preserve receiver IDs, types and completions, changing only contents.
        // Arc::make_mut must detach from the immutable published body, and the
        // real consumer boundary must reject that detached checked-body Arc.
        let mut forged = body.source().clone();
        let payload = Arc::make_mut(&mut forged);
        let first = payload.exprs[a.0].kind.clone();
        payload.exprs[a.0].kind = payload.exprs[b.0].kind.clone();
        if swap_both {
            payload.exprs[b.0].kind = first;
        }
        invalid(body.validate_source(&forged, body.facts(), &budget));
        body.validate_source(body.source(), body.facts(), &budget)
            .unwrap();
    }
}
