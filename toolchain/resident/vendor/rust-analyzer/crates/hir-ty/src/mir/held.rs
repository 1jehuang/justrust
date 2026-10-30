//! What an `async` block keeps in its future from one `poll` to the next, computed as rustc's
//! coroutine transform computes it on the runtime body (`rustc_mir_transform/src/coroutine/
//! layout.rs` at 1.99.0: `locals_live_across_suspend_points`, `compute_storage_conflicts` and
//! the variant fields of `compute_layout`), with the analyses it runs (`MaybeBorrowedLocals`,
//! `MaybeRequiresStorage`, `MaybeLiveLocals` and `always_storage_live_locals` of
//! `rustc_mir_dataflow`). Given the same body, the same locals are kept in the same variants
//! with the same conflicts, and `rustc_abi`'s coroutine layout places them as rustc does.
//!
//! rustc's runtime body has had its `FakeRead`s removed (`CleanupPostBorrowck`); they are
//! ignored here. No statement unwinds: what panics is an `Assert` before it.

use rustc_index::bit_set::DenseBitSet;
use smallvec::SmallVec;

use super::{
    AsmOperand, BasicBlockId, BorrowKind, LocalId, MirBody, Operand, OperandKind, ProjectionElem,
    Rvalue, Statement, StatementKind, StoredPlace, Terminator, TerminatorKind,
};

/// The locals an `async` block keeps in its future: rustc's `CoroutineSavedLocal`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldLocals {
    /// The locals live across some yield, in the order the body declares them.
    pub locals: Vec<LocalId>,
    /// The places the body yields at, in the order of its blocks: the states of the future
    /// after the three rustc reserves (not started, returned, panicked).
    pub yields: Vec<YieldPoint>,
    /// Whether two of the locals may need their storage at once.
    conflicts: Vec<bool>,
}

/// One place the body yields at, which is one variant of the future.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YieldPoint {
    pub resume: BasicBlockId,
    /// Where a future suspended here is dropped.
    pub drop: Option<BasicBlockId>,
    /// The fields of the variant: the held locals live here, by their index in
    /// [`HeldLocals::locals`].
    pub kept: Vec<usize>,
}

impl HeldLocals {
    pub fn contains(&self, local: LocalId) -> bool {
        self.locals.binary_search(&local).is_ok()
    }

    /// Whether the `a`th and the `b`th of the locals may need their storage at once, and so
    /// can't share memory.
    pub fn conflict(&self, a: usize, b: usize) -> bool {
        self.conflicts[a * self.locals.len() + b]
    }
}

type Set = DenseBitSet<usize>;

fn local_index(local: LocalId) -> usize {
    local.into_raw().into_u32() as usize
}

fn block_index(block: BasicBlockId) -> usize {
    block.into_raw().into_u32() as usize
}

fn is_indirect(place: &StoredPlace) -> bool {
    place.projection.as_slice().contains(&ProjectionElem::Deref)
}

fn index_locals(place: &StoredPlace) -> impl Iterator<Item = usize> + '_ {
    place.projection.as_slice().iter().filter_map(|elem| match elem {
        ProjectionElem::Index(local) => Some(local_index(*local)),
        _ => None,
    })
}

fn operand_place(operand: &Operand) -> Option<&StoredPlace> {
    match &operand.kind {
        OperandKind::Copy(place) | OperandKind::Move(place) => Some(place),
        _ => None,
    }
}

/// The edges out of a block, each with what it does on its own: a call's return writes its
/// destination, a resumed yield its resume argument.
enum Edge<'a> {
    Plain(BasicBlockId),
    Returns(BasicBlockId, &'a StoredPlace),
    Resumes(BasicBlockId, &'a StoredPlace),
}

fn edges<'a>(body: &MirBody<'_>, terminator: &'a Terminator) -> SmallVec<[Edge<'a>; 3]> {
    let unreached = |it: &BasicBlockId| body.unreached.contains(it);
    let mut edges = SmallVec::new();
    match &terminator.kind {
        TerminatorKind::Call { target, destination, .. } => {
            edges.extend(target.iter().map(|&it| Edge::Returns(it, destination)));
        }
        TerminatorKind::Yield { resume, resume_arg, drop, .. } => {
            edges.push(Edge::Resumes(*resume, resume_arg));
            edges.extend(drop.iter().map(|&it| Edge::Plain(it)));
        }
        kind => edges.extend(kind.successors().into_iter().map(Edge::Plain)),
    }
    edges.extend(terminator.kind.unwind().map(Edge::Plain));
    edges.retain(|it| match it {
        Edge::Plain(to) | Edge::Returns(to, _) | Edge::Resumes(to, _) => !unreached(to),
    });
    edges
}

/// The blocks control reaches from the start.
fn reachable(body: &MirBody<'_>) -> Vec<bool> {
    let mut reached = vec![false; body.basic_blocks.len()];
    let mut stack = vec![body.start_block];
    while let Some(id) = stack.pop() {
        if std::mem::replace(&mut reached[block_index(id)], true) {
            continue;
        }
        let block = &body.basic_blocks[id];
        if let Some(terminator) = &block.terminator {
            for edge in edges(body, terminator) {
                let (Edge::Plain(to) | Edge::Returns(to, _) | Edge::Resumes(to, _)) = edge;
                stack.push(to);
            }
        }
    }
    reached
}

/// `MaybeBorrowedLocals`: a local some pointer may point into. `Drop` counts, because a
/// destructor is handed `&mut self`.
fn borrow_statement(statement: &StatementKind, state: &mut Set) {
    match statement {
        StatementKind::Assign(_, Rvalue::Ref(kind, place))
            if *kind != BorrowKind::Shallow && !is_indirect(place) =>
        {
            state.insert(local_index(place.local));
        }
        StatementKind::StorageDead(local) => {
            state.remove(local_index(*local));
        }
        _ => (),
    }
}

fn borrow_terminator(terminator: &TerminatorKind, state: &mut Set) {
    if let TerminatorKind::Drop { place, .. } | TerminatorKind::DropAndReplace { place, .. } =
        terminator
        && !is_indirect(place)
    {
        state.insert(local_index(place.local));
    }
}

/// The locals a statement or terminator moves out of as a whole: what `MaybeRequiresStorage`
/// kills after it, unless the local was borrowed before.
fn whole_moves(operands: &mut dyn FnMut(&mut dyn FnMut(&Operand))) -> SmallVec<[usize; 4]> {
    let mut moved = SmallVec::new();
    operands(&mut |operand| {
        if let OperandKind::Move(place) = &operand.kind
            && place.projection.as_slice().is_empty()
        {
            moved.push(local_index(place.local));
        }
    });
    moved
}

fn asm_outputs(statement: &StatementKind) -> SmallVec<[&StoredPlace; 2]> {
    let mut outputs = SmallVec::new();
    if let StatementKind::InlineAsm(asm) = statement {
        for operand in &asm.operands {
            if let AsmOperand::Reg { output: Some(output), .. } = operand {
                outputs.push(output);
            }
        }
        outputs.extend(asm.taken.iter());
    }
    outputs
}

/// The forward analyses: `MaybeBorrowedLocals` and `MaybeRequiresStorage` at the entry of
/// each block.
struct Forward {
    borrowed: Vec<Option<Set>>,
    requires: Vec<Option<Set>>,
}

impl MirBody<'_> {
    /// The locals an `async` block keeps in its future, as rustc's coroutine transform finds
    /// them. The body is the runtime one, its drops elaborated.
    pub fn locals_held_across_yields(&self) -> HeldLocals {
        let count = self.locals.len();
        let reached = reachable(self);
        let always_live = self.always_storage_live();
        let forward = self.forward(&reached);
        let live_out = self.liveness(&reached);
        let receiver = self.param_locals.first().copied().map(local_index);

        let mut any = Set::new_empty(count);
        let mut at_yields = Vec::new();
        for (id, block) in self.basic_blocks.iter() {
            let Some(Terminator { kind: TerminatorKind::Yield { resume, drop, .. }, .. }) =
                &block.terminator
            else {
                continue;
            };
            if !reached[block_index(id)] {
                continue;
            }
            let (mut borrowed, mut requires) = self.entry_states(&forward, id);
            for statement in &block.statements {
                self.storage_statement(statement, &mut borrowed, &mut requires, &mut |_| ());
            }
            let mut live = live_out[block_index(id)].clone();
            // An `async` block can't be moved once polled, so a borrow may outlive the use
            // of what it points at: a borrowed local is live until its storage ends.
            live.union(&borrowed);
            live.intersect(&requires);
            if let Some(receiver) = receiver {
                live.remove(receiver);
            }
            // What the body uses in place of a capture is the future's own field, and a
            // promoted constant is no local of rustc's.
            for &(local, _) in &self.upvars_in_place {
                live.remove(local_index(local));
            }
            for &local in &self.promoted_constants {
                live.remove(local_index(local));
            }
            any.union(&live);
            at_yields.push((*resume, *drop, live));
        }

        let locals: Vec<LocalId> = self
            .locals
            .iter()
            .map(|(local, _)| local)
            .filter(|&it| any.contains(local_index(it)))
            .collect();
        let saved: Vec<usize> = locals.iter().map(|&it| local_index(it)).collect();
        let yields = at_yields
            .into_iter()
            .map(|(resume, drop, live)| YieldPoint {
                resume,
                drop,
                kept: (0..saved.len()).filter(|&at| live.contains(saved[at])).collect(),
            })
            .collect();
        let conflicts = self.storage_conflicts(&reached, &forward, &always_live, &any, &saved);
        HeldLocals { locals, yields, conflicts }
    }

    /// `always_storage_live_locals`: the locals no `StorageLive` or `StorageDead` mentions.
    fn always_storage_live(&self) -> Set {
        let mut always = Set::new_filled(self.locals.len());
        for (_, block) in self.basic_blocks.iter() {
            for statement in &block.statements {
                if let StatementKind::StorageLive(local) | StatementKind::StorageDead(local) =
                    statement.kind
                {
                    always.remove(local_index(local));
                }
            }
        }
        always
    }

    fn entry_states(&self, forward: &Forward, id: BasicBlockId) -> (Set, Set) {
        let empty = || Set::new_empty(self.locals.len());
        let at = block_index(id);
        (
            forward.borrowed[at].clone().unwrap_or_else(empty),
            forward.requires[at].clone().unwrap_or_else(empty),
        )
    }

    /// Both forward analyses over one statement. `early` is told of the storage required
    /// after the statement's early effect, which is what storage conflicts are taken from.
    fn storage_statement(
        &self,
        statement: &Statement,
        borrowed: &mut Set,
        requires: &mut Set,
        early: &mut dyn FnMut(&Set),
    ) {
        let kind = &statement.kind;
        // Early effect: a borrowed or assigned local needs its storage for the statement.
        borrow_statement(kind, requires);
        match kind {
            StatementKind::StorageDead(local) => {
                requires.remove(local_index(*local));
            }
            StatementKind::Assign(place, _) => {
                requires.insert(local_index(place.local));
            }
            _ => (),
        }
        for output in asm_outputs(kind) {
            requires.insert(local_index(output.local));
        }
        early(requires);
        // Primary effect: a local moved out of as a whole, and never borrowed before, needs
        // no storage after.
        for local in whole_moves(&mut |f| kind.for_each_operand(&mut |op| f(op))) {
            if !borrowed.contains(local) {
                requires.remove(local);
            }
        }
        if let StatementKind::Assign(place, _) = kind {
            requires.insert(local_index(place.local));
        }
        for output in asm_outputs(kind) {
            requires.insert(local_index(output.local));
        }
        borrow_statement(kind, borrowed);
    }

    /// Both forward analyses over a terminator, up to its edges.
    fn storage_terminator(
        &self,
        terminator: &TerminatorKind,
        borrowed: &mut Set,
        requires: &mut Set,
        early: &mut dyn FnMut(&Set),
    ) {
        borrow_terminator(terminator, requires);
        if let TerminatorKind::Call { destination, .. } = terminator {
            requires.insert(local_index(destination.local));
        }
        early(requires);
        if let TerminatorKind::Call { destination, .. } = terminator {
            requires.remove(local_index(destination.local));
        }
        for local in whole_moves(&mut |f| terminator.for_each_operand(&mut |op| f(op))) {
            if !borrowed.contains(local) {
                requires.remove(local);
            }
        }
        borrow_terminator(terminator, borrowed);
    }

    fn forward(&self, reached: &[bool]) -> Forward {
        let count = self.locals.len();
        let blocks = self.basic_blocks.len();
        let mut forward = Forward { borrowed: vec![None; blocks], requires: vec![None; blocks] };
        let mut start_requires = Set::new_empty(count);
        for &param in self.param_locals.iter().skip(1) {
            start_requires.insert(local_index(param));
        }
        let start = block_index(self.start_block);
        forward.borrowed[start] = Some(Set::new_empty(count));
        forward.requires[start] = Some(start_requires);
        let mut worklist = vec![self.start_block];
        let flow = |forward: &mut Forward,
                    worklist: &mut Vec<BasicBlockId>,
                    to: BasicBlockId,
                    borrowed: &Set,
                    requires: &Set| {
            let at = block_index(to);
            let mut changed = false;
            for (known, new) in
                [(&mut forward.borrowed[at], borrowed), (&mut forward.requires[at], requires)]
            {
                match known {
                    Some(known) => changed |= known.union(new),
                    None => {
                        *known = Some(new.clone());
                        changed = true;
                    }
                }
            }
            if changed {
                worklist.push(to);
            }
        };
        while let Some(id) = worklist.pop() {
            if !reached[block_index(id)] {
                continue;
            }
            let (mut borrowed, mut requires) = self.entry_states(&forward, id);
            let block = &self.basic_blocks[id];
            for statement in &block.statements {
                self.storage_statement(statement, &mut borrowed, &mut requires, &mut |_| ());
            }
            let Some(terminator) = &block.terminator else { continue };
            self.storage_terminator(&terminator.kind, &mut borrowed, &mut requires, &mut |_| ());
            for edge in edges(self, terminator) {
                match edge {
                    Edge::Plain(to) => flow(&mut forward, &mut worklist, to, &borrowed, &requires),
                    Edge::Returns(to, place) | Edge::Resumes(to, place) => {
                        let mut requires = requires.clone();
                        requires.insert(local_index(place.local));
                        flow(&mut forward, &mut worklist, to, &borrowed, &requires);
                    }
                }
            }
        }
        forward
    }

    /// `MaybeLiveLocals`: the locals live at the end of each block, after the effects its
    /// edges have on their own.
    fn liveness(&self, reached: &[bool]) -> Vec<Set> {
        let count = self.locals.len();
        let blocks = self.basic_blocks.len();
        let mut live_in = vec![Set::new_empty(count); blocks];
        let mut live_out = vec![Set::new_empty(count); blocks];
        let mut changed = true;
        while changed {
            changed = false;
            for (id, block) in self.basic_blocks.iter().rev() {
                if !reached[block_index(id)] {
                    continue;
                }
                let mut state = Set::new_empty(count);
                if let Some(terminator) = &block.terminator {
                    for edge in edges(self, terminator) {
                        match edge {
                            Edge::Plain(to) => {
                                state.union(&live_in[block_index(to)]);
                            }
                            Edge::Returns(to, place) => {
                                let mut entry = live_in[block_index(to)].clone();
                                if place.projection.as_slice().is_empty() {
                                    entry.remove(local_index(place.local));
                                }
                                state.union(&entry);
                            }
                            Edge::Resumes(to, place) => {
                                let mut entry = live_in[block_index(to)].clone();
                                def_use_write(place, &mut entry);
                                insert_all(&mut entry, index_locals(place));
                                state.union(&entry);
                            }
                        }
                    }
                }
                live_out[block_index(id)] = state.clone();
                if let Some(terminator) = &block.terminator {
                    live_terminator(&terminator.kind, &mut state);
                }
                for statement in block.statements.iter().rev() {
                    live_statement(&statement.kind, &mut state);
                }
                if live_in[block_index(id)] != state {
                    live_in[block_index(id)] = state;
                    changed = true;
                }
            }
        }
        live_out
    }

    /// `compute_storage_conflicts`: two held locals conflict if both need their storage at
    /// some statement or terminator of a block that does not end in `Unreachable`. One that
    /// is always storage live conflicts with every other.
    fn storage_conflicts(
        &self,
        reached: &[bool],
        forward: &Forward,
        always_live: &Set,
        held: &Set,
        saved: &[usize],
    ) -> Vec<bool> {
        let count = self.locals.len();
        let mut ineligible = always_live.clone();
        ineligible.intersect(held);
        let mut together: Vec<Set> = vec![ineligible.clone(); count];
        let mut apply = |state: &Set| {
            let mut eligible = state.clone();
            eligible.intersect(held);
            for local in eligible.iter() {
                together[local].union(&eligible);
            }
        };
        for (id, block) in self.basic_blocks.iter() {
            if !reached[block_index(id)] {
                continue;
            }
            let Some(terminator) = &block.terminator else { continue };
            if let TerminatorKind::Unreachable = terminator.kind {
                continue;
            }
            let (mut borrowed, mut requires) = self.entry_states(forward, id);
            for statement in &block.statements {
                self.storage_statement(statement, &mut borrowed, &mut requires, &mut apply);
            }
            self.storage_terminator(&terminator.kind, &mut borrowed, &mut requires, &mut apply);
        }
        let n = saved.len();
        let mut conflicts = vec![false; n * n];
        for (a, &local_a) in saved.iter().enumerate() {
            for (b, &local_b) in saved.iter().enumerate() {
                conflicts[a * n + b] =
                    ineligible.contains(local_a) || together[local_a].contains(local_b);
            }
        }
        conflicts
    }
}

/// A write to `place` as liveness sees it (`DefUse::for_place` of a store): the whole local is
/// defined, a write through a pointer uses the pointer, a write to a part is neither.
fn def_use_write(place: &StoredPlace, state: &mut Set) {
    if is_indirect(place) {
        state.insert(local_index(place.local));
    } else if place.projection.as_slice().is_empty() {
        state.remove(local_index(place.local));
    }
}

fn insert_all(state: &mut Set, locals: impl Iterator<Item = usize>) {
    for local in locals {
        state.insert(local);
    }
}

fn use_place(place: &StoredPlace, state: &mut Set) {
    state.insert(local_index(place.local));
    insert_all(state, index_locals(place));
}

fn use_operand(operand: &Operand, state: &mut Set) {
    if let Some(place) = operand_place(operand) {
        use_place(place, state);
    }
}

/// `MaybeLiveLocals` backwards over a statement: what it writes is defined first, then what it
/// reads is used.
fn live_statement(statement: &StatementKind, state: &mut Set) {
    match statement {
        StatementKind::Assign(place, rvalue) => {
            def_use_write(place, state);
            insert_all(state, index_locals(place));
            match rvalue {
                Rvalue::Ref(_, from)
                | Rvalue::Len(from)
                | Rvalue::Discriminant(from)
                | Rvalue::CopyForDeref(from) => use_place(from, state),
                _ => rvalue.for_each_operand(&mut |it| use_operand(it, state)),
            }
        }
        StatementKind::Deinit(place) => {
            def_use_write(place, state);
            insert_all(state, index_locals(place));
        }
        StatementKind::InlineAsm(_) => {
            for output in asm_outputs(statement) {
                def_use_write(output, state);
                insert_all(state, index_locals(output));
            }
            statement.for_each_operand(&mut |it| use_operand(it, state));
        }
        StatementKind::FakeRead(_)
        | StatementKind::StorageLive(_)
        | StatementKind::StorageDead(_)
        | StatementKind::Nop => (),
    }
}

/// `MaybeLiveLocals` backwards over a terminator. A call's destination and a yield's resume
/// argument are written on the edges, not here.
fn live_terminator(terminator: &TerminatorKind, state: &mut Set) {
    match terminator {
        TerminatorKind::Call { destination, .. } => {
            if is_indirect(destination) {
                state.insert(local_index(destination.local));
            }
            insert_all(state, index_locals(destination));
        }
        TerminatorKind::Drop { place, .. } => use_place(place, state),
        TerminatorKind::DropAndReplace { place, .. } => {
            def_use_write(place, state);
            use_place(place, state);
        }
        _ => (),
    }
    terminator.for_each_operand(&mut |it| use_operand(it, state));
}
