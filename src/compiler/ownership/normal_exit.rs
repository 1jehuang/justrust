//! Conservative, ephemeral cleanup discharge for an already sealed physical CFG.
//! This does not grant move, borrow, call, or destructor authority. The ordinary
//! physical ownership verifier must still accept the same function afterward.
use crate::compiler::{cfg, ir};

pub(super) fn fully_consumed(
    function: &cfg::Function,
    local: ir::LocalId,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    if local.0 >= function.locals.len() {
        return Err("physical cleanup proof has an invalid local".into());
    }
    budget.charge_work(function.params.len())?;
    solve(
        &function.blocks,
        function.entry,
        function.params.contains(&local),
        function.return_local == local,
        local,
        budget,
    )
}

// 0 is unreachable, 1 reachable/uninitialized, 2 reachable/may-initialized.
// Entries only increase. Each block becomes reachable once and may-initialized
// once, so no arbitrary iteration ceiling or whole-program budget is needed.
fn solve(
    blocks: &[cfg::BasicBlock],
    entry: cfg::BlockId,
    parameter: bool,
    returned: bool,
    local: ir::LocalId,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    budget.charge_work(1)?;
    if entry.0 >= blocks.len() {
        return Err("physical cleanup proof has an invalid entry".into());
    }
    budget.charge_work(blocks.len())?;
    let _storage = budget.reserve_bytes(blocks.len())?;
    let mut entries = Vec::<u8>::new();
    entries
        .try_reserve_exact(blocks.len())
        .map_err(|_| "infrastructure: physical cleanup state allocation failed".to_string())?;
    let _extra_storage = budget.reserve_bytes(entries.capacity().saturating_sub(blocks.len()))?;
    entries.resize(blocks.len(), 0);
    entries[entry.0] = if parameter { 2 } else { 1 };
    loop {
        let mut changed = false;
        for (index, block) in blocks.iter().enumerate() {
            budget.charge_work(1)?;
            if entries[index] == 0 {
                continue;
            }
            let mut initialized = entries[index] == 2;
            for statement in &block.statements {
                budget.charge_work(1)?;
                match &statement.value {
                    cfg::Statement::StorageLive(target) | cfg::Statement::StorageDead(target)
                        if *target == local =>
                    {
                        if initialized {
                            return Ok(false);
                        }
                        initialized = false;
                    }
                    cfg::Statement::Assign { destination, value } => {
                        value.visit_operands(&mut |operand| {
                            consume(operand, local, &mut initialized, budget)
                        })?;
                        if !write(destination, local, &mut initialized, budget)? {
                            return Ok(false);
                        }
                    }
                    cfg::Statement::IndexAssignOp {
                        array,
                        index,
                        value,
                        ..
                    }
                    | cfg::Statement::IndexAssign {
                        array,
                        index,
                        value,
                    } => {
                        consume(index, local, &mut initialized, budget)?;
                        consume(value, local, &mut initialized, budget)?;
                        // Indexed assignment never proves a complete owner
                        // consumed, and may overwrite a remaining component.
                        if array.local == local {
                            return Ok(false);
                        }
                    }
                    cfg::Statement::IndexInspect { index, .. } => {
                        consume(index, local, &mut initialized, budget)?;
                    }
                    cfg::Statement::Inspect(_)
                    | cfg::Statement::StorageLive(_)
                    | cfg::Statement::StorageDead(_) => {}
                }
            }
            let successors = match &block.terminator.value {
                cfg::Terminator::Return => {
                    // Return transfers exactly the designated return local.
                    if initialized && !returned {
                        return Ok(false);
                    }
                    [None, None]
                }
                cfg::Terminator::Panic(_) | cfg::Terminator::Unreachable => [None, None],
                cfg::Terminator::Goto(next) => [Some(*next), None],
                cfg::Terminator::Switch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    consume(condition, local, &mut initialized, budget)?;
                    [Some(*then_block), Some(*else_block)]
                }
                cfg::Terminator::SwitchVariant {
                    then_block,
                    else_block,
                    ..
                } => [Some(*then_block), Some(*else_block)],
                cfg::Terminator::Call {
                    args,
                    destination,
                    next,
                    ..
                } => {
                    for operand in args {
                        consume(operand, local, &mut initialized, budget)?;
                    }
                    if !write(destination, local, &mut initialized, budget)? {
                        return Ok(false);
                    }
                    [Some(*next), None]
                }
                cfg::Terminator::Drop { cleanup, next } => {
                    // This helper cannot authorize or replace an introduced
                    // destructor dependency, including a projected cleanup.
                    if cleanup.owner().local == local {
                        return Ok(false);
                    }
                    [Some(*next), None]
                }
            };
            for next in successors.into_iter().flatten() {
                budget.charge_work(1)?;
                let target = entries
                    .get_mut(next.0)
                    .ok_or_else(|| "physical cleanup proof has an invalid successor".to_string())?;
                let value = if initialized { 2 } else { 1 };
                if *target < value {
                    *target = value;
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(true);
        }
    }
}

fn consume(
    operand: &cfg::Operand,
    local: ir::LocalId,
    initialized: &mut bool,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    budget.charge_work(1)?;
    if let cfg::Operand::Move(place) = operand
        && place.local == local
        && place.projection.is_empty()
    {
        *initialized = false;
    }
    Ok(())
}

fn write(
    destination: &ir::Place,
    local: ir::LocalId,
    initialized: &mut bool,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    budget.charge_work(1)?;
    if destination.local != local {
        return Ok(true);
    }
    // Even a partial write cannot establish the absence of a destructor.
    if !destination.projection.is_empty() || *initialized {
        return Ok(false);
    }
    *initialized = true;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn located<T>(value: T) -> cfg::Located<T> {
        cfg::Located {
            value,
            origin: None,
        }
    }
    fn place(index: usize) -> ir::Place {
        ir::Place {
            local: ir::LocalId(index),
            projection: Vec::new(),
        }
    }
    fn moved() -> cfg::Statement {
        cfg::Statement::Assign {
            destination: place(1),
            value: cfg::Rvalue::Use(cfg::Operand::Move(place(0))),
        }
    }
    fn block(statements: Vec<cfg::Statement>, terminator: cfg::Terminator) -> cfg::BasicBlock {
        cfg::BasicBlock {
            statements: statements.into_iter().map(located).collect(),
            terminator: located(terminator),
        }
    }
    fn check(blocks: &[cfg::BasicBlock]) -> bool {
        solve(
            blocks,
            cfg::BlockId(0),
            true,
            false,
            ir::LocalId(0),
            &ir::PlanBudget::default(),
        )
        .unwrap()
    }
    #[test]
    fn opaque_cleanup_requires_consumption_on_each_normal_path() {
        assert!(check(&[block(vec![moved()], cfg::Terminator::Return)]));
        assert!(!check(&[block(vec![], cfg::Terminator::Return)]));
        let mut branches = vec![
            block(
                vec![],
                cfg::Terminator::Switch {
                    condition: cfg::Operand::Constant(cfg::Constant::Bool(true)),
                    then_block: cfg::BlockId(1),
                    else_block: cfg::BlockId(2),
                },
            ),
            block(vec![moved()], cfg::Terminator::Goto(cfg::BlockId(3))),
            block(vec![], cfg::Terminator::Goto(cfg::BlockId(3))),
            block(vec![], cfg::Terminator::Return),
        ];
        assert!(!check(&branches));
        branches[2].statements.push(located(moved()));
        assert!(check(&branches));
        // Unreachable owners do not invent a normal exit obligation.
        branches.push(block(vec![], cfg::Terminator::Return));
        assert!(check(&branches));
    }
    #[test]
    fn opaque_cleanup_rejects_reinitialization_kills_and_partial_moves() {
        let initialize = cfg::Statement::Assign {
            destination: place(0),
            value: cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::Unit)),
        };
        assert!(!check(&[block(
            vec![moved(), initialize.clone()],
            cfg::Terminator::Return
        )]));
        assert!(!check(&[block(vec![initialize], cfg::Terminator::Return)]));
        for kill in [
            cfg::Statement::StorageDead(ir::LocalId(0)),
            cfg::Statement::StorageLive(ir::LocalId(0)),
        ] {
            assert!(!check(&[block(
                vec![kill.clone()],
                cfg::Terminator::Return
            )]));
            assert!(check(&[block(
                vec![moved(), kill],
                cfg::Terminator::Return
            )]));
        }
        let partial = cfg::Statement::Assign {
            destination: place(1),
            value: cfg::Rvalue::Use(cfg::Operand::Move(ir::Place {
                local: ir::LocalId(0),
                projection: vec![ir::Projection::Field(0)],
            })),
        };
        assert!(!check(&[block(vec![partial], cfg::Terminator::Return)]));
        let copy = cfg::Statement::Assign {
            destination: place(1),
            value: cfg::Rvalue::Use(cfg::Operand::Copy(place(0))),
        };
        assert!(!check(&[block(vec![copy], cfg::Terminator::Return)]));
    }
    #[test]
    fn opaque_cleanup_loop_join_and_return_transfer_are_exact() {
        let blocks = vec![
            block(vec![], cfg::Terminator::Goto(cfg::BlockId(2))),
            block(vec![moved()], cfg::Terminator::Return),
            block(vec![], cfg::Terminator::Goto(cfg::BlockId(1))),
        ];
        assert!(check(&blocks));
        let loop_with_exit = vec![
            block(
                vec![],
                cfg::Terminator::Switch {
                    condition: cfg::Operand::Constant(cfg::Constant::Bool(true)),
                    then_block: cfg::BlockId(1),
                    else_block: cfg::BlockId(2),
                },
            ),
            block(vec![], cfg::Terminator::Goto(cfg::BlockId(0))),
            block(vec![moved()], cfg::Terminator::Return),
        ];
        assert!(check(&loop_with_exit));
        let mut live_exit = loop_with_exit.clone();
        live_exit[2].statements.clear();
        assert!(!check(&live_exit));
        let blocks = [block(vec![], cfg::Terminator::Return)];
        assert!(
            solve(
                &blocks,
                cfg::BlockId(0),
                true,
                true,
                ir::LocalId(0),
                &ir::PlanBudget::default()
            )
            .unwrap()
        );
        let empty = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..ir::PlanLimits::default()
        });
        assert!(
            solve(
                &blocks,
                cfg::BlockId(0),
                true,
                false,
                ir::LocalId(0),
                &empty
            )
            .is_err()
        );
    }
}
