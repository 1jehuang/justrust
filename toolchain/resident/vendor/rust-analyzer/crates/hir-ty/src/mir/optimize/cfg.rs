//! The passes that change the blocks of a body and not what they compute: rustc's
//! `SimplifyCfg` (`simplify.rs`), `SimplifyConstCondition` (`simplify_branches.rs`),
//! `RemoveUnneededDrops`, `UnreachablePropagation`, `AddCallGuards::CriticalCallEdges` and the
//! part of `CleanupPostBorrowck` that applies here.

use rustc_hash::FxHashSet;

use crate::{
    MemoryMap,
    mir::{
        BasicBlock, BasicBlockId, MirBody, Operand, OperandKind, Rvalue, StatementKind,
        SwitchTargets, Terminator, TerminatorKind, transform::Ctx,
    },
    next_solver::ConstKind,
};
use rustc_type_ir::inherent::IntoKind;

use super::{block_id, block_index, renumber_blocks, successors, successors_mut};

/// rustc's `CleanupPostBorrowck` and `EraseDerefTemps` for what lowering here makes:
/// `FakeRead`s and the false edges only borrow checking reads go, and a `CopyForDeref` is the
/// copy it is. Also the storage markers lowering gives parameters, which rustc's never have.
pub(super) fn cleanup_post_borrowck(body: &mut MirBody<'_>) {
    let params = &body.param_locals;
    for (_, block) in body.basic_blocks.iter_mut() {
        if let Some(terminator) = &mut block.terminator
            && let TerminatorKind::FalseEdge { real_target: target, .. }
            | TerminatorKind::FalseUnwind { real_target: target, .. } = terminator.kind
        {
            terminator.kind = TerminatorKind::Goto { target };
        }
        block.statements.retain(|it| match it.kind {
            StatementKind::FakeRead(_) => false,
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                !params.contains(&local)
            }
            _ => true,
        });
        for statement in &mut block.statements {
            if let StatementKind::Assign(_, rvalue) = &mut statement.kind
                && let Rvalue::CopyForDeref(place) = rvalue
            {
                let place = place.clone();
                *rvalue = Rvalue::Use(Operand { kind: OperandKind::Copy(place), span: None });
            }
        }
    }
}

/// rustc's `simplify_cfg`: goto chains collapsed, a block with one predecessor that jumps to
/// it merged into that one, a switch whose targets are all one block made a `goto`, and the
/// blocks nothing reaches removed.
pub(super) fn simplify_cfg(body: &mut MirBody<'_>) {
    let start = block_index(body.start_block);
    let mut blocks: Vec<BasicBlock> =
        std::mem::take(&mut body.basic_blocks).into_iter().map(|(_, it)| it).collect();
    CfgSimplifier::new(&mut blocks, start).simplify();
    remove_dead_blocks(body, blocks, start);
}

struct CfgSimplifier<'a> {
    blocks: &'a mut [BasicBlock],
    pred_count: Vec<u32>,
}

impl<'a> CfgSimplifier<'a> {
    fn new(blocks: &'a mut [BasicBlock], start: usize) -> Self {
        let mut pred_count = vec![0u32; blocks.len()];
        pred_count[start] = 1;
        for block in reachable(blocks, start) {
            if let Some(terminator) = &blocks[block].terminator {
                for target in successors(&terminator.kind) {
                    pred_count[block_index(target)] += 1;
                }
            }
        }
        CfgSimplifier { blocks, pred_count }
    }

    fn simplify(mut self) {
        for block in self.blocks.iter_mut() {
            block.statements.retain(|it| !matches!(it.kind, StatementKind::Nop));
        }
        let mut merged: Vec<usize> = Vec::new();
        loop {
            let mut changed = false;
            for bb in 0..self.blocks.len() {
                if self.pred_count[bb] == 0 {
                    continue;
                }
                let Some(mut terminator) = self.blocks[bb].terminator.take() else { continue };
                successors_mut(&mut terminator.kind, |successor| {
                    self.collapse_goto_chain(successor, &mut changed);
                });
                let mut inner_changed = true;
                merged.clear();
                while inner_changed {
                    inner_changed = false;
                    inner_changed |= self.simplify_branch(&mut terminator);
                    inner_changed |= self.merge_successor(&mut merged, &mut terminator);
                    changed |= inner_changed;
                }
                for &from in &merged {
                    let statements = std::mem::take(&mut self.blocks[from].statements);
                    self.blocks[bb].statements.extend(statements);
                }
                self.blocks[bb].terminator = Some(terminator);
            }
            if !changed {
                break;
            }
        }
    }

    fn take_terminator_if_simple_goto(&mut self, bb: usize) -> Option<Terminator> {
        let block = &mut self.blocks[bb];
        match &block.terminator {
            Some(Terminator { kind: TerminatorKind::Goto { .. }, .. })
                if block.statements.is_empty() =>
            {
                block.terminator.take()
            }
            _ => None,
        }
    }

    fn collapse_goto_chain(&mut self, start: &mut BasicBlockId, changed: &mut bool) {
        let mut terminators: Vec<(usize, Terminator)> = Vec::new();
        let mut current = block_index(*start);
        while let Some(terminator) = self.take_terminator_if_simple_goto(current) {
            let TerminatorKind::Goto { target } = terminator.kind else { unreachable!() };
            terminators.push((current, terminator));
            current = block_index(target);
        }
        let last = current;
        *changed |= block_index(*start) != last;
        *start = block_id(last);
        while let Some((current, mut terminator)) = terminators.pop() {
            let TerminatorKind::Goto { target } = &mut terminator.kind else { unreachable!() };
            *changed |= block_index(*target) != last;
            *target = block_id(last);
            if self.pred_count[current] == 1 {
                self.pred_count[current] = 0;
            } else {
                self.pred_count[last] += 1;
                self.pred_count[current] -= 1;
            }
            self.blocks[current].terminator = Some(terminator);
        }
    }

    fn merge_successor(&mut self, merged: &mut Vec<usize>, terminator: &mut Terminator) -> bool {
        let target = match terminator.kind {
            TerminatorKind::Goto { target } if self.pred_count[block_index(target)] == 1 => {
                block_index(target)
            }
            _ => return false,
        };
        *terminator = match self.blocks[target].terminator.take() {
            Some(it) => it,
            None => return false,
        };
        merged.push(target);
        self.pred_count[target] = 0;
        true
    }

    fn simplify_branch(&mut self, terminator: &mut Terminator) -> bool {
        let TerminatorKind::SwitchInt { targets, .. } = &terminator.kind else { return false };
        let all = targets.all_targets();
        let first = all[0];
        if all.iter().any(|&it| it != first) {
            return false;
        }
        self.pred_count[block_index(first)] -= (all.len() - 1) as u32;
        terminator.kind = TerminatorKind::Goto { target: first };
        true
    }
}

/// The blocks reachable from `start`, in preorder.
fn reachable(blocks: &[BasicBlock], start: usize) -> Vec<usize> {
    let mut seen = vec![false; blocks.len()];
    let mut order = Vec::new();
    let mut stack = vec![start];
    while let Some(block) = stack.pop() {
        if std::mem::replace(&mut seen[block], true) {
            continue;
        }
        order.push(block);
        if let Some(terminator) = &blocks[block].terminator {
            for target in successors(&terminator.kind).into_iter().rev() {
                if !seen[block_index(target)] {
                    stack.push(block_index(target));
                }
            }
        }
    }
    order
}

/// rustc's `remove_dead_blocks`: the blocks `start` does not reach go, and of the reachable
/// blocks that are nothing but `unreachable` all but one go. The rest keep their order.
fn remove_dead_blocks(body: &mut MirBody<'_>, blocks: Vec<BasicBlock>, start: usize) {
    let mut keep = vec![false; blocks.len()];
    for block in reachable(&blocks, start) {
        keep[block] = true;
    }
    let empty_unreachable = |block: &BasicBlock| {
        !block.is_cleanup
            && block.statements.is_empty()
            && matches!(
                block.terminator.as_ref().map(|it| &it.kind),
                Some(TerminatorKind::Unreachable)
            )
    };
    let mut map = vec![None; blocks.len()];
    let mut kept = Vec::with_capacity(blocks.len());
    let mut kept_unreachable = None;
    for (index, block) in blocks.into_iter().enumerate() {
        if !keep[index] {
            continue;
        }
        if empty_unreachable(&block) {
            if let Some(it) = kept_unreachable {
                map[index] = Some(it);
                continue;
            }
            kept_unreachable = Some(kept.len());
        }
        map[index] = Some(kept.len());
        kept.push(block);
    }
    renumber_blocks(body, kept, &map);
}

/// The value of a constant operand of a scalar type, as its bits.
pub(super) fn constant_bits(operand: &Operand) -> Option<u128> {
    match &operand.kind {
        OperandKind::Allocation { allocation } => {
            let allocation = allocation.as_ref();
            let bytes = &allocation.memory;
            if bytes.len() > 16 || !matches!(allocation.memory_map, MemoryMap::Empty) {
                return None;
            }
            let mut padded = [0u8; 16];
            padded[..bytes.len()].copy_from_slice(bytes);
            Some(u128::from_le_bytes(padded))
        }
        OperandKind::Constant { konst, .. } => match konst.as_ref().kind() {
            ConstKind::Value(value) => value.try_to_leaf().map(|it| it.to_bits_unchecked()),
            _ => None,
        },
        _ => None,
    }
}

fn is_constant(operand: &Operand) -> bool {
    matches!(operand.kind, OperandKind::Allocation { .. } | OperandKind::Constant { .. })
}

/// rustc's `SimplifyConstCondition`: a switch on a constant is a `goto`, and so is an assert
/// of a constant that holds. The constant may be written to the place the condition reads
/// by the statement just before.
pub(super) fn simplify_const_condition(body: &mut MirBody<'_>) {
    for (_, block) in body.basic_blocks.iter_mut() {
        let before = match block.statements.last().map(|it| &it.kind) {
            Some(StatementKind::Assign(place, Rvalue::Use(value))) if is_constant(value) => {
                Some((place.clone(), value.clone()))
            }
            _ => None,
        };
        let known = |operand: &Operand| -> Option<u128> {
            match &operand.kind {
                OperandKind::Copy(place) | OperandKind::Move(place) => {
                    let (written, value) = before.as_ref()?;
                    (written == place).then(|| constant_bits(value)).flatten()
                }
                _ => constant_bits(operand),
            }
        };
        let Some(terminator) = &mut block.terminator else { continue };
        let new = match &terminator.kind {
            TerminatorKind::SwitchInt { discr, targets } => match known(discr) {
                Some(value) => TerminatorKind::Goto { target: targets.target_for_value(value) },
                None => continue,
            },
            TerminatorKind::Assert { cond, expected, target, .. } => match known(cond) {
                Some(value) if (value != 0) == *expected => {
                    TerminatorKind::Goto { target: *target }
                }
                _ => continue,
            },
            _ => continue,
        };
        terminator.kind = new;
    }
}

/// rustc's `RemoveUnneededDrops`: a drop of what has nothing to drop is a `goto`.
pub(super) fn remove_unneeded_drops<'db>(ctx: &Ctx<'db>, body: &mut MirBody<'db>) {
    let mut changed = false;
    let mut blocks = std::mem::take(&mut body.basic_blocks);
    for (_, block) in blocks.iter_mut() {
        let Some(terminator) = &mut block.terminator else { continue };
        let TerminatorKind::Drop { place, target, .. } = &terminator.kind else { continue };
        let ty = place.as_ref().ty(body, &ctx.infcx, ctx.env).ty;
        if ctx.needs_drop(ctx.normalize(ty)) {
            continue;
        }
        terminator.kind = TerminatorKind::Goto { target: *target };
        changed = true;
    }
    body.basic_blocks = blocks;
    if changed {
        simplify_cfg(body);
    }
}

/// rustc's `AddCallGuards::CriticalCallEdges`: a call that may unwind and returns to a block
/// with more than one predecessor returns to a new block that only jumps there.
pub(super) fn critical_call_edges(body: &mut MirBody<'_>) {
    let mut pred_count = vec![0u32; body.basic_blocks.len()];
    for (_, block) in body.basic_blocks.iter() {
        if let Some(terminator) = &block.terminator {
            for target in successors(&terminator.kind) {
                pred_count[block_index(target)] = pred_count[block_index(target)].saturating_add(1);
            }
        }
    }
    let work: Vec<BasicBlockId> = body
        .basic_blocks
        .iter()
        .filter(|(_, block)| {
            matches!(
                block.terminator.as_ref().map(|it| &it.kind),
                Some(TerminatorKind::Call { target: Some(target), cleanup: Some(_), .. })
                    if pred_count[block_index(*target)] > 1
            )
        })
        .map(|(id, _)| id)
        .collect();
    for source in work {
        let (span, is_cleanup, destination) = {
            let block = &body.basic_blocks[source];
            let terminator = block.terminator.as_ref().expect("filtered");
            let TerminatorKind::Call { target: Some(target), .. } = terminator.kind else {
                unreachable!()
            };
            (terminator.span, block.is_cleanup, target)
        };
        let guard = body.basic_blocks.alloc(BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator {
                span,
                kind: TerminatorKind::Goto { target: destination },
            }),
            is_cleanup,
        });
        let terminator = body.basic_blocks[source].terminator.as_mut().expect("filtered");
        if let TerminatorKind::Call { target, .. } = &mut terminator.kind {
            *target = Some(guard);
        }
    }
}

/// rustc's `UnreachablePropagation`, without the `assume`s rustc leaves for LLVM where it takes
/// a target out of a switch: a block that can only go on to `unreachable` is unreachable
/// itself, and a switch loses the targets that are. A block with `asm!` in it is never taken
/// for unreachable, since a block that does not return is one that ends in `unreachable` here.
pub(super) fn unreachable_propagation(body: &mut MirBody<'_>) {
    let mut unreachable: FxHashSet<BasicBlockId> = FxHashSet::default();
    let mut order = super::reverse_postorder(body);
    order.reverse();
    for bb in order {
        let block = &mut body.basic_blocks[bb];
        if block.statements.iter().any(|it| matches!(it.kind, StatementKind::InlineAsm(_))) {
            continue;
        }
        let Some(terminator) = &mut block.terminator else { continue };
        let is_unreachable = match &terminator.kind {
            TerminatorKind::Unreachable => true,
            TerminatorKind::Goto { target } if unreachable.contains(target) => {
                terminator.kind = TerminatorKind::Unreachable;
                true
            }
            TerminatorKind::SwitchInt { discr, targets } => {
                let otherwise = targets.otherwise();
                let otherwise_unreachable = unreachable.contains(&otherwise);
                let kept: Vec<(u128, BasicBlockId)> =
                    targets.iter().filter(|(_, it)| !unreachable.contains(it)).collect();
                let new_targets = SwitchTargets::new(kept.iter().copied(), otherwise);
                let count = new_targets.all_targets().len();
                let new = match (count, otherwise_unreachable) {
                    (1, true) => Some(TerminatorKind::Unreachable),
                    (1, false) => Some(TerminatorKind::Goto { target: otherwise }),
                    (2, true) => Some(TerminatorKind::Goto { target: kept[0].1 }),
                    _ if count == targets.all_targets().len() => None,
                    _ => Some(TerminatorKind::SwitchInt {
                        discr: discr.clone(),
                        targets: new_targets,
                    }),
                };
                let fully = count == 1 && otherwise_unreachable;
                if let Some(new) = new {
                    terminator.kind = new;
                }
                fully
            }
            _ => false,
        };
        if is_unreachable {
            unreachable.insert(bb);
        }
    }
    for bb in unreachable {
        body.basic_blocks[bb].statements.clear();
    }
}
