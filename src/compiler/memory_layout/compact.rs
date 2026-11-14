//! Public target memory for single-storage-field products and aggregate ZSTs.
//! Every nonzero field, ZST alignment, offset and final tail padding is derived
//! from canonical field types, never from private owning dummy words.
use super::ir;

#[derive(Clone, Copy)]
pub(in crate::compiler) struct Layout {
    pub size: usize,
    pub align: usize,
    storage: Option<(usize, usize, usize)>,
}
// Rust and repr(C) agree on the sole nonzero field's offset (always 0), size
// and alignment. ZST fields sit either at 0 (sorted before the storage field)
// or just past it. `#[repr(C)]` keeps declaration order. The default Rust
// representation sorts fields by descending alignment group, a stable sort
// that puts a niche-bearing field first among equals; tuples keep their last
// field in place because it may be unsized. Ported from
// rustc_abi/src/layout.rs `layout_of_univariant_biased` (MIT OR Apache-2.0,
// Rust Project Contributors). Every niche here is at offset 0, so the
// end-biased alternative layout is never chosen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Niche {
    Absent,
    Present,
    /// A nominal struct: library structs may not mirror std's niches.
    Unknown,
}

fn niche(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<Niche, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    Ok(match ty {
        ir::Ty::Bool | ir::Ty::Char | ir::Ty::Ref { .. } => Niche::Present,
        ir::Ty::Int(_)
        | ir::Ty::Float(_)
        | ir::Ty::RawPtr { .. }
        | ir::Ty::Unit
        | ir::Ty::PhantomData(_)
        // UnsafeCell hides its contents' niches.
        | ir::Ty::Cell(_) => Niche::Absent,
        ir::Ty::ManuallyDrop(wrapper) => niche(wrapper.argument(), budget, depth + 1)?,
        ir::Ty::Array { element, len } => {
            if *len == 0 {
                Niche::Absent
            } else {
                niche(element, budget, depth + 1)?
            }
        }
        ir::Ty::Tuple(_) => match layout(ty, budget, depth + 1)?.and_then(|l| l.storage) {
            Some((index, _, _)) => niche(
                &ty.field_ty(index, budget)?.ok_or("tuple storage field is absent")?,
                budget,
                depth + 1,
            )?,
            None => Niche::Absent,
        },
        _ => Niche::Unknown,
    })
}

/// Whether a field of a compact aggregate is placed at offset 0, before the
/// storage field. `None` when the order depends on unmodelled facts.
fn precedes_storage(
    owner: &ir::Ty,
    index: usize,
    budget: &ir::PlanBudget,
) -> Result<Option<bool>, String> {
    let Some(owner_layout) = layout(owner, budget, 0)? else {
        return Ok(None);
    };
    let Some((storage, size, align)) = owner_layout.storage else {
        // Only ZSTs: every field is at offset 0.
        return Ok(Some(true));
    };
    if index == storage {
        return Ok(Some(true));
    }
    let count = owner
        .field_count()
        .ok_or("compact aggregate has no canonical fields")?;
    let (order, tail_fixed) = match owner {
        ir::Ty::Struct(s) => (s.def().field_order, false),
        ir::Ty::Tuple(_) => (ir::FieldOrder::Optimized, true),
        _ => return Ok(None),
    };
    match order {
        ir::FieldOrder::Unknown => Ok(None),
        ir::FieldOrder::Declaration => Ok(Some(index < storage)),
        ir::FieldOrder::Optimized => {
            if tail_fixed && index + 1 == count {
                return Ok(Some(false));
            }
            if tail_fixed && storage + 1 == count {
                return Ok(Some(true));
            }
            let sorted = if tail_fixed { count - 1 } else { count };
            let mut max_align = 1usize;
            for field in 0..sorted {
                let field = owner
                    .field_ty(field, budget)?
                    .ok_or("compact aggregate field is absent")?;
                let field = layout(&field, budget, 1)?.ok_or("compact field lost its layout")?;
                max_align = max_align.max(field.align);
            }
            let zst = owner
                .field_ty(index, budget)?
                .ok_or("compact aggregate field is absent")?;
            let zst = layout(&zst, budget, 1)?.ok_or("compact field lost its layout")?;
            let zst_key = zst.align.trailing_zeros();
            let plain_key = align.max(size).trailing_zeros();
            let niche_key = plain_key.min(max_align.trailing_zeros());
            let nicheless = zst_key > plain_key || zst_key == plain_key && index < storage;
            // A niche-bearing storage field wins alignment-group ties.
            let with_niche = zst_key > niche_key;
            let storage_ty = owner
                .field_ty(storage, budget)?
                .ok_or("compact aggregate field is absent")?;
            Ok(match niche(&storage_ty, budget, 1)? {
                Niche::Absent => Some(nicheless),
                Niche::Present => Some(with_niche),
                Niche::Unknown => (nicheless == with_niche).then_some(nicheless),
            })
        }
    }
}

/// Exact offset of a compact aggregate field. Where the order is unknown the
/// declaration order stands in: only ZST addresses could observe it, and
/// `validate_address_path` rejects those.
pub(in crate::compiler) fn field_offset(
    owner: &ir::Ty,
    owner_layout: Layout,
    index: usize,
    field_align: usize,
    budget: &ir::PlanBudget,
) -> Result<usize, String> {
    let Some((storage, size, _)) = owner_layout.storage else {
        return Ok(0);
    };
    let before = match precedes_storage(owner, index, budget)? {
        Some(before) => before,
        None => index <= storage,
    };
    if before {
        Ok(0)
    } else {
        align_up(size, field_align)
    }
}

pub(in crate::compiler) fn validate_address_path(
    root: &ir::Ty,
    path: &[ir::Projection],
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for (position, projection) in path.iter().enumerate() {
        let ir::Projection::Field(index) = projection else {
            continue;
        };
        let owner = ir::projected_ty(root, &path[..position], budget)?;
        if !matches!(owner, ir::Ty::Struct(_) | ir::Ty::Tuple(_)) {
            continue;
        }
        let field = owner
            .field_ty(*index, budget)?
            .ok_or("address field is absent")?;
        let Some(field) = layout(&field, budget, 0)? else {
            continue;
        };
        if field.size != 0 {
            continue;
        }
        if layout(&owner, budget, 0)?.is_none() {
            if !owner.is_closed_shallow() {
                return Err("unsupported: generic ZST field address requires an exact source representation".into());
            }
            continue;
        }
        if precedes_storage(&owner, *index, budget)?.is_none() {
            return Err(
                "unsupported: ZST field address requires an exact source representation".into(),
            );
        }
    }
    Ok(())
}

fn align_up(size: usize, align: usize) -> Result<usize, String> {
    size.checked_add(align - 1)
        .map(|size| size & !(align - 1))
        .filter(|size| *size <= super::super::backend::MAX_FRAME_BYTES as usize)
        .ok_or_else(|| "unsupported: aggregate element extent exceeds native memory limit".into())
}
fn scalar(size: usize, align: usize) -> Layout {
    Layout {
        size,
        align,
        storage: None,
    }
}

pub(in crate::compiler) fn layout(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<Layout>, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    if let Some((size, align)) = super::super::c_abi::layout::scalar_size_align(ty) {
        if let ir::Ty::Ref { referent, .. }
        | ir::Ty::RawPtr {
            pointee: referent, ..
        } = ty
        {
            if super::pointer_metadata_size_authorized(referent, None, budget, depth + 1)?
                != Some(size)
            {
                return Ok(None);
            }
        }
        return Ok(Some(scalar(size, align)));
    }
    match ty {
        ir::Ty::PhantomData(_) => Ok(Some(scalar(0, 1))),
        ir::Ty::Char => Ok(Some(scalar(4, 4))),
        ir::Ty::Cell(wrapper) => layout(wrapper.argument(), budget, depth + 1),
        ir::Ty::ManuallyDrop(wrapper) => layout(wrapper.argument(), budget, depth + 1),
        ir::Ty::Array { element, len } => {
            let Some(element) = layout(element, budget, depth + 1)? else {
                return Ok(None);
            };
            let count =
                i32::try_from(*len).map_err(|_| "array extent exceeds native memory limit")?;
            let size = element
                .size
                .checked_mul(count as usize)
                .filter(|size| *size <= super::super::backend::MAX_FRAME_BYTES as usize)
                .ok_or("unsupported: array element extent exceeds native memory limit")?;
            Ok(Some(scalar(size, element.align)))
        }
        ir::Ty::Tuple(_) | ir::Ty::Struct(_) => {
            let count = ty
                .field_count()
                .ok_or("compact aggregate has no canonical fields")?;
            let mut result = scalar(0, 1);
            for index in 0..count {
                let field = ty
                    .field_ty(index, budget)?
                    .ok_or("compact aggregate field is absent")?;
                let Some(field) = layout(&field, budget, depth + 1)? else {
                    return Ok(None);
                };
                if field.size != 0 {
                    if result.storage.is_some() {
                        return Ok(None);
                    }
                    result.storage = Some((index, field.size, field.align));
                    result.size = field.size;
                }
                result.align = result.align.max(field.align);
            }
            if let ir::Ty::Struct(s) = ty
                && let Some(alignment) = s.def().repr_align
            {
                result.align = result.align.max(alignment as usize);
            }
            result.size = align_up(result.size, result.align)?;
            Ok(Some(result))
        }
        _ => Ok(None),
    }
}
