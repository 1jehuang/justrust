//! Move paths, and where each may be uninitialized or has ever been initialized: rustc's
//! `MoveData` with `MaybeUninitializedPlaces` and `EverInitializedPlaces`.
//!
//! A move path is a local, or a field, variant, element of an array of known length or the
//! contents of a `Box` of another path. Every local has one; a part has one when the body
//! moves out of it, assigns it or drops it. What is behind any other pointer, at an index
//! known only when the body runs, or inside a value whose type implements `Drop`, has none:
//! it cannot be moved out of, and the nearest path above stands for it. A field of a union is
//! the union.

use rustc_hash::FxHashMap;
use rustc_type_ir::inherent::IntoKind;
use smallvec::SmallVec;

use hir_ty::{
    consteval::try_const_usize,
    mir::{
        AsmOperand, LocalId, MirBody, PlaceElem, PlaceTy, ProjectionElem, Rvalue, StatementKind,
        StoredPlace, TerminatorKind,
    },
    next_solver::TyKind,
};

use super::{
    Ctx,
    bitset::BitSet,
    cfg::{Cfg, Location, index},
};

pub(super) type Mpi = usize;

pub(super) struct MovePath {
    pub(super) local: LocalId,
    pub(super) projection: Vec<PlaceElem>,
    pub(super) parent: Option<Mpi>,
    pub(super) children: Vec<Mpi>,
}

/// What a place is in terms of move paths.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Lookup {
    Exact(Mpi),
    /// The place has no path of its own; this is the nearest one above it.
    Parent(Mpi),
}

impl Lookup {
    pub(super) fn path(self) -> Mpi {
        match self {
            Lookup::Exact(it) | Lookup::Parent(it) => it,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MoveOut {
    pub(super) path: Mpi,
    pub(super) location: Location,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum InitKind {
    Deep,
    /// The destination of a call, initialized only if the call returns.
    NonPanicPathOnly,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Init {
    pub(super) path: Mpi,
    /// `None` for an argument, initialized before the body starts.
    pub(super) location: Option<Location>,
    pub(super) kind: InitKind,
}

pub(super) struct MoveData {
    pub(super) paths: Vec<MovePath>,
    locals: Vec<Mpi>,
    lookup: FxHashMap<(LocalId, Vec<PlaceElem>), Mpi>,
    pub(super) moves: Vec<MoveOut>,
    pub(super) inits: Vec<Init>,
    pub(super) moves_at: FxHashMap<Location, SmallVec<[usize; 2]>>,
    pub(super) inits_at: FxHashMap<Location, SmallVec<[usize; 2]>>,
    /// The moves out of each path, by the index of the path.
    pub(super) moves_of: Vec<Vec<usize>>,
    /// The inits of each path.
    pub(super) inits_of: Vec<Vec<usize>>,
}

impl MoveData {
    pub(super) fn gather<'db>(ctx: &Ctx<'_, 'db>, body: &MirBody<'db>, cfg: &Cfg) -> Self {
        let mut data = MoveData {
            paths: Vec::new(),
            locals: Vec::new(),
            lookup: FxHashMap::default(),
            moves: Vec::new(),
            inits: Vec::new(),
            moves_at: FxHashMap::default(),
            inits_at: FxHashMap::default(),
            moves_of: Vec::new(),
            inits_of: Vec::new(),
        };
        for (local, _) in body.locals.iter() {
            let id = data.paths.len();
            data.paths.push(MovePath {
                local,
                projection: Vec::new(),
                parent: None,
                children: Vec::new(),
            });
            data.lookup.insert((local, Vec::new()), id);
            data.locals.push(id);
        }
        for &local in &body.param_locals {
            let path = data.locals[local.into_raw().into_u32() as usize];
            data.inits.push(Init { path, location: None, kind: InitKind::Deep });
        }
        for &block in &cfg.order {
            let data_block = &body.basic_blocks[block];
            for (statement, it) in data_block.statements.iter().enumerate() {
                let location = Location { block, statement };
                let compares =
                    matches!(&it.kind, StatementKind::Assign(_, rvalue) if super::compares(rvalue));
                it.kind.for_each_operand(&mut |operand| {
                    if !compares
                        && let Some(place) = ctx.moved_place(body, operand)
                        && !ctx.moves_out_of_guard(
                            place.local,
                            place.projection.as_slice(),
                            operand.span().unwrap_or(it.span),
                        )
                    {
                        data.gather_move(ctx, body, place, location);
                    }
                });
                match &it.kind {
                    StatementKind::Assign(place, rvalue) => {
                        if let Rvalue::Discriminant(moved) = rvalue
                            && ctx.cast_moves(body, moved, it.span)
                        {
                            data.gather_move(ctx, body, moved, location);
                        }
                        data.gather_init(ctx, body, place, location, InitKind::Deep)
                    }
                    StatementKind::StorageDead(local) => {
                        if !body.param_locals.contains(local) {
                            let path = data.locals[local.into_raw().into_u32() as usize];
                            data.record_move(path, location);
                        }
                    }
                    StatementKind::Deinit(place) => data.gather_move(ctx, body, place, location),
                    StatementKind::InlineAsm(asm) => {
                        for operand in &asm.operands {
                            if let AsmOperand::Reg { output: Some(output), .. } = operand {
                                data.gather_init(ctx, body, output, location, InitKind::Deep);
                            }
                        }
                    }
                    _ => (),
                }
            }
            let Some(terminator) = &data_block.terminator else { continue };
            let location = Location { block, statement: data_block.statements.len() };
            terminator.kind.for_each_operand(&mut |operand| {
                if let Some(place) = ctx.moved_place(body, operand)
                    && !ctx.moves_out_of_guard(
                        place.local,
                        place.projection.as_slice(),
                        operand.span().unwrap_or(terminator.span),
                    )
                {
                    data.gather_move(ctx, body, place, location);
                }
            });
            match &terminator.kind {
                TerminatorKind::Drop { place, .. } => data.gather_move(ctx, body, place, location),
                TerminatorKind::DropAndReplace { place, .. } => {
                    data.gather_init(ctx, body, place, location, InitKind::Deep)
                }
                TerminatorKind::Call { destination, .. } => {
                    data.gather_init(ctx, body, destination, location, InitKind::NonPanicPathOnly)
                }
                TerminatorKind::Yield { resume_arg, .. } => {
                    data.gather_init(ctx, body, resume_arg, location, InitKind::Deep)
                }
                _ => (),
            }
        }
        data.moves_of = vec![Vec::new(); data.paths.len()];
        data.inits_of = vec![Vec::new(); data.paths.len()];
        for (index, it) in data.moves.iter().enumerate() {
            data.moves_of[it.path].push(index);
        }
        for (index, it) in data.inits.iter().enumerate() {
            data.inits_of[it.path].push(index);
        }
        data
    }

    fn record_move(&mut self, path: Mpi, location: Location) {
        self.moves_at.entry(location).or_default().push(self.moves.len());
        self.moves.push(MoveOut { path, location });
    }

    fn gather_move<'db>(
        &mut self,
        ctx: &Ctx<'_, 'db>,
        body: &MirBody<'db>,
        place: &StoredPlace,
        location: Location,
    ) {
        // A move of a subslice of an array moves each element of it.
        if let Some((ProjectionElem::Subslice { from, to }, base)) =
            place.projection.as_slice().split_last()
            && let Lookup::Exact(array) = self.create_projection(ctx, body, place.local, base)
            && let TyKind::Array(_, len) = ctx.place_ty(body, place.local, base).ty.kind()
            && let Some(len) = try_const_usize(ctx.db, len)
        {
            for offset in *from..(len as u64).saturating_sub(*to) {
                let path = self.child(
                    array,
                    place.local,
                    ProjectionElem::ConstantIndex {
                        offset,
                        min_length: offset + 1,
                        from_end: false,
                    },
                );
                self.record_move(path, location);
            }
            return;
        }
        match self.create(ctx, body, place) {
            Lookup::Exact(path) => self.record_move(path, location),
            // A move out of a field of a union moves the union, as long as nothing else on the
            // way makes it an error.
            Lookup::Parent(path) => {
                let projection = place.projection.as_slice();
                let depth = self.paths[path].projection.len();
                let base_ty = ctx.place_ty(body, place.local, &projection[..depth]).ty;
                if matches!(projection[depth], ProjectionElem::Field(_))
                    && matches!(base_ty.kind(), TyKind::Adt(def, _) if def.is_union())
                    && projection[depth + 1..]
                        .iter()
                        .all(|it| matches!(it, ProjectionElem::Field(_)))
                {
                    self.record_move(path, location);
                }
            }
        }
    }

    fn child(&mut self, parent: Mpi, local: LocalId, step: PlaceElem) -> Mpi {
        let mut key = self.paths[parent].projection.clone();
        key.push(step);
        if let Some(&it) = self.lookup.get(&(local, key.clone())) {
            return it;
        }
        let id = self.paths.len();
        self.paths.push(MovePath {
            local,
            projection: key.clone(),
            parent: Some(parent),
            children: Vec::new(),
        });
        self.paths[parent].children.push(id);
        self.lookup.insert((local, key), id);
        id
    }

    fn gather_init<'db>(
        &mut self,
        ctx: &Ctx<'_, 'db>,
        body: &MirBody<'db>,
        place: &StoredPlace,
        location: Location,
        kind: InitKind,
    ) {
        let lookup = self.create(ctx, body, place);
        let path = match lookup {
            Lookup::Exact(path) => path,
            // Assigning a field of a union initializes the union.
            Lookup::Parent(path) if self.is_union_field(ctx, body, place, path) => path,
            Lookup::Parent(_) => return,
        };
        self.inits_at.entry(location).or_default().push(self.inits.len());
        self.inits.push(Init { path, location: Some(location), kind });
    }

    fn is_union_field<'db>(
        &self,
        ctx: &Ctx<'_, 'db>,
        body: &MirBody<'db>,
        place: &StoredPlace,
        parent: Mpi,
    ) -> bool {
        let projection = place.projection.as_slice();
        let depth = self.paths[parent].projection.len();
        if projection.len() != depth + 1 || !matches!(projection[depth], ProjectionElem::Field(_)) {
            return false;
        }
        let ty = ctx.place_ty(body, place.local, &projection[..depth]).ty;
        matches!(ty.kind(), TyKind::Adt(def, _) if def.is_union())
    }

    /// The path of `place`, made if it can have one.
    fn create<'db>(
        &mut self,
        ctx: &Ctx<'_, 'db>,
        body: &MirBody<'db>,
        place: &StoredPlace,
    ) -> Lookup {
        self.create_projection(ctx, body, place.local, place.projection.as_slice())
    }

    fn create_projection<'db>(
        &mut self,
        ctx: &Ctx<'_, 'db>,
        body: &MirBody<'db>,
        local: LocalId,
        projection: &[PlaceElem],
    ) -> Lookup {
        let mut path = self.locals[local.into_raw().into_u32() as usize];
        let mut ty = PlaceTy::from_ty(body.locals[local].ty.as_ref());
        for elem in projection {
            let Some(step) = ctx.trackable(ty, elem) else { return Lookup::Parent(path) };
            path = self.child(path, local, step);
            ty = ctx.project(ty, elem);
        }
        Lookup::Exact(path)
    }

    /// The path of `place` if it has been made, or the nearest one above it.
    pub(super) fn find<'db>(
        &self,
        ctx: &Ctx<'_, 'db>,
        body: &MirBody<'db>,
        local: LocalId,
        projection: &[PlaceElem],
    ) -> Lookup {
        let mut path = self.locals[local.into_raw().into_u32() as usize];
        let mut ty = PlaceTy::from_ty(body.locals[local].ty.as_ref());
        for elem in projection {
            let Some(step) = ctx.trackable(ty, elem) else { return Lookup::Parent(path) };
            let mut key = self.paths[path].projection.clone();
            key.push(step);
            match self.lookup.get(&(local, key)) {
                Some(&it) => path = it,
                None => return Lookup::Parent(path),
            }
            ty = ctx.project(ty, elem);
        }
        Lookup::Exact(path)
    }

    pub(super) fn local_path(&self, local: LocalId) -> Mpi {
        self.locals[local.into_raw().into_u32() as usize]
    }

    /// `path` and every path below it.
    pub(super) fn with_descendants(&self, path: Mpi, f: &mut impl FnMut(Mpi)) {
        f(path);
        for &child in &self.paths[path].children {
            self.with_descendants(child, f);
        }
    }

    pub(super) fn find_in_descendants(&self, path: Mpi, f: &impl Fn(Mpi) -> bool) -> Option<Mpi> {
        if f(path) {
            return Some(path);
        }
        self.paths[path].children.iter().find_map(|&child| self.find_in_descendants(child, f))
    }

    pub(super) fn is_ancestor_or_self(&self, ancestor: Mpi, mut path: Mpi) -> bool {
        loop {
            if path == ancestor {
                return true;
            }
            match self.paths[path].parent {
                Some(parent) => path = parent,
                None => return false,
            }
        }
    }
}

impl<'db> Ctx<'_, 'db> {
    /// The step of a move path that `elem` of a place of type `ty` is, if it has one.
    pub(super) fn trackable(&self, ty: PlaceTy<'db>, elem: &PlaceElem) -> Option<PlaceElem> {
        match elem {
            ProjectionElem::Deref => ty.ty.is_box().then_some(ProjectionElem::Deref),
            ProjectionElem::Field(_) => match ty.ty.kind() {
                TyKind::Adt(def, _) if def.is_union() => None,
                TyKind::Adt(def, _)
                    if ty.variant_id.is_none() && self.has_destructor(def.def_id()) =>
                {
                    None
                }
                TyKind::Adt(def, _)
                    if ty.variant_id.is_some() && self.has_destructor(def.def_id()) =>
                {
                    None
                }
                _ => Some(*elem),
            },
            ProjectionElem::Downcast(_) => match ty.ty.kind() {
                TyKind::Adt(def, _) if self.has_destructor(def.def_id()) => None,
                _ => Some(*elem),
            },
            ProjectionElem::Index(_) | ProjectionElem::Subslice { .. } => None,
            &ProjectionElem::ConstantIndex { offset, from_end, .. } => match ty.ty.kind() {
                TyKind::Array(_, len) => {
                    let len = try_const_usize(self.db, len)? as u64;
                    let offset = if from_end { len.checked_sub(offset)? } else { offset };
                    Some(ProjectionElem::ConstantIndex {
                        offset,
                        min_length: offset + 1,
                        from_end: false,
                    })
                }
                _ => None,
            },
        }
    }
}

/// Which paths may be uninitialized at the start of each block: rustc's
/// `MaybeUninitializedPlaces`. And per local, whether it may have been initialized since its
/// storage last ended: `EverInitializedPlaces`, of whole locals.
pub(super) struct InitFlow {
    pub(super) uninit_entry: Vec<BitSet>,
    pub(super) ever_entry: Vec<BitSet>,
}

impl InitFlow {
    pub(super) fn compute(body: &MirBody<'_>, cfg: &Cfg, moves: &MoveData) -> Self {
        let blocks = body.basic_blocks.len();
        let paths = moves.paths.len();
        let locals = body.locals.len();
        let mut uninit_entry = vec![BitSet::new(paths); blocks];
        let mut ever_entry = vec![BitSet::new(locals); blocks];
        let mut seen = vec![false; blocks];
        {
            let start = index(body.start_block);
            let entry = &mut uninit_entry[start];
            for path in 0..paths {
                entry.insert(path);
            }
            for init in moves.inits.iter().filter(|it| it.location.is_none()) {
                moves.with_descendants(init.path, &mut |it| entry.remove(it));
                ever_entry[start]
                    .insert(moves.paths[init.path].local.into_raw().into_u32() as usize);
            }
            seen[start] = true;
        }
        let mut changed = true;
        while changed {
            changed = false;
            for &block in &cfg.order {
                if !seen[index(block)] {
                    continue;
                }
                let mut uninit = uninit_entry[index(block)].clone();
                let mut ever = ever_entry[index(block)].clone();
                let data = &body.basic_blocks[block];
                for statement in 0..=data.statements.len() {
                    let location = Location { block, statement };
                    apply(body, moves, location, &mut uninit, &mut ever);
                }
                let Some(terminator) = &data.terminator else { continue };
                let returned = match &terminator.kind {
                    TerminatorKind::Call { target: Some(target), .. } => Some(*target),
                    _ => None,
                };
                for &target in &cfg.successors[index(block)] {
                    let (mut uninit, mut ever) = (uninit.clone(), ever.clone());
                    if Some(target) == returned {
                        call_returned(
                            moves,
                            Location { block, statement: data.statements.len() },
                            &mut uninit,
                            &mut ever,
                        );
                    }
                    let t = index(target);
                    if !seen[t] {
                        seen[t] = true;
                        uninit_entry[t] = uninit;
                        ever_entry[t] = ever;
                        changed = true;
                    } else {
                        changed |= uninit_entry[t].union(&uninit);
                        changed |= ever_entry[t].union(&ever);
                    }
                }
            }
        }
        InitFlow { uninit_entry, ever_entry }
    }
}

/// The effect of the statement or terminator at `location` on what may be uninitialized and
/// what has ever been initialized, the return of a call aside.
pub(super) fn apply(
    body: &MirBody<'_>,
    moves: &MoveData,
    location: Location,
    uninit: &mut BitSet,
    ever: &mut BitSet,
) {
    if let Some(at) = moves.moves_at.get(&location) {
        for &index in at {
            moves.with_descendants(moves.moves[index].path, &mut |it| {
                uninit.insert(it);
            });
        }
    }
    if let Some(at) = moves.inits_at.get(&location) {
        for &index in at {
            let init = moves.inits[index];
            if init.kind == InitKind::Deep {
                moves.with_descendants(init.path, &mut |it| uninit.remove(it));
                if moves.paths[init.path].projection.is_empty() {
                    ever.insert(moves.paths[init.path].local.into_raw().into_u32() as usize);
                }
            }
        }
    }
    let data = &body.basic_blocks[location.block];
    if let Some(statement) = data.statements.get(location.statement)
        && let StatementKind::StorageDead(local) = statement.kind
    {
        ever.remove(local.into_raw().into_u32() as usize);
    }
}

pub(super) fn call_returned(
    moves: &MoveData,
    location: Location,
    uninit: &mut BitSet,
    ever: &mut BitSet,
) {
    if let Some(at) = moves.inits_at.get(&location) {
        for &index in at {
            let init = moves.inits[index];
            if init.kind == InitKind::NonPanicPathOnly {
                moves.with_descendants(init.path, &mut |it| uninit.remove(it));
                if moves.paths[init.path].projection.is_empty() {
                    ever.insert(moves.paths[init.path].local.into_raw().into_u32() as usize);
                }
            }
        }
    }
}
