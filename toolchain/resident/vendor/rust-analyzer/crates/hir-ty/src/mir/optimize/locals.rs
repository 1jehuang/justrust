//! rustc's `SimplifyLocals` (`simplify.rs`) and `RemoveStorageMarkers`.

use la_arena::Arena;

use crate::mir::{CastKind, LocalId, MirBody, Rvalue, StatementKind, return_slot};

use super::{Location, Mention, local_index, renumber_locals, visit_body, visit_statement};

/// rustc's `RemoveStorageMarkers`, which runs where no lifetimes are given to LLVM.
pub(super) fn remove_storage_markers(body: &mut MirBody<'_>) {
    for (_, block) in body.basic_blocks.iter_mut() {
        block.statements.retain(|it| {
            !matches!(it.kind, StatementKind::StorageLive(_) | StatementKind::StorageDead(_))
        });
    }
}

/// rustc's `Rvalue::is_safe_to_remove`: an exposing cast is the only rvalue that does
/// something besides giving its value.
fn is_safe_to_remove(rvalue: &Rvalue) -> bool {
    !matches!(rvalue, Rvalue::Cast(CastKind::PointerExposeAddress, _, _))
}

/// How many times each local is used, as rustc's `UsedLocals` counts: not where an
/// assignment that could be removed writes it as a whole, and not in storage markers. The
/// return slot, the parameters and what a debugger is told of are always used.
struct UsedLocals {
    count: Vec<u32>,
    always: Vec<bool>,
}

impl UsedLocals {
    fn new(body: &MirBody<'_>) -> UsedLocals {
        let mut always = vec![false; body.locals.len()];
        always[local_index(return_slot())] = true;
        for &local in body.param_locals.iter().chain(body.binding_locals.values()) {
            always[local_index(local)] = true;
        }
        // The code generator gives each promoted local its constant, and an `async` body's
        // captures are the future's fields.
        let kept = body.promoted_locals.iter().map(|it| it.0);
        for local in kept.chain(body.upvars_in_place.iter().map(|it| it.0)) {
            always[local_index(local)] = true;
        }
        let mut used = UsedLocals { count: vec![0; body.locals.len()], always };
        visit_body(body, |local, mention, at| used.mention(body, local, mention, at, 1));
        used
    }

    fn is_used(&self, local: LocalId) -> bool {
        self.always[local_index(local)] || self.count[local_index(local)] != 0
    }

    fn mention(
        &mut self,
        body: &MirBody<'_>,
        local: LocalId,
        mention: Mention,
        at: Location,
        by: i32,
    ) {
        match mention {
            Mention::Storage => return,
            Mention::Assigned | Mention::AssignedPart => {
                let block = &body.basic_blocks[at.block];
                if let Some(StatementKind::Assign(_, rvalue)) =
                    block.statements.get(at.index).map(|it| &it.kind)
                    && is_safe_to_remove(rvalue)
                {
                    return;
                }
            }
            _ => (),
        }
        let count = &mut self.count[local_index(local)];
        *count = (*count as i32 + by) as u32;
    }

    /// What the use counts are without `statement`, which was at `at`.
    fn removed(&mut self, statement: &StatementKind, at: Location) {
        let removable = matches!(statement, StatementKind::Assign(_, it) if is_safe_to_remove(it));
        visit_statement(statement, at, &mut |local, mention, _| {
            if mention == Mention::Storage
                || matches!(mention, Mention::Assigned | Mention::AssignedPart) && removable
            {
                return;
            }
            let count = &mut self.count[local_index(local)];
            *count = count.saturating_sub(1);
        });
    }
}

/// rustc's `remove_unused_definitions_helper`: assignments to locals nothing reads, and their
/// storage markers, go, until no more do.
fn remove_unused_definitions_helper(used: &mut UsedLocals, body: &mut MirBody<'_>) {
    let mut modified = true;
    while modified {
        modified = false;
        let blocks: Vec<_> = body.basic_blocks.iter().map(|(id, _)| id).collect();
        for block in blocks {
            let mut index = 0;
            while index < body.basic_blocks[block].statements.len() {
                let statement = &body.basic_blocks[block].statements[index];
                let keep = match &statement.kind {
                    StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                        used.is_used(*local)
                    }
                    StatementKind::Assign(place, rvalue) => {
                        used.is_used(place.local) || !is_safe_to_remove(rvalue)
                    }
                    _ => true,
                };
                if keep {
                    index += 1;
                    continue;
                }
                let statement = body.basic_blocks[block].statements.remove(index);
                used.removed(&statement.kind, Location { block, index });
                modified = true;
            }
        }
    }
}

/// What rustc's `CopyProp` ends with: unused definitions removed, the locals kept.
pub(super) fn remove_unused_definitions(body: &mut MirBody<'_>) {
    let mut used = UsedLocals::new(body);
    remove_unused_definitions_helper(&mut used, body);
}

/// rustc's `SimplifyLocals`: what no one uses goes, and the locals left are numbered again in
/// their order.
pub(super) fn simplify_locals(body: &mut MirBody<'_>) {
    let mut used = UsedLocals::new(body);
    remove_unused_definitions_helper(&mut used, body);
    let mut map = vec![None; body.locals.len()];
    let mut new_locals = Arena::with_capacity(body.locals.len());
    let mut removed = false;
    for (local, decl) in body.locals.iter() {
        if !used.is_used(local) {
            removed = true;
            continue;
        }
        map[local_index(local)] = Some(new_locals.alloc(decl.clone()));
    }
    if !removed {
        return;
    }
    renumber_locals(body, new_locals, &map);
}
