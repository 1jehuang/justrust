//! Per-parent-body capture use classification over already checked IR.
//! Intermediate place projections are not value reads: only the selected leaf
//! determines whether a non-Copy value is consumed.
use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CaptureUse {
    pub read: bool,
    pub mutable: bool,
    pub moved: bool,
}
impl CaptureUse {
    pub(super) fn kind(self) -> ir::ClosureKind {
        if self.moved {
            ir::ClosureKind::FnOnce
        } else if self.mutable {
            ir::ClosureKind::FnMut
        } else {
            ir::ClosureKind::Fn
        }
    }
}

fn expression_root(
    exprs: &[ir::TypedExpr],
    mut id: ir::ExprId,
) -> Result<Option<ir::LocalId>, String> {
    for _ in 0..=ir::MAX_PATTERN_PROJECTION_DEPTH {
        match &exprs.get(id.0).ok_or("invalid capture expression")?.kind {
            ir::ExprKind::Local(local) => return Ok(Some(*local)),
            ir::ExprKind::Field { base, .. }
            | ir::ExprKind::Index { base, .. }
            | ir::ExprKind::DynamicIndex { base, .. } => id = *base,
            ir::ExprKind::Deref { value } => id = *value,
            _ => return Ok(None),
        }
    }
    Err("capture projection depth exceeds limit 128".into())
}

fn binding_use(
    pattern: &ir::MatchPattern,
    locals: &[ir::LocalDecl],
    facts: ir::FactsCtx<'_>,
    usage: &mut CaptureUse,
) -> Result<(), String> {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        match pattern {
            ir::MatchPattern::Or { rows, .. } => pending.extend(rows),
            ir::MatchPattern::Binding(local) => {
                usage.moved |= !facts.must_copy(
                    &locals
                        .get(local.0)
                        .ok_or("invalid capture pattern binding")?
                        .ty,
                )?;
            }
            ir::MatchPattern::Aggregate { bindings, .. }
            | ir::MatchPattern::EnumVariant { bindings, .. }
            | ir::MatchPattern::Decision { bindings, .. } => {
                for binding in bindings {
                    match binding.borrow {
                        Some(ir::BorrowKind::Mut) => usage.mutable = true,
                        Some(ir::BorrowKind::Shared) => {}
                        None => {
                            usage.moved |= !facts.must_copy(
                                &locals
                                    .get(binding.local.0)
                                    .ok_or("invalid capture pattern binding")?
                                    .ty,
                            )?
                        }
                    }
                }
            }
            ir::MatchPattern::Alternatives { .. }
            | ir::MatchPattern::Wildcard
            | ir::MatchPattern::Bool(_)
            | ir::MatchPattern::Int(_)
            | ir::MatchPattern::Unit => {}
        }
    }
    Ok(())
}

pub(super) fn analyze(
    exprs: &[ir::TypedExpr],
    body: &ir::Block,
    locals: &[ir::LocalDecl],
    candidates: &HashMap<ir::LocalId, usize>,
    facts: ir::FactsCtx<'_>,
) -> Result<Vec<CaptureUse>, String> {
    let mut uses = vec![CaptureUse::default(); candidates.len()];
    let mut projected = HashSet::new();
    let mut blocks = vec![body];
    for expression in exprs {
        match &expression.kind {
            ir::ExprKind::Field { base, .. }
            | ir::ExprKind::Index { base, .. }
            | ir::ExprKind::DynamicIndex { base, .. } => {
                projected.insert(*base);
            }
            ir::ExprKind::Deref { value } => {
                projected.insert(*value);
            }
            ir::ExprKind::Borrow { kind, place, .. } => {
                if let Some(index) = candidates.get(&place.local) {
                    let usage = uses.get_mut(*index).ok_or("invalid capture index")?;
                    usage.read = true;
                    usage.mutable |= *kind == ir::BorrowKind::Mut;
                }
            }
            ir::ExprKind::Match { scrutinee, arms } => {
                projected.insert(*scrutinee);
                if let Some(root) = expression_root(exprs, *scrutinee)?
                    && let Some(index) = candidates.get(&root)
                {
                    let usage = uses.get_mut(*index).ok_or("invalid capture index")?;
                    usage.read = true;
                    for arm in arms {
                        binding_use(&arm.pattern, locals, facts, usage)?;
                    }
                }
            }
            ir::ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                blocks.push(then_block);
                blocks.push(else_block);
            }
            ir::ExprKind::Block(block)
            | ir::ExprKind::LabeledBlock { body: block, .. }
            | ir::ExprKind::Loop { body: block, .. } => blocks.push(block),
            _ => {}
        }
    }
    while let Some(block) = blocks.pop() {
        for statement in &block.statements {
            let mutable = match statement {
                ir::Stmt::Assign { local, .. } => Some(*local),
                ir::Stmt::AssignPlace { place, .. } | ir::Stmt::AssignOp { place, .. } => {
                    Some(place.local)
                }
                ir::Stmt::AssignIndex { array, .. } | ir::Stmt::AssignIndexOp { array, .. } => {
                    Some(array.local)
                }
                ir::Stmt::While { body, .. } => {
                    blocks.push(body);
                    None
                }
                ir::Stmt::LetPattern {
                    value, bindings, ..
                } => {
                    projected.insert(*value);
                    if let Some(root) = expression_root(exprs, *value)?
                        && let Some(index) = candidates.get(&root)
                    {
                        let usage = uses.get_mut(*index).ok_or("invalid capture index")?;
                        usage.read = true;
                        for binding in bindings {
                            match binding.borrow {
                                Some(ir::BorrowKind::Mut) => usage.mutable = true,
                                Some(ir::BorrowKind::Shared) => {}
                                None => {
                                    usage.moved |= !facts.must_copy(
                                        &locals
                                            .get(binding.local.0)
                                            .ok_or("invalid capture binding")?
                                            .ty,
                                    )?
                                }
                            }
                        }
                    }
                    None
                }
                ir::Stmt::LetElse { value, pattern, .. } => {
                    projected.insert(*value);
                    if let Some(root) = expression_root(exprs, *value)?
                        && let Some(index) = candidates.get(&root)
                    {
                        let usage = uses.get_mut(*index).ok_or("invalid capture index")?;
                        usage.read = true;
                        binding_use(pattern, locals, facts, usage)?;
                    }
                    None
                }
                ir::Stmt::Declare { .. }
                | ir::Stmt::Let { .. }
                | ir::Stmt::Expr(_)
                | ir::Stmt::Return(_) => None,
            };
            if let Some(local) = mutable
                && let Some(index) = candidates.get(&local)
            {
                let usage = uses.get_mut(*index).ok_or("invalid capture index")?;
                usage.read = true;
                usage.mutable = true;
            }
        }
    }
    for (index, expression) in exprs.iter().enumerate() {
        if projected.contains(&ir::ExprId(index)) {
            continue;
        }
        if let Some(root) = expression_root(exprs, ir::ExprId(index))?
            && let Some(index) = candidates.get(&root)
        {
            let usage = uses.get_mut(*index).ok_or("invalid capture index")?;
            usage.read = true;
            usage.moved |= !facts.must_copy(&expression.ty)?;
        }
    }
    Ok(uses)
}
