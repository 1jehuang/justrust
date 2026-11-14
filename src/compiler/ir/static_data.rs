//! Immutable `static` item storage borrowed as `&'static T`.
//!
//! Unlike rvalue promotion, every borrow of one static declaration denotes the
//! same address (stock guarantees `ptr::eq(&S, &S)`). The checked payload is
//! keyed by the canonical declaration identity, and the linker interns one
//! data symbol per identity. The payload is the flattened integer/bool/char
//! leaves of arrays, tuples and structs in declaration order; `static mut`
//! and interior mutability stay unsupported.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StaticData {
    declaration: DeclId,
    ty: Ty,
    values: Arc<[PromotedScalar]>,
}

impl StaticData {
    pub(in crate::compiler) fn new(
        declaration: DeclId,
        ty: Ty,
        values: Vec<PromotedScalar>,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        budget.retain_bytes(
            values
                .len()
                .checked_mul(std::mem::size_of::<PromotedScalar>())
                .and_then(|n| n.checked_add(2 * std::mem::size_of::<usize>()))
                .ok_or("static data storage overflow")?,
        )?;
        let data = Self {
            declaration,
            ty,
            values: values.into(),
        };
        data.validate(budget)?;
        Ok(data)
    }
    /// The payload is exactly the flattened closed scalars of its type.
    pub(in crate::compiler) fn validate(&self, budget: &PlanBudget) -> Result<(), String> {
        if !self.declaration.is_source() {
            return Err("static data requires a source declaration identity".into());
        }
        let mut next = 0usize;
        check_shape(&self.ty, &self.values, &mut next, budget, 0)?;
        if next != self.values.len() {
            return Err("static data payload differs from its declared type".into());
        }
        Ok(())
    }
    pub fn declaration(&self) -> DeclId {
        self.declaration
    }
    pub fn value_ty(&self) -> &Ty {
        &self.ty
    }
    pub fn values(&self) -> &[PromotedScalar] {
        &self.values
    }
    pub fn reference_ty(&self) -> Ty {
        Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(self.ty.clone()),
        }
    }
    /// Injective link identity: the canonical declaration words.
    pub fn link_key(&self) -> Vec<u8> {
        let mut key = Vec::with_capacity(17);
        key.push(0x53);
        key.extend_from_slice(&self.declaration.snapshot.graph_word().to_le_bytes());
        key.extend_from_slice(&self.declaration.index.to_le_bytes());
        key
    }
}

fn check_shape(
    ty: &Ty,
    values: &[PromotedScalar],
    next: &mut usize,
    budget: &PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.charge_work(1)?;
    if depth > 128 {
        return Err("unsupported: static array nesting exceeds 128".into());
    }
    match ty {
        Ty::Int(_) | Ty::Bool | Ty::Char => {
            let value = values
                .get(*next)
                .ok_or("static data payload is shorter than its type")?;
            value.validate()?;
            if &value.value_ty() != ty {
                return Err("static data scalar differs from its declared type".into());
            }
            *next += 1;
            Ok(())
        }
        Ty::Array { element, len } if *len <= MAX_ARRAY_LEN => {
            for _ in 0..*len {
                check_shape(element, values, next, budget, depth + 1)?;
            }
            Ok(())
        }
        Ty::Tuple(fields) if !fields.is_empty() => {
            for field in fields.iter() {
                check_shape(field, values, next, budget, depth + 1)?;
            }
            Ok(())
        }
        Ty::Struct(structure) if !structure.def().fields.is_empty() => {
            for field in structure.def().fields.read(budget)?.iter() {
                check_shape(&field.ty, values, next, budget, depth + 1)?;
            }
            Ok(())
        }
        _ => Err("unsupported: static data type has a non-scalar leaf".into()),
    }
}
