//! Liveness, as rustc's `type_check/liveness`: a region holds at each point where a local
//! whose type has it may still be used, and where a local may still be dropped while it may
//! be initialized, for the regions dropping it uses.

use hir_def::{AdtId, attrs::AttrFlags};
use rustc_hash::{FxHashMap, FxHashSet};
use rustc_type_ir::{
    ClauseKind, OutlivesPredicate, RegionKind,
    inherent::{
        AdtDef as _, GenericArgs as _, IntoKind, ParamEnv as _, Region as _, SliceLike as _,
    },
};
use salsa::SalsaValue;
use smallvec::SmallVec;

use hir_ty::{
    db::HirDatabase,
    mir::{
        BasicBlockId, LocalId, MirBody, Operand, OperandKind, PlaceElem, Rvalue, StatementKind,
        TerminatorKind,
    },
    next_solver::{
        Clause, DbInterner, GenericArgKind, GenericArgs, Region, StoredRegion, StoredTy, Ty, TyKind,
    },
};

use super::{
    Ctx,
    bitset::BitSet,
    cfg::{Cfg, Location, index},
    intervals::IntervalSet,
    moves::{InitKind, MoveData},
    regions::{Regions, Rv, regions_of, rv_of},
};

#[derive(Default)]
struct Effects {
    uses: Vec<LocalId>,
    defs: Vec<LocalId>,
    drops: Vec<LocalId>,
}

fn place_uses(local: LocalId, projection: &[PlaceElem], uses: &mut Vec<LocalId>) {
    uses.push(local);
    for elem in projection {
        if let PlaceElem::Index(index) = elem {
            uses.push(*index);
        }
    }
}

fn operand_uses(operand: &Operand, uses: &mut Vec<LocalId>) {
    if let OperandKind::Copy(place) | OperandKind::Move(place) = &operand.kind {
        place_uses(place.local, place.projection.as_slice(), uses);
    }
}

/// A write of a whole local defines it; a write of part of it uses it.
fn assigned(local: LocalId, projection: &[PlaceElem], effects: &mut Effects) {
    if projection.is_empty() {
        effects.defs.push(local);
    } else {
        place_uses(local, projection, &mut effects.uses);
    }
}

/// The values of expression statements (`match x { None => &temp(), Some(it) => it };`),
/// which lowering reads once the statement's temporaries are dropped, and rustc does not; with,
/// for a `match`, the type of its value, which tells that read from the read of what it matches
/// (they have the same span), where the two types differ.
fn statement_values<'db>(ctx: &Ctx<'_, 'db>) -> FxHashMap<hir_def::hir::ExprId, Option<Ty<'db>>> {
    let erased = |ty: Ty<'db>| {
        rustc_type_ir::fold_regions(ctx.infcx.interner, ty, |_, _| {
            Region::new_erased(ctx.infcx.interner)
        })
    };
    let mut found = FxHashMap::default();
    for (_, expr) in ctx.store.exprs() {
        let hir_def::hir::Expr::Block { statements, .. } = expr else { continue };
        for statement in statements.iter() {
            let hir_def::hir::Statement::Expr { expr, .. } = statement else { continue };
            match ctx.store[*expr] {
                hir_def::hir::Expr::Match { expr: matched, .. } => {
                    let value = erased(ctx.infer.expr_ty(*expr));
                    if value != erased(ctx.infer.expr_ty(matched)) {
                        found.insert(*expr, Some(value));
                    }
                }
                _ => {
                    found.insert(*expr, None);
                }
            }
        }
    }
    found
}

fn effects_at<'db>(
    ctx: &Ctx<'_, 'db>,
    body: &MirBody<'db>,
    location: Location,
    statement_values: &FxHashMap<hir_def::hir::ExprId, Option<Ty<'db>>>,
) -> Effects {
    let mut effects = Effects::default();
    let data = &body.basic_blocks[location.block];
    if let Some(statement) = data.statements.get(location.statement) {
        match &statement.kind {
            StatementKind::Assign(place, rvalue) => {
                match rvalue {
                    Rvalue::Ref(_, borrowed)
                    | Rvalue::Len(borrowed)
                    | Rvalue::Discriminant(borrowed)
                    | Rvalue::CopyForDeref(borrowed) => place_uses(
                        borrowed.local,
                        borrowed.projection.as_slice(),
                        &mut effects.uses,
                    ),
                    _ => rvalue.for_each_operand(&mut |it| operand_uses(it, &mut effects.uses)),
                }
                assigned(place.local, place.projection.as_slice(), &mut effects);
            }
            StatementKind::FakeRead(place)
                if let hir_ty::mir::MirSpan::ExprId(expr) = statement.span
                    && let Some(value) = statement_values.get(&expr)
                    && value.is_none_or(|value| {
                        place.projection.is_empty()
                            && rustc_type_ir::fold_regions(
                                ctx.infcx.interner,
                                body.locals[place.local].ty.as_ref(),
                                |_, _| Region::new_erased(ctx.infcx.interner),
                            ) == value
                    }) => {}
            StatementKind::FakeRead(place) => {
                place_uses(place.local, place.projection.as_slice(), &mut effects.uses)
            }
            StatementKind::Deinit(place) => {
                assigned(place.local, place.projection.as_slice(), &mut effects)
            }
            StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                effects.defs.push(*local)
            }
            StatementKind::InlineAsm(_) => {
                statement.kind.for_each_operand(&mut |it| operand_uses(it, &mut effects.uses))
            }
            StatementKind::Nop => (),
        }
    } else if let Some(terminator) = &data.terminator {
        match &terminator.kind {
            TerminatorKind::Drop { place, .. } => {
                if place.projection.is_empty() {
                    effects.drops.push(place.local);
                } else {
                    place_uses(place.local, place.projection.as_slice(), &mut effects.uses);
                }
            }
            TerminatorKind::DropAndReplace { place, value, .. } => {
                operand_uses(value, &mut effects.uses);
                place_uses(place.local, place.projection.as_slice(), &mut effects.uses);
            }
            TerminatorKind::Call { func, args, destination, .. } => {
                // The destination is written after the arguments are read.
                assigned(destination.local, destination.projection.as_slice(), &mut effects);
                operand_uses(func, &mut effects.uses);
                for arg in args.iter() {
                    operand_uses(arg, &mut effects.uses);
                }
            }
            TerminatorKind::Return => effects.uses.push(super::return_slot()),
            kind => kind.for_each_operand(&mut |it| operand_uses(it, &mut effects.uses)),
        }
    }
    effects
}

/// The points each local is live at, by a backward analysis where `uses` says which locals a
/// point uses, as ranges: what liveness gives a region is the points of the locals it is in.
fn live_ranges(
    body: &MirBody<'_>,
    cfg: &Cfg,
    effects: &[Effects],
    uses: impl Fn(&Effects) -> &[LocalId],
    edge: impl Fn(BasicBlockId, BasicBlockId, &mut BitSet),
) -> Vec<IntervalSet> {
    let locals = body.locals.len();
    let blocks = body.basic_blocks.len();
    let mut entry = vec![BitSet::new(locals); blocks];
    let mut changed = true;
    while changed {
        changed = false;
        for &block in cfg.order.iter().rev() {
            let mut live = BitSet::new(locals);
            for &successor in &cfg.successors[index(block)] {
                let mut from = entry[index(successor)].clone();
                edge(block, successor, &mut from);
                live.union(&from);
            }
            let start = cfg.block_start(block);
            let end = cfg.block_end(body, block);
            for point in (start..=end).rev() {
                let it = &effects[point];
                for local in &it.defs {
                    live.remove(local.into_raw().into_u32() as usize);
                }
                for local in uses(it) {
                    live.insert(local.into_raw().into_u32() as usize);
                }
            }
            if entry[index(block)] != live {
                entry[index(block)] = live;
                changed = true;
            }
        }
    }
    let mut ranges = vec![IntervalSet::default(); locals];
    // Where the run of points each live local is live at ends, walking backwards.
    let mut run_end = vec![0usize; locals];
    for &block in &cfg.order {
        let mut live = BitSet::new(locals);
        for &successor in &cfg.successors[index(block)] {
            let mut from = entry[index(successor)].clone();
            edge(block, successor, &mut from);
            live.union(&from);
        }
        let start = cfg.block_start(block);
        let end = cfg.block_end(body, block);
        for local in live.iter() {
            run_end[local] = end;
        }
        for point in (start..=end).rev() {
            let it = &effects[point];
            // A local is live at a point that uses it, and where it is assigned if it is live
            // after, as rustc's `compute_use_live_points_for` has it; it is not live before
            // the assignment.
            for local in uses(it) {
                let local = local.into_raw().into_u32() as usize;
                if live.insert(local) {
                    run_end[local] = point;
                }
            }
            for local in &it.defs {
                let local = local.into_raw().into_u32() as usize;
                if uses(it).iter().any(|it| it.into_raw().into_u32() as usize == local) {
                    continue;
                }
                if live.contains(local) {
                    live.remove(local);
                    ranges[local].insert_range(point, run_end[local]);
                }
            }
        }
        for local in live.iter() {
            ranges[local].insert_range(start, run_end[local]);
        }
    }
    ranges
}

/// The value of each region from liveness alone.
pub(super) fn region_liveness<'db>(
    ctx: &Ctx<'_, 'db>,
    body: &MirBody<'db>,
    cfg: &Cfg,
    moves: &MoveData,
    regions: &Regions<'db>,
    count: usize,
) -> Vec<IntervalSet> {
    let mut values = vec![IntervalSet::default(); count];
    let mut effects: Vec<Effects> = (0..cfg.points).map(|_| Effects::default()).collect();
    let statement_values = statement_values(ctx);
    for &block in &cfg.order {
        for point in cfg.block_start(block)..=cfg.block_end(body, block) {
            effects[point] = effects_at(ctx, body, cfg.location(point), &statement_values);
        }
    }
    for &(local, location) in &regions.callee_reads {
        effects[cfg.point(location)].uses.push(local);
    }
    // A drop of what cannot be initialized there does nothing.
    let (maybe_init, init_at_exit) = maybe_initialized_at_drops(body, cfg, moves);
    for (point, it) in effects.iter_mut().enumerate() {
        it.drops.retain(|&local| maybe_init.get(&point).is_some_and(|set| set.contains(&local)));
    }
    let local_regions: Vec<Vec<Rv>> = regions
        .local_tys
        .iter()
        .map(|&ty| {
            let mut out = Vec::new();
            live_regions_of(ctx, regions, ty, &mut out);
            out
        })
        .collect();
    let use_live = live_ranges(body, cfg, &effects, |it| &it.uses, |_, _, _| ());
    for (local, points) in use_live.iter().enumerate() {
        if points.is_empty() {
            continue;
        }
        for &rv in &local_regions[local] {
            if rv < count {
                values[rv].union(points);
            }
        }
    }
    let mut drop_live = live_ranges(
        body,
        cfg,
        &effects,
        |it| &it.drops,
        // Dropping a local makes it live back to where it may be initialized only, as
        // rustc's `compute_drop_live_points_for` has it.
        |block, successor, live| init_at_exit.restrict(block, successor, live),
    );
    // Where a local is use live, what dropping it uses is live already.
    let mut dropped = Vec::new();
    for (local, points) in drop_live.iter_mut().enumerate() {
        points.subtract(&use_live[local]);
        if !points.is_empty() {
            dropped.push(local);
        }
    }
    if dropped.is_empty() {
        return values;
    }
    // What dropping each local uses, found in the order a walk of the blocks in reverse
    // postorder, each from its end, first meets the local at a point it is drop live at only:
    // finding it may normalize, which makes region variables.
    let mut rank = vec![usize::MAX; cfg.points];
    let mut next = 0;
    for &block in &cfg.order {
        for point in (cfg.block_start(block)..=cfg.block_end(body, block)).rev() {
            rank[point] = next;
            next += 1;
        }
    }
    dropped.sort_by_cached_key(|&local| {
        (drop_live[local].iter().map(|point| rank[point]).min().unwrap_or(usize::MAX), local)
    });
    let mut components = DropComponents::new(ctx, regions);
    for local in dropped {
        for &rv in components.of(regions.local_tys[local]) {
            if rv < count {
                values[rv].union(&drop_live[local]);
            }
        }
    }
    values
}

/// For each drop, the locals it drops that may be initialized there; and for each edge
/// between blocks, the locals that may be initialized, if partly, leaving the first for the
/// second.
fn maybe_initialized_at_drops<'m>(
    body: &MirBody<'_>,
    cfg: &Cfg,
    moves: &'m MoveData,
) -> (FxHashMap<usize, Vec<LocalId>>, InitAtExit<'m>) {
    let paths = moves.paths.len();
    let blocks = body.basic_blocks.len();
    let mut entry: Vec<Option<BitSet>> = vec![None; blocks];
    let mut start = BitSet::new(paths);
    for init in moves.inits.iter().filter(|it| it.location.is_none()) {
        moves.with_descendants(init.path, &mut |it| {
            start.insert(it);
        });
    }
    entry[index(body.start_block)] = Some(start);
    let apply = |location: Location, state: &mut BitSet, returned: bool| {
        if !returned {
            if let Some(at) = moves.moves_at.get(&location) {
                for &it in at {
                    moves.with_descendants(moves.moves[it].path, &mut |path| state.remove(path));
                }
            }
        }
        if let Some(at) = moves.inits_at.get(&location) {
            for &it in at {
                let init = moves.inits[it];
                if (init.kind == InitKind::Deep) != returned {
                    moves.with_descendants(init.path, &mut |path| {
                        state.insert(path);
                    });
                }
            }
        }
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &block in &cfg.order {
            let Some(mut state) = entry[index(block)].clone() else { continue };
            let data = &body.basic_blocks[block];
            for statement in 0..=data.statements.len() {
                apply(Location { block, statement }, &mut state, false);
            }
            let returned = match data.terminator.as_ref().map(|it| &it.kind) {
                Some(TerminatorKind::Call { target: Some(target), .. }) => Some(*target),
                _ => None,
            };
            for &successor in &cfg.successors[index(block)] {
                let mut state = state.clone();
                if Some(successor) == returned {
                    apply(Location { block, statement: data.statements.len() }, &mut state, true);
                }
                match &mut entry[index(successor)] {
                    Some(it) => changed |= it.union(&state),
                    slot @ None => {
                        *slot = Some(state);
                        changed = true;
                    }
                }
            }
        }
    }
    let mut exits: Vec<Option<(BitSet, Option<(BasicBlockId, BitSet)>)>> = vec![None; blocks];
    let mut found: FxHashMap<usize, Vec<LocalId>> = FxHashMap::default();
    for &block in &cfg.order {
        let Some(mut state) = entry[index(block)].take() else { continue };
        let data = &body.basic_blocks[block];
        for statement in 0..data.statements.len() {
            apply(Location { block, statement }, &mut state, false);
        }
        let mut exit = state.clone();
        apply(Location { block, statement: data.statements.len() }, &mut exit, false);
        let returned = match data.terminator.as_ref().map(|it| &it.kind) {
            Some(TerminatorKind::Call { target: Some(target), .. }) => {
                let mut returned = exit.clone();
                apply(Location { block, statement: data.statements.len() }, &mut returned, true);
                Some((*target, returned))
            }
            _ => None,
        };
        exits[index(block)] = Some((exit, returned));
        if let Some(TerminatorKind::Drop { place, .. }) =
            data.terminator.as_ref().map(|it| &it.kind)
            && place.projection.is_empty()
        {
            let path = moves.local_path(place.local);
            if moves.find_in_descendants(path, &|it| state.contains(it)).is_some() {
                let point = cfg.point(Location { block, statement: data.statements.len() });
                found.entry(point).or_default().push(place.local);
            }
        }
    }
    (found, InitAtExit { moves, exits })
}

/// The paths that may be initialized leaving each block, and leaving a call's block for the
/// call's return.
struct InitAtExit<'m> {
    moves: &'m MoveData,
    exits: Vec<Option<(BitSet, Option<(BasicBlockId, BitSet)>)>>,
}

impl InitAtExit<'_> {
    /// Keeps of `live` the locals that may be initialized, if partly, leaving `block` for
    /// `successor`: every path of a local is below the local's own, so where one of its paths
    /// is. Asked of the few locals live there rather than of every path that is initialized.
    fn restrict(&self, block: BasicBlockId, successor: BasicBlockId, live: &mut BitSet) {
        let state = match &self.exits[index(block)] {
            Some((_, Some((target, returned)))) if *target == successor => returned,
            Some((exit, _)) => exit,
            None => {
                live.clear();
                return;
            }
        };
        let gone: SmallVec<[usize; 8]> = live
            .iter()
            .filter(|&local| {
                let path = self
                    .moves
                    .local_path(LocalId::from_raw(la_arena::RawIdx::from_u32(local as u32)));
                self.moves.find_in_descendants(path, &|it| state.contains(it)).is_none()
            })
            .collect();
        for local in gone {
            live.remove(local);
        }
    }
}

/// The regions dropping a value of a type may use: rustc's dropck outlives constraints, found
/// over the graph of the types dropping it drops. A recursive type and the types it holds
/// that hold it again are one strongly connected component of that graph with one answer, so
/// each type is expanded once, as rustc's dropck keeps a set of the types it has seen; walking
/// the tree of drops instead is exponential in a type like `Value(Vec<Value>, Box<Value>)`.
struct DropComponents<'c, 'a, 'db> {
    ctx: &'c Ctx<'a, 'db>,
    regions: &'c Regions<'db>,
    done: FxHashMap<Ty<'db>, Vec<Rv>>,
    /// Tarjan's: each type on the stack, its number in the walk, and its place on the stack.
    open: FxHashMap<Ty<'db>, (usize, usize)>,
    stack: Vec<(Ty<'db>, Vec<Rv>)>,
    next: usize,
}

/// How deep in the walk a type stops being expanded, which bounds types that grow as they
/// recurse, `struct S<T>(Option<Box<S<(T, T)>>>)`.
const DROP_DEPTH: usize = 12;

impl<'c, 'a, 'db> DropComponents<'c, 'a, 'db> {
    fn new(ctx: &'c Ctx<'a, 'db>, regions: &'c Regions<'db>) -> Self {
        DropComponents {
            ctx,
            regions,
            done: FxHashMap::default(),
            open: FxHashMap::default(),
            stack: Vec::new(),
            next: 0,
        }
    }

    fn of(&mut self, ty: Ty<'db>) -> &[Rv] {
        self.visit(ty, 0);
        self.done.get(&ty).map_or(&[], |it| it.as_slice())
    }

    /// Walks `ty`: `Some(low)`, the lowest number it reaches, while its component is open.
    fn visit(&mut self, ty: Ty<'db>, depth: usize) -> Option<usize> {
        if depth > DROP_DEPTH || self.done.contains_key(&ty) {
            return None;
        }
        if let Some(&(number, _)) = self.open.get(&ty) {
            return Some(number);
        }
        let number = self.next;
        self.next += 1;
        let at = self.stack.len();
        self.open.insert(ty, (number, at));
        self.stack.push((ty, Vec::new()));
        let mut found = Vec::new();
        let mut children = Vec::new();
        self.expand(ty, &mut found, &mut children);
        let mut low = number;
        for child in children {
            match self.visit(child, depth + 1) {
                Some(it) => low = low.min(it),
                None => found.extend(self.done.get(&child).into_iter().flatten()),
            }
        }
        if low < number {
            self.stack[at].1 = found;
            return Some(low);
        }
        let members: Vec<_> = self.stack.drain(at..).collect();
        for (_, theirs) in &members {
            found.extend_from_slice(theirs);
        }
        found.sort_unstable();
        found.dedup();
        for (member, _) in members {
            self.open.remove(&member);
            self.done.insert(member, found.clone());
        }
        None
    }

    /// The regions dropping a `ty` uses itself, and the types it drops.
    fn expand(&self, ty: Ty<'db>, found: &mut Vec<Rv>, children: &mut Vec<Ty<'db>>) {
        let ctx = self.ctx;
        match ty.kind() {
            // Dropping a `ManuallyDrop` drops nothing.
            TyKind::Adt(def, _) if def.is_manually_drop() => (),
            TyKind::Adt(def, args) => {
                let adt = def.def_id();
                if def.is_box() || def.is_phantom_data() {
                    if let Some(GenericArgKind::Type(inner)) =
                        args.iter().next().map(|it| it.kind())
                    {
                        children.push(inner);
                    }
                    return;
                }
                let interner = ctx.infcx.interner;
                let constraint = drop_constraint(ctx.db, adt);
                for region in &constraint.regions {
                    let region = rustc_type_ir::EarlyBinder::bind(region.as_ref())
                        .instantiate(interner, args)
                        .skip_norm_wip();
                    found.extend(super::regions::rv_of(region));
                }
                for whole in &constraint.whole {
                    let whole = rustc_type_ir::EarlyBinder::bind(whole.as_ref())
                        .instantiate(interner, args)
                        .skip_norm_wip();
                    regions_of(whole, found);
                }
                children.extend(constraint.tys.iter().map(|it| {
                    rustc_type_ir::EarlyBinder::bind(it.as_ref())
                        .instantiate(interner, args)
                        .skip_norm_wip()
                }));
            }
            TyKind::Array(inner, _) | TyKind::Slice(inner) => children.push(inner),
            TyKind::Tuple(tys) => children.extend(tys.iter()),
            TyKind::Closure(_, args) => children.push(args.as_closure().tupled_upvars_ty()),
            // rustc's dropck drops what a coroutine captures too, all of it where what it keeps
            // across an `await` needs dropping, which is not known here.
            TyKind::CoroutineClosure(_, args) => {
                children.push(args.as_coroutine_closure().tupled_upvars_ty())
            }
            TyKind::Coroutine(_, args) => children.push(args.as_coroutine().tupled_upvars_ty()),
            TyKind::Dynamic(..) => regions_of(ty, found),
            // A projection is what it normalizes to, `NoisyDrop<T>` of `<T as Associator>::As`;
            // another alias holds what its bounds and captures let it, as for liveness.
            TyKind::Alias(alias) => {
                let normalized = match alias.kind {
                    rustc_type_ir::AliasTyKind::Projection { .. } => {
                        super::regions::normalize(ctx, ty)
                            .filter(|it| !matches!(it.kind(), TyKind::Alias(_) | TyKind::Infer(_)))
                    }
                    _ => None,
                };
                match normalized {
                    Some(it) => children.push(it),
                    None => live_regions_of(ctx, self.regions, ty, found),
                }
            }
            _ => (),
        }
    }
}

/// What dropping a value of an ADT other than `Box`, `PhantomData` and `ManuallyDrop` uses, in
/// its own generic parameters: rustc's `adt_dtorck_constraint`. Its fields are taken apart as
/// far as their types tell without an instantiation, an ADT in them by its own constraint;
/// what is left, parameters, aliases and the types an ADT in them leaves, is taken apart where
/// the ADT is dropped, which is also what keeps two ADTs that hold each other from asking each
/// other. Once per ADT, so a body that drops a `Value` instantiates what that leaves rather
/// than every field of every variant.
#[derive(Debug, Clone, PartialEq, Eq, Default, SalsaValue)]
struct DropConstraint {
    /// The lifetimes its `Drop` impl uses.
    regions: Vec<StoredRegion>,
    /// Types all of whose regions dropping it uses: those its `Drop` impl uses, trait objects.
    whole: Vec<StoredTy>,
    /// The types dropping it drops, taken apart where it is dropped.
    tys: Vec<StoredTy>,
}

#[salsa::tracked(returns(ref), cycle_result = drop_constraint_cycle)]
fn drop_constraint(db: &dyn HirDatabase, adt: AdtId) -> DropConstraint {
    let interner = DbInterner::new_no_crate(db);
    let mut regions = Vec::new();
    let mut whole = Vec::new();
    let mut tys = Vec::new();
    if let Some(drop_impl) = hir_ty::drop::destructor(db, adt) {
        let args = GenericArgs::identity_for_item(interner, adt.into());
        let dangling = may_dangle(db, *drop_impl, args.len());
        for (index, arg) in args.iter().enumerate() {
            let dangles = dangling.get(index).copied().unwrap_or(false);
            match arg.kind() {
                GenericArgKind::Type(arg) if dangles => tys.push(arg),
                GenericArgKind::Type(arg) => whole.push(arg),
                GenericArgKind::Lifetime(region) if !dangles => regions.push(region),
                _ => (),
            }
        }
    }
    let variants: Vec<hir_def::VariantId> = match adt {
        AdtId::StructId(it) => vec![it.into()],
        AdtId::UnionId(_) => Vec::new(),
        AdtId::EnumId(it) => {
            it.enum_variants(db).variants.values().map(|&(it, _)| it.into()).collect()
        }
    };
    for variant in variants {
        for (_, field) in db.field_types(variant).iter() {
            let field = field.ty().instantiate_identity().skip_norm_wip();
            take_apart(db, interner, field, &mut regions, &mut whole, &mut tys);
        }
    }
    let mut seen = FxHashSet::default();
    tys.retain(|it| seen.insert(*it));
    DropConstraint {
        regions: regions.into_iter().map(|it| it.store()).collect(),
        whole: whole.into_iter().map(|it| it.store()).collect(),
        tys: tys.into_iter().map(|it| it.store()).collect(),
    }
}

/// An ADT that holds itself by value has no size and no answer.
fn drop_constraint_cycle(_db: &dyn HirDatabase, _: salsa::Id, _adt: AdtId) -> DropConstraint {
    DropConstraint::default()
}

/// Takes `ty`, a field's type or part of one, apart for [`drop_constraint`].
fn take_apart<'db>(
    db: &'db dyn HirDatabase,
    interner: DbInterner<'db>,
    ty: Ty<'db>,
    regions: &mut Vec<Region<'db>>,
    whole: &mut Vec<Ty<'db>>,
    tys: &mut Vec<Ty<'db>>,
) {
    match ty.kind() {
        TyKind::Adt(def, _) if def.is_manually_drop() => (),
        TyKind::Adt(def, args) if def.is_box() || def.is_phantom_data() => {
            if let Some(GenericArgKind::Type(inner)) = args.iter().next().map(|it| it.kind()) {
                tys.push(inner);
            }
        }
        TyKind::Adt(def, args) => {
            let constraint = drop_constraint(db, def.def_id());
            let instantiate = |ty: &StoredTy| {
                rustc_type_ir::EarlyBinder::bind(ty.as_ref())
                    .instantiate(interner, args)
                    .skip_norm_wip()
            };
            regions.extend(constraint.regions.iter().map(|it| {
                rustc_type_ir::EarlyBinder::bind(it.as_ref())
                    .instantiate(interner, args)
                    .skip_norm_wip()
            }));
            whole.extend(constraint.whole.iter().map(instantiate));
            tys.extend(constraint.tys.iter().map(instantiate));
        }
        TyKind::Array(inner, _) | TyKind::Slice(inner) => {
            take_apart(db, interner, inner, regions, whole, tys)
        }
        TyKind::Tuple(parts) => {
            for it in parts.iter() {
                take_apart(db, interner, it, regions, whole, tys);
            }
        }
        TyKind::Dynamic(..) => whole.push(ty),
        TyKind::Param(_)
        | TyKind::Alias(_)
        | TyKind::Closure(..)
        | TyKind::CoroutineClosure(..)
        | TyKind::Coroutine(..) => tys.push(ty),
        _ => (),
    }
}

/// Per generic argument of the type a `Drop` impl is for, whether the impl says
/// `#[may_dangle]` of it: that dropping does not use it.
fn may_dangle(db: &dyn HirDatabase, drop_impl: hir_def::ImplId, count: usize) -> Vec<bool> {
    let (lifetimes, types) = AttrFlags::query_generic_params(db, drop_impl.into());
    let self_ty = db.impl_self_ty(drop_impl).skip_binder();
    let TyKind::Adt(_, args) = self_ty.kind() else { return vec![false; count] };
    args.iter()
        .map(|arg| match arg.kind() {
            GenericArgKind::Type(ty) => match ty.kind() {
                TyKind::Param(param) => types
                    .get(param.id.local_id())
                    .is_some_and(|it| it.contains(AttrFlags::MAY_DANGLE)),
                _ => false,
            },
            GenericArgKind::Lifetime(region) => match region.kind() {
                rustc_type_ir::RegionKind::ReEarlyParam(early) => lifetimes
                    .get(early.id.local_id)
                    .is_some_and(|it| it.contains(AttrFlags::MAY_DANGLE)),
                _ => false,
            },
            GenericArgKind::Const(_) => false,
        })
        .collect()
}

/// The regions of `ty` a value of it can hold on to, as rustc's liveness `FreeRegionsVisitor`
/// has them: an alias known to outlive a region holds only that one, and none when it
/// outlives `'static`.
fn live_regions_of<'db>(
    ctx: &Ctx<'_, 'db>,
    regions: &Regions<'db>,
    ty: Ty<'db>,
    out: &mut Vec<Rv>,
) {
    struct Visit<'c, 'a, 'db> {
        ctx: &'c Ctx<'a, 'db>,
        regions: &'c Regions<'db>,
        out: &'c mut Vec<Rv>,
    }
    impl<'db> rustc_type_ir::TypeVisitor<DbInterner<'db>> for Visit<'_, '_, 'db> {
        type Result = ();
        fn visit_region(&mut self, region: Region<'db>) {
            if let Some(rv) = rv_of(region) {
                self.out.push(rv);
            }
        }
        fn visit_ty(&mut self, ty: Ty<'db>) {
            if let TyKind::Alias(alias) = ty.kind() {
                match alias_bound(self.ctx, self.regions, ty, alias) {
                    Some(None) => return,
                    Some(Some(rv)) => {
                        self.out.push(rv);
                        return;
                    }
                    None => (),
                }
                // What an `impl Trait` does not capture need not be live.
                if let rustc_type_ir::AliasTyKind::Opaque { .. } = alias.kind
                    && let Some(captured) = super::regions::opaque_captures(
                        self.ctx.db,
                        alias.kind.def_id(),
                        alias.args.len(),
                    )
                {
                    for (arg, captured) in alias.args.iter().zip(captured) {
                        if captured {
                            rustc_type_ir::TypeVisitable::visit_with(&arg, self);
                        }
                    }
                    return;
                }
            }
            rustc_type_ir::TypeSuperVisitable::super_visit_with(&ty, self)
        }
    }
    rustc_type_ir::TypeVisitable::visit_with(&ty, &mut Visit { ctx, regions, out });
}

/// [`live_regions_of`] `ty`.
pub(super) fn live_regions<'db>(
    ctx: &Ctx<'_, 'db>,
    regions: &Regions<'db>,
    ty: Ty<'db>,
) -> Vec<Rv> {
    let mut out = Vec::new();
    live_regions_of(ctx, regions, ty, &mut out);
    out
}

/// The one region the bounds of the alias `ty` say it outlives: `Some(None)` for `'static`,
/// `None` when they name none or several, or when `ty` is under a binder of its own type,
/// `fn(T::Gat<'a, '_>)`, whose bounds rustc does not match.
fn alias_bound<'db>(
    ctx: &Ctx<'_, 'db>,
    regions: &Regions<'db>,
    ty: Ty<'db>,
    alias: rustc_type_ir::AliasTy<DbInterner<'db>>,
) -> Option<Option<Rv>> {
    if rustc_type_ir::TypeVisitableExt::has_escaping_bound_vars(&ty) {
        return None;
    }
    let interner = ctx.infcx.interner;
    let erase = |ty: Ty<'db>| {
        rustc_type_ir::fold_regions(interner, ty, |_, _| Region::new_static(interner))
    };
    let erased = erase(ty);
    let item_bounds: Vec<Clause<'db>> =
        rustc_type_ir::Interner::item_bounds(interner, alias.kind.def_id())
            .iter_instantiated(interner, alias.args)
            .map(|it| it.skip_norm_wip())
            .collect();
    let mut found: Option<Option<Rv>> = None;
    for clause in item_bounds.into_iter().chain(ctx.env.caller_bounds().iter()) {
        let ClauseKind::TypeOutlives(OutlivesPredicate(bounded, region)) =
            clause.kind().skip_binder()
        else {
            continue;
        };
        let same = erase(bounded) == erased;
        if !same && normalize_erased(ctx, erase(bounded)) != Some(erased) {
            continue;
        }
        let bound = match region.kind() {
            RegionKind::ReStatic => return Some(None),
            // `for<'a> T: 'a` is `T: 'static`; `for<'a, 'b> T::Changes<'a, 'b>: 'b` that the
            // alias outlives what it has where the bound names `'b`.
            RegionKind::ReBound(_, bound) => {
                let regions_in = |ty: Ty<'db>| {
                    let mut found = Vec::new();
                    rustc_type_ir::fold_regions(interner, ty, |region, _| {
                        found.push(region);
                        region
                    });
                    found
                };
                let at = regions_in(bounded).iter().position(|it| {
                    matches!(it.kind(), RegionKind::ReBound(_, other) if other.var == bound.var)
                });
                match at {
                    None => return Some(None),
                    Some(at) if same => regions_in(ty).get(at).copied().and_then(rv_of),
                    Some(_) => None,
                }
            }
            RegionKind::ReVar(_) => rv_of(region),
            RegionKind::ReEarlyParam(early) => regions.early.get(&early.index).copied(),
            _ => None,
        };
        let Some(bound) = bound else { continue };
        match found {
            None => found = Some(Some(bound)),
            Some(Some(it)) if it == bound => (),
            Some(_) => return None,
        }
    }
    found
}

/// `ty`, its regions all `'static`, normalized as far as the solver goes: rustc normalizes the
/// outlives clauses of the parameter environment before alias liveness matches them.
pub(super) fn normalize_erased<'db>(ctx: &Ctx<'_, 'db>, ty: Ty<'db>) -> Option<Ty<'db>> {
    use hir_ty::next_solver::{infer::traits::ObligationCause, obligation_ctxt::ObligationCtxt};
    if !rustc_type_ir::TypeVisitableExt::has_aliases(&ty) {
        return None;
    }
    let infcx = &ctx.infcx;
    infcx.probe(|_| {
        let ocx = ObligationCtxt::new(infcx);
        let normalized = ocx.deeply_normalize(&ObligationCause::dummy(), ctx.env, ty).ok()?;
        let interner = infcx.interner;
        Some(rustc_type_ir::fold_regions(
            interner,
            infcx.resolve_vars_if_possible(normalized),
            |_, _| Region::new_static(interner),
        ))
    })
}
