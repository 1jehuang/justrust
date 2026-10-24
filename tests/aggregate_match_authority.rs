//! Public checked-IR integrity controls for irrefutable aggregate matches.
//! These are structural validation tests, not source visibility or ABI claims.
use justrust_engine::compiler::ir::{
    BorrowKind, CheckedFunction, Completion, ExprId, ExprKind, IntTy, LocalDecl, LocalId,
    MAX_PATTERN_BINDINGS, MAX_PATTERN_PROJECTION_DEPTH, MatchPattern, PatternBinding, Projection,
    Ty, TypedBlock, TypedExpr, TypedMatchArm, TypedStmt,
};
use std::sync::Arc;

fn domain() -> Ty {
    Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into())
}

fn binding(local: usize, field: usize) -> PatternBinding {
    PatternBinding {
        local: LocalId(local),
        projection: vec![Projection::Field(field)],
        borrow: None,
    }
}

fn fixture() -> CheckedFunction {
    let ty = domain();
    let function = CheckedFunction {
        runtime_sites: Default::default(),
        call_evidence: Arc::from([]),
        body_substitution: None,
        ref_contract: Default::default(),
        name: "aggregate_authority".into(),
        params: vec![LocalId(0)],
        result: Ty::Unit,
        locals: vec![
            LocalDecl {
                name: "input".into(),
                ty: ty.clone(),
                mutable: false,
            },
            LocalDecl {
                name: "first".into(),
                ty: Ty::Int(IntTy::I32),
                mutable: false,
            },
            LocalDecl {
                name: "second".into(),
                ty: Ty::Bool,
                mutable: false,
            },
            LocalDecl {
                name: "other_first".into(),
                ty: Ty::Int(IntTy::I32),
                mutable: false,
            },
            LocalDecl {
                name: "other_second".into(),
                ty: Ty::Bool,
                mutable: false,
            },
        ],
        exprs: vec![
            TypedExpr {
                ty: ty.clone(),
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(0)),
            },
            TypedExpr {
                ty: Ty::Unit,
                completion: Completion::Returns,
                kind: ExprKind::Block(TypedBlock {
                    statements: vec![],
                    tail: None,
                }),
            },
            TypedExpr {
                ty: Ty::Unit,
                completion: Completion::Returns,
                kind: ExprKind::Match {
                    scrutinee: ExprId(0),
                    arms: vec![TypedMatchArm {
                        pattern: MatchPattern::Aggregate {
                            ty,
                            bindings: vec![binding(1, 0), binding(2, 1)],
                        },
                        guard: None,
                        body: ExprId(1),
                    }],
                    scrutinee_scope: false,
                },
            },
        ],
        body: TypedBlock {
            statements: vec![TypedStmt::Expr(ExprId(2))],
            tail: None,
        },
    };
    function
        .validate()
        .expect("baseline public aggregate IR must validate");
    function
}

fn arms(function: &mut CheckedFunction) -> &mut Vec<TypedMatchArm> {
    match &mut function.exprs[2].kind {
        ExprKind::Match { arms, .. } => arms,
        _ => panic!("fixture lost its match"),
    }
}

fn pattern(function: &mut CheckedFunction) -> (&mut Ty, &mut Vec<PatternBinding>) {
    match &mut arms(function)[0].pattern {
        MatchPattern::Aggregate { ty, bindings } => (ty, bindings),
        _ => panic!("fixture lost its aggregate pattern"),
    }
}

fn reject(function: CheckedFunction) {
    let error = function
        .validate()
        .expect_err("malformed aggregate IR gained validation authority");
    assert!(!error.is_empty());
}

#[test]
fn exact_structural_aggregate_domain_and_field_bindings_validate() {
    fixture().validate().unwrap();
}

#[test]
fn empty_bindings_are_valid_for_an_actual_aggregate_domain() {
    let mut function = fixture();
    pattern(&mut function).1.clear();
    function.validate().unwrap();
}

#[test]
fn nested_plain_fields_with_distinct_leaf_bindings_validate() {
    let mut function = fixture();
    let outer = Ty::Tuple(vec![domain(), Ty::Bool].into());
    function.locals[0].ty = outer.clone();
    function.exprs[0].ty = outer.clone();
    let (ty, bindings) = pattern(&mut function);
    *ty = outer;
    bindings[0].projection = vec![Projection::Field(0), Projection::Field(0)];
    bindings[1].projection = vec![Projection::Field(0), Projection::Field(1)];
    function.validate().unwrap();
}

#[test]
fn source_binding_order_does_not_require_sorted_field_indices() {
    let mut function = fixture();
    pattern(&mut function).1.reverse();
    function.validate().unwrap();
}

#[test]
fn separate_catchall_arms_may_use_distinct_fresh_locals() {
    let mut function = fixture();
    arms(&mut function).push(TypedMatchArm {
        pattern: MatchPattern::Aggregate {
            ty: domain(),
            bindings: vec![binding(3, 0), binding(4, 1)],
        },
        guard: None,
        body: ExprId(1),
    });
    function.validate().unwrap();
}

#[test]
fn never_scrutinee_retains_explicit_aggregate_pattern_domain() {
    let mut function = fixture();
    function.exprs[0] = TypedExpr {
        ty: Ty::Never,
        completion: Completion::Never,
        kind: ExprKind::Block(TypedBlock {
            statements: vec![TypedStmt::Return(None)],
            tail: None,
        }),
    };
    function.exprs[2].ty = Ty::Never;
    function.exprs[2].completion = Completion::Never;
    function.validate().unwrap();
}

#[test]
fn scalar_domain_cannot_launder_empty_bindings_into_aggregate_authority() {
    let mut function = fixture();
    let (ty, bindings) = pattern(&mut function);
    *ty = Ty::Bool;
    bindings.clear();
    function.locals[0].ty = Ty::Bool;
    function.exprs[0].ty = Ty::Bool;
    reject(function);
}

#[test]
fn canonical_unit_is_not_an_aggregate_pattern_domain() {
    let mut function = fixture();
    let (ty, bindings) = pattern(&mut function);
    *ty = Ty::Unit;
    bindings.clear();
    function.locals[0].ty = Ty::Unit;
    function.exprs[0].ty = Ty::Unit;
    reject(function);
}

#[test]
fn empty_structural_tuple_is_not_an_aggregate_pattern_domain() {
    let mut function = fixture();
    let invalid = Ty::Tuple(Arc::from([]));
    let (ty, bindings) = pattern(&mut function);
    *ty = invalid.clone();
    bindings.clear();
    function.locals[0].ty = invalid.clone();
    function.exprs[0].ty = invalid;
    reject(function);
}

#[test]
fn array_domain_with_no_bindings_is_admitted_by_aggregate_match_variant() {
    let mut function = fixture();
    let invalid = Ty::Array {
        element: Arc::new(Ty::Bool),
        len: 2,
    };
    let (ty, bindings) = pattern(&mut function);
    *ty = invalid.clone();
    bindings.clear();
    function.locals[0].ty = invalid.clone();
    function.exprs[0].ty = invalid;
    function.validate().unwrap();
}

#[test]
fn reference_to_aggregate_is_not_implicit_match_ergonomics_authority() {
    let mut function = fixture();
    let invalid = Ty::Ref {
        kind: BorrowKind::Shared,
        referent: Arc::new(domain()),
    };
    let (ty, bindings) = pattern(&mut function);
    *ty = invalid.clone();
    bindings.clear();
    function.locals[0].ty = invalid.clone();
    function.exprs[0].ty = invalid;
    reject(function);
}

#[test]
fn explicit_aggregate_domain_must_equal_actual_scrutinee_type() {
    let mut function = fixture();
    let (ty, bindings) = pattern(&mut function);
    *ty = Ty::Tuple(vec![Ty::Bool, Ty::Int(IntTy::I32)].into());
    bindings.clear();
    reject(function);
}

#[test]
fn aggregate_binding_must_name_an_existing_local() {
    let mut function = fixture();
    pattern(&mut function).1[0].local = LocalId(usize::MAX);
    reject(function);
}

#[test]
fn aggregate_binding_cannot_reuse_a_parameter_local() {
    let mut function = fixture();
    pattern(&mut function).1[0].local = LocalId(0);
    reject(function);
}

#[test]
fn aggregate_binding_locals_must_be_distinct_within_arm() {
    let mut function = fixture();
    pattern(&mut function).1[1].local = LocalId(1);
    reject(function);
}

#[test]
fn aggregate_binding_local_cannot_be_shared_across_arms() {
    let mut function = fixture();
    arms(&mut function).push(TypedMatchArm {
        pattern: MatchPattern::Aggregate {
            ty: domain(),
            bindings: vec![binding(1, 0), binding(4, 1)],
        },
        guard: None,
        body: ExprId(1),
    });
    reject(function);
}

#[test]
fn aggregate_field_binding_requires_exact_local_type() {
    let mut function = fixture();
    function.locals[1].ty = Ty::Bool;
    reject(function);
}

#[test]
fn aggregate_field_index_must_be_in_bounds() {
    let mut function = fixture();
    pattern(&mut function).1[0].projection = vec![Projection::Field(usize::MAX)];
    reject(function);
}

#[test]
fn aggregate_binding_paths_cannot_project_through_scalar_leaf() {
    let mut function = fixture();
    pattern(&mut function).1[0]
        .projection
        .push(Projection::Field(0));
    reject(function);
}

/// `&mut i32` is not Copy, so overlapping by-value bindings of it are moves.
/// Stock rustc accepts overlapping Copy bindings (`x @ (a, b)` on Copy data),
/// so the rejection controls below must use a non-Copy domain.
fn mut_ref() -> Ty {
    Ty::Ref {
        kind: BorrowKind::Mut,
        referent: Arc::new(Ty::Int(IntTy::I32)),
    }
}

fn noncopy_domain() -> Ty {
    Ty::Tuple(vec![mut_ref(), Ty::Bool].into())
}

/// Rebuilds the fixture over `(&mut i32, bool)` with matching local types.
fn noncopy_fixture() -> CheckedFunction {
    let mut function = fixture();
    let domain = noncopy_domain();
    function.locals[0].ty = domain.clone();
    function.exprs[0].ty = domain.clone();
    function.locals[1].ty = mut_ref();
    function.locals[3].ty = mut_ref();
    *pattern(&mut function).0 = domain;
    function
        .validate()
        .expect("non-Copy aggregate baseline must validate");
    function
}

#[test]
fn aggregate_binding_cannot_claim_the_whole_root() {
    let mut function = noncopy_fixture();
    function.locals[1].ty = noncopy_domain();
    let (_, bindings) = pattern(&mut function);
    bindings.truncate(1);
    bindings[0].projection.clear();
    reject(function);
}

#[test]
fn copy_aggregate_binding_may_claim_the_whole_root_like_stock() {
    // `let x @ (_, _) = (1i32, true);` is accepted by stock rustc.
    let mut function = fixture();
    function.locals[1].ty = domain();
    let (_, bindings) = pattern(&mut function);
    bindings.truncate(1);
    bindings[0].projection.clear();
    function.validate().unwrap();
}

#[test]
fn aggregate_binding_paths_cannot_overlap_the_same_field() {
    let mut function = noncopy_fixture();
    *pattern(&mut function).1 = vec![binding(1, 0), binding(3, 0)];
    reject(function);
}

#[test]
fn copy_aggregate_binding_paths_may_overlap_like_stock() {
    let mut function = fixture();
    *pattern(&mut function).1 = vec![binding(1, 0), binding(3, 0)];
    function.validate().unwrap();
}

#[test]
fn aggregate_binding_paths_cannot_overlap_parent_and_nested_field() {
    let mut function = noncopy_fixture();
    let inner = noncopy_domain();
    let outer = Ty::Tuple(vec![inner.clone(), Ty::Bool].into());
    function.locals[0].ty = outer.clone();
    function.exprs[0].ty = outer.clone();
    function.locals[1].ty = inner;
    let (ty, bindings) = pattern(&mut function);
    *ty = outer;
    *bindings = vec![
        binding(1, 0),
        PatternBinding {
            local: LocalId(3),
            projection: vec![Projection::Field(0), Projection::Field(0)],
            borrow: None,
        },
    ];
    reject(function);
}

#[test]
fn aggregate_binding_admits_checked_field_then_constant_index_projection() {
    let mut function = fixture();
    let domain = Ty::Tuple(
        vec![
            Ty::Array {
                element: Arc::new(Ty::Int(IntTy::I32)),
                len: 1,
            },
            Ty::Bool,
        ]
        .into(),
    );
    function.locals[0].ty = domain.clone();
    function.exprs[0].ty = domain.clone();
    let (ty, bindings) = pattern(&mut function);
    *ty = domain;
    bindings[0].projection = vec![Projection::Field(0), Projection::ConstantIndex(0)];
    function.validate().unwrap();
}

#[test]
fn aggregate_binding_deref_projection_requires_exact_referent_type() {
    let mut function = fixture();
    let domain = Ty::Tuple(
        vec![
            Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            },
            Ty::Bool,
        ]
        .into(),
    );
    function.locals[0].ty = domain.clone();
    function.exprs[0].ty = domain.clone();
    let (ty, bindings) = pattern(&mut function);
    *ty = domain;
    // A by-value binding of a Copy referent is an explicit `(&x, _)` pattern.
    // It is admitted only with the referent's exact type, never the carrier's.
    bindings[0].projection = vec![Projection::Field(0), Projection::Deref];
    function.validate().unwrap();
    let local = pattern(&mut function).1[0].local.0;
    function.locals[local].ty = Ty::Ref {
        kind: BorrowKind::Shared,
        referent: Arc::new(Ty::Int(IntTy::I32)),
    };
    reject(function);
}

#[test]
fn unreachable_aggregate_arm_still_requires_valid_binding_metadata() {
    let mut function = fixture();
    arms(&mut function).insert(
        0,
        TypedMatchArm {
            pattern: MatchPattern::Wildcard,
            guard: None,
            body: ExprId(1),
        },
    );
    if let MatchPattern::Aggregate { bindings, .. } = &mut arms(&mut function)[1].pattern {
        bindings[0].local = LocalId(usize::MAX);
    } else {
        panic!("expected aggregate second arm");
    }
    reject(function);
}

#[test]
fn never_scrutinee_does_not_skip_aggregate_projection_validation() {
    let mut function = fixture();
    function.exprs[0] = TypedExpr {
        ty: Ty::Never,
        completion: Completion::Never,
        kind: ExprKind::Block(TypedBlock {
            statements: vec![TypedStmt::Return(None)],
            tail: None,
        }),
    };
    function.exprs[2].ty = Ty::Never;
    function.exprs[2].completion = Completion::Never;
    pattern(&mut function).1[0].projection = vec![Projection::Field(usize::MAX)];
    reject(function);
}

#[test]
fn aggregate_binding_count_limit_is_not_removed() {
    let mut function = fixture();
    *pattern(&mut function).1 = vec![binding(1, 0); MAX_PATTERN_BINDINGS + 1];
    let error = function.validate().unwrap_err();
    assert!(error.contains("limit"), "wrong rejection: {error}");
}

#[test]
fn aggregate_projection_depth_limit_is_not_removed() {
    let mut function = fixture();
    pattern(&mut function).1[0].projection =
        vec![Projection::Field(0); MAX_PATTERN_PROJECTION_DEPTH + 1];
    let error = function.validate().unwrap_err();
    assert!(error.contains("depth"), "wrong rejection: {error}");
}
