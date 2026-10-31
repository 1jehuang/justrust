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
    if query.is_semantic_size() {
        return semantic_size(query.queried_type(), facts, budget);
    }
    if query.is_private_element_stride() || query.is_supported_pointer_stride() {
        if matches!(query.queried_type(), ir::Ty::Param(_)) {
            return Ok(None);
        }
        let semantic = if query.is_supported_pointer_stride() {
            let Some(size) = semantic_size(query.queried_type(), facts, budget)? else {
                return Ok(None);
            };
            Some(size)
        } else {
            None
        };
        let stride = usize::try_from(super::backend::array_element_stride(
            query.queried_type(),
            budget,
        )?)
        .map_err(|_| "private element stride is negative")?;
        if semantic.is_some_and(|size| size != stride) {
            return Err("unsupported: public pointer stride requires exact semantic and physical representation agreement".into());
        }
        return Ok(Some(stride));
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

/// Exact public size authority for the implemented Linux x86-64 domain.
/// No aggregate/private carrier extent is a public representation proof.
pub(super) fn semantic_size(
    ty: &ir::Ty,
    facts: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<Option<usize>, String> {
    facts.validate_ty_with_budget(ty, budget)?;
    walk_size_authorized(ty, facts.validated(), budget, 0)
}

// This is public metadata authority, not private carrier classification.
// Nominal tails require exact canonical Sized evidence, never private layout.
fn pointer_metadata_size_authorized(
    referent: &ir::Ty,
    facts: Option<&super::traits::ValidatedFacts>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<usize>, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match referent {
        ir::Ty::FnPointer(_) | ir::Ty::PhantomData(_) => Ok(Some(8)),
        ir::Ty::Param(_) => Ok(None),
        ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_) => Ok(Some(16)),
        ir::Ty::Unit
        | ir::Ty::Bool
        | ir::Ty::Int(_)
        | ir::Ty::Float(_)
        | ir::Ty::Char
        | ir::Ty::Never
        | ir::Ty::Ref { .. }
        | ir::Ty::RawPtr { .. } => Ok(Some(8)),
        ir::Ty::Array { element, .. } => {
            match pointer_metadata_size_authorized(element, facts, budget, depth + 1)? {
                Some(8) => Ok(Some(8)),
                None => Ok(None),
                _ => Err("unsupported: array pointee requires a proven Sized element".into()),
            }
        }
        _ => {
            let Some(facts) = facts else {
                return Ok(None);
            };
            match facts.require_sized(referent) {
                Ok(()) => Ok(Some(8)),
                Err(error)
                    if error.kind() == super::traits::FactsErrorKind::Unproved
                        && facts.phase() != super::traits::FactsPhase::Closed =>
                {
                    Ok(None)
                }
                Err(error) if error.kind() == super::traits::FactsErrorKind::Unproved => Err(
                    "unsupported: nominal-tail pointer size metadata has no implemented public target representation".into(),
                ),
                Err(error) => Err(error.into_legacy_diagnostic(budget)),
            }
        }
    }
}

#[cfg(test)]
fn walk_size(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<Option<usize>, String> {
    walk_size_authorized(ty, None, budget, depth)
}
fn walk_size_authorized(
    ty: &ir::Ty,
    facts: Option<&super::traits::ValidatedFacts>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<usize>, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    // Pointer metadata is checked before scalar_size_align: a reference to a
    // rigid ?Sized parameter is not known to be a thin pointer yet.
    if let ir::Ty::Ref { referent, .. }
    | ir::Ty::RawPtr {
        pointee: referent, ..
    } = ty
    {
        return pointer_metadata_size_authorized(referent, facts, budget, depth + 1);
    }
    if let Some((size, _)) = super::c_abi::layout::scalar_size_align(ty) {
        return Ok(Some(size));
    }
    match ty {
        // Function-pointer values are one Linux x86-64 pointer. This public
        // size observation grants no native callable ABI or invocation support.
        ir::Ty::FnPointer(_) => Ok(Some(8)),
        ir::Ty::PhantomData(_) => Ok(Some(0)),
        ir::Ty::Param(_) => Ok(None),
        ir::Ty::Char => Ok(Some(4)),
        ir::Ty::Array { element, len } => {
            let Some(size) = walk_size_authorized(element, facts, budget, depth + 1)? else {
                return Ok(None);
            };
            let extent = size
                .checked_mul(*len)
                .filter(|size| *size <= i64::MAX as usize)
                .ok_or("unsupported: target array size exceeds the Linux x86-64 object limit")?;
            Ok(Some(extent))
        }
        ir::Ty::Unit | ir::Ty::Bool | ir::Ty::Int(_) | ir::Ty::Float(_) => {
            Err("target scalar size authority is missing".into())
        }
        ir::Ty::Tuple(_)
        | ir::Ty::Struct(_)
        | ir::Ty::Enum(_)
        | ir::Ty::Closure(_)
        | ir::Ty::FnItem(_) => Err(
            "unsupported: target aggregate size requires a genuine public target representation"
                .into(),
        ),
        ir::Ty::Opaque(_) | ir::Ty::AssociatedType(_) => {
            Err("unsupported: target size requires graph-authorized type normalization".into())
        }
        ir::Ty::Str | ir::Ty::Slice { .. } | ir::Ty::Dyn(_) | ir::Ty::Never => {
            Err("unsupported: target size requires an implemented Sized type".into())
        }
        ir::Ty::Ref { .. } | ir::Ty::RawPtr { .. } => unreachable!(),
    }
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
        ir::Ty::PhantomData(_) => Ok(Some(1)),
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
        ir::Ty::FnPointer(_) => Ok(Some(8)),
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
    fn public_size_is_exact_for_scalars_and_recursive_fixed_arrays() {
        let budget = ir::PlanBudget::default();
        for (ty, size) in [
            (ir::Ty::Unit, 0),
            (ir::Ty::Bool, 1),
            (ir::Ty::Char, 4),
            (ir::Ty::Int(ir::IntTy::U8), 1),
            (ir::Ty::Int(ir::IntTy::I8), 1),
            (ir::Ty::Int(ir::IntTy::U16), 2),
            (ir::Ty::Int(ir::IntTy::I16), 2),
            (ir::Ty::Int(ir::IntTy::U32), 4),
            (ir::Ty::Int(ir::IntTy::I32), 4),
            (ir::Ty::Int(ir::IntTy::U64), 8),
            (ir::Ty::Int(ir::IntTy::I64), 8),
            (ir::Ty::Int(ir::IntTy::U128), 16),
            (ir::Ty::Int(ir::IntTy::I128), 16),
            (ir::Ty::Int(ir::IntTy::Usize), 8),
            (ir::Ty::Int(ir::IntTy::Isize), 8),
            (ir::Ty::Float(ir::FloatTy::F32), 4),
            (ir::Ty::Float(ir::FloatTy::F64), 8),
        ] {
            assert_eq!(walk_size(&ty, &budget, 0).unwrap(), Some(size));
            for len in [0, 1, 3, 17] {
                let nested = ir::Ty::Array {
                    element: Arc::new(ir::Ty::Array {
                        element: Arc::new(ty.clone()),
                        len,
                    }),
                    len: 3,
                };
                assert_eq!(
                    walk_size(&nested, &budget, 0).unwrap(),
                    Some(3 * len * size)
                );
                for limits in [
                    ir::PlanLimits {
                        depth: 0,
                        ..Default::default()
                    },
                    ir::PlanLimits {
                        work: 0,
                        ..Default::default()
                    },
                ] {
                    assert!(walk_size(&nested, &ir::PlanBudget::new(limits), 0).is_err());
                }
            }
        }
        for referent in [
            ir::Ty::Int(ir::IntTy::U8),
            ir::Ty::Str,
            ir::Ty::Slice {
                element: Arc::new(ir::Ty::Int(ir::IntTy::U16)),
            },
        ] {
            let size = if matches!(referent, ir::Ty::Int(_)) {
                8
            } else {
                16
            };
            for kind in [ir::BorrowKind::Shared, ir::BorrowKind::Mut] {
                assert_eq!(
                    walk_size(
                        &ir::Ty::Ref {
                            kind,
                            referent: Arc::new(referent.clone())
                        },
                        &budget,
                        0
                    )
                    .unwrap(),
                    Some(size)
                );
                assert_eq!(
                    walk_size(
                        &ir::Ty::RawPtr {
                            kind,
                            pointee: Arc::new(referent.clone())
                        },
                        &budget,
                        0
                    )
                    .unwrap(),
                    Some(size)
                );
            }
        }
        assert!(
            walk_size(
                &ir::Ty::Tuple(Arc::from([ir::Ty::Int(ir::IntTy::U8)])),
                &budget,
                0
            )
            .unwrap_err()
            .contains("unsupported")
        );
        let overflow = ir::Ty::Array {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U128)),
            len: usize::MAX,
        };
        assert!(
            walk_size(&overflow, &budget, 0)
                .unwrap_err()
                .contains("object limit")
        );
    }

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
