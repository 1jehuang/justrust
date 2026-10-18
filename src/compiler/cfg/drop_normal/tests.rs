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
    let budget = ir::PlanBudget::default();
    let publication = publish(TWO, &budget);
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
    for change in 0..6 {
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
    let budget = ir::PlanBudget::default();
    let publication = publish(TWO, &budget);
    let foreign = publish(TWO, &ir::PlanBudget::default());
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
