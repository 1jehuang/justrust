//! Per-parent-body capture use classification over checked places.
//! Capture identity retains disjoint field paths and leading language-reference
//! dereferences. Owning/raw pointers and runtime indices retain their carrier.
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
    fn merge(&mut self, other: Self) {
        self.read |= other.read;
        self.mutable |= other.mutable;
        self.moved |= other.moved;
    }
}
#[derive(Clone, Debug)]
pub(super) struct Capture {
    pub place: ir::Place,
    pub usage: CaptureUse,
}

pub(super) fn expression_place(
    exprs: &[ir::TypedExpr],
    mut id: ir::ExprId,
) -> Result<Option<ir::Place>, String> {
    let mut projection = Vec::new();
    for _ in 0..=ir::MAX_PATTERN_PROJECTION_DEPTH {
        let expression = exprs.get(id.0).ok_or("invalid capture expression")?;
        match &expression.kind {
            ir::ExprKind::Local(local) => {
                projection.reverse();
                return Ok(Some(ir::Place {
                    local: *local,
                    projection,
                }));
            }
            ir::ExprKind::Field { base, index } => {
                projection.push(ir::Projection::Field(*index));
                id = *base;
            }
            ir::ExprKind::Index { base, index } => {
                projection.push(ir::Projection::ConstantIndex(*index));
                id = *base;
            }
            ir::ExprKind::DynamicIndex { .. } => return Ok(None),
            ir::ExprKind::Deref { value } => {
                projection.push(ir::Projection::Deref);
                id = *value;
            }
            _ => return Ok(None),
        }
    }
    Err("capture projection depth exceeds limit 128".into())
}

fn record(
    uses: &mut [Vec<Capture>],
    candidates: &HashMap<ir::LocalId, usize>,
    locals: &[ir::LocalDecl],
    mut place: ir::Place,
    usage: CaptureUse,
) -> Result<(), String> {
    let Some(index) = candidates.get(&place.local) else {
        return Ok(());
    };
    // A leading language reference dereference names the borrowed pointee,
    // not the reference binding. Retain it so mutation captures a reborrow of
    // the pointee even when the reference binding itself is immutable.
    // Raw/owning pointers and indices still require the original carrier.
    let mut referent = &locals.get(place.local.0).ok_or("invalid capture local")?.ty;
    let mut precise = 0;
    for step in &place.projection {
        let (ir::Projection::Deref, ir::Ty::Ref { referent: next, .. }) = (step, referent) else {
            break;
        };
        precise += 1;
        referent = next;
    }
    precise += place.projection[precise..]
        .iter()
        .take_while(|p| matches!(p, ir::Projection::Field(_)))
        .count();
    place.projection.truncate(precise);
    let captures = uses.get_mut(*index).ok_or("invalid capture index")?;
    if let Some(existing) = captures
        .iter_mut()
        .find(|c| place.projection.starts_with(&c.place.projection))
    {
        existing.usage.merge(usage);
        return Ok(());
    }
    let mut combined = usage;
    captures.retain(|existing| {
        if existing.place.projection.starts_with(&place.projection) {
            combined.merge(existing.usage);
            false
        } else {
            true
        }
    });
    captures.push(Capture {
        place,
        usage: combined,
    });
    Ok(())
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
                )?
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
            _ => {}
        }
    }
    Ok(())
}

/// Editions 2015/2018 capture whole variables (no RFC 2229 disjoint field
/// capture): collapse every candidate's captures to its root place.
pub(super) fn whole_variable(uses: &mut [Vec<Capture>]) {
    for captures in uses.iter_mut() {
        let Some(first) = captures.first() else {
            continue;
        };
        let local = first.place.local;
        let mut usage = CaptureUse::default();
        for capture in captures.iter() {
            usage.merge(capture.usage);
        }
        captures.clear();
        captures.push(Capture {
            place: ir::Place {
                local,
                projection: Vec::new(),
            },
            usage,
        });
    }
}

pub(super) fn analyze(
    exprs: &[ir::TypedExpr],
    body: &ir::TypedBlock,
    locals: &[ir::LocalDecl],
    candidates: &HashMap<ir::LocalId, usize>,
    facts: ir::FactsCtx<'_>,
) -> Result<Vec<Vec<Capture>>, String> {
    let mut uses = vec![Vec::new(); candidates.len()];
    let mut projected = HashSet::new();
    let mut blocks = vec![body];
    for expression in exprs {
        match &expression.kind {
            ir::ExprKind::Field { base, .. } | ir::ExprKind::Index { base, .. } => {
                projected.insert(*base);
            }
            ir::ExprKind::DynamicIndex { base, .. } => {
                projected.insert(*base);
                if let Some(place) = expression_place(exprs, *base)? {
                    record(
                        &mut uses,
                        candidates,
                        locals,
                        place,
                        CaptureUse {
                            read: true,
                            mutable: false,
                            moved: !facts.must_copy(&expression.ty)?,
                        },
                    )?;
                }
            }
            ir::ExprKind::Deref { value } => {
                projected.insert(*value);
            }
            ir::ExprKind::BoundsCheck { array, .. } => record(
                &mut uses,
                candidates,
                locals,
                array.clone(),
                CaptureUse {
                    read: true,
                    mutable: false,
                    moved: false,
                },
            )?,
            ir::ExprKind::Borrow { kind, place, .. } => record(
                &mut uses,
                candidates,
                locals,
                place.clone(),
                CaptureUse {
                    read: true,
                    mutable: *kind == ir::BorrowKind::Mut,
                    moved: false,
                },
            )?,
            ir::ExprKind::Match {
                scrutinee, arms, ..
            } => {
                projected.insert(*scrutinee);
                if let Some(place) = expression_place(exprs, *scrutinee)? {
                    let mut usage = CaptureUse {
                        read: true,
                        ..CaptureUse::default()
                    };
                    for arm in arms {
                        binding_use(&arm.pattern, locals, facts, &mut usage)?;
                    }
                    record(&mut uses, candidates, locals, place, usage)?;
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
            let place = match statement {
                ir::TypedStmt::Assign { local, .. } => Some(ir::Place {
                    local: *local,
                    projection: Vec::new(),
                }),
                ir::TypedStmt::AssignPlace { place, .. }
                | ir::TypedStmt::AssignOp { place, .. } => Some(place.clone()),
                ir::TypedStmt::AssignIndex { array, .. }
                | ir::TypedStmt::AssignIndexOp { array, .. } => Some(array.clone()),
                ir::TypedStmt::While { body, .. } => {
                    blocks.push(body);
                    None
                }
                ir::TypedStmt::LetPattern {
                    value, bindings, ..
                } => {
                    projected.insert(*value);
                    if let Some(place) = expression_place(exprs, *value)? {
                        let mut usage = CaptureUse {
                            read: true,
                            ..CaptureUse::default()
                        };
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
                        record(&mut uses, candidates, locals, place, usage)?;
                    }
                    None
                }
                ir::TypedStmt::LetElse { value, pattern, .. } => {
                    projected.insert(*value);
                    if let Some(place) = expression_place(exprs, *value)? {
                        let mut usage = CaptureUse {
                            read: true,
                            ..CaptureUse::default()
                        };
                        binding_use(pattern, locals, facts, &mut usage)?;
                        record(&mut uses, candidates, locals, place, usage)?;
                    }
                    None
                }
                _ => None,
            };
            if let Some(place) = place {
                record(
                    &mut uses,
                    candidates,
                    locals,
                    place,
                    CaptureUse {
                        read: true,
                        mutable: true,
                        moved: false,
                    },
                )?;
            }
        }
    }
    for (index, expression) in exprs.iter().enumerate() {
        if projected.contains(&ir::ExprId(index)) {
            continue;
        }
        if let Some(place) = expression_place(exprs, ir::ExprId(index))? {
            record(
                &mut uses,
                candidates,
                locals,
                place,
                CaptureUse {
                    read: true,
                    mutable: false,
                    moved: !facts.must_copy(&expression.ty)?,
                },
            )?;
        }
    }
    for captures in &mut uses {
        captures.sort_by(|a, b| a.place.projection.cmp(&b.place.projection));
    }
    Ok(uses)
}
