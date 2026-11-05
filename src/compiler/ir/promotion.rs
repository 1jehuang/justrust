//! Closed, immutable array rvalue promotion. No runtime place or reference is
//! accepted: the source publication still authenticates the complete typed tree.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotedArray {
    ty: Ty,
    owner: super::super::source_replay::OriginalBodyId,
    root: ExprId,
    site: ExprId,
    values: Arc<[PromotedScalar]>,
}
impl PromotedArray {
    pub(in crate::compiler) fn from_expression(
        root: ExprId,
        site: ExprId,
        owner: &super::super::source_replay::OriginalBodyId,
        expressions: &[TypedExpr],
        budget: &PlanBudget,
    ) -> Result<Option<Self>, String> {
        if root.0 >= site.0 || site.0 > expressions.len() {
            return Err("promoted array occurrence changed".into());
        }
        let Some(expression) = expressions.get(root.0) else {
            return Err("promoted array root is absent".into());
        };
        if !matches!(expression.ty, Ty::Array { .. }) {
            return Ok(None);
        }
        if !closed_type(&expression.ty, budget, 0)? {
            return Ok(None);
        }
        let mut values = Vec::new();
        if !collect(root, expressions, &mut values, budget, 0)? {
            return Ok(None);
        }
        budget.retain_bytes(
            values
                .len()
                .checked_mul(std::mem::size_of::<PromotedScalar>())
                .and_then(|n| n.checked_add(2 * std::mem::size_of::<usize>()))
                .ok_or("promoted array storage overflow")?,
        )?;
        Ok(Some(Self {
            ty: expression.ty.clone(),
            owner: owner.clone(),
            root,
            site,
            values: values.into(),
        }))
    }
    pub(in crate::compiler) fn validate(
        &self,
        site: ExprId,
        expressions: &[TypedExpr],
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if site != self.site {
            return Err("promoted array typed occurrence changed".into());
        }
        let replay = Self::from_expression(self.root, self.site, &self.owner, expressions, budget)?
            .ok_or("promoted array no longer has a closed constant initializer")?;
        if replay != *self {
            return Err("promoted array initializer or type changed".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn validate_owner(
        &self,
        owner: &super::super::source_replay::OriginalBodyId,
    ) -> Result<(), String> {
        if &self.owner != owner {
            return Err("promoted array original source owner changed".into());
        }
        Ok(())
    }
    pub fn reference_ty(&self) -> Ty {
        Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(self.ty.clone()),
        }
    }
    pub fn values(&self) -> &[PromotedScalar] {
        &self.values
    }
    pub fn value_ty(&self) -> &Ty {
        &self.ty
    }
}

fn closed_type(ty: &Ty, budget: &PlanBudget, depth: usize) -> Result<bool, String> {
    budget.charge_work(1)?;
    if depth > 128 {
        return Err("unsupported: promoted array type nesting exceeds 128".into());
    }
    Ok(match ty {
        Ty::Int(_) | Ty::Bool | Ty::Char => true,
        Ty::Array { element, len } if *len <= MAX_ARRAY_LEN => {
            closed_type(element, budget, depth + 1)?
        }
        _ => false,
    })
}

fn collect(
    id: ExprId,
    expressions: &[TypedExpr],
    values: &mut Vec<PromotedScalar>,
    budget: &PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.charge_work(1)?;
    if depth > 128 {
        return Err("unsupported: promoted array nesting exceeds 128".into());
    }
    let expression = expressions
        .get(id.0)
        .ok_or("promoted array expression missing")?;
    if expression.completion != Completion::Returns {
        return Ok(false);
    }
    let scalar = match &expression.kind {
        ExprKind::Int(value) => Some(PromotedScalar::Int(*value)),
        ExprKind::Bool(value) => Some(PromotedScalar::Bool(*value)),
        ExprKind::Char(value) => Some(PromotedScalar::Char(*value)),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            value,
        } => {
            let Some(TypedExpr {
                kind: ExprKind::Int(literal),
                ..
            }) = expressions.get(value.0)
            else {
                return Ok(false);
            };
            if !literal.ty.is_signed() || literal.bits > (1u128 << (literal.ty.bits() - 1)) {
                return Ok(false);
            }
            let bits = literal.ty.bits();
            let negated = literal.bits.wrapping_neg();
            Some(PromotedScalar::Int(IntValue {
                ty: literal.ty,
                bits: if bits == 128 {
                    negated
                } else {
                    negated & ((1u128 << bits) - 1)
                },
            }))
        }
        ExprKind::Array(elements) => {
            let Ty::Array { element, len } = &expression.ty else {
                return Ok(false);
            };
            if *len != elements.len() {
                return Ok(false);
            }
            for child in elements {
                if expressions.get(child.0).is_none_or(|e| e.ty != **element)
                    || !collect(*child, expressions, values, budget, depth + 1)?
                {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        ExprKind::Repeat { value, len } => {
            let Ty::Array {
                element,
                len: count,
            } = &expression.ty
            else {
                return Ok(false);
            };
            if count != len || expressions.get(value.0).is_none_or(|e| e.ty != **element) {
                return Ok(false);
            }
            let start = values.len();
            if !collect(*value, expressions, values, budget, depth + 1)? {
                if *len == 0 {
                    return Err("unsupported: static zero-repeat promotion with runtime operand requires preserved evaluation".into());
                }
                return Ok(false);
            }
            let width = values.len() - start;
            if *len == 0 {
                values.truncate(start);
                return Ok(true);
            }
            for _ in 1..*len {
                for index in start..start + width {
                    push(values, values[index], budget)?;
                }
            }
            return Ok(true);
        }
        ExprKind::Binary { left, right, .. } => {
            let start = values.len();
            let closed = collect(*left, expressions, values, budget, depth + 1)?
                && collect(*right, expressions, values, budget, depth + 1)?;
            values.truncate(start);
            if closed {
                return Err(
                    "unsupported: array rvalue promotion of constant arithmetic is not implemented"
                        .into(),
                );
            }
            return Ok(false);
        }
        _ => None,
    };
    let Some(scalar) = scalar else {
        return Ok(false);
    };
    scalar.validate()?;
    if scalar.value_ty() != expression.ty {
        return Ok(false);
    }
    push(values, scalar, budget)?;
    Ok(true)
}
fn push(
    values: &mut Vec<PromotedScalar>,
    value: PromotedScalar,
    budget: &PlanBudget,
) -> Result<(), String> {
    if values.len() >= MAX_ARRAY_LEN {
        return Err("unsupported: promoted array scalar payload exceeds 65535 elements".into());
    }
    budget.charge_work(1)?;
    if values.len() == values.capacity() {
        let additional = values.capacity().max(4);
        budget.retain_bytes(
            additional
                .checked_mul(std::mem::size_of::<PromotedScalar>())
                .ok_or("promoted array capacity overflow")?,
        )?;
        values
            .try_reserve_exact(additional)
            .map_err(|_| "promoted array allocation failed")?;
    }
    values.push(value);
    Ok(())
}
