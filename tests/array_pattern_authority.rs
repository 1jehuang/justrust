//! Public array-pattern IR and actual parsed-source authority controls.
//! Projection classes do not replace per-step type/bounds checks or move analysis.
use frontend::{Expr, Pattern, PlanCtx, Program, Stmt};
use ir::{
    CheckedFunction, Completion, ExprId, ExprKind, IntTy, LocalDecl, LocalId, MatchPattern,
    PatternBinding, Projection, Ty, TypedBlock, TypedExpr, TypedMatchArm,
};
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

fn array(element: Ty, len: usize) -> Ty {
    Ty::Array {
        element: Arc::new(element),
        len,
    }
}
fn domain() -> Ty {
    array(Ty::Int(IntTy::I32), 3)
}
fn binding(local: usize, index: usize) -> PatternBinding {
    PatternBinding {
        local: LocalId(local),
        projection: vec![Projection::ConstantIndex(index)],
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
        name: "array_authority".into(),
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
                name: "last".into(),
                ty: Ty::Int(IntTy::I32),
                mutable: false,
            },
            LocalDecl {
                name: "other".into(),
                ty: Ty::Int(IntTy::I32),
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
                            bindings: vec![binding(1, 0), binding(2, 2)],
                        },
                        guard: None,
                        body: ExprId(1),
                    }],
                },
            },
        ],
        body: TypedBlock {
            statements: vec![ir::TypedStmt::Expr(ExprId(2))],
            tail: None,
        },
    };
    accept(&function);
    function
}

fn arms(function: &mut CheckedFunction) -> &mut Vec<TypedMatchArm> {
    let ExprKind::Match { arms, .. } = &mut function.exprs[2].kind else {
        panic!("fixture match required")
    };
    arms
}
fn pattern(function: &mut CheckedFunction) -> (&mut Ty, &mut Vec<PatternBinding>) {
    let MatchPattern::Aggregate { ty, bindings } = &mut arms(function)[0].pattern else {
        panic!("fixture aggregate required")
    };
    (ty, bindings)
}
fn replace_domain(function: &mut CheckedFunction, ty: Ty) {
    function.locals[0].ty = ty.clone();
    function.exprs[0].ty = ty.clone();
    *pattern(function).0 = ty;
}
fn accept(function: &CheckedFunction) {
    function.validate().expect("valid array IR must validate");
    cfg::lower(function)
        .expect("valid array IR must lower")
        .validate()
        .unwrap();
}
fn reject(function: &CheckedFunction) {
    assert!(
        function.validate().is_err(),
        "malformed array pattern acquired IR authority"
    );
    assert!(
        cfg::lower(function).is_err(),
        "malformed array pattern bypassed CFG ingress validation"
    );
}

#[test]
fn nonempty_array_domain_and_constant_index_copy_bindings_validate_and_lower() {
    accept(&fixture());
}

#[test]
fn bindingless_array_pattern_is_valid_without_fabricated_child_bindings() {
    let mut function = fixture();
    pattern(&mut function).1.clear();
    accept(&function);
}

#[test]
fn array_tuple_nested_path_uses_constant_index_then_field() {
    let mut function = fixture();
    replace_domain(
        &mut function,
        array(Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into()), 2),
    );
    function.locals[2].ty = Ty::Bool;
    *pattern(&mut function).1 = vec![
        PatternBinding {
            local: LocalId(1),
            projection: vec![Projection::ConstantIndex(0), Projection::Field(0)],
            borrow: None,
        },
        PatternBinding {
            local: LocalId(2),
            projection: vec![Projection::ConstantIndex(1), Projection::Field(1)],
            borrow: None,
        },
    ];
    accept(&function);
}

#[test]
fn nested_fixed_array_path_preserves_two_independent_index_steps() {
    let mut function = fixture();
    replace_domain(&mut function, array(array(Ty::Int(IntTy::I32), 3), 2));
    *pattern(&mut function).1 = vec![
        PatternBinding {
            local: LocalId(1),
            projection: vec![Projection::ConstantIndex(0), Projection::ConstantIndex(2)],
            borrow: None,
        },
        PatternBinding {
            local: LocalId(2),
            projection: vec![Projection::ConstantIndex(1), Projection::ConstantIndex(0)],
            borrow: None,
        },
    ];
    accept(&function);
}

#[test]
fn never_scrutinee_retains_explicit_nonempty_array_domain() {
    let mut function = fixture();
    function.exprs[0] = TypedExpr {
        ty: Ty::Never,
        completion: Completion::Never,
        kind: ExprKind::Block(TypedBlock {
            statements: vec![ir::TypedStmt::Return(None)],
            tail: None,
        }),
    };
    function.exprs[2].ty = Ty::Never;
    function.exprs[2].completion = Completion::Never;
    accept(&function);
}

#[test]
fn allowed_constant_index_class_still_rejects_index_on_structural_tuple() {
    let mut function = fixture();
    replace_domain(
        &mut function,
        Ty::Tuple(vec![Ty::Int(IntTy::I32); 3].into()),
    );
    reject(&function);
}

#[test]
fn allowed_field_class_still_rejects_field_on_array() {
    let mut function = fixture();
    let bindings = pattern(&mut function).1;
    bindings[0].projection = vec![Projection::Field(0)];
    bindings[1].projection = vec![Projection::Field(2)];
    reject(&function);
}

#[test]
fn array_constant_index_equal_to_length_is_out_of_bounds() {
    let mut function = fixture();
    pattern(&mut function).1[1].projection[0] = Projection::ConstantIndex(3);
    reject(&function);
}

#[test]
fn array_maximum_constant_index_rejects_without_overflow() {
    let mut function = fixture();
    pattern(&mut function).1[1].projection[0] = Projection::ConstantIndex(usize::MAX);
    reject(&function);
}

#[test]
fn array_binding_local_requires_exact_selected_element_type() {
    let mut function = fixture();
    function.locals[1].ty = Ty::Bool;
    reject(&function);
}

#[test]
fn array_binding_cannot_reuse_parameter_local() {
    let mut function = fixture();
    let parameter = function.params[0];
    pattern(&mut function).1[0].local = parameter;
    reject(&function);
}

#[test]
fn array_binding_requires_existing_local() {
    let mut function = fixture();
    pattern(&mut function).1[0].local = LocalId(usize::MAX);
    reject(&function);
}

#[test]
fn array_pattern_bindings_cannot_share_one_local() {
    let mut function = fixture();
    pattern(&mut function).1[1].local = LocalId(1);
    reject(&function);
}

#[test]
fn array_pattern_bindings_cannot_overlap_same_index() {
    let mut function = fixture();
    pattern(&mut function).1[1].projection[0] = Projection::ConstantIndex(0);
    reject(&function);
}

#[test]
fn array_pattern_bindings_cannot_overlap_nested_parent_and_child() {
    let mut function = fixture();
    let element = Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into());
    replace_domain(&mut function, array(element.clone(), 2));
    function.locals[1].ty = element;
    *pattern(&mut function).1 = vec![
        PatternBinding {
            local: LocalId(1),
            projection: vec![Projection::ConstantIndex(0)],
            borrow: None,
        },
        PatternBinding {
            local: LocalId(2),
            projection: vec![Projection::ConstantIndex(0), Projection::Field(0)],
            borrow: None,
        },
    ];
    reject(&function);
}

#[test]
fn array_pattern_cannot_bind_entire_root_using_empty_projection() {
    let mut function = fixture();
    function.locals[1].ty = domain();
    let bindings = pattern(&mut function).1;
    bindings.truncate(1);
    bindings[0].projection.clear();
    reject(&function);
}

#[test]
fn empty_array_is_not_an_admitted_aggregate_match_domain() {
    let mut function = fixture();
    replace_domain(&mut function, array(Ty::Int(IntTy::I32), 0));
    pattern(&mut function).1.clear();
    reject(&function);
}

#[test]
fn array_domain_above_existing_limit_is_not_admitted_even_without_bindings() {
    let mut function = fixture();
    replace_domain(
        &mut function,
        array(Ty::Int(IntTy::I32), ir::MAX_ARRAY_LEN + 1),
    );
    pattern(&mut function).1.clear();
    reject(&function);
}

#[test]
fn array_pattern_domain_must_match_actual_array_length() {
    let mut function = fixture();
    let (ty, bindings) = pattern(&mut function);
    *ty = array(Ty::Int(IntTy::I32), 2);
    bindings.clear();
    reject(&function);
}

#[test]
fn array_pattern_domain_must_match_actual_element_type() {
    let mut function = fixture();
    let (ty, bindings) = pattern(&mut function);
    *ty = array(Ty::Bool, 3);
    bindings.clear();
    reject(&function);
}

#[test]
fn nested_allowed_index_class_cannot_index_tuple_child() {
    let mut function = fixture();
    replace_domain(
        &mut function,
        array(Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into()), 2),
    );
    let bindings = pattern(&mut function).1;
    bindings.truncate(1);
    bindings[0].projection = vec![Projection::ConstantIndex(0), Projection::ConstantIndex(0)];
    reject(&function);
}

#[test]
fn nested_array_step_has_its_own_bounds() {
    let mut function = fixture();
    replace_domain(&mut function, array(array(Ty::Int(IntTy::I32), 2), 3));
    let bindings = pattern(&mut function).1;
    bindings.truncate(1);
    bindings[0].projection = vec![Projection::ConstantIndex(0), Projection::ConstantIndex(2)];
    reject(&function);
}

#[test]
fn array_projection_cannot_continue_indexing_scalar_element() {
    let mut function = fixture();
    pattern(&mut function).1[0]
        .projection
        .push(Projection::ConstantIndex(0));
    reject(&function);
}

#[test]
fn array_deref_projection_requires_the_referent_binding_type() {
    let mut function = fixture();
    replace_domain(
        &mut function,
        array(
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            },
            3,
        ),
    );
    let bindings = pattern(&mut function).1;
    bindings.truncate(1);
    // `[&x, ..]` legitimately binds the Copy referent; the binding local
    // must then have the referent type, never the reference element type.
    bindings[0].projection = vec![Projection::ConstantIndex(0), Projection::Deref];
    let local = bindings[0].local.0;
    function.locals[local].ty = Ty::Ref {
        kind: ir::BorrowKind::Shared,
        referent: Arc::new(Ty::Int(IntTy::I32)),
    };
    reject(&function);
}

#[test]
fn unreachable_array_arm_still_validates_physical_indices() {
    let mut function = fixture();
    let mut second = arms(&mut function)[0].clone();
    let MatchPattern::Aggregate { bindings, .. } = &mut second.pattern else {
        unreachable!()
    };
    bindings[0].projection = vec![Projection::ConstantIndex(usize::MAX)];
    *arms(&mut function) = vec![
        TypedMatchArm {
            pattern: MatchPattern::Wildcard,
            guard: None,
            body: ExprId(1),
        },
        second,
    ];
    reject(&function);
}

const SOURCE: &str = r#"
struct Value { field:i32 }
fn origin(value:[Value;3]) {}
fn fixed()->i32 {let [first,_,last]=[1i32,2,3];first+last}
fn middle()->i32 {let [first,..,last]=[1i32,2,3];first+last}
fn leading()->i32 {let [..,last]=[1i32,2,3];last}
fn trailing()->i32 {let [first,..]=[1i32,2,3];first}
fn matching()->i32 {match [1i32,2,3] {[first,..,last]=>first+last}}
fn nested()->i32 {let [[first,..],..]=[[1i32,2],[3,4]];first}
fn ignored()->i32 {let values=[Value{field:1},Value{field:2}];let [..]=values;let next=values;0}
fn main() {}
"#;

#[test]
fn structural_ir_admission_does_not_grant_noncopy_indexed_move_permission() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .find(|declaration| {
            let body = plan.callable_body(&program, *declaration).unwrap();
            body.name.len() == 1
                && program.names.spelling(body.name.segments()[0]).unwrap() == "origin"
        })
        .unwrap();
    let original =
        frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let parameter = original.params[0];
    let actual_array = original.locals[parameter.0].ty.clone();
    let Ty::Array { element, len } = &actual_array else {
        panic!("actual source array parameter required")
    };
    assert_eq!(*len, 3);
    let actual_nominal = (**element).clone();
    assert!(matches!(actual_nominal, Ty::Struct(_)));
    let facts = plan
        .instance_facts(&ir::CallTarget {
            declaration,
            type_args: [].into(),
        })
        .unwrap();
    original
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();

    cfg::lower_with_facts(&original, &facts, facts.budget())
        .expect("unmodified exact-world source function must lower")
        .validate()
        .unwrap();

    // Preserve the original canonical signature and all subject metadata.
    let mut function = original;
    let selected = LocalId(function.locals.len());
    function.locals.push(LocalDecl {
        name: "selected".into(),
        ty: actual_nominal,
        mutable: false,
    });
    function.exprs = vec![
        TypedExpr {
            ty: actual_array.clone(),
            completion: Completion::Returns,
            kind: ExprKind::Local(parameter),
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
                        ty: actual_array,
                        bindings: vec![PatternBinding {
                            local: selected,
                            projection: vec![Projection::ConstantIndex(0)],
                            borrow: None,
                        }],
                    },
                    guard: None,
                    body: ExprId(1),
                }],
            },
        },
    ];
    function.body = TypedBlock {
        statements: vec![ir::TypedStmt::Expr(ExprId(2))],
        tail: None,
    };
    // This is well-formed structural IR, not a claim that Value implements Copy.
    function
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let lowered = cfg::lower_with_facts(&function, &facts, facts.budget())
        .expect("structurally valid indexed move must reach CFG validation");
    let error = lowered
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .expect_err("array-pattern admission bypassed the independent indexed non-Copy move fence");
    assert!(
        error.contains("cannot move non-Copy value out of array index"),
        "wrong rejection: {error}"
    );
}

fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("actual resolved fixed-array patterns");
    frontend::check(&program).expect("actual source baseline checks");
    program
}
fn source_function(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap()
}
fn source_pattern<'a>(program: &'a Program, name: &str) -> &'a Pattern {
    let body = &program.functions[source_function(program, name)].body;
    if let Some(pattern) = body
        .statements
        .iter()
        .find_map(|statement| match statement {
            Stmt::LetPattern { pattern, .. } => Some(pattern),
            _ => None,
        })
    {
        return pattern;
    }
    let Some(Expr::Match { arms, .. }) = body.tail.as_deref() else {
        panic!("actual array match required")
    };
    &arms[0].pattern
}
fn source_pattern_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Pattern {
    let index = source_function(program, name);
    let body = &mut program.functions[index].body;
    if let Some(index) = body
        .statements
        .iter()
        .position(|statement| matches!(statement, Stmt::LetPattern { .. }))
    {
        let Stmt::LetPattern { pattern, .. } = &mut body.statements[index] else {
            unreachable!()
        };
        return pattern;
    }
    let Some(Expr::Match { arms, .. }) = body.tail.as_deref_mut() else {
        panic!("actual array match required")
    };
    &mut arms[0].pattern
}
fn reject_foreign(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign array child acquired source authority"),
    };
    let diagnostic = error.to_ascii_lowercase();
    assert!(
        diagnostic.contains("contract") || diagnostic.contains("foreign"),
        "wrong rejection: {error}"
    );
    assert!(
        !diagnostic.contains("unsupported"),
        "wrong boundary: {error}"
    );
    let usage = budget.usage_snapshot();
    assert_eq!(usage.applications, 0);
    assert_eq!(usage.obligations, 0);
    assert!(!usage.poisoned);
}
fn reject_semantic(program: Program) {
    let error = frontend::check(&program).expect_err("invalid array pattern checked");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "semantic error became unsupported: {error}"
    );
}

#[test]
fn actual_parsed_array_and_rest_positions_retain_only_written_children() {
    let program = parsed();
    let Pattern::Array(children) = source_pattern(&program, "fixed") else {
        panic!("actual fixed array AST required")
    };
    assert_eq!(children.len(), 3);
    assert!(matches!(children[1], Pattern::Wildcard));
    for (name, prefix, suffix) in [
        ("middle", 1, 1),
        ("leading", 0, 1),
        ("trailing", 1, 0),
        ("matching", 1, 1),
        ("ignored", 0, 0),
    ] {
        let Pattern::ArrayRest(fields) = source_pattern(&program, name) else {
            panic!("actual ArrayRest AST required")
        };
        assert_eq!(fields.prefix.len(), prefix);
        assert_eq!(fields.suffix.len(), suffix);
    }
}

#[test]
fn fixed_array_rejects_foreign_actual_child_before_source_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::Array(children) = source_pattern(&foreign, "fixed") else {
        unreachable!()
    };
    let child = children[0].clone();
    let Pattern::Array(children) = source_pattern_mut(&mut program, "fixed") else {
        unreachable!()
    };
    children[0] = child;
    reject_foreign(program);
}

#[test]
fn array_rest_prefix_rejects_foreign_actual_child_before_source_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::ArrayRest(fields) = source_pattern(&foreign, "middle") else {
        unreachable!()
    };
    let child = fields.prefix[0].clone();
    let Pattern::ArrayRest(fields) = source_pattern_mut(&mut program, "middle") else {
        unreachable!()
    };
    fields.prefix[0] = child;
    reject_foreign(program);
}

#[test]
fn array_rest_suffix_rejects_foreign_actual_child_before_source_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::ArrayRest(fields) = source_pattern(&foreign, "middle") else {
        unreachable!()
    };
    let child = fields.suffix[0].clone();
    let Pattern::ArrayRest(fields) = source_pattern_mut(&mut program, "middle") else {
        unreachable!()
    };
    fields.suffix[0] = child;
    reject_foreign(program);
}

#[test]
fn fixed_array_arity_mutation_is_semantically_invalid() {
    let mut program = parsed();
    let Pattern::Array(children) = source_pattern_mut(&mut program, "fixed") else {
        unreachable!()
    };
    children.push(Pattern::Wildcard);
    reject_semantic(program);
}

#[test]
fn array_rest_explicit_children_cannot_exceed_actual_array_length() {
    let mut program = parsed();
    let Pattern::ArrayRest(fields) = source_pattern_mut(&mut program, "middle") else {
        unreachable!()
    };
    fields.prefix.extend([Pattern::Wildcard, Pattern::Wildcard]);
    reject_semantic(program);
}

#[test]
fn array_rest_cannot_duplicate_real_binding_across_rest_boundary() {
    let mut program = parsed();
    let Pattern::ArrayRest(fields) = source_pattern_mut(&mut program, "middle") else {
        unreachable!()
    };
    fields.suffix[0] = fields.prefix[0].clone();
    let error = frontend::check(&program).expect_err("duplicate array binding checked");
    assert!(
        error.to_ascii_lowercase().contains("duplicate"),
        "wrong rejection: {error}"
    );
}
