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
impl Layout {
    pub fn field_offset(self, index: usize, field_align: usize) -> Result<usize, String> {
        match self.storage {
            Some((storage, size, _)) if index > storage => align_up(size, field_align),
            _ => Ok(0),
        }
    }
}
// Rust and repr(C) agree on the sole nonzero field's offset, size and
// alignment. ZST field addresses need more: Rust can move a high-alignment ZST
// before storage, or a low-alignment leading ZST after it. Without exact source
// representation metadata those ambiguous observations must fail closed.
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
        if !matches!(owner, ir::Ty::Struct(_)) {
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
        let Some(owner_layout) = layout(&owner, budget, 0)? else {
            if !owner.is_closed_shallow() {
                return Err("unsupported: generic ZST field address requires an exact source representation".into());
            }
            continue;
        };
        if let Some((storage, _, align)) = owner_layout.storage
            && (*index < storage && field.align < align || *index > storage && field.align > align)
        {
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
            result.size = align_up(result.size, result.align)?;
            Ok(Some(result))
        }
        _ => Ok(None),
    }
}
