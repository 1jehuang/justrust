//! The parts of locals that a body moves out of or assigns on their own, and where each may hold
//! a value: rustc's move paths with its `MaybeInitializedPlaces` and
//! `MaybeUninitializedPlaces`, in one forward analysis.
//!
//! A move path is a local, or a field, variant field, element of an array of known length, or
//! the contents of a `Box` of another path. Only parts that are moved out of or assigned
//! somewhere in the body get one; the rest of a value goes with the path above it. A part
//! behind any other pointer, at an index that is only known when the body runs, or of a value
//! whose type implements `Drop` has none, since it cannot be moved out of. A field of a union
//! is the union.

use la_arena::ArenaMap;
use rustc_hash::{FxHashMap, FxHashSet};
use rustc_type_ir::inherent::IntoKind;
use smallvec::SmallVec;

use crate::{
    consteval::try_const_usize,
    mir::{
        BasicBlockId, BorrowKind, FieldIndex, LocalId, MirBody, OperandKind, Place, PlaceElem,
        PlaceTy, Projection, ProjectionElem, Rvalue, StatementKind, StoredPlace, TerminatorKind,
    },
    next_solver::TyKind,
};

use super::Ctx;

pub(super) type PathId = usize;

/// How a move path is reached from the one above it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Step {
    Field(FieldIndex),
    Variant(hir_def::VariantId),
    BoxContents,
    /// An element of an array whose length is known, counted from the front.
    Element(u64),
}

pub(super) struct MovePath<'db> {
    pub(super) place: StoredPlace,
    pub(super) ty: PlaceTy<'db>,
    pub(super) children: Vec<(Step, PathId)>,
}

/// What a place is in terms of move paths.
pub(super) enum Lookup {
    /// The place is this path.
    Exact(PathId),
    /// The place is a part of a path that has none of its own, as what a reference points
    /// to, or of a local that has no path at all.
    Untracked,
}

pub(super) struct MoveData<'db> {
    pub(super) paths: Vec<MovePath<'db>>,
    locals: ArenaMap<LocalId, PathId>,
}

/// Whether the projection `elem` of a place of type `ty` gets a move path of its own.
enum Tracked {
    Step(Step),
    /// A field of a union: the union stands for it.
    Union,
    /// Moves cannot reach in here.
    No,
}

impl<'db> MoveData<'db> {
    /// The move paths of `body`: one for each local whose type has drop glue, and one for each
    /// part of those that the blocks in `live` move out of, assign or drop.
    pub(super) fn gather(ctx: &Ctx<'db>, body: &MirBody<'db>, live: &[bool]) -> Self {
        let mut data = MoveData { paths: Vec::new(), locals: ArenaMap::default() };
        for (local, decl) in body.locals.iter() {
            let ty = decl.ty.as_ref();
            if ctx.needs_drop(ty) {
                let id = data.paths.len();
                data.paths.push(MovePath {
                    place: Place::from(local).store(),
                    ty: PlaceTy::from_ty(ty),
                    children: Vec::new(),
                });
                data.locals.insert(local, id);
            }
        }
        for (id, block) in body.basic_blocks.iter() {
            if !live[id.into_raw().into_u32() as usize] {
                continue;
            }
            for statement in &block.statements {
                if let StatementKind::Assign(place, _) = &statement.kind {
                    data.create(ctx, place);
                }
                statement.kind.for_each_operand(&mut |operand| {
                    if let OperandKind::Move(place) = &operand.kind {
                        data.create(ctx, place);
                    }
                });
            }
            let Some(terminator) = &block.terminator else { continue };
            terminator.kind.for_each_operand(&mut |operand| {
                if let OperandKind::Move(place) = &operand.kind {
                    data.create(ctx, place);
                }
            });
            match &terminator.kind {
                TerminatorKind::Call { destination: place, .. }
                | TerminatorKind::Drop { place, .. }
                | TerminatorKind::Yield { resume_arg: place, .. } => {
                    data.create(ctx, place);
                }
                _ => (),
            }
        }
        data
    }

    pub(super) fn for_local(&self, local: LocalId) -> Option<PathId> {
        self.locals.get(local).copied()
    }

    /// Makes the paths down to `place`, as far as they can go.
    fn create(&mut self, ctx: &Ctx<'db>, place: &StoredPlace) {
        let Some(&root) = self.locals.get(place.local) else { return };
        let mut current = root;
        for elem in place.projection.as_slice() {
            match self.tracked(ctx, current, elem) {
                Tracked::Step(step) => current = self.child(ctx, current, step),
                // Each element of a subslice of an array is moved on its own.
                Tracked::No if matches!(elem, ProjectionElem::Subslice { .. }) => {
                    for index in self.subslice(ctx, current, elem) {
                        self.child(ctx, current, Step::Element(index));
                    }
                    return;
                }
                Tracked::Union | Tracked::No => return,
            }
        }
    }

    /// The paths of the elements that a subslice projection of `parent` covers, if `parent`
    /// is an array of known length.
    fn subslice(&self, ctx: &Ctx<'db>, parent: PathId, elem: &PlaceElem) -> Vec<u64> {
        let &ProjectionElem::Subslice { from, to } = elem else { return Vec::new() };
        match self.array_len(ctx, parent) {
            Some(len) if from + to <= len => (from..len - to).collect(),
            _ => Vec::new(),
        }
    }

    fn array_len(&self, ctx: &Ctx<'db>, path: PathId) -> Option<u64> {
        let ty = &self.paths[path].ty;
        if ty.variant_id.is_some() {
            return None;
        }
        match ctx.normalize(ty.ty).kind() {
            TyKind::Array(_, len) => try_const_usize(ctx.db, len).map(|it| it as u64),
            _ => None,
        }
    }

    fn tracked(&self, ctx: &Ctx<'db>, path: PathId, elem: &PlaceElem) -> Tracked {
        let place_ty = &self.paths[path].ty;
        if let Some(_variant) = place_ty.variant_id {
            return match elem {
                ProjectionElem::Field(field) => Tracked::Step(Step::Field(*field)),
                _ => Tracked::No,
            };
        }
        let ty = ctx.normalize(place_ty.ty);
        match (elem, ty.kind()) {
            (ProjectionElem::Deref, TyKind::Adt(adt, _)) if adt.is_box() => {
                Tracked::Step(Step::BoxContents)
            }
            (ProjectionElem::Field(_), TyKind::Adt(adt, _)) if adt.is_union() => Tracked::Union,
            (ProjectionElem::Field(field), TyKind::Adt(adt, _))
                if !adt.is_box() && !ctx.has_destructor(adt.def_id()) =>
            {
                Tracked::Step(Step::Field(*field))
            }
            (ProjectionElem::Field(field), TyKind::Tuple(_) | TyKind::Closure(..)) => {
                Tracked::Step(Step::Field(*field))
            }
            (ProjectionElem::Downcast(variant), TyKind::Adt(adt, _))
                if adt.is_enum() && !ctx.has_destructor(adt.def_id()) =>
            {
                Tracked::Step(Step::Variant(*variant))
            }
            (&ProjectionElem::ConstantIndex { offset, from_end, .. }, TyKind::Array(_, len)) => {
                let Some(len) = try_const_usize(ctx.db, len).map(|it| it as u64) else {
                    return Tracked::No;
                };
                let index = if from_end { len.checked_sub(offset) } else { Some(offset) };
                match index {
                    Some(index) if index < len => Tracked::Step(Step::Element(index)),
                    _ => Tracked::No,
                }
            }
            _ => Tracked::No,
        }
    }

    /// The path `step` below `parent`, made if there is none yet.
    pub(super) fn child(&mut self, ctx: &Ctx<'db>, parent: PathId, step: Step) -> PathId {
        if let Some(&(_, id)) = self.paths[parent].children.iter().find(|(it, _)| *it == step) {
            return id;
        }
        let parent_place = self.paths[parent].place.as_ref();
        let elem = match step {
            Step::Field(field) => ProjectionElem::Field(field),
            Step::Variant(variant) => ProjectionElem::Downcast(variant),
            Step::BoxContents => ProjectionElem::Deref,
            Step::Element(index) => ProjectionElem::ConstantIndex {
                offset: index,
                min_length: index + 1,
                from_end: false,
            },
        };
        let ty = self.paths[parent].ty.projection_ty(&ctx.infcx, &elem, ctx.env);
        let place = Place {
            local: parent_place.local,
            projection: Projection::new_from_iter(
                parent_place.projection.as_slice().iter().copied().chain([elem]),
            ),
        };
        let id = self.paths.len();
        self.paths.push(MovePath { place: place.store(), ty, children: Vec::new() });
        self.paths[parent].children.push((step, id));
        id
    }

    /// The path `step` below `parent`, if there is one.
    pub(super) fn find_child(&self, parent: PathId, step: Step) -> Option<PathId> {
        self.paths[parent].children.iter().find(|(it, _)| *it == step).map(|&(_, id)| id)
    }

    /// The paths that `place` is: one, or the elements of a subslice of an array. A part of a
    /// union is the union. Empty for a place that has no path, and for a part of one that has
    /// no path of its own, as for a move out of a place behind a reference.
    pub(super) fn paths_of(&self, ctx: &Ctx<'db>, place: &StoredPlace) -> SmallVec<[PathId; 1]> {
        let Some(&root) = self.locals.get(place.local) else { return SmallVec::new() };
        let mut current = root;
        for elem in place.projection.as_slice() {
            match self.tracked(ctx, current, elem) {
                Tracked::Step(step) => match self.find_child(current, step) {
                    Some(child) => current = child,
                    None => return SmallVec::new(),
                },
                Tracked::Union => return SmallVec::from_buf([current]),
                Tracked::No if matches!(elem, ProjectionElem::Subslice { .. }) => {
                    return self
                        .subslice(ctx, current, elem)
                        .into_iter()
                        .filter_map(|index| self.find_child(current, Step::Element(index)))
                        .collect();
                }
                Tracked::No => return SmallVec::new(),
            }
        }
        SmallVec::from_buf([current])
    }

    /// What `place` is in terms of paths, for a drop of it.
    pub(super) fn lookup(&self, ctx: &Ctx<'db>, place: &StoredPlace) -> Lookup {
        let Some(&root) = self.locals.get(place.local) else { return Lookup::Untracked };
        let mut current = root;
        for elem in place.projection.as_slice() {
            match self.tracked(ctx, current, elem) {
                Tracked::Step(step) => match self.find_child(current, step) {
                    Some(child) => current = child,
                    None => return Lookup::Untracked,
                },
                Tracked::Union | Tracked::No => return Lookup::Untracked,
            }
        }
        Lookup::Exact(current)
    }

    /// `path` and every path below it.
    pub(super) fn subtree(&self, path: PathId) -> Vec<PathId> {
        let mut all = vec![path];
        let mut at = 0;
        while at < all.len() {
            all.extend(self.paths[all[at]].children.iter().map(|&(_, id)| id));
            at += 1;
        }
        all
    }

    /// Whether no part of `place` may hold a value in `state`: rustc's `is_unwind_dead`.
    pub(super) fn holds_nothing(
        &self,
        ctx: &Ctx<'db>,
        place: &StoredPlace,
        state: &InitState,
    ) -> bool {
        match self.lookup(ctx, place) {
            Lookup::Exact(path) => {
                self.subtree(path).iter().all(|&it| !state.maybe_init.contains(it))
            }
            Lookup::Untracked => false,
        }
    }

    /// The enum that the `SwitchInt` ending `block` switches on the variant of, when the
    /// block's last statement reads its discriminant: rustc's `MaybePlacesSwitchIntData`.
    pub(super) fn switch_on_variant(
        &self,
        ctx: &Ctx<'db>,
        body: &MirBody<'db>,
        block: BasicBlockId,
    ) -> Option<VariantSwitch> {
        let block = &body.basic_blocks[block];
        let Some(TerminatorKind::SwitchInt { discr, .. }) =
            block.terminator.as_ref().map(|it| &it.kind)
        else {
            return None;
        };
        let (OperandKind::Copy(discr) | OperandKind::Move(discr)) = &discr.kind else {
            return None;
        };
        let StatementKind::Assign(read, Rvalue::Discriminant(place)) =
            &block.statements.last()?.kind
        else {
            return None;
        };
        if read != discr {
            return None;
        }
        let ty = ctx.normalize(place.as_ref().ty(body, &ctx.infcx, ctx.env).ty);
        let TyKind::Adt(adt, _) = ty.kind() else { return None };
        let hir_def::AdtId::EnumId(id) = adt.def_id() else { return None };
        let Lookup::Exact(path) = self.lookup(ctx, place) else { return None };
        let width = ctx.size_of(ctx.normalize(discr.as_ref().ty(body, &ctx.infcx, ctx.env).ty));
        let mask = match width {
            Some(bytes @ ..16) => (1u128 << (bytes * 8)) - 1,
            _ => u128::MAX,
        };
        let mut variants = Vec::new();
        for &(variant, _) in id.enum_variants(ctx.db).variants.values() {
            let value = ctx.db.const_eval_discriminant(variant).ok()?;
            variants.push((hir_def::VariantId::from(variant), value as u128 & mask));
        }
        Some(VariantSwitch {
            variants,
            children: self.paths[path]
                .children
                .iter()
                .filter_map(|&(step, child)| match step {
                    Step::Variant(variant) => Some((variant, child)),
                    _ => None,
                })
                .collect(),
        })
    }
}

/// A `SwitchInt` on the variant of an enum, with the paths of the enum's variants.
pub(super) struct VariantSwitch {
    variants: Vec<(hir_def::VariantId, u128)>,
    children: Vec<(hir_def::VariantId, PathId)>,
}

impl VariantSwitch {
    /// Each edge of the switch with the paths of the variants that are not the one the enum
    /// holds along it, which hold nothing there (`on_all_inactive_variants`): every variant but
    /// a value's own, and on the `otherwise` edge those of the values the switch names.
    pub(super) fn edges(
        &self,
        targets: &crate::mir::SwitchTargets,
    ) -> Vec<(BasicBlockId, Vec<PathId>)> {
        let variant_of =
            |value: u128| self.variants.iter().find(|&&(_, it)| it == value).map(|&(it, _)| it);
        let mut edges = Vec::new();
        let mut named = Vec::new();
        for (value, to) in targets.iter() {
            let Some(active) = variant_of(value) else {
                return targets.all_targets().iter().map(|&to| (to, Vec::new())).collect();
            };
            named.push(active);
            let inactive = self.children.iter().filter(|&&(it, _)| it != active);
            edges.push((to, inactive.map(|&(_, path)| path).collect()));
        }
        let inactive = self.children.iter().filter(|(it, _)| named.contains(it));
        edges.push((targets.otherwise(), inactive.map(|&(_, path)| path).collect()));
        edges
    }
}

/// A set of move paths.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Bits(Vec<u64>);

impl Bits {
    pub(super) fn new(len: usize) -> Self {
        Bits(vec![0; len.div_ceil(64)])
    }

    pub(super) fn contains(&self, at: PathId) -> bool {
        self.0[at / 64] & (1 << (at % 64)) != 0
    }

    pub(super) fn insert(&mut self, at: PathId) {
        self.0[at / 64] |= 1 << (at % 64);
    }

    pub(super) fn remove(&mut self, at: PathId) {
        self.0[at / 64] &= !(1 << (at % 64));
    }

    /// Adds the members of `other`, and says whether that changed anything.
    fn union(&mut self, other: &Bits) -> bool {
        let mut changed = false;
        for (word, &more) in self.0.iter_mut().zip(&other.0) {
            changed |= *word | more != *word;
            *word |= more;
        }
        changed
    }
}

/// Which paths may hold a value, and which may not, at one point of the body.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct InitState {
    pub(super) maybe_init: Bits,
    pub(super) maybe_uninit: Bits,
}

impl InitState {
    pub(super) fn bottom(len: usize) -> Self {
        InitState { maybe_init: Bits::new(len), maybe_uninit: Bits::new(len) }
    }

    fn join(&mut self, other: &InitState) -> bool {
        let init = self.maybe_init.union(&other.maybe_init);
        let uninit = self.maybe_uninit.union(&other.maybe_uninit);
        init || uninit
    }

    pub(super) fn set(&mut self, paths: &[PathId], init: bool) {
        for &path in paths {
            if init {
                self.maybe_init.insert(path);
                self.maybe_uninit.remove(path);
            } else {
                self.maybe_init.remove(path);
                self.maybe_uninit.insert(path);
            }
        }
    }

    pub(super) fn apply(&mut self, paths: &[PathId], change: Change) {
        match change {
            Change::Init => self.set(paths, true),
            Change::Uninit => self.set(paths, false),
            Change::Borrowed => paths.iter().for_each(|&path| self.maybe_init.insert(path)),
        }
    }

    /// (maybe init, maybe uninit) of `path` itself.
    pub(super) fn of(&self, path: PathId) -> (bool, bool) {
        (self.maybe_init.contains(path), self.maybe_uninit.contains(path))
    }
}

/// Something a statement or terminator does to the paths, with every path below the one it
/// names.
#[derive(Clone, Copy, Debug)]
pub(super) struct Effect {
    pub(super) path: PathId,
    pub(super) change: Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// An assignment: the paths hold a value.
    Init,
    /// A move out, a drop, `StorageDead`: they hold none.
    Uninit,
    /// A mutable borrow (`&mut`, `&raw mut`), through which they may be assigned: they may hold a
    /// value, and may still hold none, as rustc's `MaybeInitializedPlaces` has it (#90752: an enum
    /// a `match` found `None` and then `*(&mut x) = Some(..)`). It writes no drop flag. rustc
    /// counts `&raw const` too, which MIR lowering here makes a shared borrow.
    Borrowed,
}

/// The effects of the statement `at` of `block`, or of its terminator when `at` is the number
/// of statements, apart from what a terminator does on one of its edges only. In the order
/// they happen: moves out of the operands, then the write.
pub(super) fn effects_at<'db>(
    ctx: &Ctx<'db>,
    data: &MoveData<'db>,
    body: &MirBody<'db>,
    block: BasicBlockId,
    at: usize,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    let moved = |operand: &crate::mir::Operand, effects: &mut Vec<Effect>| {
        if let OperandKind::Move(place) = &operand.kind {
            for path in data.paths_of(ctx, place) {
                effects.push(Effect { path, change: Change::Uninit });
            }
        }
    };
    let data_block = &body.basic_blocks[block];
    if let Some(statement) = data_block.statements.get(at) {
        statement.kind.for_each_operand(&mut |operand| moved(operand, &mut effects));
        match &statement.kind {
            StatementKind::Assign(place, rvalue) => {
                for path in data.paths_of(ctx, &init_target(ctx, body, place)) {
                    effects.push(Effect { path, change: Change::Init });
                }
                if let Rvalue::Ref(BorrowKind::Mut { .. }, borrowed) = rvalue
                    && let Lookup::Exact(path) = data.lookup(ctx, borrowed)
                {
                    effects.push(Effect { path, change: Change::Borrowed });
                }
            }
            StatementKind::Deinit(place) => {
                for path in data.paths_of(ctx, place) {
                    effects.push(Effect { path, change: Change::Uninit });
                }
            }
            StatementKind::StorageDead(local) => {
                if let Some(path) = data.for_local(*local) {
                    effects.push(Effect { path, change: Change::Uninit });
                }
            }
            StatementKind::InlineAsm(asm) => {
                for operand in &asm.operands {
                    if let crate::mir::AsmOperand::Reg { output: Some(place), .. } = operand {
                        for path in data.paths_of(ctx, place) {
                            effects.push(Effect { path, change: Change::Init });
                        }
                    }
                }
            }
            StatementKind::FakeRead(_) | StatementKind::StorageLive(_) | StatementKind::Nop => (),
        }
        return effects;
    }
    let Some(terminator) = &data_block.terminator else { return effects };
    terminator.kind.for_each_operand(&mut |operand| moved(operand, &mut effects));
    if let TerminatorKind::Drop { place, .. } = &terminator.kind {
        for path in data.paths_of(ctx, place) {
            effects.push(Effect { path, change: Change::Uninit });
        }
    }
    effects
}

/// The place an assignment to `place` initializes: the union, for a field of one.
fn init_target<'db>(ctx: &Ctx<'db>, body: &MirBody<'db>, place: &StoredPlace) -> StoredPlace {
    let projection = place.projection.as_slice();
    if let Some((ProjectionElem::Field(_), base)) = projection.split_last() {
        let base_place = Place { local: place.local, projection: Projection::new_from_slice(base) };
        let base_ty = base_place.ty(body, &ctx.infcx, ctx.env);
        if base_ty.variant_id.is_none()
            && let TyKind::Adt(adt, _) = ctx.normalize(base_ty.ty).kind()
            && adt.is_union()
        {
            return base_place.store();
        }
    }
    place.clone()
}

/// What the edge from the terminator of a block to `to` initializes on its own: the
/// destination of a call when it returns, what a coroutine is resumed with.
pub(super) fn edge_effects<'db>(
    ctx: &Ctx<'db>,
    data: &MoveData<'db>,
    body: &MirBody<'db>,
    from: BasicBlockId,
    to: BasicBlockId,
) -> SmallVec<[PathId; 1]> {
    match body.basic_blocks[from].terminator.as_ref().map(|it| &it.kind) {
        Some(TerminatorKind::Call { destination, target: Some(target), .. }) if *target == to => {
            data.paths_of(ctx, destination)
        }
        Some(TerminatorKind::Yield { resume_arg, resume, .. }) if *resume == to => {
            data.paths_of(ctx, resume_arg)
        }
        _ => SmallVec::new(),
    }
}

/// The blocks of the body control can reach from where it starts without going into a block
/// that rustc builds no way into ([`MirBody::unreached`]), cleanup blocks included.
pub(super) fn live_blocks(body: &MirBody<'_>) -> Vec<bool> {
    let index = |id: BasicBlockId| id.into_raw().into_u32() as usize;
    let mut live = vec![false; body.basic_blocks.len()];
    let unreached: rustc_hash::FxHashSet<_> = body.unreached.iter().copied().collect();
    let mut stack = vec![body.start_block];
    while let Some(id) = stack.pop() {
        if std::mem::replace(&mut live[index(id)], true) {
            continue;
        }
        let block = &body.basic_blocks[id];
        let Some(terminator) = &block.terminator else { continue };
        let next = terminator
            .kind
            .successors()
            .into_iter()
            .filter(|it| !unreached.contains(it))
            .chain(terminator.kind.unwind());
        stack.extend(next.filter(|&it| !live[index(it)]));
    }
    live
}

/// Where each path may hold a value where each live block starts.
pub(super) struct InitAnalysis {
    pub(super) entry: Vec<Option<InitState>>,
}

impl InitAnalysis {
    /// The analysis, which does not go into cleanup from the drops in `dead_unwinds` with what
    /// may be uninitialized there, as rustc's `MaybeUninitializedPlaces` skips their unwind
    /// edges (`skipping_unreachable_unwind`).
    pub(super) fn run<'db>(
        ctx: &Ctx<'db>,
        data: &MoveData<'db>,
        body: &MirBody<'db>,
        live: &[bool],
        dead_unwinds: &FxHashSet<BasicBlockId>,
    ) -> Self {
        let index = |id: BasicBlockId| id.into_raw().into_u32() as usize;
        let len = data.paths.len();
        let unreached: rustc_hash::FxHashSet<_> = body.unreached.iter().copied().collect();
        let mut entry: Vec<Option<InitState>> = vec![None; body.basic_blocks.len()];
        let mut start = InitState::bottom(len);
        start.set(&(0..len).collect::<Vec<_>>(), false);
        for param in body.entry_locals() {
            if let Some(path) = data.for_local(param) {
                start.set(&data.subtree(path), true);
            }
        }
        entry[index(body.start_block)] = Some(start);
        let subtrees: Vec<Vec<PathId>> = (0..len).map(|it| data.subtree(it)).collect();
        let effects: FxHashMap<(BasicBlockId, usize), Vec<Effect>> = FxHashMap::default();
        let mut effects = effects;
        let mut pending = vec![body.start_block];
        let mut queued = vec![false; body.basic_blocks.len()];
        queued[index(body.start_block)] = true;
        while let Some(id) = pending.pop() {
            queued[index(id)] = false;
            let block = &body.basic_blocks[id];
            let mut state = entry[index(id)].clone().expect("a block is queued with a state");
            let mut flow = |to: BasicBlockId,
                            state: &InitState,
                            entry: &mut Vec<Option<InitState>>,
                            pending: &mut Vec<BasicBlockId>| {
                if !live[index(to)] {
                    return;
                }
                let changed = match &mut entry[index(to)] {
                    Some(known) => known.join(state),
                    slot @ None => {
                        *slot = Some(state.clone());
                        true
                    }
                };
                if changed && !std::mem::replace(&mut queued[index(to)], true) {
                    pending.push(to);
                }
            };
            for at in 0..block.statements.len() {
                let here =
                    effects.entry((id, at)).or_insert_with(|| effects_at(ctx, data, body, id, at));
                for effect in here.iter() {
                    state.apply(&subtrees[effect.path], effect.change);
                }
            }
            let Some(terminator) = &block.terminator else { continue };
            // rustc's `skipping_unreachable_unwind`: a drop of a place that holds nothing does
            // nothing, so nothing unwinds out of it. `MaybeInitializedPlaces` finds that as it
            // goes, `MaybeUninitializedPlaces` takes the drops the first found.
            let (skip_init, skip_uninit) = match &terminator.kind {
                TerminatorKind::Drop { place, unwind: Some(_), .. } => {
                    (data.holds_nothing(ctx, place, &state), dead_unwinds.contains(&id))
                }
                _ => (false, false),
            };
            let at = block.statements.len();
            let here =
                effects.entry((id, at)).or_insert_with(|| effects_at(ctx, data, body, id, at));
            for effect in here.iter() {
                state.apply(&subtrees[effect.path], effect.change);
            }
            if let Some(cleanup) = terminator.kind.unwind() {
                let mut unwound = state.clone();
                if skip_init {
                    unwound.maybe_init = Bits::new(len);
                }
                if skip_uninit {
                    unwound.maybe_uninit = Bits::new(len);
                }
                flow(cleanup, &unwound, &mut entry, &mut pending);
            }
            if let TerminatorKind::SwitchInt { targets, .. } = &terminator.kind
                && let Some(switch) = data.switch_on_variant(ctx, body, id)
            {
                for (to, inactive) in switch.edges(targets) {
                    if unreached.contains(&to) {
                        continue;
                    }
                    let mut state = state.clone();
                    for path in inactive {
                        state.set(&subtrees[path], false);
                    }
                    flow(to, &state, &mut entry, &mut pending);
                }
                continue;
            }
            for to in terminator.kind.successors() {
                if unreached.contains(&to) {
                    continue;
                }
                let initialized = edge_effects(ctx, data, body, id, to);
                if initialized.is_empty() {
                    flow(to, &state, &mut entry, &mut pending);
                } else {
                    let mut state = state.clone();
                    for path in initialized {
                        state.set(&subtrees[path], true);
                    }
                    flow(to, &state, &mut entry, &mut pending);
                }
            }
        }
        InitAnalysis { entry }
    }

    /// rustc's `compute_dead_unwinds`: the drops that may unwind of a place that holds nothing
    /// where they are. Such a drop does nothing, so nothing unwinds out of it: the operand of
    /// an aggregate a scope drops after the aggregate took it, say.
    pub(super) fn dead_unwinds<'db>(
        &self,
        ctx: &Ctx<'db>,
        data: &MoveData<'db>,
        body: &MirBody<'db>,
    ) -> FxHashSet<BasicBlockId> {
        let index = |id: BasicBlockId| id.into_raw().into_u32() as usize;
        let mut dead = FxHashSet::default();
        for (id, block) in body.basic_blocks.iter() {
            let Some(TerminatorKind::Drop { place, unwind: Some(_), .. }) =
                block.terminator.as_ref().map(|it| &it.kind)
            else {
                continue;
            };
            let Some(entry) = &self.entry[index(id)] else { continue };
            let mut state = entry.clone();
            for at in 0..block.statements.len() {
                for effect in effects_at(ctx, data, body, id, at) {
                    state.apply(&data.subtree(effect.path), effect.change);
                }
            }
            if data.holds_nothing(ctx, place, &state) {
                dead.insert(id);
            }
        }
        dead
    }
}
