//! LP64 C layout, deliberately separate from the stack emitter's carriers.
use crate::compiler::{backend, ir};

pub const MAX_BYTES: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    Integer { signed: bool },
    Float,
    Bool,
    Pointer,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub c_offset: usize,
    pub private_offset: usize,
    pub bytes: usize,
    pub scalar: Scalar,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: usize,
    pub align: usize,
    pub private_size: usize,
    pub indirect: bool,
    pub leaves: Vec<Leaf>,
    pub fields: Vec<usize>,
}
pub fn scalar_size_align(ty: &ir::Ty) -> Option<(usize, usize)> {
    match ty {
        ir::Ty::Unit => Some((0, 1)),
        ir::Ty::Bool => Some((1, 1)),
        ir::Ty::Int(i) => {
            let n = i.bits() as usize / 8;
            Some((n, n))
        }
        ir::Ty::Float(ir::FloatTy::F32) => Some((4, 4)),
        ir::Ty::Float(ir::FloatTy::F64) => Some((8, 8)),
        ir::Ty::RawPtr { pointee, .. }
            if !matches!(
                pointee.as_ref(),
                ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_)
            ) =>
        {
            Some((8, 8))
        }
        ir::Ty::Ref { referent, .. }
            if !matches!(
                referent.as_ref(),
                ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_)
            ) =>
        {
            Some((8, 8))
        }
        _ => None,
    }
}
pub(crate) fn align_up(n: usize, alignment: usize) -> Result<usize, String> {
    if !alignment.is_power_of_two() || alignment > 16 {
        return Err("unsupported: C alignment exceeds 16".into());
    }
    n.checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
        .filter(|v| *v <= MAX_BYTES)
        .ok_or_else(|| "resource limit: C layout exceeds 1048576 bytes".into())
}
/// `repr_c` answers using declaration identity, never a nominal display name.
/// It must come from the owning parsed program, not a caller's field shape.
pub fn of(
    ty: &ir::Ty,
    repr_c: &dyn Fn(ir::DeclId) -> bool,
    budget: &ir::PlanBudget,
) -> Result<Layout, String> {
    build(ty, repr_c, budget, 0)
}
fn build(
    ty: &ir::Ty,
    repr_c: &dyn Fn(ir::DeclId) -> bool,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Layout, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let private_size =
        usize::try_from(backend::value_size(ty)?).map_err(|_| "invalid private carrier size")?;
    let indirect = backend::indirect(ty)?;
    if let Some((size, align)) = scalar_size_align(ty) {
        // References are not admitted across this frozen C boundary. In particular
        // a pointer to a private aggregate is not a pointer to repr(C) storage.
        let scalar = match ty {
            ir::Ty::Unit => None,
            ir::Ty::Int(i) => Some(Scalar::Integer { signed: i.is_signed() }),
            ir::Ty::Float(_) => Some(Scalar::Float),
            ir::Ty::Bool => Some(Scalar::Bool),
            ir::Ty::RawPtr { pointee, .. } if matches!(pointee.as_ref(),
                ir::Ty::Int(ir::IntTy::I64 | ir::IntTy::U64 | ir::IntTy::Isize | ir::IntTy::Usize)
                    | ir::Ty::Float(ir::FloatTy::F64)) => Some(Scalar::Pointer),
            _ => return Err("unsupported: C raw pointers require canonical 8-byte i64/u64/isize/usize/f64 pointees; narrow, ZST, aggregate, nested-pointer and Rust-reference boundary storage is unsupported".into()),
        };
        let count = if size == 16 {
            2
        } else {
            usize::from(size != 0)
        };
        budget.retain_bytes(count * std::mem::size_of::<Leaf>())?;
        let mut leaves = Vec::with_capacity(count);
        if let Some(scalar) = scalar {
            for offset in (0..size).step_by(8) {
                leaves.push(Leaf {
                    c_offset: offset,
                    private_offset: offset,
                    bytes: (size - offset).min(8),
                    scalar,
                });
            }
        }
        return Ok(Layout {
            size,
            align,
            private_size,
            indirect,
            leaves,
            fields: Vec::new(),
        });
    }
    let mut parts = Vec::new();
    match ty {
        ir::Ty::Struct(s) if repr_c(s.def().source_decl) => {
            if s.def().fields.is_empty() {
                return Err("unsupported: empty nominal structs have only a private carrier, not a C ABI layout".into());
            }
            budget.retain_bytes(s.def().fields.len() * std::mem::size_of::<Layout>())?;
            for field in s.def().fields.iter() {
                parts.push(build(&field.ty, repr_c, budget, depth + 1)?);
            }
        }
        ir::Ty::Array { element, len } => {
            if *len > ir::MAX_ARRAY_LEN {
                return Err("resource limit: C array length".into());
            }
            let element = build(element, repr_c, budget, depth + 1)?;
            budget.charge_work(*len)?;
            budget.retain_bytes(
                len.checked_mul(
                    std::mem::size_of::<Layout>()
                        + element.leaves.len() * std::mem::size_of::<Leaf>(),
                )
                .ok_or("C layout storage overflow")?,
            )?;
            parts.resize(*len, element);
        }
        _ => {
            return Err(
                "unsupported: C ABI requires scalar, array, or authenticated repr(C) struct layout"
                    .into(),
            );
        }
    }
    let mut result = Layout {
        size: 0,
        align: 1,
        private_size,
        indirect,
        leaves: Vec::new(),
        fields: Vec::new(),
    };
    let leaves = parts.iter().try_fold(0usize, |n, p| {
        n.checked_add(p.leaves.len()).ok_or("C leaf count overflow")
    })?;
    budget.retain_bytes(
        leaves
            .checked_mul(std::mem::size_of::<Leaf>())
            .and_then(|n| n.checked_add(parts.len() * std::mem::size_of::<usize>()))
            .ok_or("C layout storage overflow")?,
    )?;
    result.leaves.reserve_exact(leaves);
    result.fields.reserve_exact(parts.len());
    let mut private = 0usize;
    for part in parts {
        budget.charge_work(part.leaves.len() + 1)?;
        result.size = align_up(result.size, part.align)?;
        result.fields.push(result.size);
        for mut leaf in part.leaves {
            leaf.c_offset += result.size;
            leaf.private_offset += private;
            result.leaves.push(leaf);
        }
        result.size = result
            .size
            .checked_add(part.size)
            .filter(|v| *v <= MAX_BYTES)
            .ok_or("resource limit: C aggregate size")?;
        result.align = result.align.max(part.align);
        private = private
            .checked_add(part.private_size)
            .ok_or("C private carrier overflow")?;
    }
    result.size = align_up(result.size, result.align)?;
    Ok(result)
}
