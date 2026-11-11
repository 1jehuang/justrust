use super::*;
use crate::compiler::{frontend, ownership, source_replay::CanonicalSourcePublication};

fn publish(text: &str, budget: &ir::PlanBudget) -> Arc<CanonicalSourcePublication> {
    let program = frontend::parse(text).unwrap();
    let context = frontend::PlanCtx::from_program(&program, budget).unwrap();
    let frontend::PublishedCallableBody::Source(publication) = context
        .publish_callable_body(context.function_decl(0).unwrap(), budget)
        .unwrap()
    else {
        panic!("source publication expected")
    };
    publication
}
const TWO: &str = "fn f(){let a=D{v:1};let b=D{v:2};} struct D{v:i32} impl Drop for D{fn drop(&mut self){self.v=0;}} fn main(){}";
const BORROWED: &str = "fn f(){let a=&D{v:1};let b=&mut E{v:2};let n=a.v;b.v=n;} struct D{v:i32} impl Drop for D{fn drop(&mut self){self.v=0;}} struct E{v:i32} impl Drop for E{fn drop(&mut self){self.v=0;}} fn main(){}";
const EXACT_BORROWED: &str = r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn main() {
    let borrowed = &Guard { n: 7 };
    println!("value {}", borrowed.n);
    println!("end");
}
"#;

#[test]
fn abstract_scope_events_follow_owned_fields_not_reference_pointees() {
    let budget = ir::PlanBudget::default();
    let publication = publish(
        "fn f<T>(value:T,borrowed:&T,raw:*const T,wrapped:Wrap<T>){}struct Wrap<T>{value:T}fn main(){}",
        &budget,
    );
    let facts = publication.facts();
    for (index, expected) in [true, false, false, true].into_iter().enumerate() {
        let ty = &publication.source().locals[publication.source().params[index].0].ty;
        assert_eq!(abstract_scope::required(ty, facts).unwrap(), expected);
        assert!(
            descriptor(ty, ir::FactsCtx::from_validated(facts))
                .unwrap()
                .is_none()
        );
    }
    let template = lower_source_template(&publication, &budget).unwrap();
    template
        .function()
        .validate_source_publication(&publication, &budget)
        .unwrap();
}

#[test]
fn direct_let_borrowed_owners_have_lexical_storage_and_canonical_cleanup() {
    let budget = ir::PlanBudget::default();
    let publication = publish(BORROWED, &budget);
    let function = lower_source_closed(&publication, &budget).unwrap();
    let owners: Vec<_> = function
        .locals
        .iter()
        .enumerate()
        .filter(|(_, local)| local.name == "$borrow_temporary")
        .map(|(index, _)| LocalId(index))
        .collect();
    assert_eq!(owners.len(), 2);
    for owner in owners {
        let statements: Vec<_> = function.blocks.iter().flat_map(|b| &b.statements).collect();
        assert_eq!(
            statements
                .iter()
                .filter(|s| matches!(s.value, Statement::StorageLive(local) if local == owner))
                .count(),
            1
        );
        assert_eq!(
            statements
                .iter()
                .filter(|s| matches!(s.value, Statement::StorageDead(local) if local == owner))
                .count(),
            1
        );
        assert_eq!(function.blocks.iter().filter(|b| matches!(&b.terminator.value, Terminator::Drop {cleanup, ..} if cleanup.owner() == &place(owner))).count(), 1);
        assert_eq!(
            function
                .loan_sites
                .iter()
                .filter(|site| site.place.local == owner)
                .count(),
            1
        );
    }
    let concrete = ownership::analyze_closed(
        close(Arc::new(function), publication.facts().clone(), &budget).unwrap(),
    )
    .unwrap();
    assert!(concrete.analysis().passed());
    reject_cleanup_forgery(BORROWED, false);
}

#[test]
fn direct_let_borrowed_owner_rejects_typed_storage_and_initializer_transplants() {
    for text in [BORROWED, EXACT_BORROWED] {
        let budget = ir::PlanBudget::default();
        let publication = publish(text, &budget);
        let source = publication.source();
        crate::compiler::source_replay::replay_original_calls(
            source.clone(),
            publication.original(),
            publication.facts(),
            &budget,
        )
        .unwrap();
        let owners: Vec<_> = source
            .body
            .statements
            .iter()
            .enumerate()
            .filter_map(|(index, statement)| match statement {
                ir::TypedStmt::Let { local, .. }
                    if source.locals[local.0].name == "$borrow_temporary" =>
                {
                    Some(index)
                }
                _ => None,
            })
            .collect();
        assert_eq!(owners.len(), if text == BORROWED { 2 } else { 1 });
        for mutation in 0..4 {
            let mut forged = source.as_ref().clone();
            match mutation {
                0 => {
                    let ir::TypedStmt::Let { local, .. } = forged.body.statements[owners[0]] else {
                        unreachable!()
                    };
                    forged.body.statements[owners[0]] = ir::TypedStmt::Declare { local };
                }
                1 => forged.body.statements.swap(owners[0], owners[0] + 1),
                2 => {
                    let ir::TypedStmt::Let { value: other, .. } =
                        forged.body.statements[owners[0] + 1]
                    else {
                        unreachable!()
                    };
                    let ir::TypedStmt::Let { value, .. } = &mut forged.body.statements[owners[0]]
                    else {
                        unreachable!()
                    };
                    *value = other;
                }
                _ => {
                    let owner = forged.body.statements.remove(owners[0]);
                    let ir::TypedStmt::Let { value, .. } = &mut forged.body.statements[owners[0]]
                    else {
                        unreachable!()
                    };
                    let inner = *value;
                    *value = ir::ExprId(forged.exprs.len());
                    let mut wrapper = forged.exprs[inner.0].clone();
                    wrapper.kind = ir::ExprKind::Block(ir::TypedBlock {
                        statements: vec![owner],
                        tail: Some(inner),
                    });
                    forged.exprs.push(wrapper);
                }
            }
            let forged = Arc::new(forged);
            assert!(
                publication
                    .validate_source(&forged, publication.facts(), &budget)
                    .is_err(),
                "typed mutation {mutation} reused exact immutable source authority"
            );
            // Retained-call replay is not source authentication. Even if its
            // narrower correspondence checks pass, it cannot mint publication
            // authority for this supplied replacement arena.
        }
        let foreign_budget = ir::PlanBudget::default();
        let foreign = publish(text, &foreign_budget);
        assert!(
            crate::compiler::source_replay::replay_original_calls(
                source.clone(),
                foreign.original(),
                publication.facts(),
                &budget,
            )
            .is_err()
        );
        let function = lower_source_closed(&publication, &budget).unwrap();
        let mut forged = function.clone();
        let block = forged.blocks.iter_mut().find(|block| matches!(&block.terminator.value, Terminator::Drop {cleanup, ..} if forged.locals[cleanup.owner().local.0].name == "$borrow_temporary")).unwrap();
        let Terminator::Drop { next, .. } = block.terminator.value else {
            unreachable!()
        };
        block.terminator.value = Terminator::Goto(next);
        assert!(
            forged
                .validate_source_publication(&publication, &budget)
                .is_err()
        );
        assert!(close(Arc::new(forged), publication.facts().clone(), &budget).is_err());
    }
}

#[test]
fn structural_tuple_destructuring_retains_move_and_cleanup_proof() {
    for pattern in ["(left, right)", "(left, _)", "(_, right)"] {
        let text = format!(
            "fn f(){{let pair=(D{{v:1}},D{{v:2}});let {pattern}=pair;}} struct D{{v:i32}} impl Drop for D{{fn drop(&mut self){{}}}} fn main(){{}}"
        );
        let budget = ir::PlanBudget::default();
        let publication = publish(&text, &budget);
        let function = lower_source_closed(&publication, &budget).unwrap();
        ownership::analyze_closed(
            close(
                Arc::new(function.clone()),
                publication.facts().clone(),
                &budget,
            )
            .unwrap(),
        )
        .unwrap();

        let mut forged = function;
        let operand = forged
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match &mut statement.value {
                Statement::Assign {
                    value: Rvalue::Use(operand),
                    ..
                } => {
                    let selected = matches!(
                        &*operand,
                        Operand::Move(place) if !place.projection.is_empty()
                    );
                    selected.then_some(operand)
                }
                _ => None,
            })
            .expect("tuple binding must move a selected field");
        let Operand::Move(selected) = operand else {
            unreachable!()
        };
        *operand = Operand::Copy(selected.clone());
        assert!(
            forged
                .validate_source_publication(&publication, &budget)
                .is_err()
        );
        assert!(close(Arc::new(forged), publication.facts().clone(), &budget).is_err());
    }
}

#[test]
fn structural_tuple_admission_does_not_allow_own_drop_destructuring() {
    let text = "fn f(){let owner=Owner{child:D{v:1}};let Owner{child}=owner;} struct D{v:i32} impl Drop for D{fn drop(&mut self){}} struct Owner{child:D} impl Drop for Owner{fn drop(&mut self){}} fn main(){}";
    let budget = ir::PlanBudget::default();
    let program = frontend::parse(text).unwrap();
    let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    assert!(
        context
            .publish_callable_body(context.function_decl(0).unwrap(), &budget)
            .is_err(),
        "an owner's own Drop must still prohibit extracting its non-Copy field"
    );
}

#[test]
fn own_drop_enum_borrow_copy_and_wildcard_are_not_e0509() {
    for body in [
        "match e{E::One(ref d)=>()}",
        "match e{E::One(_)=>()}",
        "match e{_=>()}",
    ] {
        let text = format!(
            "fn f(){{let e=E::One(D{{v:1}});{body};}} struct D{{v:i32}} impl Drop for D{{fn drop(&mut self){{}}}} enum E{{One(D)}} impl Drop for E{{fn drop(&mut self){{}}}} fn main(){{}}"
        );
        let budget = ir::PlanBudget::default();
        let publication = publish(&text, &budget);
        if let Err(error) = lower_source_closed(&publication, &budget) {
            assert!(!error.to_string().contains("E0509"), "{body}: {error}");
            assert!(error.to_string().contains("unsupported"), "{body}: {error}");
        }
    }
    let budget = ir::PlanBudget::default();
    let publication = publish(
        "fn f(){let e=E::One(1);match e{E::One(n)=>()};} enum E{One(i32)} impl Drop for E{fn drop(&mut self){}} fn main(){}",
        &budget,
    );
    if let Err(error) = lower_source_closed(&publication, &budget) {
        assert!(!error.to_string().contains("E0509"), "{error}");
        assert!(error.to_string().contains("unsupported"), "{error}");
    }
}

#[test]
fn normal_drop_implicit_mutable_access_conflicts_with_a_live_field_loan() {
    let budget = ir::PlanBudget::default();
    let publication = publish(
        "fn f(){let d=D{v:1};let r=&d.v;let n=*r;}struct D{v:i32}impl Drop for D{fn drop(&mut self){self.v=0;}}fn main(){}",
        &budget,
    );
    let mut function = lower_source_closed(&publication, &budget).unwrap();
    let root = LocalId(
        publication
            .source()
            .locals
            .iter()
            .position(|l| l.name == "d")
            .unwrap(),
    );
    let cleanup = function
        .blocks
        .iter()
        .find_map(|block| match &block.terminator.value {
            Terminator::Drop { cleanup, .. } if cleanup.owner().local == root => {
                Some(cleanup.clone())
            }
            _ => None,
        })
        .unwrap();
    let (block, statement) = function
        .blocks
        .iter()
        .enumerate()
        .find_map(|(b, block)| {
            block
                .statements
                .iter()
                .position(|s| {
                    matches!(
                        s.value,
                        Statement::Assign {
                            value: Rvalue::Borrow(_),
                            ..
                        }
                    )
                })
                .map(|s| (b, s))
        })
        .unwrap();
    // Simulate a faulty but shape-certified canonical lowering. The ordinary
    // analyzer, not just the exact-shape guard, must enforce the implicit use.
    let suffix = function.blocks[block].statements.split_off(statement + 1);
    let next = BlockId(function.blocks.len());
    let original = std::mem::replace(
        &mut function.blocks[block].terminator,
        Located {
            value: Terminator::Drop { cleanup, next },
            origin: None,
        },
    );
    function.blocks.push(BasicBlock {
        statements: suffix,
        terminator: original,
    });
    function.lowering_proof = None;
    function.lowering_proof = Some(Arc::new(
        GeneratedCfgShape::capture_with_publication(
            &function,
            publication.facts(),
            Some(&publication),
            &budget,
        )
        .unwrap(),
    ));
    let closed = close(Arc::new(function), publication.facts().clone(), &budget).unwrap();
    let error = ownership::analyze_closed(closed).unwrap_err();
    assert!(
        error
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, ownership::ErrorKind::BorrowConflict(_))),
        "{error:?}"
    );
}

#[test]
fn normal_drop_exact_shape_rejects_omission_transplant_move_and_stale_proof() {
    reject_cleanup_forgery(TWO, false);
}

#[test]
fn enum_drop_exact_shape_rejects_cleanup_and_move_flag_forgery() {
    reject_cleanup_forgery(
        "fn f(){let e=E::Pair(D{v:1},D{v:2});match e{E::Pair(a,_)=>{let b=a;},E::Empty=>()};let other=D{v:3};} enum E{Pair(D,D),Empty} struct D{v:i32} impl Drop for D{fn drop(&mut self){self.v=0;}} fn main(){}",
        true,
    );
}

fn reject_cleanup_forgery(text: &str, enum_moves: bool) {
    let budget = ir::PlanBudget::default();
    let publication = publish(text, &budget);
    let function = lower_source_closed(&publication, &budget).unwrap();
    let sites: Vec<_> = function
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match &b.terminator.value {
            Terminator::Drop { cleanup, next } => Some((i, cleanup.clone(), *next)),
            _ => None,
        })
        .collect();
    assert!(sites.len() >= 2);
    let facts = publication.facts();
    let concrete = ownership::analyze_closed(
        close(Arc::new(function.clone()), facts.clone(), &budget).unwrap(),
    )
    .unwrap();
    assert!(concrete.analysis().passed());
    for change in 0..if enum_moves { 7 } else { 6 } {
        let mut forged = function.clone();
        let (block, cleanup, next) = &sites[0];
        match change {
            0 => forged.blocks[*block].terminator.value = Terminator::Goto(*next),
            1 => {
                let different = sites
                    .iter()
                    .find(|(_, other, _)| other.owner() != cleanup.owner())
                    .unwrap();
                let mut changed = cleanup.as_ref().clone();
                changed.owner = different.1.owner().clone();
                forged.blocks[*block].terminator.value = Terminator::Drop {
                    cleanup: Arc::new(changed),
                    next: *next,
                };
            }
            2 => forged.lowering_proof = None,
            3 => {
                let owner = cleanup.owner().local;
                let statements = forged
                    .blocks
                    .iter_mut()
                    .find_map(|block| {
                        block.statements.iter().position(|statement| {
                        matches!(statement.value, Statement::StorageLive(local) if local == owner)
                    }).map(|index| (&mut block.statements, index))
                    })
                    .unwrap();
                statements.0.remove(statements.1);
            }
            4 => {
                let other = sites.iter().find(|(index, _, _)| index != block).unwrap();
                forged.blocks[*block].terminator.value = Terminator::Drop {
                    cleanup: cleanup.clone(),
                    next: other.2,
                };
            }
            6 => {
                let operand = forged
                    .blocks
                    .iter_mut()
                    .flat_map(|b| &mut b.statements)
                    .find_map(|s| match &mut s.value {
                        Statement::Assign {
                            value: Rvalue::Use(Operand::Move(owner)),
                            ..
                        } if owner
                            .projection
                            .iter()
                            .any(|p| matches!(p, ir::Projection::Downcast(_))) =>
                        {
                            Some(owner)
                        }
                        _ => None,
                    })
                    .unwrap();
                let field = operand
                    .projection
                    .iter_mut()
                    .find_map(|p| match p {
                        ir::Projection::Field(index) => Some(index),
                        _ => None,
                    })
                    .unwrap();
                *field = 1;
            }
            _ => {
                let statement = forged
                    .blocks
                    .iter_mut()
                    .flat_map(|b| &mut b.statements)
                    .find(|s| {
                        matches!(
                            &s.value,
                            Statement::Assign {
                                value: Rvalue::Use(Operand::Move(_)),
                                ..
                            }
                        )
                    })
                    .unwrap();
                let Statement::Assign {
                    value: Rvalue::Use(operand),
                    ..
                } = &mut statement.value
                else {
                    unreachable!()
                };
                let Operand::Move(owner) = operand else {
                    unreachable!()
                };
                *operand = Operand::Copy(owner.clone());
            }
        }
        assert!(
            forged
                .validate_source_publication(&publication, &budget)
                .is_err(),
            "mutation {change} bypassed exact source CFG proof"
        );
        assert!(
            close(Arc::new(forged), facts.clone(), &budget).is_err(),
            "mutation {change} acquired a closed seal"
        );
    }
}

#[test]
fn normal_drop_foreign_world_cleanup_is_rejected() {
    for text in [TWO, BORROWED] {
        let budget = ir::PlanBudget::default();
        let publication = publish(text, &budget);
        let foreign = publish(text, &ir::PlanBudget::default());
        let function = lower_source_closed(&publication, &budget).unwrap();
        let other = lower_source_closed(&foreign, foreign.facts().budget()).unwrap();
        let foreign_cleanup = other
            .blocks
            .iter()
            .find_map(|b| match &b.terminator.value {
                Terminator::Drop { cleanup, .. } => Some(cleanup.clone()),
                _ => None,
            })
            .unwrap();
        let mut forged = function.clone();
        let target = forged
            .blocks
            .iter_mut()
            .find_map(|b| match &mut b.terminator.value {
                Terminator::Drop { cleanup, .. } => Some(cleanup),
                _ => None,
            })
            .unwrap();
        *target = foreign_cleanup;
        assert!(close(Arc::new(forged), publication.facts().clone(), &budget).is_err());
    }
}

#[test]
fn normal_drop_scope_expansion_exhaustion_never_publishes_a_cfg() {
    let budget = ir::PlanBudget::default();
    let publication = publish(TWO, &budget);
    let remaining = budget.limits().work - budget.usage_snapshot().work;
    budget.charge_work(remaining).unwrap();
    assert!(lower_source_closed(&publication, &budget).is_err());
    let usage = budget.usage_snapshot();
    assert!(usage.first_failure.is_some());
    assert!(!usage.poisoned);
}

#[test]
fn normal_drop_conditional_move_and_reinitialization_pass_complete_analysis() {
    for body in [
        "let d=D{v:1};if b{take(d);}",
        "let mut d=D{v:1};let moved=d;d=D{v:2};d=d;",
        "let d:D;if b{d=D{v:1};}",
        "let a=[1,2];let i:usize=0;let d=D{v:1};let _=a[i];let e=D{v:2};",
        "let a=[1,2];let i:usize=0;let d=D{v:1};{let _=a[i];let e=D{v:2};}let f=D{v:3};",
        "let a=[1,2];let i:usize=0;let d=D{v:1};loop{let _=a[i];let e=D{v:2};break;}",
    ] {
        let text = format!(
            "fn f(b:bool){{{body}}} struct D{{v:i32}} impl Drop for D{{fn drop(&mut self){{self.v=0;}}}} fn take(d:D){{}} fn main(){{}}"
        );
        let budget = ir::PlanBudget::default();
        let publication = publish(&text, &budget);
        let function = lower_source_closed(&publication, &budget).unwrap();
        ownership::analyze_closed(
            close(Arc::new(function), publication.facts().clone(), &budget).unwrap(),
        )
        .unwrap();
    }
}
