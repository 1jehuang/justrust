//! Replace capture-check scratch locals with ordinary environment places.
//! No capture value remains in a side slot or in an abstract generic receiver.
use super::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(super) struct CaptureProjection {
    pub local: ir::LocalId,
    pub field: usize,
    pub prefix: Vec<ir::Projection>,
    /// A shared/mutable reference capture has one extra field dereference.
    pub indirect: bool,
}

fn captured_place(
    place: &mut ir::Place,
    receiver: ir::LocalId,
    borrowed_receiver: bool,
    captures: &[CaptureProjection],
) -> Result<(), String> {
    let Some(capture) = captures.iter().find(|capture| {
        capture.local == place.local && place.projection.starts_with(&capture.prefix)
    }) else {
        return Ok(());
    };
    let additional = usize::from(borrowed_receiver) + 1 + usize::from(capture.indirect);
    if place.projection.len().saturating_add(additional) > ir::MAX_PATTERN_PROJECTION_DEPTH {
        return Err("closure capture projection depth exceeds limit 128".into());
    }
    let mut projection = Vec::with_capacity(place.projection.len() + additional);
    if borrowed_receiver {
        projection.push(ir::Projection::Deref);
    }
    projection.push(ir::Projection::Field(capture.field));
    if capture.indirect {
        projection.push(ir::Projection::Deref);
    }
    projection.extend_from_slice(&place.projection[capture.prefix.len()..]);
    place.local = receiver;
    place.projection = projection;
    Ok(())
}

fn rewrite_block(
    block: &mut ir::Block,
    receiver: ir::LocalId,
    borrowed_receiver: bool,
    captures: &[CaptureProjection],
) -> Result<(), String> {
    let mut pending = vec![block];
    while let Some(block) = pending.pop() {
        for statement in &mut block.statements {
            if let ir::Stmt::Assign { local, value } = statement
                && captures
                    .iter()
                    .any(|capture| capture.local == *local && capture.prefix.is_empty())
            {
                let mut place = ir::Place {
                    local: *local,
                    projection: Vec::new(),
                };
                captured_place(&mut place, receiver, borrowed_receiver, captures)?;
                *statement = ir::Stmt::AssignPlace {
                    place,
                    value: *value,
                };
            }
            match statement {
                ir::Stmt::AssignPlace { place, .. } | ir::Stmt::AssignOp { place, .. } => {
                    captured_place(place, receiver, borrowed_receiver, captures)?;
                }
                ir::Stmt::AssignIndex { array, .. } | ir::Stmt::AssignIndexOp { array, .. } => {
                    captured_place(array, receiver, borrowed_receiver, captures)?;
                }
                ir::Stmt::While { body, .. } => pending.push(body),
                ir::Stmt::Assign { .. }
                | ir::Stmt::Declare { .. }
                | ir::Stmt::Let { .. }
                | ir::Stmt::LetPattern { .. }
                | ir::Stmt::LetElse { .. }
                | ir::Stmt::Expr(_)
                | ir::Stmt::Return(_) => {}
            }
        }
    }
    Ok(())
}

/// `environment` is a checked concrete ClosureTy, never the adapter's rigid S.
/// Source expression ids remain stable so call evidence and source locations
/// keep referring to their original expressions. New environment-place nodes
/// contain no calls and cannot manufacture dispatch or borrow authority.
pub(super) fn rewrite(
    exprs: &mut Vec<ir::TypedExpr>,
    body: &mut ir::Block,
    receiver: ir::LocalId,
    receiver_ty: ir::Ty,
    environment: Arc<ir::ClosureTy>,
    captures: &[CaptureProjection],
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let borrowed_receiver = matches!(receiver_ty, ir::Ty::Ref { .. });
    let original_len = exprs.len();
    let count = captures
        .len()
        .checked_add(2)
        .ok_or("closure capture expression count overflow")?;
    budget.charge_work(original_len.saturating_add(count))?;
    budget.charge_nodes(count)?;
    exprs
        .try_reserve(count)
        .map_err(|_| "closure capture expression allocation failed")?;
    let receiver_expr = ir::ExprId(exprs.len());
    exprs.push(ir::TypedExpr {
        ty: receiver_ty,
        completion: ir::Completion::Returns,
        kind: ir::ExprKind::Local(receiver),
    });
    let environment_expr = if borrowed_receiver {
        let id = ir::ExprId(exprs.len());
        exprs.push(ir::TypedExpr {
            ty: ir::Ty::Closure(environment.clone()),
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::Deref {
                value: receiver_expr,
            },
        });
        id
    } else {
        receiver_expr
    };
    let mut indirect = HashMap::new();
    let mut ordered: Vec<_> = captures.iter().collect();
    ordered.sort_unstable_by_key(|capture| capture.field);
    for capture in ordered {
        if capture.indirect {
            let ty = environment
                .captures()
                .get(capture.field)
                .ok_or("capture field index outside final environment")?
                .clone();
            let id = ir::ExprId(exprs.len());
            exprs.push(ir::TypedExpr {
                ty,
                completion: ir::Completion::Returns,
                kind: ir::ExprKind::Field {
                    base: environment_expr,
                    index: capture.field,
                },
            });
            indirect.insert(capture.field, id);
        }
    }
    let places = (0..original_len)
        .map(|index| capture_use::expression_place(exprs, ir::ExprId(index)))
        .collect::<Result<Vec<_>, _>>()?;
    for (index, expression) in exprs[..original_len].iter_mut().enumerate() {
        if let Some(place) = &places[index]
            && let Some(capture) = captures
                .iter()
                .find(|capture| capture.local == place.local && capture.prefix == place.projection)
        {
            expression.kind = if capture.indirect {
                ir::ExprKind::Deref {
                    value: *indirect
                        .get(&capture.field)
                        .ok_or("missing indirect capture field")?,
                }
            } else {
                ir::ExprKind::Field {
                    base: environment_expr,
                    index: capture.field,
                }
            };
        }
        match &mut expression.kind {
            ir::ExprKind::Borrow { place, .. } => {
                captured_place(place, receiver, borrowed_receiver, captures)?
            }
            ir::ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                rewrite_block(then_block, receiver, borrowed_receiver, captures)?;
                rewrite_block(else_block, receiver, borrowed_receiver, captures)?;
            }
            ir::ExprKind::Block(block)
            | ir::ExprKind::LabeledBlock { body: block, .. }
            | ir::ExprKind::Loop { body: block, .. } => {
                rewrite_block(block, receiver, borrowed_receiver, captures)?
            }
            _ => {}
        }
    }
    rewrite_block(body, receiver, borrowed_receiver, captures)
}
