//! Native typed pointer spacing. Aggregate pointers address the same canonical
//! field representation as array indexing and raw dereference. This is not an
//! authorization to expose a private carrier as Rust `size_of` or a C layout.
use super::{ir, walk_size_authorized};

pub(super) fn extent(
    ty: &ir::Ty,
    facts: Option<&super::super::traits::ValidatedFacts>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<usize>, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match ty {
        ir::Ty::Param(_) | ir::Ty::AssociatedType(_) => Ok(None),
        ir::Ty::Cell(wrapper) => extent(wrapper.argument(), facts, budget, depth + 1),
        ir::Ty::ManuallyDrop(wrapper) => extent(wrapper.argument(), facts, budget, depth + 1),
        ir::Ty::Array { element, len } => {
            let element = extent(element, facts, budget, depth + 1)?;
            // Validate every nested extent even below an empty array. A zero
            // outer length does not authenticate unsupported element metadata.
            let count =
                i32::try_from(*len).map_err(|_| "array extent exceeds native memory limit")?;
            element
                .map(|element| {
                    element
                        .checked_mul(count as usize)
                        .filter(|size| *size <= super::super::backend::MAX_FRAME_BYTES as usize)
                        .ok_or_else(|| {
                            "unsupported: array element extent exceeds native memory limit".into()
                        })
                })
                .transpose()
        }
        ir::Ty::Tuple(_) | ir::Ty::Struct(_) => {
            let count = ty
                .field_count()
                .ok_or("native aggregate has no canonical fields")?;
            let mut closed = true;
            for index in 0..count {
                let field = ty
                    .field_ty(index, budget)?
                    .ok_or("native aggregate field is absent")?;
                closed &= extent(&field, facts, budget, depth + 1)?.is_some();
            }
            if !closed {
                return Ok(None);
            }
            let layout = super::compact::layout(ty, budget, depth + 1)?
                .ok_or("unsupported: pointer aggregate requires an exact public target memory representation")?;
            let stride = usize::try_from(super::super::backend::array_element_stride(ty, budget)?)
                .map_err(|_| "native aggregate stride is negative")?;
            if stride != layout.size {
                return Err("unsupported: public pointer stride requires exact semantic and physical representation agreement".into());
            }
            Ok(Some(stride))
        }
        _ => {
            let Some(size) = walk_size_authorized(ty, facts, budget, depth + 1)? else {
                return Ok(None);
            };
            let stride = if matches!(ty, ir::Ty::Ref { referent, .. } if matches!(referent.as_ref(), ir::Ty::Slice { .. }))
            {
                // This is the stride of references stored as elements, not
                // the stride of their slice's elements. Metadata above has
                // already established the exact two-word descriptor.
                2 * usize::from(super::super::backend::SHARED_BYTE_SLICE_LAYOUT.word_bytes)
            } else {
                usize::try_from(super::super::backend::array_element_stride(ty, budget)?)
                    .map_err(|_| "native pointer stride is negative")?
            };
            if size != stride {
                return Err("unsupported: public pointer stride requires exact semantic and physical representation agreement".into());
            }
            Ok(Some(stride))
        }
    }
}
