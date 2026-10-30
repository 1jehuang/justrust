//! rustc's `RemoveNoopLandingPads`, which runs before drop elaboration: a cleanup block that
//! does nothing on the way out but end storage, and leads only to resuming the unwind or to
//! such blocks, is not unwound to. The call, drop or assert just goes on unwinding, and the
//! storage states of its scopes are never seen by the coroutine transform there.

use rustc_hash::FxHashSet;

use crate::mir::{BasicBlockId, MirBody, StatementKind, TerminatorKind};

pub(in crate::mir) fn remove_noop_landing_pads(body: &mut MirBody<'_>) {
    let mut noop: FxHashSet<BasicBlockId> = FxHashSet::default();
    loop {
        let before = noop.len();
        for (id, block) in body.basic_blocks.iter() {
            if !block.is_cleanup || noop.contains(&id) {
                continue;
            }
            let quiet = block.statements.iter().all(|it| {
                matches!(
                    it.kind,
                    StatementKind::StorageLive(_)
                        | StatementKind::StorageDead(_)
                        | StatementKind::FakeRead(_)
                        | StatementKind::Nop
                )
            });
            let ends = match block.terminator.as_ref().map(|it| &it.kind) {
                Some(TerminatorKind::UnwindResume) => true,
                Some(TerminatorKind::Goto { target }) => noop.contains(target),
                _ => false,
            };
            if quiet && ends {
                noop.insert(id);
            }
        }
        if noop.len() == before {
            break;
        }
    }
    for (_, block) in body.basic_blocks.iter_mut() {
        let Some(terminator) = &mut block.terminator else { continue };
        if let Some(unwind) = terminator.kind.unwind_mut()
            && unwind.is_some_and(|it| noop.contains(&it))
        {
            *unwind = None;
        }
    }
}
