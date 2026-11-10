//! Parameter-origin reference values. Captures and body-owned locals never seed a result.
use super::*;

pub(super) fn parameter_origin(
    exprs: &[ir::TypedExpr],
    locals: &[ir::LocalDecl],
    prefix: &[ir::TypedStmt],
    tuple: ir::LocalId,
    tail: ir::ExprId,
    budget: &ir::PlanBudget,
) -> Result<Option<ir::ClosureInputValueOrigin>, String> {
    if !matches!(
        exprs.get(tail.0).map(|expr| &expr.ty),
        Some(ir::Ty::Param(_))
    ) {
        return Ok(None);
    }
    value_origin(exprs, locals, prefix, tuple, tail, budget)
}

pub(super) fn value_origin(
    exprs: &[ir::TypedExpr],
    locals: &[ir::LocalDecl],
    prefix: &[ir::TypedStmt],
    tuple: ir::LocalId,
    tail: ir::ExprId,
    budget: &ir::PlanBudget,
) -> Result<Option<ir::ClosureInputValueOrigin>, String> {
    let expression = exprs
        .get(tail.0)
        .ok_or("invalid closure input value source")?;
    let Some(place) = capture_use::expression_place(exprs, tail)? else {
        return Ok(None);
    };
    let Some(ir::Ty::Tuple(parameters)) = locals.get(tuple.0).map(|local| &local.ty) else {
        return Err("closure input result lost its original argument tuple".into());
    };
    let mut result = None;
    for statement in prefix {
        budget.charge_work(1)?;
        let (value, matches) = match statement {
            ir::TypedStmt::Let { local, value } => (
                *value,
                (*local == place.local).then_some((Vec::new(), None)),
            ),
            ir::TypedStmt::LetPattern {
                value, bindings, ..
            } => {
                budget.charge_work(bindings.len())?;
                let binding = bindings.iter().find(|binding| binding.local == place.local);
                (
                    *value,
                    binding.map(|binding| (binding.projection.clone(), binding.borrow)),
                )
            }
            _ => {
                return Err(
                    "unsupported: closure input reference result requires a straight-line body"
                        .into(),
                );
            }
        };
        let Some(ir::TypedExpr {
            kind: ir::ExprKind::Field { base, index },
            ..
        }) = exprs.get(value.0)
        else {
            return Err("closure input result changed its original parameter entry".into());
        };
        if !matches!(exprs.get(base.0).map(|expr| &expr.kind), Some(ir::ExprKind::Local(local)) if *local == tuple)
        {
            return Err("closure input result parameter belongs to another argument tuple".into());
        }
        let Some((projection, borrow)) = matches else {
            continue;
        };
        if result.is_some() {
            return Err("closure input result repeats its parameter binding".into());
        }
        budget.retain_bytes(
            (projection.len() + place.projection.len()) * std::mem::size_of::<ir::Projection>(),
        )?;
        let mut input = ir::ClosureInputValueOrigin {
            argument: *index,
            tuple,
            local: place.local,
            value: tail,
            projection: projection.into(),
        };
        let mut local_ty = input.projected_type(parameters, budget)?;
        if let Some(kind) = borrow {
            budget
                .retain_bytes(std::mem::size_of::<ir::Ty>() + 2 * std::mem::size_of::<usize>())?;
            local_ty = ir::Ty::Ref {
                kind,
                referent: Arc::new(local_ty),
            };
        }
        if locals
            .get(place.local.0)
            .is_none_or(|local| local.ty != local_ty)
        {
            return Err("closure input result changed its checked pattern binding type".into());
        }
        let read = match borrow {
            None => place.projection.as_slice(),
            Some(ir::BorrowKind::Shared)
                if place.projection.first() == Some(&ir::Projection::Deref) =>
            {
                &place.projection[1..]
            }
            _ => {
                return Err(
                    "unsupported: closure result retains a parameter-pattern borrow".into(),
                );
            }
        };
        let mut combined = input.projection.to_vec();
        combined.extend_from_slice(read);
        input.projection = combined.into();
        if input.projected_type(parameters, budget)? != expression.ty {
            return Err("closure input result changed its original reference value type".into());
        }
        // Formation checks every output channel against the original input
        // carrier domain before a receipt can be minted.
        input.value_carriers(parameters, budget)?;
        result = Some(input);
    }
    Ok(result)
}
