//! Which locals of a body may hold storage at the same time: what LLVM's stack colouring finds
//! from the lifetime markers rustc writes. Two locals that never do can share memory in the
//! frame.
//!
//! A local whose address is never taken holds a value from where it is assigned whole to where
//! it is last read (liveness), whatever its markers say: they are too coarse to share by (in
//! hir-ide's `hir_fmt` they had each of 724 locals live with 722 others at some point). A
//! borrowed local has storage where `StorageLive` and `StorageDead` say it
//! does (`MaybeStorageLive` of `rustc_mir_dataflow`), and all through the body when they do not
//! mention it or it is mentioned where they say it has none. So does a local a debugger is told
//! of: it reads the local's memory all through the local's scope, past its last read.

use rustc_index::bit_set::{BitMatrix, DenseBitSet};

use super::{
    BasicBlockId, LocalId, MirBody, ProjectionElem, Rvalue, StatementKind, StoredPlace,
    TerminatorKind, return_slot,
};

/// Whether two of a list of locals may hold storage at once, by their index in the list.
#[derive(Debug, Clone)]
pub struct StorageConflicts {
    /// The ones that have storage all through the body.
    always: DenseBitSet<usize>,
    conflicts: BitMatrix<usize, usize>,
}

impl StorageConflicts {
    /// Whether the `a`th and the `b`th local may hold storage at once.
    pub fn conflict(&self, a: usize, b: usize) -> bool {
        a == b
            || self.always.contains(a)
            || self.always.contains(b)
            || self.conflicts.contains(a, b)
    }
}

fn mentioned(place: &StoredPlace, f: &mut impl FnMut(LocalId)) {
    f(place.local);
    for elem in place.projection.as_slice() {
        if let ProjectionElem::Index(local) = elem {
            f(*local);
        }
    }
}

/// Whether `place` is its local's own memory, not memory it points to.
fn in_frame(place: &StoredPlace) -> bool {
    !matches!(place.projection.as_slice().first(), Some(ProjectionElem::Deref))
}

impl MirBody<'_> {
    /// Which of `locals` may hold storage at the same point of the body. Those of `scoped` hold
    /// it where their markers say, as borrowed ones do.
    pub fn frame_storage_conflicts(
        &self,
        locals: &[LocalId],
        scoped: impl Fn(LocalId) -> bool,
    ) -> StorageConflicts {
        let count = locals.len();
        let mut index = vec![None; self.locals.len()];
        for (at, &local) in locals.iter().enumerate() {
            index[local.into_raw().into_u32() as usize] = Some(at);
        }
        let index_of = |local: LocalId| index[local.into_raw().into_u32() as usize];
        let block_index = |block: BasicBlockId| block.into_raw().into_u32() as usize;

        // The locals with markers, and of the others the ones liveness tells of.
        let mut marked = DenseBitSet::new_empty(count);
        let mut borrowed = DenseBitSet::new_empty(count);
        for (at, &local) in locals.iter().enumerate() {
            if scoped(local) {
                borrowed.insert(at);
            }
        }
        for (_, block) in self.basic_blocks.iter() {
            for statement in &block.statements {
                match &statement.kind {
                    StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                        if let Some(at) = index_of(*local) {
                            marked.insert(at);
                        }
                    }
                    StatementKind::Assign(_, Rvalue::Ref(_, place)) if in_frame(place) => {
                        if let Some(at) = index_of(place.local) {
                            borrowed.insert(at);
                        }
                    }
                    _ => (),
                }
            }
        }
        // A local whose address is never taken holds a value only from where it is assigned
        // whole to where it is last read, whatever its markers say; one that is borrowed has
        // storage where its markers say, or all through the body when it has none.
        let mut always = DenseBitSet::new_filled(count);
        always.subtract(&marked);
        always.intersect(&borrowed);
        for local in self.entry_locals() {
            if let Some(at) = index_of(local) {
                always.insert(at);
            }
        }
        let mut tracked = DenseBitSet::new_filled(count);
        tracked.subtract(&borrowed);
        tracked.subtract(&always);
        marked.intersect(&borrowed);
        marked.subtract(&always);

        // Marked locals: the ones that may have storage where each block starts, for the
        // blocks reached.
        let mut entry: Vec<Option<DenseBitSet<usize>>> = vec![None; self.basic_blocks.len()];
        entry[block_index(self.start_block)] = Some(DenseBitSet::new_empty(count));
        let mut worklist = vec![self.start_block];
        while let Some(id) = worklist.pop() {
            let block = &self.basic_blocks[id];
            let mut live = entry[block_index(id)].clone().expect("a reached block");
            for statement in &block.statements {
                match statement.kind {
                    StatementKind::StorageLive(local) => {
                        if let Some(at) = index_of(local).filter(|&at| marked.contains(at)) {
                            live.insert(at);
                        }
                    }
                    StatementKind::StorageDead(local) => {
                        if let Some(at) = index_of(local).filter(|&at| marked.contains(at)) {
                            live.remove(at);
                        }
                    }
                    _ => (),
                }
            }
            let Some(terminator) = &block.terminator else { continue };
            let kind = &terminator.kind;
            for to in kind.successors().into_iter().chain(kind.unwind()) {
                let changed = match &mut entry[block_index(to)] {
                    Some(state) => state.union(&live),
                    state @ None => {
                        *state = Some(live.clone());
                        true
                    }
                };
                if changed {
                    worklist.push(to);
                }
            }
        }

        // Tracked locals: the ones that hold a value where each block starts.
        let read = |live: &mut DenseBitSet<usize>, place: &StoredPlace| {
            mentioned(place, &mut |local| {
                if let Some(at) = index_of(local)
                    && tracked.contains(at)
                {
                    live.insert(at);
                }
            })
        };
        let write = |live: &mut DenseBitSet<usize>, place: &StoredPlace| {
            if place.projection.as_slice().is_empty() {
                if let Some(at) = index_of(place.local) {
                    live.remove(at);
                }
            } else {
                read(live, place);
            }
        };
        let through_statement = |live: &mut DenseBitSet<usize>, kind: &StatementKind| match kind {
            StatementKind::Assign(place, _) => {
                write(live, place);
                let mut first = true;
                Self::visit_statement_places(kind, &mut |it| {
                    // The first place is the assigned one.
                    if !std::mem::take(&mut first) {
                        read(live, it);
                    }
                });
            }
            kind => Self::visit_statement_places(kind, &mut |it| read(live, it)),
        };
        let live_in = |live_in: &[DenseBitSet<usize>], block: BasicBlockId| {
            live_in[block_index(block)].clone()
        };
        // What holds a value before a terminator, given what does where its successors start.
        let through_terminator = |live_in_of: &[DenseBitSet<usize>], kind: &TerminatorKind| {
            let mut live = DenseBitSet::new_empty(count);
            match kind {
                TerminatorKind::Call { destination, target, cleanup, .. } => {
                    // The destination is written when the call returns, not when it unwinds.
                    if let Some(target) = target {
                        live.union(&live_in(live_in_of, *target));
                        write(&mut live, destination);
                    }
                    if let Some(cleanup) = cleanup {
                        live.union(&live_in(live_in_of, *cleanup));
                    }
                    let mut places = Vec::new();
                    Self::visit_terminator_places(kind, &mut |it| places.push(it.clone()));
                    // The last place is the destination.
                    places.pop();
                    for it in &places {
                        read(&mut live, it);
                    }
                }
                TerminatorKind::Return => {
                    for to in kind.successors() {
                        live.union(&live_in(live_in_of, to));
                    }
                    if let Some(at) = index_of(return_slot())
                        && tracked.contains(at)
                    {
                        live.insert(at);
                    }
                }
                _ => {
                    for to in kind.successors().into_iter().chain(kind.unwind()) {
                        live.union(&live_in(live_in_of, to));
                    }
                    Self::visit_terminator_places(kind, &mut |it| read(&mut live, it));
                }
            }
            live
        };
        let mut holding: Vec<DenseBitSet<usize>> =
            vec![DenseBitSet::new_empty(count); self.basic_blocks.len()];
        let mut changed = true;
        while changed {
            changed = false;
            for (id, block) in self.basic_blocks.iter().rev() {
                let mut live = match &block.terminator {
                    Some(terminator) => through_terminator(&holding, &terminator.kind),
                    None => DenseBitSet::new_empty(count),
                };
                for statement in block.statements.iter().rev() {
                    through_statement(&mut live, &statement.kind);
                }
                if holding[block_index(id)] != live {
                    holding[block_index(id)] = live;
                    changed = true;
                }
            }
        }

        // Every two locals with storage at the same point conflict. A marked local mentioned
        // where it may have no storage has it all through the body.
        let mut conflicts = BitMatrix::new(count, count);
        let all_pairs = |conflicts: &mut BitMatrix<usize, usize>, at_once: &DenseBitSet<usize>| {
            for at in at_once.iter() {
                conflicts.union_row_with(at_once, at);
            }
        };
        for (id, block) in self.basic_blocks.iter() {
            let Some(mut storage) = entry[block_index(id)].clone() else { continue };
            // What is marked live before each statement and before the terminator.
            let mut storage_at = Vec::with_capacity(block.statements.len() + 1);
            for statement in &block.statements {
                let before = storage.clone();
                match statement.kind {
                    StatementKind::StorageLive(local) => {
                        if let Some(at) = index_of(local).filter(|&at| marked.contains(at)) {
                            storage.insert(at);
                        }
                    }
                    StatementKind::StorageDead(local) => {
                        if let Some(at) = index_of(local).filter(|&at| marked.contains(at)) {
                            storage.remove(at);
                        }
                    }
                    ref kind => Self::visit_statement_places(kind, &mut |place| {
                        mentioned(place, &mut |local| {
                            if let Some(at) = index_of(local)
                                && marked.contains(at)
                                && !storage.contains(at)
                            {
                                always.insert(at);
                            }
                        })
                    }),
                }
                let mut at_once = before;
                at_once.union(&storage);
                storage_at.push(at_once);
            }
            if let Some(terminator) = &block.terminator {
                Self::visit_terminator_places(&terminator.kind, &mut |place| {
                    mentioned(place, &mut |local| {
                        if let Some(at) = index_of(local)
                            && marked.contains(at)
                            && !storage.contains(at)
                        {
                            always.insert(at);
                        }
                    })
                });
            }
            storage_at.push(storage);

            let mut after = match &block.terminator {
                Some(terminator) => {
                    let kind = &terminator.kind;
                    let before = through_terminator(&holding, kind);
                    // What the terminator writes holds a value while what it reads does.
                    let mut at_once = before.clone();
                    Self::visit_terminator_places(kind, &mut |place| read(&mut at_once, place));
                    for to in kind.successors().into_iter().chain(kind.unwind()) {
                        at_once.union(&holding[block_index(to)]);
                    }
                    at_once.union(&storage_at[block.statements.len()]);
                    all_pairs(&mut conflicts, &at_once);
                    before
                }
                None => DenseBitSet::new_empty(count),
            };
            for (at, statement) in block.statements.iter().enumerate().rev() {
                let mut before = after.clone();
                through_statement(&mut before, &statement.kind);
                let mut at_once = before.clone();
                at_once.union(&after);
                Self::visit_statement_places(&statement.kind, &mut |place| {
                    read(&mut at_once, place)
                });
                at_once.union(&storage_at[at]);
                all_pairs(&mut conflicts, &at_once);
                after = before;
            }
        }
        StorageConflicts { always, conflicts }
    }
}
