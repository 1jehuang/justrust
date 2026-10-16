//! Independent synthetic CFG ownership tests. Explicit Move operands exercise
//! ownership even though every currently source-admitted value is Copy.
use cfg::{BasicBlock, BlockId, Constant, Located, Operand, Rvalue, Statement, Terminator};
use ir::{LocalDecl, LocalId, Place, Signature, Ty};
use justrust_engine::compiler::ownership::ErrorKind;
use justrust_engine::compiler::{cfg, ir, ownership};

fn p(local: usize, fields: &[usize]) -> Place {
    Place {
        local: LocalId(local),
        projection: fields.iter().copied().map(ir::Projection::Field).collect(),
    }
}
fn loc<T>(value: T) -> Located<T> {
    Located {
        value,
        origin: None,
    }
}
fn live(local: usize) -> Statement {
    Statement::StorageLive(LocalId(local))
}
fn set(local: usize, fields: &[usize], value: Operand) -> Statement {
    Statement::Assign {
        destination: p(local, fields),
        value: Rvalue::Use(value),
    }
}
fn int(value: i32) -> Operand {
    Operand::Constant(Constant::I32(value))
}
fn copy(local: usize, fields: &[usize]) -> Operand {
    Operand::Copy(p(local, fields))
}
fn take(local: usize, fields: &[usize]) -> Operand {
    Operand::Move(p(local, fields))
}
fn block(statements: Vec<Statement>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        statements: statements.into_iter().map(loc).collect(),
        terminator: loc(terminator),
    }
}
fn tuple(fields: Vec<Ty>) -> Ty {
    Ty::Tuple(fields.into())
}
fn function(decls: Vec<(Ty, bool)>, blocks: Vec<BasicBlock>) -> cfg::Function {
    let mut locals = vec![LocalDecl {
        name: "$return".into(),
        ty: Ty::Unit,
        mutable: true,
    }];
    locals.extend(
        decls
            .into_iter()
            .enumerate()
            .map(|(i, (ty, mutable))| LocalDecl {
                name: format!("x{i}"),
                ty,
                mutable,
            }),
    );
    let mut function = cfg::Function {
        name: "synthetic".into(),
        params: vec![],
        result: Ty::Unit,
        locals,
        blocks,
        entry: BlockId(0),
        return_local: LocalId(0),
    };
    function.blocks[0].statements.splice(
        0..0,
        [
            loc(live(0)),
            loc(set(0, &[], Operand::Constant(Constant::Unit))),
        ],
    );
    function
}
fn analyze(function: &cfg::Function) -> ownership::Analysis {
    function
        .validate()
        .expect("test fixture must be structurally valid");
    ownership::analyze(function)
}
fn accepts(function: &cfg::Function) {
    let analysis = analyze(function);
    assert!(
        analysis.passed(),
        "unexpected ownership diagnostics: {:?}",
        analysis.diagnostics
    );
}
fn rejects(function: &cfg::Function, kind: ErrorKind) {
    let analysis = analyze(function);
    assert!(!analysis.passed(), "invalid ownership accepted");
    assert!(
        analysis.diagnostics.iter().any(|d| d.kind == kind),
        "expected {kind:?}, got {:?}",
        analysis.diagnostics
    );
}
fn branch(yes: usize, no: usize) -> Terminator {
    Terminator::Switch {
        condition: Operand::Constant(Constant::Bool(true)),
        then_block: BlockId(yes),
        else_block: BlockId(no),
    }
}

#[test]
fn uninitialized_read_is_rejected_but_first_assignment_is_allowed() {
    let mut f = function(
        vec![(Ty::I32, false), (Ty::I32, true)],
        vec![block(
            vec![live(1), live(2), set(2, &[], copy(1, &[]))],
            Terminator::Return,
        )],
    );
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    f.blocks[0].statements.insert(4, loc(set(1, &[], int(7))));
    accepts(&f);
}

#[test]
fn whole_move_invalidates_source_not_destination_and_copy_does_not_move() {
    let mut f = function(
        vec![(Ty::I32, true), (Ty::I32, true)],
        vec![block(
            vec![
                live(1),
                live(2),
                set(1, &[], int(7)),
                set(2, &[], take(1, &[])),
                set(2, &[], copy(2, &[])),
            ],
            Terminator::Return,
        )],
    );
    accepts(&f);
    f.blocks[0].statements.push(loc(set(2, &[], copy(1, &[]))));
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    f.blocks[0].statements[5] = loc(set(2, &[], copy(1, &[])));
    accepts(&f);
}

#[test]
fn partial_move_preserves_sibling_but_invalidates_ancestor_until_reinitialized() {
    let pair = tuple(vec![Ty::I32, Ty::I32]);
    let mut f = function(
        vec![(pair.clone(), true), (Ty::I32, true), (pair, true)],
        vec![block(
            vec![
                live(2),
                live(3),
                set(2, &[], take(1, &[0])),
                set(2, &[], copy(1, &[1])),
            ],
            Terminator::Return,
        )],
    );
    f.params = vec![LocalId(1)];
    accepts(&f);
    f.blocks[0].statements.push(loc(set(3, &[], copy(1, &[]))));
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    let end = f.blocks[0].statements.len() - 1;
    f.blocks[0]
        .statements
        .insert(end, loc(set(1, &[0], int(9))));
    accepts(&f);
}

#[test]
fn moved_field_itself_cannot_be_read_twice() {
    let mut f = function(
        vec![(tuple(vec![Ty::I32, Ty::I32]), true), (Ty::I32, true)],
        vec![block(
            vec![
                live(2),
                set(2, &[], take(1, &[0])),
                set(2, &[], take(1, &[0])),
            ],
            Terminator::Return,
        )],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
}

#[test]
fn field_write_requires_shallow_ancestor_initialization() {
    let pair = tuple(vec![Ty::I32, Ty::I32]);
    let mut f = function(
        vec![(pair, true)],
        vec![block(
            vec![live(1), set(1, &[0], int(1)), set(1, &[1], int(2))],
            Terminator::Return,
        )],
    );
    rejects(&f, ErrorKind::WriteIntoUninitializedAggregate);
    // A parameter supplies the shallow initialization required by field writes.
    f.params = vec![LocalId(1)];
    f.blocks[0].statements.remove(2);
    accepts(&f);
}

#[test]
fn nested_move_reinitializes_only_the_written_subtree() {
    let nested = tuple(vec![tuple(vec![Ty::I32, Ty::I32]), Ty::I32]);
    let mut f = function(
        vec![(nested, true), (Ty::I32, true)],
        vec![block(
            vec![
                live(2),
                set(2, &[], take(1, &[0, 0])),
                set(2, &[], take(1, &[0, 1])),
                set(1, &[0, 0], int(1)),
                set(2, &[], copy(1, &[1])),
                set(2, &[], copy(1, &[0, 1])),
            ],
            Terminator::Return,
        )],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    let end = f.blocks[0].statements.len() - 1;
    f.blocks[0]
        .statements
        .insert(end, loc(set(1, &[0, 1], int(2))));
    accepts(&f);
}

#[test]
fn immutable_binding_cannot_be_reassigned_after_move() {
    let f = function(
        vec![(Ty::I32, false), (Ty::I32, true)],
        vec![block(
            vec![
                live(1),
                live(2),
                set(1, &[], int(1)),
                set(2, &[], take(1, &[])),
                set(1, &[], int(2)),
            ],
            Terminator::Return,
        )],
    );
    rejects(&f, ErrorKind::ImmutableReassignment);
}

#[test]
fn branch_join_requires_initialization_on_every_predecessor() {
    let mut f = function(
        vec![(Ty::I32, false), (Ty::I32, true)],
        vec![
            block(vec![live(1), live(2)], branch(1, 2)),
            block(vec![set(1, &[], int(1))], Terminator::Goto(BlockId(3))),
            block(vec![], Terminator::Goto(BlockId(3))),
            block(vec![set(2, &[], copy(1, &[]))], Terminator::Return),
        ],
    );
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    f.blocks[2].statements.push(loc(set(1, &[], int(2))));
    accepts(&f);
}

#[test]
fn move_on_one_branch_invalidates_join_read() {
    let mut f = function(
        vec![(Ty::I32, true), (Ty::I32, true)],
        vec![
            block(vec![live(2)], branch(1, 2)),
            block(
                vec![set(2, &[], take(1, &[]))],
                Terminator::Goto(BlockId(3)),
            ),
            block(vec![], Terminator::Goto(BlockId(3))),
            block(vec![set(2, &[], copy(1, &[]))], Terminator::Return),
        ],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    f.blocks[1].statements.push(loc(set(1, &[], int(2))));
    accepts(&f);
}

#[test]
fn loop_backedge_propagates_move_to_later_iterations() {
    let mut f = function(
        vec![(Ty::I32, true), (Ty::I32, true)],
        vec![
            block(vec![live(2)], Terminator::Goto(BlockId(1))),
            block(vec![set(2, &[], take(1, &[]))], branch(1, 2)),
            block(vec![], Terminator::Return),
        ],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    f.blocks[1].statements.push(loc(set(1, &[], int(2))));
    accepts(&f);
}

#[test]
fn zero_iteration_loop_cannot_definitely_initialize_exit() {
    let f = function(
        vec![(Ty::I32, true), (Ty::I32, true)],
        vec![
            block(vec![live(1), live(2)], Terminator::Goto(BlockId(1))),
            block(vec![], branch(2, 3)),
            block(vec![set(1, &[], int(1))], Terminator::Goto(BlockId(1))),
            block(vec![set(2, &[], copy(1, &[]))], Terminator::Return),
        ],
    );
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
}

#[test]
fn storage_live_resets_initialization_and_immutable_assignment_history() {
    let mut f = function(
        vec![(Ty::I32, false), (Ty::I32, true)],
        vec![block(
            vec![
                live(1),
                live(2),
                set(1, &[], int(1)),
                Statement::StorageDead(LocalId(1)),
                live(1),
                set(2, &[], copy(1, &[])),
            ],
            Terminator::Return,
        )],
    );
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
    let end = f.blocks[0].statements.len() - 1;
    f.blocks[0].statements.insert(end, loc(set(1, &[], int(2))));
    accepts(&f);
}

#[test]
fn immutable_loop_local_can_be_initialized_each_dynamic_lifetime() {
    let f = function(
        vec![(Ty::I32, false)],
        vec![
            block(vec![], Terminator::Goto(BlockId(1))),
            block(
                vec![
                    live(1),
                    set(1, &[], int(1)),
                    Statement::StorageDead(LocalId(1)),
                ],
                branch(1, 2),
            ),
            block(vec![], Terminator::Return),
        ],
    );
    accepts(&f);
}

#[test]
fn duplicate_storage_live_cannot_launder_immutable_assignment_history() {
    let f = function(
        vec![(Ty::I32, false)],
        vec![block(
            vec![live(1), set(1, &[], int(1)), live(1), set(1, &[], int(2))],
            Terminator::Return,
        )],
    );
    rejects(&f, ErrorKind::StorageAlreadyLive);
}

#[test]
fn call_arguments_move_before_destination_is_initialized() {
    let mut f = function(
        vec![(Ty::I32, true)],
        vec![
            block(
                vec![],
                Terminator::Call {
                    name: "identity".into(),
                    signature: Signature {
                        params: vec![Ty::I32],
                        result: Ty::I32,
                    },
                    args: vec![take(1, &[])],
                    destination: p(1, &[]),
                    next: BlockId(1),
                },
            ),
            block(vec![], Terminator::Return),
        ],
    );
    f.params = vec![LocalId(1)];
    accepts(&f);
    f.blocks[1].statements.push(loc(set(1, &[], copy(1, &[]))));
    accepts(&f);
    if let Terminator::Call {
        signature, args, ..
    } = &mut f.blocks[0].terminator.value
    {
        signature.params.push(Ty::I32);
        args.push(copy(1, &[]));
    }
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
}

#[test]
fn call_cannot_move_the_same_argument_twice() {
    let mut f = function(
        vec![(Ty::I32, true)],
        vec![
            block(
                vec![],
                Terminator::Call {
                    name: "pair".into(),
                    signature: Signature {
                        params: vec![Ty::I32, Ty::I32],
                        result: Ty::I32,
                    },
                    args: vec![take(1, &[]), take(1, &[])],
                    destination: p(1, &[]),
                    next: BlockId(1),
                },
            ),
            block(vec![], Terminator::Return),
        ],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
}

#[test]
fn unreachable_ownership_errors_are_ignored_but_malformed_cfg_is_rejected() {
    let mut f = function(
        vec![(Ty::I32, true), (Ty::I32, true)],
        vec![
            block(vec![], Terminator::Return),
            block(vec![set(2, &[], copy(1, &[]))], Terminator::Unreachable),
        ],
    );
    accepts(&f);
    f.blocks[1].terminator = loc(Terminator::Goto(BlockId(99)));
    assert!(f.validate().is_err());
    let analysis = ownership::analyze(&f);
    assert!(!analysis.passed());
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, ErrorKind::InvalidCfg(_))),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn whole_move_removes_shallow_initialization_for_field_writes() {
    let pair = tuple(vec![Ty::I32, Ty::I32]);
    let mut f = function(
        vec![(pair.clone(), true), (pair, true)],
        vec![block(
            vec![live(2), set(2, &[], take(1, &[])), set(1, &[0], int(7))],
            Terminator::Return,
        )],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::WriteIntoUninitializedAggregate);
}

#[test]
fn immutable_history_is_joined_even_when_value_is_moved() {
    let f = function(
        vec![(Ty::I32, false), (Ty::I32, true)],
        vec![
            block(vec![live(1), live(2)], branch(1, 2)),
            block(
                vec![set(1, &[], int(1)), set(2, &[], take(1, &[]))],
                Terminator::Goto(BlockId(3)),
            ),
            block(vec![], Terminator::Goto(BlockId(3))),
            block(vec![set(1, &[], int(2))], Terminator::Return),
        ],
    );
    rejects(&f, ErrorKind::ImmutableReassignment);
}

#[test]
fn nested_field_write_requires_every_shallow_ancestor() {
    let pair = tuple(vec![Ty::I32, Ty::I32]);
    let outer = tuple(vec![pair.clone(), Ty::I32]);
    let mut f = function(
        vec![(outer, true), (pair, true)],
        vec![block(
            vec![live(2), set(2, &[], take(1, &[0])), set(1, &[0, 0], int(1))],
            Terminator::Return,
        )],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::WriteIntoUninitializedAggregate);
    f.blocks[0].statements[4] = loc(set(1, &[0], copy(2, &[])));
    accepts(&f);
}

#[test]
fn diagnostic_preserves_read_location_origin_and_place() {
    let mut f = function(
        vec![(Ty::I32, true), (Ty::I32, true)],
        vec![block(
            vec![live(1), live(2), set(2, &[], copy(1, &[]))],
            Terminator::Return,
        )],
    );
    f.blocks[0].statements[4].origin = Some(ir::ExprId(42));
    let analysis = analyze(&f);
    let diagnostic = analysis
        .diagnostics
        .iter()
        .find(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
        .expect("uninitialized read diagnostic");
    assert_eq!(diagnostic.block, BlockId(0));
    assert_eq!(diagnostic.statement, Some(4));
    assert_eq!(diagnostic.origin, Some(ir::ExprId(42)));
    assert_eq!(diagnostic.place, Some(p(1, &[])));
}

#[test]
fn acyclic_branch_analysis_matches_concrete_path_oracle() {
    // Enumerate actual paths, not a joined dataflow lattice. The four branch
    // programs are empty, initialize, move, and initialize-then-move.
    fn path(branch: usize, overwrite: bool) -> bool {
        let mut initialized = false;
        let mut written = false;
        if branch & 1 != 0 {
            initialized = true;
            written = true;
        }
        if branch & 2 != 0 {
            if !initialized {
                return false;
            }
            initialized = false;
        }
        if overwrite {
            if written {
                return false;
            }
            initialized = true;
        }
        initialized
    }
    for yes in 0..4 {
        for no in 0..4 {
            for overwrite in [false, true] {
                let branch_statements = |program: usize| {
                    let mut statements = vec![];
                    if program & 1 != 0 {
                        statements.push(set(1, &[], int(1)));
                    }
                    if program & 2 != 0 {
                        statements.push(set(2, &[], take(1, &[])));
                    }
                    statements
                };
                let mut join = vec![];
                if overwrite {
                    join.push(set(1, &[], int(2)));
                }
                join.push(set(2, &[], copy(1, &[])));
                let f = function(
                    vec![(Ty::I32, false), (Ty::I32, true)],
                    vec![
                        block(vec![live(1), live(2)], branch(1, 2)),
                        block(branch_statements(yes), Terminator::Goto(BlockId(3))),
                        block(branch_statements(no), Terminator::Goto(BlockId(3))),
                        block(join, Terminator::Return),
                    ],
                );
                let analysis = analyze(&f);
                assert_eq!(
                    analysis.passed(),
                    path(yes, overwrite) && path(no, overwrite),
                    "yes={yes}, no={no}, overwrite={overwrite}: {:?}",
                    analysis.diagnostics
                );
            }
        }
    }
}

#[test]
fn aggregate_operands_are_consumed_in_order() {
    let pair = tuple(vec![Ty::I32, Ty::I32]);
    let mut f = function(
        vec![(Ty::I32, true), (pair.clone(), true)],
        vec![block(
            vec![
                live(2),
                Statement::Assign {
                    destination: p(2, &[]),
                    value: Rvalue::Aggregate {
                        ty: pair,
                        fields: vec![take(1, &[]), take(1, &[])],
                    },
                },
            ],
            Terminator::Return,
        )],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
}

#[test]
fn immutable_aggregate_field_assignment_is_reassignment() {
    let mut f = function(
        vec![(tuple(vec![Ty::I32, Ty::I32]), false)],
        vec![block(vec![set(1, &[0], int(1))], Terminator::Return)],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ImmutableReassignment);
}

#[test]
fn entry_backedge_must_not_reinitialize_parameters() {
    let mut f = function(
        vec![(Ty::I32, true), (Ty::I32, true)],
        vec![
            block(
                vec![
                    live(2),
                    set(2, &[], take(1, &[])),
                    Statement::StorageDead(LocalId(2)),
                    Statement::StorageDead(LocalId(0)),
                ],
                branch(0, 1),
            ),
            block(vec![], Terminator::Unreachable),
        ],
    );
    f.params = vec![LocalId(1)];
    rejects(&f, ErrorKind::ReadNotDefinitelyInitialized);
}
