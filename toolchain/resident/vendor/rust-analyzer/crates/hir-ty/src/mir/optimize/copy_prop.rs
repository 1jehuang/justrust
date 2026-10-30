//! rustc's `CopyProp` (`copy_prop.rs`) and the part of its `SsaLocals` (`ssa.rs`) it uses.
//!
//! A local is SSA when it is assigned once, as a parameter, by an assignment to the whole of
//! it or as where a call returns, that assignment comes before every use on every path, and
//! nothing changes it otherwise: no `&mut`, no assignment to a part, no drop. A shared borrow
//! is allowed of a local with no cell in it. Of
//!
//! ```text
//! _a = rvalue
//! _b = move? _a
//! _c = move? _b
//! ```
//!
//! with all three SSA, `_b` and `_c` are `_a`: every use of them reads `_a`, the copies go,
//! and what moved out of one of them copies.
//!
//! Promoted locals are never SSA here: the code generator gives them their constant and skips
//! what assigns them.

use rustc_hash::FxHashSet;

use crate::mir::{
    LocalId, MirBody, OperandKind, ProjectionElem, Rvalue, StatementKind, return_slot,
    transform::Ctx,
};

use super::{
    Dominators, Location, Mention, local_id, local_index, locals::remove_unused_definitions,
    map_place, reverse_postorder, visit_statement, visit_terminator,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Def {
    Argument,
    Assignment(Location),
    /// Returned to by the call ending `call`, which goes on to `target`.
    CallReturn {
        target: Option<crate::mir::BasicBlockId>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Assignments {
    Empty,
    One(Def),
    Many,
}

impl Assignments {
    fn insert(&mut self, def: Def) {
        *self = match *self {
            Assignments::Empty => Assignments::One(def),
            _ => Assignments::Many,
        };
    }
}

fn def_dominates(def: Def, at: Location, dominators: &Dominators, single_pred: &[bool]) -> bool {
    match def {
        Def::Argument => true,
        Def::Assignment(def) if def.block == at.block => def.index < at.index,
        Def::Assignment(def) => dominators.dominates(def.block, at.block),
        // The value is there on the edge into `target`; with the call as the only way in, it
        // is there wherever `target` dominates.
        Def::CallReturn { target: Some(target) } => {
            single_pred[super::block_index(target)] && dominators.dominates(target, at.block)
        }
        Def::CallReturn { target: None } => false,
    }
}

struct Ssa {
    assignments: Vec<Assignments>,
    /// The SSA locals in the order of their assignments, in reverse postorder.
    order: Vec<LocalId>,
    borrowed: Vec<bool>,
}

fn ssa_locals<'db>(ctx: &Ctx<'db>, body: &MirBody<'db>) -> Ssa {
    let count = body.locals.len();
    let dominators = Dominators::new(body);
    let mut preds = vec![0u32; body.basic_blocks.len()];
    for (_, block) in body.basic_blocks.iter() {
        if let Some(terminator) = &block.terminator {
            for target in super::successors(&terminator.kind) {
                preds[super::block_index(target)] += 1;
            }
        }
    }
    let single_pred: Vec<bool> = preds.iter().map(|&it| it == 1).collect();
    let mut assignments = vec![Assignments::Empty; count];
    let mut order = Vec::new();
    let mut borrowed = vec![false; count];
    for &param in &body.param_locals {
        assignments[local_index(param)] = Assignments::One(Def::Argument);
        order.push(param);
    }
    for &(local, _) in &body.promoted_locals {
        assignments[local_index(local)] = Assignments::Many;
    }
    for &(local, _) in &body.upvars_in_place {
        assignments[local_index(local)] = Assignments::Many;
    }
    let check_dominates = |assignments: &mut Vec<Assignments>, local: LocalId, at: Location| {
        let slot = &mut assignments[local_index(local)];
        let dominated = match *slot {
            Assignments::One(def) => def_dominates(def, at, &dominators, &single_pred),
            _ => false,
        };
        if !dominated {
            *slot = Assignments::Many;
        }
    };
    let mut on_mention = |local: LocalId, mention: Mention, at: Location| match mention {
        Mention::Assigned => {
            assignments[local_index(local)].insert(Def::Assignment(at));
            if let Assignments::One(_) = assignments[local_index(local)] {
                order.push(local);
            }
        }
        Mention::CallReturned => {
            let target = match &body.basic_blocks[at.block].terminator.as_ref().map(|it| &it.kind) {
                Some(crate::mir::TerminatorKind::Call { target, .. }) => *target,
                _ => None,
            };
            assignments[local_index(local)].insert(Def::CallReturn { target });
            if let Assignments::One(_) = assignments[local_index(local)] {
                order.push(local);
            }
        }
        Mention::AssignedPart | Mention::Mutated => {
            assignments[local_index(local)] = Assignments::Many;
        }
        Mention::SharedBorrow => {
            borrowed[local_index(local)] = true;
            check_dominates(&mut assignments, local, at);
        }
        Mention::Read => check_dominates(&mut assignments, local, at),
        Mention::Storage => (),
    };
    for block in reverse_postorder(body) {
        let data = &body.basic_blocks[block];
        for (index, statement) in data.statements.iter().enumerate() {
            visit_statement(&statement.kind, Location { block, index }, &mut on_mention);
        }
        if let Some(terminator) = &data.terminator {
            let at = Location { block, index: data.statements.len() };
            visit_terminator(&terminator.kind, at, &mut on_mention);
        }
    }
    for (local, decl) in body.locals.iter() {
        if borrowed[local_index(local)]
            && !crate::drop::is_freeze(&ctx.infcx, decl.ty.as_ref(), ctx.env)
        {
            assignments[local_index(local)] = Assignments::Many;
        }
    }
    order.retain(|&it| matches!(assignments[local_index(it)], Assignments::One(_)));
    Ssa { assignments, order, borrowed }
}

/// rustc's `compute_copy_classes`: for each local, the local it is a copy of, itself if none.
fn copy_classes(ssa: &Ssa, body: &MirBody<'_>) -> Vec<LocalId> {
    let mut copies: Vec<LocalId> = (0..body.locals.len()).map(local_id).collect();
    let is_ssa =
        |local: LocalId| matches!(ssa.assignments[local_index(local)], Assignments::One(_));
    let is_temp = |local: LocalId| local != return_slot() && !body.param_locals.contains(&local);
    for &local in &ssa.order {
        let Assignments::One(Def::Assignment(at)) = ssa.assignments[local_index(local)] else {
            continue;
        };
        let Some(StatementKind::Assign(target, Rvalue::Use(operand))) =
            body.basic_blocks[at.block].statements.get(at.index).map(|it| &it.kind)
        else {
            continue;
        };
        debug_assert_eq!(target.as_ref().as_local(), Some(local));
        let (OperandKind::Copy(place) | OperandKind::Move(place)) = &operand.kind else {
            continue;
        };
        let Some(rhs) = place.as_ref().as_local() else { continue };
        if body.locals[local].ty != body.locals[rhs].ty || !is_ssa(rhs) {
            continue;
        }
        let head = copies[local_index(rhs)];
        if ssa.borrowed[local_index(local)] {
            continue;
        }
        if local == return_slot() {
            // The return slot keeps its number: the class is renamed to it instead.
            if !is_temp(head) {
                continue;
            }
            for it in copies.iter_mut() {
                if *it == head {
                    *it = return_slot();
                }
            }
        } else {
            copies[local_index(local)] = head;
        }
    }
    copies
}

/// rustc's `CopyProp`. With `lifetimes` (rustc's `emit_lifetime_markers`), storage markers of
/// a class are kept where rustc would keep them; here they all go, which rustc does when
/// `lifetimes` is off and which is always correct.
pub(super) fn copy_prop<'db>(ctx: &Ctx<'db>, body: &mut MirBody<'db>, lifetimes: bool) {
    let _ = lifetimes;
    let ssa = ssa_locals(ctx, body);
    let copies = copy_classes(&ssa, body);
    let mut unified: FxHashSet<LocalId> = FxHashSet::default();
    for (index, &head) in copies.iter().enumerate() {
        if local_id(index) != head {
            unified.insert(head);
            unified.insert(local_id(index));
        }
    }
    if unified.is_empty() {
        return;
    }
    let mut lookup = |local: LocalId| copies[local_index(local)];
    for (_, block) in body.basic_blocks.iter_mut() {
        for statement in &mut block.statements {
            if let StatementKind::StorageLive(local) | StatementKind::StorageDead(local) =
                statement.kind
                && unified.contains(&local)
            {
                statement.kind = StatementKind::Nop;
            }
        }
    }
    // A move out of a local of a class, or a part of one not behind a pointer, copies: the
    // head may be read again.
    let copy_moves = |operand: &mut crate::mir::Operand| {
        if let OperandKind::Move(place) = &operand.kind
            && place.projection.as_slice().first() != Some(&ProjectionElem::Deref)
            && unified.contains(&place.local)
        {
            operand.kind = OperandKind::Copy(place.clone());
        }
    };
    let mut copy_moves = copy_moves;
    for (_, block) in body.basic_blocks.iter_mut() {
        for statement in &mut block.statements {
            statement.kind.for_each_operand_mut(&mut copy_moves);
        }
        if let Some(terminator) = &mut block.terminator {
            terminator.kind.for_each_operand_mut(&mut copy_moves);
        }
    }
    body.walk_places(|place| map_place(place, &mut lookup));
    let mut bindings = la_arena::ArenaMap::default();
    for (binding, &local) in body.binding_locals.iter() {
        bindings.insert(binding, lookup(local));
    }
    body.binding_locals = bindings;
    for (_, block) in body.basic_blocks.iter_mut() {
        for statement in &mut block.statements {
            if let StatementKind::Assign(to, Rvalue::Use(operand)) = &statement.kind
                && let OperandKind::Copy(from) | OperandKind::Move(from) = &operand.kind
                && to == from
            {
                statement.kind = StatementKind::Nop;
            }
        }
        block.statements.retain(|it| !matches!(it.kind, StatementKind::Nop));
    }
    remove_unused_definitions(body);
}
