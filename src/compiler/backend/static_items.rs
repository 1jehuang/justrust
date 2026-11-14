//! Immutable static item storage: one interned data blob per declaration.
use super::*;

/// The canonical declaration key, its exact immutable bytes and the storage
/// alignment. Linkers intern by `key` and reject two objects that disagree on
/// the bytes or alignment.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StaticBlob {
    pub key: Vec<u8>,
    pub bytes: Vec<u8>,
    pub align: usize,
}

impl Emitter<'_> {
    pub(super) fn static_ref(&mut self, data: &ir::StaticData) -> Result<(), String> {
        let budget = self.concrete.facts().budget().clone();
        data.validate(&budget)?;
        let ty = data.value_ty();
        let align = usize::try_from(frame_value_alignment(ty, &budget)?)
            .map_err(|_| "static alignment is negative")?
            .max(16);
        let bytes = match ty {
            ir::Ty::Array { .. } if align == 16 && scalar_leaf_array(ty) => {
                self.promoted_array_bytes(ty, data.values())?
            }
            ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char => match data.values() {
                [scalar] => self.promoted_scalar_bytes(*scalar)?,
                _ => return Err("static scalar payload changed".into()),
            },
            _ => static_bytes(ty, data.values(), &budget)?,
        };
        let key = data.link_key();
        budget.retain_bytes(
            std::mem::size_of::<(usize, StaticBlob)>()
                .checked_add(key.len())
                .ok_or("static relocation storage overflow")?,
        )?;
        self.object
            .statics
            .try_reserve_exact(1)
            .map_err(|_| "static relocation allocation failed")?;
        self.bytes(&[0x48, 0xbe]); // mov rsi, imm64
        let at = self.position();
        self.bytes(&[0; 8]);
        self.object
            .statics
            .push((at, StaticBlob { key, bytes, align }));
        self.bytes(&[0x48, 0x89, 0xf0]); // mov rax, rsi: thin static pointer
        Ok(())
    }
}

/// Arrays (of arrays) of scalars: the packed promoted-array image.
fn scalar_leaf_array(ty: &ir::Ty) -> bool {
    match ty {
        ir::Ty::Array { element, .. } => scalar_leaf_array(element),
        ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char => true,
        _ => false,
    }
}

/// The exact private storage image of an aggregate static: the same offsets
/// `field_offset`/`array_offset` address and the same scalar widths the
/// loads use (normalized words in private slots, exact widths in packed
/// array elements and compact aggregates). Padding is zero.
fn static_bytes(
    ty: &ir::Ty,
    values: &[ir::PromotedScalar],
    budget: &ir::PlanBudget,
) -> Result<Vec<u8>, String> {
    let size = usize::try_from(value_size(ty, budget)?)
        .map_err(|_| "static extent is negative")?
        .max(16);
    budget.charge_work(size)?;
    budget.retain_bytes(size)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| "static data allocation failed")?;
    bytes.resize(size, 0);
    let mut next = 0usize;
    write_static(ty, values, &mut next, &mut bytes, 0, false, budget, 0)?;
    if next != values.len() {
        return Err("static data payload differs from its declared type".into());
    }
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
fn write_static(
    ty: &ir::Ty,
    values: &[ir::PromotedScalar],
    next: &mut usize,
    out: &mut [u8],
    at: usize,
    exact: bool,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match ty {
        ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char => {
            let scalar = *values.get(*next).ok_or("static data payload is short")?;
            *next += 1;
            scalar.validate()?;
            if scalar.value_ty() != *ty {
                return Err("static data scalar differs from its declared type".into());
            }
            let (word, high): (u64, u64) = match scalar {
                ir::PromotedScalar::Int(value) if value.ty.bits() == 128 => {
                    (value.bits as u64, (value.bits >> 64) as u64)
                }
                ir::PromotedScalar::Int(value) => {
                    let bits = value.ty.bits();
                    let raw = value.bits as u64;
                    let word = if bits == 64 {
                        raw
                    } else if value.ty.is_signed() {
                        (((raw << (64 - bits)) as i64) >> (64 - bits)) as u64
                    } else {
                        raw & ((1u64 << bits) - 1)
                    };
                    (word, 0)
                }
                ir::PromotedScalar::Bool(value) => (u64::from(value), 0),
                ir::PromotedScalar::Char(value) => (u64::from(value.value), 0),
            };
            let width = if wide_integer(ty) {
                16
            } else if exact {
                scalar_memory_bytes(ty) as usize
            } else {
                8
            };
            let mut image = [0u8; 16];
            image[..8].copy_from_slice(&word.to_le_bytes());
            image[8..].copy_from_slice(&high.to_le_bytes());
            let end = at.checked_add(width).ok_or("static offset overflow")?;
            out.get_mut(at..end)
                .ok_or("static leaf exceeds its storage")?
                .copy_from_slice(&image[..width]);
            Ok(())
        }
        ir::Ty::Array { len, .. } => {
            for index in 0..*len {
                let (offset, element) = array_offset(ty, index, budget)?;
                let offset = usize::try_from(offset).map_err(|_| "negative array offset")?;
                write_static(
                    &element,
                    values,
                    next,
                    out,
                    at.checked_add(offset).ok_or("static offset overflow")?,
                    true,
                    budget,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        ir::Ty::Tuple(_) | ir::Ty::Struct(_) => {
            let compact = crate::compiler::memory_layout::compact::layout(ty, budget, 0)?.is_some();
            let count = ty
                .field_count()
                .ok_or("static aggregate has no canonical fields")?;
            for index in 0..count {
                let (offset, field) = field_offset(ty, index, budget)?;
                let offset = usize::try_from(offset).map_err(|_| "negative field offset")?;
                write_static(
                    &field,
                    values,
                    next,
                    out,
                    at.checked_add(offset).ok_or("static offset overflow")?,
                    compact,
                    budget,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        _ => Err("unsupported: static data type has a non-scalar leaf".into()),
    }
}
