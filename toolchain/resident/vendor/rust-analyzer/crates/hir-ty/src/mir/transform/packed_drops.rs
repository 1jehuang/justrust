//! rustc's `add_moves_for_packed_drops`: a value in a field that a `#[repr(packed)]` struct
//! leaves misaligned is moved to a temporary, which is aligned, and dropped there. `Drop::drop`
//! takes a reference, and a reference has to be aligned.

use rustc_type_ir::inherent::IntoKind;

use crate::{
    mir::{
        Local, MirBody, Operand, OperandKind, Place, Projection, ProjectionElem, Rvalue, Statement,
        StatementKind, StoredPlace, TerminatorKind,
    },
    next_solver::TyKind,
};

use super::Ctx;

pub(super) fn add_moves_for_packed_drops<'db>(ctx: &Ctx<'db>, body: &mut MirBody<'db>) {
    let ids: Vec<_> = body.basic_blocks.iter().map(|(id, _)| id).collect();
    for id in ids {
        let Some(terminator) = &body.basic_blocks[id].terminator else { continue };
        let TerminatorKind::Drop { place, .. } = &terminator.kind else { continue };
        if !is_disaligned(ctx, body, place) {
            continue;
        }
        let span = terminator.span;
        let place = place.clone();
        let ty = place.as_ref().ty(body, &ctx.infcx, ctx.env).ty;
        let temp = body.locals.alloc(Local { ty: ty.store() });
        let block = &mut body.basic_blocks[id];
        block.statements.push(Statement {
            kind: StatementKind::Assign(
                Place::from(temp).store(),
                Rvalue::Use(Operand { kind: OperandKind::Move(place), span: None }),
            ),
            span,
        });
        let Some(TerminatorKind::Drop { place, .. }) =
            block.terminator.as_mut().map(|it| &mut it.kind)
        else {
            unreachable!()
        };
        *place = Place::from(temp).store();
    }
}

/// Whether `place` is in a field of a packed struct that may leave it less aligned than its
/// type asks for. One whose layout is not known yet, as of a type parameter, may be.
fn is_disaligned<'db>(ctx: &Ctx<'db>, body: &MirBody<'db>, place: &StoredPlace) -> bool {
    let projection = place.projection.as_slice();
    let mut pack: Option<u64> = None;
    for (at, elem) in projection.iter().enumerate().rev() {
        match elem {
            ProjectionElem::Deref => break,
            ProjectionElem::Field(_) => {
                let base = Place {
                    local: place.local,
                    projection: Projection::new_from_slice(&projection[..at]),
                };
                let base_ty = base.ty(body, &ctx.infcx, ctx.env);
                if base_ty.variant_id.is_some() {
                    continue;
                }
                if let TyKind::Adt(adt, _) = ctx.normalize(base_ty.ty).kind()
                    && let Some(packed) = adt.repr(ctx.db).pack
                {
                    let packed = packed.bytes();
                    pack = Some(pack.map_or(packed, |it| it.min(packed)));
                }
            }
            _ => (),
        }
    }
    let Some(pack) = pack else { return false };
    let ty = place.as_ref().ty(body, &ctx.infcx, ctx.env).ty;
    match ctx.layout_align(ty) {
        Some(align) => align > pack,
        None => true,
    }
}
