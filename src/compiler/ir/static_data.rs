//! Immutable, canonical storage values for one source `static` declaration.
use super::*;

/// Flattened leaves of the static's type. References contain their promoted
/// target, never a host address. The backend lays out targets and relocations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StaticValue {
    Scalar(PromotedScalar),
    Text(Arc<str>),
    Slice {
        len: usize,
        values: Arc<[StaticValue]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StaticData {
    declaration: DeclId,
    ty: Ty,
    values: Arc<[StaticValue]>,
}

impl StaticData {
    pub(in crate::compiler) fn new(
        declaration: DeclId,
        ty: Ty,
        values: Vec<StaticValue>,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        retain_values(&values, budget, 0)?;
        let data = Self {
            declaration,
            ty,
            values: values.into(),
        };
        data.validate(budget)?;
        Ok(data)
    }
    pub(in crate::compiler) fn validate(&self, budget: &PlanBudget) -> Result<(), String> {
        if !self.declaration.is_source() {
            return Err("static data requires a source declaration identity".into());
        }
        let mut next = 0;
        check_shape(&self.ty, &self.values, &mut next, budget, 0)?;
        if next != self.values.len() {
            return Err("static data payload differs from its declared type".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn retain_storage(&self, budget: &PlanBudget) -> Result<(), String> {
        retain_values(&self.values, budget, 0)
    }
    pub fn declaration(&self) -> DeclId {
        self.declaration
    }
    pub fn value_ty(&self) -> &Ty {
        &self.ty
    }
    pub fn values(&self) -> &[StaticValue] {
        &self.values
    }
    pub fn reference_ty(&self) -> Ty {
        Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(self.ty.clone()),
        }
    }
    pub fn link_key(&self) -> Vec<u8> {
        let mut key = Vec::with_capacity(17);
        key.push(0x53);
        key.extend_from_slice(&self.declaration.snapshot.graph_word().to_le_bytes());
        key.extend_from_slice(&self.declaration.index.to_le_bytes());
        key
    }
}

fn retain_values(values: &[StaticValue], budget: &PlanBudget, depth: usize) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.retain_bytes(
        values
            .len()
            .checked_mul(std::mem::size_of::<StaticValue>())
            .and_then(|n| n.checked_add(2 * std::mem::size_of::<usize>()))
            .ok_or("static data storage overflow")?,
    )?;
    for value in values {
        budget.charge_work(1)?;
        match value {
            StaticValue::Scalar(_) => {}
            StaticValue::Text(text) => budget.retain_bytes(text.len())?,
            StaticValue::Slice { values, .. } => retain_values(values, budget, depth + 1)?,
        }
    }
    Ok(())
}

fn check_shape(
    ty: &Ty,
    values: &[StaticValue],
    next: &mut usize,
    budget: &PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match ty {
        Ty::Unit => Ok(()),
        Ty::Int(_) | Ty::Bool | Ty::Char => {
            let Some(StaticValue::Scalar(value)) = values.get(*next) else {
                return Err("static scalar payload is absent".into());
            };
            value.validate()?;
            if value.value_ty() != *ty {
                return Err("static data scalar differs from its declared type".into());
            }
            *next += 1;
            Ok(())
        }
        Ty::Ref {
            kind: BorrowKind::Shared,
            referent,
        } => {
            let value = values
                .get(*next)
                .ok_or("static reference payload is absent")?;
            match (referent.as_ref(), value) {
                (Ty::Str, StaticValue::Text(_)) => {}
                (Ty::Slice { element }, StaticValue::Slice { len, values })
                    if *len <= MAX_ARRAY_LEN =>
                {
                    let mut inner = 0;
                    for _ in 0..*len {
                        check_shape(element, values, &mut inner, budget, depth + 1)?;
                    }
                    if inner != values.len() {
                        return Err("static slice payload differs from its declared type".into());
                    }
                }
                _ => return Err("static reference payload differs from its declared type".into()),
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
        Ty::Tuple(fields) => {
            for field in fields.iter() {
                check_shape(field, values, next, budget, depth + 1)?;
            }
            Ok(())
        }
        Ty::Struct(structure) => {
            for field in structure.def().fields.read(budget)?.iter() {
                check_shape(&field.ty, values, next, budget, depth + 1)?;
            }
            Ok(())
        }
        _ => Err("unsupported: static data has an unsupported leaf".into()),
    }
}
