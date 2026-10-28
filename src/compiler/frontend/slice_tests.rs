//! Phase-1 shared byte slice (`&[u8]`) frontend tests: parsing, admission,
//! rejection, typed builtin `.len()` occurrence capture and exact replay.
use super::*;

const ADLER: &str = "fn adler2_adapted_update(bytes: &[u8], checksum: u32) -> u32 {
    let mut a: u32 = checksum % 65536;
    let mut b: u32 = checksum / 65536;
    let mut i: usize = 0;
    while i < bytes.len() {
        a = (a + bytes[i] as u32) % 65521;
        b = (b + a) % 65521;
        i = i + 1;
    }
    b * 65536 + a
}
fn main() {
    let p0: [u8; 4] = [0, 1, 128, 255];
    let mut state: u32 = 1;
    state = adler2_adapted_update(&p0, state);
}";

fn program(source: &str) -> Program {
    let mut program = parse(source).unwrap();
    if !program.functions.iter().any(|f| {
        f.name.check(program.names.as_ref()).unwrap();
        f.name.anchor() == PathAnchor::Crate
            && f.name.len() == 1
            && program.names.spelling(f.name.segments()[0]).unwrap() == "main"
    }) {
        program = parse(&format!("{source}\nfn main(){{}}")).unwrap();
    }
    program
}
fn publish(source: &str, index: usize) -> Result<Arc<sr::CanonicalSourcePublication>, String> {
    let program = program(source);
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget)?;
    let declaration = plan.function_decl(index)?;
    match plan.publish_callable_body(declaration, &budget) {
        Ok(PublishedCallableBody::Source(body)) => Ok(body),
        Ok(_) => Err("expected original source".into()),
        Err(error) => Err(error.into_legacy_diagnostic(&budget)),
    }
}
fn published(source: &str, index: usize) -> (ir::PlanBudget, Arc<sr::CanonicalSourcePublication>) {
    let program = program(source);
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(index).unwrap();
    match plan.publish_callable_body(declaration, &budget) {
        Ok(PublishedCallableBody::Source(body)) => (budget, body),
        Ok(_) => panic!("expected original source"),
        Err(error) => panic!("{error:?}"),
    }
}
fn rejected(source: &str, index: usize) -> String {
    match publish(source, index) {
        Ok(_) => panic!("unexpected admission of {source}"),
        Err(error) => error,
    }
}
fn replay(
    original: &OriginalSource,
    publication: &sr::CanonicalSourcePublication,
    budget: &ir::PlanBudget,
) -> Result<Vec<sr::NarrowValidatedCall>, FactsError> {
    replay_source_body(original, publication.source(), publication.facts(), budget)
}
fn invalid<T>(result: Result<T, FactsError>) {
    match result {
        Err(error) => assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata),
        Ok(_) => panic!("mutated replay accepted"),
    }
}
fn exprs<'a>(
    source: &'a ir::CheckedFunction,
    pick: impl Fn(&ir::ExprKind) -> bool + 'a,
) -> Vec<ir::ExprId> {
    source
        .exprs
        .iter()
        .enumerate()
        .filter(|(_, e)| pick(&e.kind))
        .map(|(i, _)| ir::ExprId(i))
        .collect()
}

#[test]
fn array_len_rejects_replaced_length_receiver() {
    let (budget, f) = published("fn f(a:[u8;2], b:[u8;2])->usize { a.len() }", 0);
    let original = f.original().source_for_test();
    replay(&original, &f, &budget).unwrap();
    let row = original.builtin_len_mapping[0];
    let mut replaced = f.source().as_ref().clone();
    replaced.exprs[row.expr.0].kind = ir::ExprKind::Int(ir::IntValue::usize(2));
    invalid(replay_source_body(
        &original,
        &Arc::new(replaced),
        f.facts(),
        &budget,
    ));
    let mut changed_receiver = f.source().as_ref().clone();
    let borrow = changed_receiver
        .exprs
        .iter_mut()
        .find_map(|e| match &mut e.kind {
            ir::ExprKind::Borrow { place, .. } => Some(place),
            _ => None,
        })
        .expect("array receiver must have a real borrow");
    borrow.local = f.source().params[1];
    assert!(
        f.validate_source(&Arc::new(changed_receiver), f.facts(), &budget)
            .is_err()
    );
    let lowered = super::super::cfg::lower_source_closed(&f, &budget).unwrap();
    let mut altered = lowered.clone();
    let node = altered
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.statements)
        .find(|s| {
            matches!(
                s.value,
                super::super::cfg::Statement::Assign {
                    value: super::super::cfg::Rvalue::ArrayToSlice { .. },
                    ..
                }
            )
        })
        .expect("array receiver must use canonical unsizing");
    node.origin = None;
    assert!(
        altered
            .validate_with_facts(ir::FactsCtx::Validated(f.facts()))
            .is_err()
    );
    let mut missing = original.clone();
    missing.builtin_len_mapping.clear();
    invalid(replay(&missing, &f, &budget));
}

#[test]
fn parses_slice_and_array_types() {
    let p = parse("fn f(a: &[u8], b: [u8; 3], c: &&[u8]) {}").unwrap();
    let f = &p.functions[0];
    assert!(matches!(&f.params[0].1, Type::Ref { referent, .. }
        if matches!(referent.as_ref(), Type::Slice(e) if matches!(e.as_ref(), Type::Int(ir::IntTy::U8)))));
    assert!(matches!(&f.params[1].1, Type::Array(_, 3)));
    assert!(parse("fn f(a: &[u8;]) {}").is_err());
}

#[test]
fn ty_helpers_are_exact() {
    let slice = ir::Ty::shared_byte_slice_ty();
    assert!(slice.shared_byte_slice());
    assert!(slice.contains_slice(&ir::PlanBudget::default()).unwrap());
    assert!(slice.is_copy());
    let array = ir::Ty::Ref {
        kind: ir::BorrowKind::Shared,
        referent: Arc::new(ir::Ty::Array {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
            len: 4,
        }),
    };
    assert_eq!(array.shared_byte_array_len(), Some(4));
    assert!(!array.shared_byte_slice());
    let mutable = ir::Ty::Ref {
        kind: ir::BorrowKind::Mut,
        referent: Arc::new(ir::Ty::Slice {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
        }),
    };
    assert!(!mutable.shared_byte_slice());
    let wide = ir::Ty::Ref {
        kind: ir::BorrowKind::Shared,
        referent: Arc::new(ir::Ty::Slice {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U16)),
        }),
    };
    assert!(!wide.shared_byte_slice());
    assert!(
        !ir::Ty::Slice {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U8))
        }
        .is_copy()
    );
}

#[test]
fn adler_recurrence_types_with_slice_operations() {
    let (budget, update) = published(ADLER, 0);
    let source = update.source();
    assert!(source.locals[source.params[0].0].ty.shared_byte_slice());
    let lens = exprs(source, |k| matches!(k, ir::ExprKind::SliceLen { .. }));
    assert_eq!(lens.len(), 1);
    let ir::ExprKind::SliceLen { slice } = source.exprs[lens[0].0].kind else {
        unreachable!()
    };
    assert!(matches!(source.exprs[slice.0].kind, ir::ExprKind::Local(l) if l == source.params[0]));
    assert_eq!(source.exprs[lens[0].0].ty, ir::Ty::Int(ir::IntTy::Usize));
    let reads = exprs(source, |k| matches!(k, ir::ExprKind::SliceIndex { .. }));
    assert_eq!(reads.len(), 1);
    assert_eq!(source.exprs[reads[0].0].ty, ir::Ty::Int(ir::IntTy::U8));
    // No legacy array indexing or ordinary call masquerades as slice access.
    assert!(
        exprs(source, |k| matches!(
            k,
            ir::ExprKind::DynamicIndex { .. } | ir::ExprKind::Call { .. }
        ))
        .is_empty()
    );
    ir::validate_published_source(&update, &budget).unwrap();
    let original = update.original().source_for_test();
    replay(&original, &update, &budget).unwrap();

    let (budget, main) = published(ADLER, 1);
    let source = main.source();
    let wrappers = exprs(source, |k| matches!(k, ir::ExprKind::ArrayToSlice { .. }));
    assert_eq!(wrappers.len(), 1);
    let ir::ExprKind::ArrayToSlice { reference } = source.exprs[wrappers[0].0].kind else {
        unreachable!()
    };
    assert_eq!(
        source.exprs[reference.0].ty.shared_byte_array_len(),
        Some(4)
    );
    assert!(matches!(
        source.exprs[reference.0].kind,
        ir::ExprKind::Borrow { .. }
    ));
    assert!(source.exprs[wrappers[0].0].ty.shared_byte_slice());
    let calls = exprs(source, |k| matches!(k, ir::ExprKind::Call { .. }));
    let ir::ExprKind::Call { args, .. } = &source.exprs[calls[0].0].kind else {
        unreachable!()
    };
    assert_eq!(args[0], wrappers[0]);
    replay(&main.original().source_for_test(), &main, &budget).unwrap();
}

#[test]
fn constant_slice_index_is_runtime_checked() {
    let (_, f) = published("fn f(b: &[u8]) -> u8 { b[0] }", 0);
    let reads = exprs(f.source(), |k| matches!(k, ir::ExprKind::SliceIndex { .. }));
    assert_eq!(reads.len(), 1);
    assert!(exprs(f.source(), |k| matches!(k, ir::ExprKind::Index { .. })).is_empty());
}

#[test]
fn slice_locals_and_coercion_sites() {
    for source in [
        "fn f() { let s: [u8; 2] = [1, 2]; let b: &[u8] = &s; let n: usize = b.len(); }",
        "fn f() { let s: [u8; 0] = [0u8; 0]; let b: &[u8] = &s; let n = b.len(); }",
        "fn g(b: &[u8]) -> usize { b.len() } fn f() { let s: [u8; 1] = [7]; let r: &[u8; 1] = &s; let n = g(r); }",
        "fn g(b: &[u8]) -> usize { b.len() } fn f(b: &[u8]) -> usize { g(b) }",
        "fn f() { let s: [u8; 2] = [1, 2]; let mut b: &[u8] = &s; b = &s; }",
    ] {
        publish(source, 0).unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}

#[test]
fn typed_shared_slice_parameters_are_admitted() {
    for source in [
        "fn f(b: &[u16]) {}",
        "fn f(b: &[i32]) {}",
        "fn f<T>(b: &[T]) {}",
        "fn f(b: &mut [u8]) {}",
        "fn f<'a>(b: &'a [u8]) -> &'a [u8] { b }",
        "fn f<T>(b: &[u8], t: T) {}",
    ] {
        publish(source, 0).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
}

#[test]
fn rejects_outside_admitted_domain() {
    let invalid_array_assignment = "fn f() { let s: [u8; 1] = [7]; let b: [u8] = s; }";
    let error = rejected(invalid_array_assignment, 0);
    let stock = super::parser_tests::stock_compile(
        &format!("{invalid_array_assignment} fn main() {{}}"),
        std::path::Path::new("unused-slice-metadata"),
        true,
    );
    assert!(!stock.status.success());
    assert!(String::from_utf8_lossy(&stock.stderr).contains("E0308"));
    assert!(
        error.contains("expected") && error.contains("got"),
        "{error}"
    );
    for workers in [1, 8] {
        for check in [true, false] {
            assert!(
                super::super::compile(
                    format!("{invalid_array_assignment} fn main() {{}}"),
                    workers,
                    check
                )
                .is_err()
            );
        }
    }
    for (source, index, needle) in [
        ("fn f(b: [u8]) {}", 0, "bare by-value slice"),
        ("struct S { b: &[u8] } fn f(s: S) {}", 0, ""),
        ("struct S { b: [u8] } fn f(s: S) {}", 0, ""),
    ] {
        let message = match publish(source, index) {
            Ok(_) => panic!("admitted {source}"),
            Err(error) => error,
        };
        assert!(
            message.to_lowercase().contains("unsupported") || needle.is_empty(),
            "{source}: {message}"
        );
        assert!(message.contains(needle), "{source}: {message}");
    }
    // Mutation through or of a slice element is never admitted.
    let message = rejected("fn f(b: &[u8]) { b[0] = 1; }", 0);
    assert!(!message.is_empty());
    // Usize index is required, without i32 backward inference (stock E0277).
    let message = rejected("fn f(b: &[u8], i: i32) -> u8 { b[i] }", 0);
    assert!(
        message.contains("E0277") && message.contains("usize"),
        "{message}"
    );
    // `.len()` with arguments is not the builtin.
    rejected("fn f(b: &[u8]) -> usize { b.len(1) }", 0);
    // Local inference alone does not unsize an array reference.
    let message = rejected(
        "fn g(b: &[u8]) {} fn f() { let s: [u8; 1] = [7]; let r = &s; let x: u8 = r; }",
        1,
    );
    assert!(!message.is_empty());
}

#[test]
fn unused_and_dead_slice_bodies_are_still_validated() {
    let message = rejected(
        "fn unused(bytes: &[u8]) -> u8 { missing_byte + bytes[0] }",
        0,
    );
    assert!(message.contains("missing_byte"), "{message}");
    let message = rejected(
        "fn unused(bytes: &[u8]) { return; let n: bool = bytes.len(); }",
        0,
    );
    assert!(!message.contains("unsupported"), "{message}");
}

// Replay obligation case 1: one typed builtin per source occurrence, exact
// receiver, zero arity, usize type and source order.
#[test]
fn replay_valid_builtin_len_occurrences() {
    let (budget, f) = published(
        "fn f(a: &[u8], b: &[u8]) -> usize { let x = a.len(); let y = b.len(); x + y }",
        0,
    );
    let original = f.original().source_for_test();
    assert_eq!(original.builtin_len_mapping.len(), 2);
    assert!(original.call_mapping.is_empty());
    let rows = &original.builtin_len_mapping;
    assert!(rows[0].source_index < rows[1].source_index);
    for (row, param) in rows.iter().zip(&f.source().params) {
        assert!(
            matches!(f.source().exprs[row.receiver.0].kind, ir::ExprKind::Local(l) if l == *param)
        );
        assert!(original.calls[row.source_index].len_candidate);
    }
    replay(&original, &f, &budget).unwrap();
}

#[test]
fn replay_for_iterable_lengths_are_exact_source_occurrences() {
    for source in [
        "fn f(a:&[u8]){for x in a {let n=*x;}}",
        "fn f(){for x in [1,2] {let n=x;}}",
        "fn f(a:&[u8]){let n=a.len();for x in a {let m=a.len();let v=*x;}}",
    ] {
        let (budget, f) = published(source, 0);
        let original = f.original().source_for_test();
        replay(&original, &f, &budget).unwrap();
        let index = original
            .builtin_len_mapping
            .iter()
            .position(|row| original.calls[row.source_index].for_iterable)
            .unwrap();
        let row = original.builtin_len_mapping[index];
        assert!(!original.calls[row.source_index].len_candidate);
        let mut missing = original.clone();
        missing.builtin_len_mapping.remove(index);
        invalid(replay(&missing, &f, &budget));
        let mut duplicate = original.clone();
        duplicate.builtin_len_mapping.insert(index, row);
        invalid(replay(&duplicate, &f, &budget));
        let mut operation = f.source().as_ref().clone();
        operation.exprs[row.expr.0].kind = ir::ExprKind::Int(ir::IntValue::usize(99));
        invalid(replay_source_body(
            &original,
            &Arc::new(operation),
            f.facts(),
            &budget,
        ));
        let mut site = original.clone();
        site.calls[row.source_index].for_iterable = false;
        invalid(replay(&site, &f, &budget));
        let mut receiver = original.clone();
        receiver.builtin_len_mapping[index].receiver = row.expr;
        invalid(replay(&receiver, &f, &budget));
    }
}

// Case 2: a missing checked length occurrence fails, including dead code.
#[test]
fn replay_rejects_missing_builtin_len() {
    for source in [
        "fn f(a: &[u8]) -> usize { a.len() }",
        "fn f(a: &[u8]) { return; let n = a.len(); }",
    ] {
        let (budget, f) = published(source, 0);
        let mut original = f.original().source_for_test();
        assert_eq!(original.builtin_len_mapping.len(), 1, "{source}");
        original.builtin_len_mapping.clear();
        invalid(replay(&original, &f, &budget));
    }
}

// Case 3: a replaced receiver or replaced operation at the same node fails.
#[test]
fn replay_rejects_replaced_builtin_len() {
    let (budget, f) = published("fn f(a: &[u8], b: &[u8]) -> usize { a.len() }", 0);
    let original = f.original().source_for_test();
    let row = original.builtin_len_mapping[0];
    // Point the checked SliceLen at a different (sibling parameter) receiver.
    let mut payload = f.source().as_ref().clone();
    let mut sibling = payload.exprs[row.receiver.0].clone();
    sibling.kind = ir::ExprKind::Local(payload.params[1]);
    let replacement = ir::ExprId(payload.exprs.len());
    payload.exprs.push(sibling);
    payload.exprs[row.expr.0].kind = ir::ExprKind::SliceLen { slice: replacement };
    invalid(replay_source_body(
        &original,
        &Arc::new(payload),
        f.facts(),
        &budget,
    ));
    let mut forged = original.clone();
    forged.builtin_len_mapping[0].receiver = ir::ExprId(row.expr.0);
    invalid(replay(&forged, &f, &budget));
    let mut payload = f.source().as_ref().clone();
    payload.exprs[row.expr.0].kind = ir::ExprKind::Int(ir::IntValue::usize(1));
    invalid(replay_source_body(
        &original,
        &Arc::new(payload),
        f.facts(),
        &budget,
    ));
    let mut payload = f.source().as_ref().clone();
    payload.exprs[row.expr.0].ty = ir::Ty::Int(ir::IntTy::U32);
    invalid(replay_source_body(
        &original,
        &Arc::new(payload),
        f.facts(),
        &budget,
    ));
}

// Case 4: added, duplicated or reordered length occurrences fail.
#[test]
fn replay_rejects_duplicate_and_reordered_builtin_len() {
    let (budget, f) = published(
        "fn f(a: &[u8], b: &[u8]) -> usize { let x = a.len(); let y = b.len(); x + y }",
        0,
    );
    let original = f.original().source_for_test();
    let mut duplicated = original.clone();
    duplicated
        .builtin_len_mapping
        .push(duplicated.builtin_len_mapping[0]);
    invalid(replay(&duplicated, &f, &budget));
    let mut reordered = original.clone();
    reordered.builtin_len_mapping.swap(0, 1);
    invalid(replay(&reordered, &f, &budget));
    let mut swapped = original.clone();
    let (a, b) = (
        swapped.builtin_len_mapping[0].source_index,
        swapped.builtin_len_mapping[1].source_index,
    );
    swapped.builtin_len_mapping[0].source_index = b;
    swapped.builtin_len_mapping[1].source_index = a;
    invalid(replay(&swapped, &f, &budget));
    // An extra checked SliceLen with no source occurrence is an orphan.
    let mut payload = f.source().as_ref().clone();
    let row = original.builtin_len_mapping[0];
    payload.exprs.push(payload.exprs[row.expr.0].clone());
    invalid(replay_source_body(
        &original,
        &Arc::new(payload),
        f.facts(),
        &budget,
    ));
    // A non-candidate source call cannot be claimed as builtin.
    let (budget, g) = published(
        "fn h(x: u8) -> usize { 0 } fn g(a: &[u8]) -> usize { let n = a.len(); h(1) + n }",
        1,
    );
    let mut forged = g.original().source_for_test();
    let ordinary = forged
        .calls
        .iter()
        .position(|site| site.ordinary.is_some())
        .unwrap();
    forged.builtin_len_mapping[0].source_index = ordinary;
    invalid(replay(&forged, &g, &budget));
    let mut flipped = g.original().source_for_test();
    let index = flipped.builtin_len_mapping[0].source_index;
    flipped.calls[index].len_candidate = false;
    invalid(replay(&flipped, &g, &budget));
}

// Case 5: an ordinary `len` method on a non-slice receiver keeps ordinary call
// evidence and never acquires builtin privilege.
#[test]
fn ordinary_len_method_keeps_call_evidence() {
    let source = "struct S { v: usize } impl S { fn len(&self) -> usize { self.v } }
        fn f(s: &S) -> usize { s.len() }";
    let program = program(source);
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let Ok(PublishedCallableBody::Source(f)) = plan.publish_callable_body(declaration, &budget)
    else {
        panic!("expected source publication")
    };
    assert!(exprs(f.source(), |k| matches!(k, ir::ExprKind::SliceLen { .. })).is_empty());
    assert_eq!(
        exprs(f.source(), |k| matches!(k, ir::ExprKind::Call { .. })).len(),
        1
    );
    let original = f.original().source_for_test();
    assert!(original.builtin_len_mapping.is_empty());
    assert_eq!(original.call_mapping.len(), 1);
    replay(&original, &f, &budget).unwrap();
    // Forging a builtin row for the ordinary call is rejected.
    let mut forged = original.clone();
    let call = original.call_mapping[0].clone();
    forged.builtin_len_mapping.push(BuiltinLenOccurrence {
        source_index: call.0,
        expr: call.1,
        receiver: call.1,
    });
    invalid(replay(&forged, &f, &budget));
}

// Case 6: a source Call coerced by ArrayToSlice retains exact underlying Call
// identity; missing or replaced underlying operands reject.
#[test]
fn array_to_slice_wrapper_preserves_call_identity() {
    let source = "fn mk(s: &[u8; 2]) -> &[u8; 2] { s }
        fn g(b: &[u8]) -> usize { b.len() }
        fn f() -> usize { let s: [u8; 2] = [1, 2]; g(mk(&s)) }";
    let (budget, f) = published(source, 2);
    let checked = f.source();
    let wrappers = exprs(checked, |k| matches!(k, ir::ExprKind::ArrayToSlice { .. }));
    assert_eq!(wrappers.len(), 1);
    let ir::ExprKind::ArrayToSlice { reference } = checked.exprs[wrappers[0].0].kind else {
        unreachable!()
    };
    assert!(
        matches!(&checked.exprs[reference.0].kind, ir::ExprKind::Call { name, .. } if name == "crate::mk")
    );
    let original = f.original().source_for_test();
    // The original mk occurrence maps to the inner Call, not the wrapper.
    assert!(original.call_mapping.iter().any(|row| row.1 == reference));
    assert!(!original.call_mapping.iter().any(|row| row.1 == wrappers[0]));
    assert_eq!(original.unsize_mapping.len(), 1);
    replay(&original, &f, &budget).unwrap();
    let mut missing = original.clone();
    missing.unsize_mapping.clear();
    invalid(replay(&missing, &f, &budget));
    let mut replaced = original.clone();
    replaced.unsize_mapping[0].reference = wrappers[0];
    invalid(replay(&replaced, &f, &budget));
    let mut payload = checked.as_ref().clone();
    let borrow = exprs(&payload, |k| matches!(k, ir::ExprKind::Borrow { .. }))[0];
    payload.exprs[wrappers[0].0].kind = ir::ExprKind::ArrayToSlice { reference: borrow };
    invalid(replay_source_body(
        &original,
        &Arc::new(payload),
        f.facts(),
        &budget,
    ));
    let mut moved = original.clone();
    let row = moved
        .call_mapping
        .iter_mut()
        .find(|row| row.1 == reference)
        .unwrap();
    row.1 = wrappers[0];
    invalid(replay(&moved, &f, &budget));
}

// Case 7: ordinary calls around `.len()` retain their full call inventory.
#[test]
fn ordinary_calls_around_len_keep_full_inventory() {
    let source = "fn h(x: usize) -> usize { x }
        fn f(a: &[u8]) -> usize { let n = h(a.len()); h(n) + a.len() }";
    let (budget, f) = published(source, 1);
    let original = f.original().source_for_test();
    assert_eq!(
        original
            .calls
            .iter()
            .filter(|call| !call.operator_candidate)
            .count(),
        4
    );
    assert_eq!(
        original
            .calls
            .iter()
            .filter(|call| call.operator_candidate)
            .count(),
        10
    );
    assert_eq!(original.call_mapping.len(), 2);
    for (index, _, _) in &original.call_mapping {
        assert!(!original.calls[*index].operator_candidate);
        assert!(original.calls[*index].ordinary.is_some());
    }
    assert_eq!(original.builtin_len_mapping.len(), 2);
    replay(&original, &f, &budget).unwrap();
    let mut dropped = original.clone();
    dropped.call_mapping.pop();
    invalid(replay(&dropped, &f, &budget));
    let mut payload = f.source().as_ref().clone();
    let call = exprs(&payload, |k| matches!(k, ir::ExprKind::Call { .. }))[0];
    payload.exprs.push(payload.exprs[call.0].clone());
    invalid(replay_source_body(
        &original,
        &Arc::new(payload),
        f.facts(),
        &budget,
    ));
}

#[test]
fn downstream_passes_lower_and_check_slices() {
    let (_, f) = published(ADLER, 0);
    let lowered = super::super::cfg::lower(f.source()).expect("CFG lowers admitted slices");
    lowered.validate().unwrap();
    if let Err(error) = super::super::compile(ADLER.into(), 1, true) {
        panic!("adler slice program must pass check: {error}");
    }
}
