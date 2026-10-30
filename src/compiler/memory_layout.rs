//! Target Rust alignment observations, separate from private frame/carrier storage.
//! Callers must authenticate the source intrinsic and validate the queried type
//! against their own immutable caller facts before admitting a query.
use super::ir;
pub(super) mod domain;

/// Compiler-private carrier extent. This does not authorize public native
/// pointer offsets, Rust size_of, external ABI layout, or allocation effects.
pub(super) fn query_value(
    query: &ir::TypeAlignmentQuery,
    facts: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<Option<usize>, String> {
    query.validate(facts, budget)?;
    if query.is_packed_byte_reserve() {
        // A byte-count query only. No allocation, pointer or initialized-range
        // authority is granted here; private storage_stride::<u8>() stays eight.
        return Ok(Some(1));
    }
    if query.is_private_storage_alignment() {
        facts.validate_ty_with_budget(query.queried_type(), budget)?;
        return walk_alignment(query.queried_type(), budget, 0, true);
    }
    if !query.is_private_storage_stride() {
        return alignment(query.queried_type(), facts, budget);
    }
    if matches!(query.queried_type(), ir::Ty::Param(_)) {
        return Ok(None);
    }
    budget.charge_work(1)?;
    usize::try_from(super::backend::value_size(query.queried_type(), budget)?)
        .map(Some)
        .map_err(|_| "private storage stride is negative".into())
}

/// `None` retains a genuinely symbolic generic query until ordinary instance
/// substitution. It is never a guessed alignment or a published placeholder.
pub(super) fn alignment(
    ty: &ir::Ty,
    facts: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<Option<usize>, String> {
    facts.validate_ty_with_budget(ty, budget)?;
    walk_alignment(ty, budget, 0, false)
}

fn walk_alignment(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
    private: bool,
) -> Result<Option<usize>, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    if let Some((_, alignment)) = super::c_abi::layout::scalar_size_align(ty) {
        return Ok(Some(alignment));
    }
    match ty {
        ir::Ty::Opaque(_) => Err(
            "unsupported: opaque target alignment requires graph-authorized physical normalization"
                .into(),
        ),
        ir::Ty::Param(_) => Ok(None),
        ir::Ty::Char => Ok(Some(4)),
        ir::Ty::Unit | ir::Ty::Bool | ir::Ty::Int(_) | ir::Ty::Float(_) => {
            Err("target scalar alignment authority is missing".into())
        }
        // `*const dyn P` / `*mut dyn P`: two pointer-sized words.
        ir::Ty::RawPtr { pointee, .. } if matches!(pointee.as_ref(), ir::Ty::Dyn(_)) => Ok(Some(8)),
        ir::Ty::RawPtr { .. } => Err("unsupported: target layout of an unsized raw pointer".into()),
        ir::Ty::Ref { .. } => Ok(Some(8)),
        ir::Ty::Array { element, .. } => walk_alignment(element, budget, depth + 1, private),
        ir::Ty::Tuple(fields) => field_alignment(fields.iter(), budget, depth + 1, private),
        ir::Ty::Struct(structure) => field_alignment(
            structure
                .def()
                .fields
                .read(budget)?
                .iter()
                .map(|field| &field.ty),
            budget,
            depth + 1,
            private,
        ),
        ir::Ty::AssociatedType(_) => {
            Err("unsupported: unresolved associated target alignment".into())
        }
        ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_) => {
            Err("unsupported: target alignment requires a Sized type".into())
        }
        // PRIVATE enum layout (backend::value_size): an eight-byte tag word
        // followed by the largest variant payload.
        ir::Ty::Enum(enumeration) if private => {
            let mut alignment = Some(8);
            for variant in enumeration.def().variants.iter() {
                match field_alignment(
                    variant.fields.read(budget)?.iter().map(|f| &f.ty),
                    budget,
                    depth + 1,
                    private,
                )? {
                    Some(value) => alignment = alignment.map(|a: usize| a.max(value)),
                    None => alignment = None,
                }
            }
            Ok(alignment)
        }
        // Zero-sized fn items and capture-free closures own one dummy word.
        ir::Ty::FnItem(_) if private => Ok(Some(8)),
        ir::Ty::Closure(closure) if private && closure.captures().is_empty() => Ok(Some(8)),
        ir::Ty::Enum(_) | ir::Ty::FnItem(_) | ir::Ty::Closure(_) | ir::Ty::Never => {
            Err("unsupported: target alignment for this type is not implemented".into())
        }
    }
}

fn field_alignment<'a>(
    fields: impl Iterator<Item = &'a ir::Ty>,
    budget: &ir::PlanBudget,
    depth: usize,
    private: bool,
) -> Result<Option<usize>, String> {
    let mut alignment = 1;
    let mut symbolic = false;
    for field in fields {
        match walk_alignment(field, budget, depth, private)? {
            Some(value) => alignment = alignment.max(value),
            None => symbolic = true,
        }
    }
    Ok((!symbolic).then_some(alignment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn private_alignment_walk_keeps_per_task_depth_and_work_limits() {
        let ty = ir::Ty::Array {
            element: Arc::new(ir::Ty::Array {
                element: Arc::new(ir::Ty::Int(ir::IntTy::U16)),
                len: 3,
            }),
            len: 2,
        };
        let budget = ir::PlanBudget::default();
        assert_eq!(walk_alignment(&ty, &budget, 0, false).unwrap(), Some(2));
        assert_eq!(walk_alignment(&ty, &budget, 0, true).unwrap(), Some(2));
        for private in [false, true] {
            let limited = ir::PlanBudget::new(ir::PlanLimits {
                depth: 0,
                ..Default::default()
            });
            assert!(walk_alignment(&ty, &limited, 0, private).is_err());
            let limited = ir::PlanBudget::new(ir::PlanLimits {
                work: 0,
                ..Default::default()
            });
            assert!(walk_alignment(&ty, &limited, 0, private).is_err());
        }
    }
}
