//! Source-backed authority for the bounded char cast domain.
//! Structural publication/lowering checks only, not native execution claims.
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

const INTS: [(&str, ir::IntTy); 12] = [
    ("u8", ir::IntTy::U8),
    ("u16", ir::IntTy::U16),
    ("u32", ir::IntTy::U32),
    ("u64", ir::IntTy::U64),
    ("u128", ir::IntTy::U128),
    ("usize", ir::IntTy::Usize),
    ("i8", ir::IntTy::I8),
    ("i16", ir::IntTy::I16),
    ("i32", ir::IntTy::I32),
    ("i64", ir::IntTy::I64),
    ("i128", ir::IntTy::I128),
    ("isize", ir::IntTy::Isize),
];

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx) {
    let program = frontend::parse(source).unwrap();
    frontend::check(&program).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    (program, plan)
}

fn declaration(program: &frontend::Program, plan: &frontend::PlanCtx, name: &str) -> ir::DeclId {
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap();
    plan.function_decl(index).unwrap()
}

fn retained(
    program: &frontend::Program,
    plan: &frontend::PlanCtx,
    name: &str,
) -> (ir::CheckedFunction, Arc<ValidatedFacts>) {
    let id = declaration(program, plan, name);
    let facts = plan.callable_facts(id).unwrap();
    let target = ir::CallTarget {
        declaration: id,
        type_args: plan
            .callable_env(id)
            .unwrap()
            .ordered_params()
            .map(ir::Ty::Param)
            .collect::<Vec<_>>()
            .into(),
    };
    let checked = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    (checked, facts)
}

fn reject(checked: ir::CheckedFunction, facts: &Arc<ValidatedFacts>) {
    assert!(
        checked
            .validate_with_facts(ir::FactsCtx::Validated(facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).is_err());
}

fn cast_index(checked: &ir::CheckedFunction) -> usize {
    let indices: Vec<_> = checked
        .exprs
        .iter()
        .enumerate()
        .filter_map(|(index, expr)| {
            matches!(
                expr.kind,
                ir::ExprKind::CharToInt { .. } | ir::ExprKind::U8ToChar { .. }
            )
            .then_some(index)
        })
        .collect();
    assert_eq!(indices.len(), 1);
    indices[0]
}

fn cast_location(function: &cfg::Function) -> (usize, usize) {
    let locations: Vec<_> = function
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(block, body)| {
            body.statements
                .iter()
                .enumerate()
                .filter_map(move |(statement, located)| {
                    matches!(
                        &located.value,
                        cfg::Statement::Assign {
                            value: cfg::Rvalue::CharToInt { .. } | cfg::Rvalue::U8ToChar { .. },
                            ..
                        }
                    )
                    .then_some((block, statement))
                })
        })
        .collect();
    assert_eq!(locations.len(), 1);
    locations[0]
}

#[test]
fn every_integer_target_and_u8_source_survive_retained_publication_and_exact_lowering() {
    let cases = INTS
        .iter()
        .map(|(name, ty)| {
            (
                format!("fn value()->{name}{{'🦀' as {name}}} fn main(){{}}"),
                Some(*ty),
            )
        })
        .chain([(
            "fn value()->char{255u8 as char} fn main(){}".to_owned(),
            None,
        )]);
    for (source, to) in cases {
        let (program, plan) = plan(&source);
        assert!(matches!(
            program.functions[0].body.tail.as_deref(),
            Some(frontend::Expr::Cast { .. })
        ));
        let (checked, facts) = retained(&program, &plan, "value");
        let index = cast_index(&checked);
        let expr = &checked.exprs[index];
        let source_id = match (to, &expr.kind) {
            (Some(expected), ir::ExprKind::CharToInt { value, to }) => {
                assert_eq!(*to, expected);
                assert_eq!(expr.ty, ir::Ty::Int(expected));
                assert_eq!(checked.exprs[value.0].ty, ir::Ty::Char);
                assert!(matches!(
                    checked.exprs[value.0].kind,
                    ir::ExprKind::Char(ir::CharValue { value: 0x1f980 })
                ));
                *value
            }
            (None, ir::ExprKind::U8ToChar { value }) => {
                assert_eq!(expr.ty, ir::Ty::Char);
                assert_eq!(checked.exprs[value.0].ty, ir::Ty::Int(ir::IntTy::U8));
                assert!(matches!(
                    checked.exprs[value.0].kind,
                    ir::ExprKind::Int(ir::IntValue {
                        ty: ir::IntTy::U8,
                        bits: 255
                    })
                ));
                *value
            }
            _ => panic!("wrong cast kind for {source}"),
        };
        assert_eq!(expr.completion, ir::Completion::Returns);
        assert_eq!(checked.exprs[source_id.0].completion, expr.completion);
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        let function = template.function();
        function
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let (block, statement) = cast_location(function);
        let located = &function.blocks[block].statements[statement];
        assert_eq!(located.origin, Some(ir::ExprId(index)));
        let cfg::Statement::Assign { value, .. } = &located.value else {
            unreachable!()
        };
        let (operand, expected) = match (to, value) {
            (Some(expected), cfg::Rvalue::CharToInt { value, to }) => {
                assert_eq!(*to, expected);
                (value, cfg::Constant::Char(ir::CharValue { value: 0x1f980 }))
            }
            (None, cfg::Rvalue::U8ToChar { value }) => (
                value,
                cfg::Constant::Int(ir::IntValue {
                    ty: ir::IntTy::U8,
                    bits: 255,
                }),
            ),
            _ => panic!("wrong lowered cast for {source}"),
        };
        // Even a scalar leaf gets exactly one snapshot, copied into the cast.
        let cfg::Operand::Copy(snapshot) = operand else {
            panic!("scalar cast must copy its evaluated source snapshot")
        };
        let initializers: Vec<_> = function.blocks.iter().flat_map(|body| &body.statements)
            .filter(|located| matches!(&located.value, cfg::Statement::Assign { destination, .. } if destination == snapshot))
            .collect();
        assert_eq!(initializers.len(), 1);
        assert_eq!(initializers[0].origin, Some(source_id));
        assert!(
            function.blocks[block].statements[..statement]
                .iter()
                .any(|candidate| std::ptr::eq(candidate, initializers[0]))
        );
        assert!(
            matches!(&initializers[0].value, cfg::Statement::Assign { value: cfg::Rvalue::Use(cfg::Operand::Constant(actual)), .. } if *actual == expected)
        );
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn ir_rejects_forged_domains_results_completion_and_handles_even_in_unused_rows() {
    for source in [
        "fn value()->u32{'λ' as u32} fn main(){}",
        "fn value()->char{255u8 as char} fn main(){}",
    ] {
        let (program, plan) = plan(source);
        let (checked, facts) = retained(&program, &plan, "value");
        let original = cast_index(&checked);
        for unused in [false, true] {
            for forgery in 0..5 {
                let mut changed = checked.clone();
                let index = if unused {
                    changed.exprs.push(checked.exprs[original].clone());
                    changed.exprs.len() - 1
                } else {
                    original
                };
                let value = match changed.exprs[index].kind {
                    ir::ExprKind::CharToInt { value, .. } | ir::ExprKind::U8ToChar { value } => {
                        value
                    }
                    _ => unreachable!(),
                };
                match forgery {
                    0 => changed.exprs[index].ty = ir::Ty::Bool,
                    1 => changed.exprs[index].completion = ir::Completion::Never,
                    2 => {
                        // A valid bool leaf isolates the cast's domain check.
                        let mut leaf = checked.exprs[value.0].clone();
                        leaf.kind = ir::ExprKind::Bool(true);
                        leaf.ty = ir::Ty::Bool;
                        changed.exprs.push(leaf);
                        let wrong = ir::ExprId(changed.exprs.len() - 1);
                        match &mut changed.exprs[index].kind {
                            ir::ExprKind::CharToInt { value, .. }
                            | ir::ExprKind::U8ToChar { value } => *value = wrong,
                            _ => unreachable!(),
                        }
                    }
                    3 => {
                        let invalid = ir::ExprId(changed.exprs.len() + 1);
                        match &mut changed.exprs[index].kind {
                            ir::ExprKind::CharToInt { value, .. }
                            | ir::ExprKind::U8ToChar { value } => *value = invalid,
                            _ => unreachable!(),
                        }
                    }
                    4 => {
                        // Swapping cast families must not authorize their old source/result.
                        changed.exprs[index].kind = match changed.exprs[index].kind {
                            ir::ExprKind::CharToInt { value, .. } => {
                                ir::ExprKind::U8ToChar { value }
                            }
                            ir::ExprKind::U8ToChar { value } => ir::ExprKind::CharToInt {
                                value,
                                to: ir::IntTy::U32,
                            },
                            _ => unreachable!(),
                        };
                    }
                    _ => unreachable!(),
                }
                reject(changed, &facts);
            }
        }
    }
}

#[test]
fn invalid_unicode_scalars_cannot_enter_casts_through_used_or_unused_ir() {
    let (program, plan) = plan("fn value()->u32{'λ' as u32} fn main(){}");
    let (checked, facts) = retained(&program, &plan, "value");
    let index = cast_index(&checked);
    let ir::ExprKind::CharToInt { value, .. } = checked.exprs[index].kind else {
        unreachable!()
    };
    for scalar in [0xd800, 0xdbff, 0xdc00, 0xdfff, 0x110000, u32::MAX] {
        for unused in [false, true] {
            let mut changed = checked.clone();
            let leaf = if unused {
                changed.exprs.push(checked.exprs[value.0].clone());
                let leaf = changed.exprs.len() - 1;
                let mut cast = checked.exprs[index].clone();
                cast.kind = ir::ExprKind::CharToInt {
                    value: ir::ExprId(leaf),
                    to: ir::IntTy::U32,
                };
                changed.exprs.push(cast);
                leaf
            } else {
                value.0
            };
            changed.exprs[leaf].kind = ir::ExprKind::Char(ir::CharValue { value: scalar });
            reject(changed, &facts);
        }
    }
}

#[test]
fn cfg_rejects_wrong_domains_results_scalars_and_forged_lowering_in_all_blocks() {
    for source in [
        "fn value()->u32{'λ' as u32} fn main(){}",
        "fn value()->char{255u8 as char} fn main(){}",
    ] {
        let (program, plan) = plan(source);
        let (checked, facts) = retained(&program, &plan, "value");
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        let original = template.function().as_ref();
        let (block, statement) = cast_location(original);
        // Check raw semantic validity separately, so malformed-domain tests do
        // not pass merely because the generated-shape proof noticed an edit.
        let mut semantic = original.clone();
        semantic.lowering_proof = None;
        semantic
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        for unused in [false, true] {
            for forgery in (0..8).filter(|value| !matches!(value, 4 | 5)) {
                let mut changed = original.clone();
                let block = if unused {
                    // No incoming edge: validators must still inspect this row.
                    changed.blocks.push(original.blocks[block].clone());
                    changed.blocks.len() - 1
                } else {
                    block
                };
                let located = &mut changed.blocks[block].statements[statement];
                let cfg::Statement::Assign { value, .. } = &mut located.value else {
                    unreachable!()
                };
                match forgery {
                    0 => match value {
                        cfg::Rvalue::CharToInt { value, .. } | cfg::Rvalue::U8ToChar { value } => {
                            *value = cfg::Operand::Constant(cfg::Constant::Bool(true))
                        }
                        _ => unreachable!(),
                    },
                    1 => {
                        *value = cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::Bool(true)))
                    }
                    2 => {
                        *value = cfg::Rvalue::CharToInt {
                            value: cfg::Operand::Constant(cfg::Constant::Char(ir::CharValue {
                                value: 0xd800,
                            })),
                            to: ir::IntTy::U32,
                        }
                    }
                    3 => {
                        *value = cfg::Rvalue::U8ToChar {
                            value: cfg::Operand::Constant(cfg::Constant::Int(ir::IntValue {
                                ty: ir::IntTy::U32,
                                bits: 65,
                            })),
                        }
                    }
                    4 => located.origin = None,
                    5 => match value {
                        // Same domain and result type, different payload: only exact
                        // lowering authority distinguishes this from the source.
                        cfg::Rvalue::CharToInt { value, .. } => {
                            *value = cfg::Operand::Constant(cfg::Constant::Char(ir::CharValue {
                                value: 65,
                            }))
                        }
                        cfg::Rvalue::U8ToChar { value } => {
                            *value = cfg::Operand::Constant(cfg::Constant::Int(ir::IntValue {
                                ty: ir::IntTy::U8,
                                bits: 65,
                            }))
                        }
                        _ => unreachable!(),
                    },
                    6 => {
                        *value = cfg::Rvalue::U8ToChar {
                            value: cfg::Operand::Constant(cfg::Constant::Int(ir::IntValue {
                                ty: ir::IntTy::U8,
                                bits: 256,
                            })),
                        }
                    }
                    7 => {
                        *value = cfg::Rvalue::CharToInt {
                            value: cfg::Operand::Constant(cfg::Constant::Char(ir::CharValue {
                                value: 0x110000,
                            })),
                            to: ir::IntTy::U32,
                        }
                    }
                    _ => unreachable!(),
                }
                assert!(
                    changed
                        .validate_with_facts(ir::FactsCtx::Validated(&facts))
                        .is_err()
                        || changed.validate_provenance(&facts, facts.budget()).is_err(),
                    "unused={unused}, forgery={forgery}"
                );
                changed.lowering_proof = None;
                if !matches!(forgery, 4 | 5) {
                    assert!(
                        changed
                            .validate_with_facts(ir::FactsCtx::Validated(&facts))
                            .is_err(),
                        "raw semantics: unused={unused}, forgery={forgery}"
                    );
                } else if !unused {
                    // Origin/payload edits preserve types but not exact lowering.
                    changed
                        .validate_with_facts(ir::FactsCtx::Validated(&facts))
                        .unwrap();
                }
            }
        }
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn cast_cfg_source_correspondence_uses_retained_call_evidence() {
    for source in [
        "fn identity(c:char)->char{c} fn value()->u32{identity('λ') as u32} fn main(){}",
        "fn identity(c:u8)->u8{c} fn value()->char{identity(255u8) as char} fn main(){}",
    ] {
        let (program, plan) = plan(source);
        let (checked, facts) = retained(&program, &plan, "value");
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        let original = template.function();
        assert!(original.lowering_proof.is_some());
        original
            .validate_provenance(&facts, facts.budget())
            .unwrap();
        let return_block = original
            .blocks
            .iter()
            .position(|b| matches!(b.terminator.value, cfg::Terminator::Return))
            .unwrap();
        for unused in [false, true] {
            let mut changed = original.as_ref().clone();
            let index = if unused {
                changed.blocks.push(original.blocks[return_block].clone());
                changed.blocks.len() - 1
            } else {
                return_block
            };
            changed.blocks[index].terminator.value = cfg::Terminator::Unreachable;
            assert!(changed.validate_provenance(&facts, facts.budget()).is_err());
        }
        let (block, statement) = cast_location(original);
        let mut changed = original.as_ref().clone();
        changed.blocks[block].statements[statement].origin = None;
        assert!(changed.validate_provenance(&facts, facts.budget()).is_err());
        let mut changed = original.as_ref().clone();
        let cfg::Statement::Assign { value, .. } =
            &mut changed.blocks[block].statements[statement].value
        else {
            unreachable!()
        };
        match value {
            cfg::Rvalue::CharToInt { value, .. } => {
                *value = cfg::Operand::Constant(cfg::Constant::Char(ir::CharValue { value: 65 }))
            }
            cfg::Rvalue::U8ToChar { value } => {
                *value = cfg::Operand::Constant(cfg::Constant::Int(ir::IntValue {
                    ty: ir::IntTy::U8,
                    bits: 65,
                }))
            }
            _ => unreachable!(),
        }
        assert!(changed.validate_provenance(&facts, facts.budget()).is_err());
    }
}

#[test]
fn only_u8_can_enter_char_and_char_can_only_leave_for_integers() {
    for (name, _) in INTS {
        if name != "u8" {
            let source = format!("fn unused()->char{{65{name} as char}} fn main(){{}}");
            let program = frontend::parse(&source).unwrap();
            assert!(frontend::check(&program).is_err(), "{source}");
        }
    }
    for expression in [
        "true as char",
        "65f32 as char",
        "65f64 as char",
        "{let x=65u8; (&x) as char}",
        "'a' as bool",
        "'a' as f32",
        "'a' as f64",
        "'a' as &u8",
        "{let x='a'; (&x) as u32}",
    ] {
        let source = format!("fn unused(){{let x={expression};}} fn main(){{}}");
        let program = frontend::parse(&source).unwrap();
        assert!(frontend::check(&program).is_err(), "{source}");
    }
}

#[test]
fn publication_binds_cast_bodies_to_their_world_and_universal_callable_facts() {
    let source = "fn value()->u32{'λ' as u32} fn peer()->u32{'🦀' as u32} fn generic<T>(x:T)->char{255u8 as char} fn main(){}";
    let (program, plan) = plan(source);
    let (checked, facts) = retained(&program, &plan, "value");
    let (_, peer) = retained(&program, &plan, "peer");
    let (foreign_program, foreign_plan) = self::plan(source);
    let (_, foreign) = retained(&foreign_program, &foreign_plan, "value");
    let target = ir::CallTarget {
        declaration: declaration(&program, &plan, "value"),
        type_args: Arc::from([]),
    };
    // Scalar-only raw IR has no callable identity. Test wrong facts at the
    // retained publication boundary rather than assuming leaf IR binds it.
    for wrong in [peer, foreign] {
        assert!(plan.checked_body(&target, &wrong, wrong.budget()).is_err());
    }
    let clone = program.clone();
    let cloned =
        frontend::type_callable_with_plan(&clone, target.declaration, &plan, facts.budget())
            .unwrap();
    assert_eq!(cloned.exprs.len(), checked.exprs.len());
    let template = cfg::lower_template(Arc::new(cloned), facts.clone(), facts.budget()).unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
    let generic = declaration(&program, &plan, "generic");
    let instance_target = ir::CallTarget {
        declaration: generic,
        type_args: Arc::from([ir::Ty::Char]),
    };
    let instance = plan.instance_facts(&instance_target).unwrap();
    assert!(
        plan.checked_body(&instance_target, &instance, instance.budget())
            .is_err()
    );
    let (universal, facts) = retained(&program, &plan, "generic");
    let template = cfg::lower_template(Arc::new(universal), facts.clone(), facts.budget()).unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}
