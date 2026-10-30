//! Loan liveness as rustc's `-Zpolonius=next` computes it: a loan is live where it can reach a
//! live region in a graph whose nodes are a region at a point. Edges within a point are the
//! outlives constraints that hold there; edges between points follow a region while it is
//! live, forwards for a covariant one, backwards for a contravariant one and both ways for an
//! invariant one. A loan is in scope until the first point it is not live at.

use rustc_hash::FxHashSet;
use rustc_type_ir::{Variance, inherent::IntoKind};
use smallvec::SmallVec;

use hir_ty::{
    mir::{MirBody, OperandKind, Rvalue, StatementKind},
    next_solver::{GenericArgKind, GenericArgs, Region, Ty, TyKind},
};

use super::{
    Ctx,
    cfg::{Cfg, Location, index},
    intervals::IntervalSet,
    loans::Loans,
    regions::{Regions, Rv, rv_of},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Direction {
    Forward,
    Backward,
    Both,
}

/// The points each loan is live at.
pub(super) fn live_loans<'db>(
    ctx: &Ctx<'_, 'db>,
    body: &MirBody<'db>,
    cfg: &Cfg,
    regions: &Regions<'db>,
    count: usize,
    liveness: &[IntervalSet],
    loans: &Loans,
) -> Vec<IntervalSet> {
    let mut directions: Vec<Option<Direction>> = vec![None; count];
    for &ty in &regions.local_tys {
        record_variances(ctx, ty, Variance::Covariant, &mut directions);
    }
    // rust-analyzer reaches a `static mut` through a shared reference, rustc through a
    // `*mut T`, in which `T` is invariant.
    for (_, block) in body.basic_blocks.iter() {
        for statement in &block.statements {
            if let StatementKind::Assign(place, Rvalue::Use(operand)) = &statement.kind
                && let OperandKind::Static(id) = operand.kind
                && place.projection.is_empty()
                && hir_def::signatures::StaticSignature::of(ctx.db, id)
                    .flags
                    .contains(hir_def::signatures::StaticFlags::MUTABLE)
                && let TyKind::Ref(_, inner, _) =
                    regions.local_tys[place.local.into_raw().into_u32() as usize].kind()
            {
                record_variances(ctx, inner, Variance::Invariant, &mut directions);
            }
        }
    }
    let universal: FxHashSet<Rv> = regions.universals.iter().map(|&(rv, _)| rv).collect();
    let direction = |rv: Rv| {
        if universal.contains(&rv) {
            Direction::Forward
        } else {
            // rustc falls back to both ways for a region it has no variance of.
            directions.get(rv).copied().flatten().unwrap_or(Direction::Both)
        }
    };
    // The points each region is live at; a universal region is live everywhere.
    let all = {
        let mut all = IntervalSet::default();
        if cfg.points > 0 {
            all.insert_range(0, cfg.points - 1);
        }
        all
    };
    let empty = IntervalSet::default();
    let live_at =
        |rv: Rv| if universal.contains(&rv) { &all } else { liveness.get(rv).unwrap_or(&empty) };
    // The localized edges out of each region, by point.
    let mut localized: Vec<Vec<(usize, Rv)>> = vec![Vec::new(); count];
    let mut logical: Vec<SmallVec<[Rv; 4]>> = vec![SmallVec::new(); count];
    for constraint in &regions.outlives {
        if constraint.sup >= count || constraint.sub >= count {
            continue;
        }
        match constraint.location {
            Some(location) => localized[constraint.sup].push((cfg.point(location), constraint.sub)),
            None => {
                if !logical[constraint.sup].contains(&constraint.sub) {
                    logical[constraint.sup].push(constraint.sub);
                }
            }
        }
    }
    for edges in &mut localized {
        edges.sort_unstable();
        edges.dedup();
    }
    let last_of =
        |block| cfg.point(Location { block, statement: body.basic_blocks[block].statements.len() });

    // The graph's nodes are a region at a point, but it is walked a run of points of one block
    // at a time: what reaches a region at a run of points reaches its subregions at all of
    // them, and a region live at a run of points flows along it.
    let mut live = Vec::with_capacity(loans.loans.len());
    let mut visited: Vec<IntervalSet> = vec![IntervalSet::default(); count];
    let mut touched: Vec<Rv> = Vec::new();
    let mut stack: Vec<(Rv, usize, usize)> = Vec::new();
    for loan in &loans.loans {
        let mut points = IntervalSet::default();
        for rv in touched.drain(..) {
            visited[rv].clear();
        }
        let issued = cfg.point(loan.location);
        stack.push((loan.region, issued, issued));
        while let Some((region, start, end)) = stack.pop() {
            if region >= count {
                continue;
            }
            if visited[region].is_empty() {
                touched.push(region);
            }
            let block = cfg.location(start).block;
            let (first, last) = (cfg.point(Location { block, statement: 0 }), last_of(block));
            let region_live = live_at(region);
            let direction = direction(region);
            for (start, end) in visited[region].insert_new(start, end) {
                for &sub in &logical[region] {
                    stack.push((sub, start, end));
                }
                let edges = &localized[region];
                let from = edges.partition_point(|&(point, _)| point < start);
                for &(point, sub) in edges[from..].iter().take_while(|&&(point, _)| point <= end) {
                    stack.push((sub, point, point));
                }
                for (s, e) in region_live.runs_in(start, end) {
                    points.insert_range(s, e);
                    if matches!(direction, Direction::Backward | Direction::Both) {
                        if s > first {
                            stack.push((region, s - 1, e - 1));
                        } else {
                            if e > s {
                                stack.push((region, s, e - 1));
                            }
                            let location = Location { block, statement: 0 };
                            for previous in cfg.predecessor_locations(body, location) {
                                let point = cfg.point(previous);
                                stack.push((region, point, point));
                            }
                        }
                    }
                }
                if matches!(direction, Direction::Forward | Direction::Both) {
                    // Past the end of the run, the region flows on while it is live.
                    let mut follow = |next: usize, last: usize| {
                        if region_live.contains(next) {
                            let to =
                                region_live.first_absent_in(next, last).map_or(last, |it| it - 1);
                            stack.push((region, next, to));
                        }
                    };
                    if end < last {
                        follow(end + 1, last);
                    } else {
                        for &successor in &cfg.successors[index(block)] {
                            follow(
                                cfg.point(Location { block: successor, statement: 0 }),
                                last_of(successor),
                            );
                        }
                    }
                }
            }
        }
        live.push(points);
    }
    live
}

/// Records the variance each region of `ty` has in it, as rustc's `VarianceExtractor`: a
/// region in two places of different variances is invariant.
fn record_variances<'db>(
    ctx: &Ctx<'_, 'db>,
    ty: Ty<'db>,
    variance: Variance,
    directions: &mut Vec<Option<Direction>>,
) {
    if variance == Variance::Bivariant {
        return;
    }
    match ty.kind() {
        TyKind::Ref(region, inner, mutability) => {
            record_region(region, variance, directions);
            let inner_variance = match mutability {
                rustc_ast_ir::Mutability::Not => variance,
                rustc_ast_ir::Mutability::Mut => variance.xform(Variance::Invariant),
            };
            record_variances(ctx, inner, inner_variance, directions);
        }
        TyKind::RawPtr(inner, mutability) => {
            let inner_variance = match mutability {
                rustc_ast_ir::Mutability::Not => variance,
                rustc_ast_ir::Mutability::Mut => variance.xform(Variance::Invariant),
            };
            record_variances(ctx, inner, inner_variance, directions);
        }
        TyKind::Adt(def, args) => {
            let variances = ctx.db.variances_of(hir_def::GenericDefId::AdtId(def.def_id()));
            record_args(ctx, args, Some(variances.as_slice()), variance, directions);
        }
        TyKind::Tuple(tys) => {
            for ty in tys.iter() {
                record_variances(ctx, ty, variance, directions);
            }
        }
        TyKind::Array(inner, _) | TyKind::Slice(inner) => {
            record_variances(ctx, inner, variance, directions)
        }
        TyKind::FnPtr(sig, _) => {
            let tys = sig.skip_binder().inputs_and_output;
            let last = tys.len().saturating_sub(1);
            for (index, ty) in tys.iter().enumerate() {
                let inner =
                    if index == last { variance } else { variance.xform(Variance::Contravariant) };
                record_variances(ctx, ty, inner, directions);
            }
        }
        TyKind::Dynamic(predicates, region) => {
            record_region(region, variance, directions);
            for predicate in predicates.iter() {
                if let rustc_type_ir::ExistentialPredicate::Trait(it) = predicate.skip_binder() {
                    record_args(
                        ctx,
                        it.args,
                        None,
                        variance.xform(Variance::Invariant),
                        directions,
                    );
                }
                if let rustc_type_ir::ExistentialPredicate::Projection(it) = predicate.skip_binder()
                {
                    record_args(
                        ctx,
                        it.args,
                        None,
                        variance.xform(Variance::Invariant),
                        directions,
                    );
                }
            }
        }
        TyKind::Alias(alias) => {
            let variances = match alias.kind {
                rustc_type_ir::AliasTyKind::Opaque { .. } => {
                    super::regions::opaque_captures(ctx.db, alias.kind.def_id(), alias.args.len())
                        .map(|it| {
                            it.into_iter()
                                .map(|captured| match captured {
                                    true => Variance::Invariant,
                                    false => Variance::Bivariant,
                                })
                                .collect::<Vec<_>>()
                        })
                }
                _ => None,
            };
            record_args(
                ctx,
                alias.args,
                variances.as_deref(),
                variance.xform(Variance::Invariant),
                directions,
            )
        }
        TyKind::Closure(_, args)
        | TyKind::CoroutineClosure(_, args)
        | TyKind::Coroutine(_, args)
        | TyKind::CoroutineWitness(_, args)
        | TyKind::FnDef(_, args) => {
            record_args(ctx, args, None, variance.xform(Variance::Invariant), directions)
        }
        _ => (),
    }
}

fn record_args<'db>(
    ctx: &Ctx<'_, 'db>,
    args: GenericArgs<'db>,
    variances: Option<&[Variance]>,
    variance: Variance,
    directions: &mut Vec<Option<Direction>>,
) {
    for (index, arg) in args.iter().enumerate() {
        let param = variances.and_then(|it| it.get(index)).copied().unwrap_or(Variance::Invariant);
        let variance = variance.xform(param);
        match arg.kind() {
            GenericArgKind::Type(ty) => record_variances(ctx, ty, variance, directions),
            GenericArgKind::Lifetime(region) => record_region(region, variance, directions),
            GenericArgKind::Const(_) => (),
        }
    }
}

fn record_region(region: Region<'_>, variance: Variance, directions: &mut Vec<Option<Direction>>) {
    let Some(rv) = rv_of(region) else { return };
    let direction = match variance {
        Variance::Covariant => Direction::Forward,
        Variance::Contravariant => Direction::Backward,
        Variance::Invariant => Direction::Both,
        Variance::Bivariant => return,
    };
    if rv >= directions.len() {
        directions.resize(rv + 1, None);
    }
    let entry = &mut directions[rv];
    *entry = match *entry {
        Some(existing) if existing != direction => Some(Direction::Both),
        _ => Some(direction),
    };
}
