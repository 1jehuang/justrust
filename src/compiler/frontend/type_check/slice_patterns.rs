//! Slice patterns over `&[T]` / `&mut [T]` scrutinees, including subslice
//! bindings `rest @ ..`.
//!
//! A match with a slice pattern arm lowers to one checker-made labeled block:
//! the scrutinee and its length are read once, each arm tests the length and
//! then matches its written element patterns against `&s[i]` (so the default
//! binding mode binds references exactly as stock does), and a selected arm
//! leaves the block with its body value. No source AST is synthesized apart
//! from the plain binding that names a subslice.
use super::*;

pub(super) fn is_slice_pattern(pattern: &Pattern) -> bool {
    matches!(pattern, Pattern::Array(_) | Pattern::ArrayRest(_))
}

fn has_subslice_binding(pattern: &Pattern) -> bool {
    matches!(pattern, Pattern::Array(fields) if fields.iter().any(is_subslice_binding))
}

/// Slice references always take this path. References to arrays take it only
/// for subslice bindings: other array patterns keep exact place projections.
pub(super) fn applies<'p>(ty: &ir::Ty, mut patterns: impl Iterator<Item = &'p Pattern>) -> bool {
    if ty.slice_ref_element().is_some() {
        patterns.any(is_slice_pattern)
    } else {
        ty.array_ref_element_len().is_some() && patterns.any(has_subslice_binding)
    }
}

#[derive(Clone, Copy)]
enum StepKind {
    /// The whole scrutinee, for a catch-all binding arm.
    Whole,
    Front(usize),
    /// `len - n`.
    Back(usize),
    /// `s[front..len - back]`.
    Rest {
        front: usize,
        back: usize,
    },
}
#[derive(Clone, Copy)]
pub(super) struct SliceStep<'a> {
    kind: StepKind,
    pattern: &'a Pattern,
    /// A plain binding: cannot fail, so no fallback arm is generated.
    simple: bool,
}
#[derive(Clone, Copy)]
pub(super) struct SliceChain<'a> {
    target: ir::LoopId,
    slice: ir::LocalId,
    len: ir::LocalId,
    kind: ir::BorrowKind,
    steps: &'a [SliceStep<'a>],
    guard: Option<&'a Expr>,
    body: &'a MatchBody<'a>,
    expectation: Expectation<'a>,
}

struct ArmShape {
    /// Written elements, excluding the rest.
    front: usize,
    back: usize,
    /// `..` or `name @ ..` was written.
    variable: bool,
    rest: Option<Pattern>,
}

fn arm_shape(pattern: &Pattern) -> Result<Option<ArmShape>, String> {
    Ok(Some(match pattern {
        Pattern::ArrayRest(fields) => ArmShape {
            front: fields.prefix.len(),
            back: fields.suffix.len(),
            variable: true,
            rest: None,
        },
        Pattern::Array(fields) => match fields.iter().position(is_subslice_binding) {
            None => ArmShape {
                front: fields.len(),
                back: 0,
                variable: false,
                rest: None,
            },
            Some(at) => {
                let Pattern::At {
                    name,
                    mutable,
                    by_ref,
                    ..
                } = &fields[at]
                else {
                    return Err("contract: subslice binding shape".into());
                };
                if *mutable || by_ref.is_some() {
                    return Err("binding modifiers may only be written when the default binding mode is `move`".into());
                }
                ArmShape {
                    front: at,
                    back: fields.len() - at - 1,
                    variable: true,
                    rest: Some(Pattern::Binding {
                        name: *name,
                        mutable: false,
                        by_ref: None,
                    }),
                }
            }
        },
        Pattern::Wildcard | Pattern::Binding { .. } => return Ok(None),
        Pattern::Alternatives(_) => {
            return Err("unsupported: or-patterns of slice patterns are unsupported".into());
        }
        _ => {
            return Err(
                "unsupported: this pattern is unsupported in a match with slice patterns".into(),
            );
        }
    }))
}

fn elements(pattern: &Pattern) -> (&[Pattern], &[Pattern]) {
    match pattern {
        Pattern::ArrayRest(fields) => (&fields.prefix, &fields.suffix),
        Pattern::Array(fields) => match fields.iter().position(is_subslice_binding) {
            None => (fields, &[]),
            Some(at) => (&fields[..at], &fields[at + 1..]),
        },
        _ => (&[], &[]),
    }
}

/// Whether matching `pattern` introduces any binding.
fn binds(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Binding { .. } | Pattern::At { .. } => true,
        Pattern::Apply { pattern, .. } | Pattern::Ref { pattern, .. } => binds(pattern),
        Pattern::Tuple(fields) | Pattern::Array(fields) | Pattern::Alternatives(fields) => {
            fields.iter().any(binds)
        }
        Pattern::TupleRest(fields)
        | Pattern::ArrayRest(fields)
        | Pattern::VariantRest { fields, .. } => {
            fields.prefix.iter().chain(&fields.suffix).any(binds)
        }
        Pattern::Variant { fields, .. } => fields.iter().any(|(_, field)| binds(field)),
        Pattern::Struct { fields, .. } => fields.iter().any(|(_, field)| binds(field)),
        _ => false,
    }
}

/// Elements are matched through `&T`, so stock's default binding mode is a
/// borrow: explicit modifiers and reference patterns are errors (2024).
fn element_step<'a>(kind: StepKind, pattern: &'a Pattern) -> Result<Option<SliceStep<'a>>, String> {
    match pattern {
        Pattern::Wildcard => Ok(None),
        Pattern::Binding {
            mutable, by_ref, ..
        }
        | Pattern::At {
            mutable, by_ref, ..
        } if *mutable || by_ref.is_some() => Err(
            "binding modifiers may only be written when the default binding mode is `move`".into(),
        ),
        Pattern::Ref { .. } => Err(
            "reference patterns may only be written when the default binding mode is `move`".into(),
        ),
        Pattern::Binding { .. } => Ok(Some(SliceStep {
            kind,
            pattern,
            simple: true,
        })),
        _ => Ok(Some(SliceStep {
            kind,
            pattern,
            simple: false,
        })),
    }
}

impl Checker<'_> {
    fn slice_usize(&mut self, value: usize) -> Result<ir::ExprId, String> {
        self.for_expr(
            ir::Ty::Int(ir::IntTy::Usize),
            ir::ExprKind::Int(ir::IntValue::usize(value as u64)),
        )
    }
    fn slice_len_minus(&mut self, len: ir::LocalId, back: usize) -> Result<ir::ExprId, String> {
        let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
        let left = self.for_expr(usize_ty.clone(), ir::ExprKind::Local(len))?;
        if back == 0 {
            return Ok(left);
        }
        let right = self.slice_usize(back)?;
        self.for_expr(
            usize_ty,
            ir::ExprKind::Binary {
                op: BinaryOp::Sub,
                left,
                right,
            },
        )
    }
    /// The scrutinee carrier itself: a copy of `&[T]`, a reborrow of `&mut [T]`.
    fn slice_carrier(
        &mut self,
        slice: ir::LocalId,
        kind: ir::BorrowKind,
    ) -> Result<ir::ExprId, String> {
        if kind == ir::BorrowKind::Shared
            && self.locals[slice.0].ty.shared_slice_element().is_some()
        {
            let ty = self.locals[slice.0].ty.clone();
            return self.for_expr(ty, ir::ExprKind::Local(slice));
        }
        self.borrow_expression(
            kind,
            ir::Place {
                local: slice,
                projection: vec![ir::Projection::Deref],
            },
        )
    }
    fn slice_wrap(&mut self, kind: ir::ExprKind, flow: &Flow) -> Result<ir::ExprId, String> {
        let id = self.for_expr(flow.ty.clone(), kind)?;
        if !flow.completes {
            self.exprs[id.0].completion = ir::Completion::Never;
        }
        Ok(id)
    }

    pub(super) fn slice_chain_body(
        &mut self,
        chain: &SliceChain<'_>,
        reachable: bool,
    ) -> Result<ir::ExprId, String> {
        let previous = self.reachable;
        self.reachable = reachable;
        let result = self.slice_chain_step(chain, reachable);
        self.reachable = previous;
        result
    }

    fn slice_chain_step(
        &mut self,
        chain: &SliceChain<'_>,
        reachable: bool,
    ) -> Result<ir::ExprId, String> {
        let Some((step, rest)) = chain.steps.split_first() else {
            return self.slice_chain_end(chain, reachable);
        };
        let next = SliceChain {
            steps: rest,
            ..*chain
        };
        let mut statements = Vec::new();
        // A test that binds nothing only reads, even through `&mut [T]`.
        let chain = &SliceChain {
            kind: if binds(step.pattern) {
                chain.kind
            } else {
                ir::BorrowKind::Shared
            },
            ..*chain
        };
        let scrutinee = match step.kind {
            StepKind::Whole => self.slice_carrier(chain.slice, chain.kind)?,
            StepKind::Front(_) | StepKind::Back(_) => {
                let value = match step.kind {
                    StepKind::Front(at) => self.slice_usize(at)?,
                    StepKind::Back(back) => self.slice_len_minus(chain.len, back)?,
                    _ => unreachable!(),
                };
                let index =
                    self.temporary("$slice_pattern_index", ir::Ty::Int(ir::IntTy::Usize), false)?;
                self.charge_node(std::mem::size_of::<ir::TypedStmt>())?;
                statements.push(ir::TypedStmt::Let {
                    local: index,
                    value,
                });
                self.borrow_expression(
                    chain.kind,
                    ir::Place {
                        local: chain.slice,
                        projection: vec![
                            ir::Projection::Deref,
                            ir::Projection::DynamicIndex(index),
                        ],
                    },
                )?
            }
            StepKind::Rest { front, back } => {
                let slice = self.slice_carrier(chain.slice, chain.kind)?;
                let start = self.slice_usize(front)?;
                let end = self.slice_len_minus(chain.len, back)?;
                let ty = self.exprs[slice.0].ty.clone();
                self.for_expr(
                    ty,
                    ir::ExprKind::Subslice {
                        slice,
                        start: Some(start),
                        end: Some(end),
                        inclusive: false,
                    },
                )?
            }
        };
        let arms = [
            BorrowedMatchArm {
                pattern: Some(step.pattern),
                guarded: false,
                guard: None,
                body: MatchBody::Slice(&next),
            },
            BorrowedMatchArm {
                pattern: None,
                guarded: false,
                guard: None,
                body: MatchBody::ImplicitUnit,
            },
        ];
        let count = if step.simple { 1 } else { 2 };
        let (kind, flow) = self.lower_match_value(
            scrutinee,
            arms[..count].iter().copied(),
            Some(&ir::Ty::Unit),
        )?;
        let matched = self.slice_wrap(kind, &flow)?;
        if statements.is_empty() {
            return Ok(matched);
        }
        self.slice_wrap(
            ir::ExprKind::Block(ir::TypedBlock {
                statements,
                tail: Some(matched),
            }),
            &flow,
        )
    }

    /// Every test passed and every binding is in scope: evaluate the guard,
    /// then leave the match with the body value.
    fn slice_chain_end(
        &mut self,
        chain: &SliceChain<'_>,
        reachable: bool,
    ) -> Result<ir::ExprId, String> {
        let Some(guard) = chain.guard else {
            return self.slice_chain_break(chain, reachable);
        };
        let condition = self.expr_expected(guard, Some(&ir::Ty::Bool))?;
        let condition_flow = self.flow(condition);
        self.require(&condition_flow, &ir::Ty::Bool)?;
        let selected = self.slice_chain_break(chain, reachable && condition_flow.completes)?;
        self.slice_wrap(
            ir::ExprKind::If {
                condition,
                then_block: ir::TypedBlock {
                    statements: Vec::new(),
                    tail: Some(selected),
                },
                else_block: ir::TypedBlock {
                    statements: Vec::new(),
                    tail: None,
                },
            },
            &Flow {
                ty: ir::Ty::Unit,
                completes: condition_flow.completes,
            },
        )
    }

    fn slice_chain_break(
        &mut self,
        chain: &SliceChain<'_>,
        reachable: bool,
    ) -> Result<ir::ExprId, String> {
        let value =
            self.match_arm_body_with_expectation(*chain.body, reachable, chain.expectation)?;
        let flow = self.flow(value);
        if !flow.completes && flow.ty == ir::Ty::Never {
            return Ok(value);
        }
        let index = self
            .loops
            .iter()
            .rposition(|frame| frame.id == chain.target)
            .ok_or("contract: missing slice match target")?;
        if reachable && flow.completes {
            self.loops[index].can_exit = true;
        }
        if flow.ty != ir::Ty::Never {
            let previous = self.loops[index].break_ty.clone().unwrap_or(ir::Ty::Never);
            let ty = self.join(&previous, &flow.ty, "match arms")?;
            self.loops[index].break_ty = Some(ty);
        }
        self.slice_wrap(
            ir::ExprKind::Break {
                target: chain.target,
                value: Some(value),
            },
            &Flow::never(),
        )
    }

    pub(super) fn lower_slice_match(
        &mut self,
        scrutinee: ir::ExprId,
        arms: &[BorrowedMatchArm<'_>],
        expectation: Expectation<'_>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        let loop_depth = self.loops.len();
        let outer_reachable = self.reachable;
        let result = self.lower_slice_match_inner(scrutinee, arms, expectation);
        self.loops.truncate(loop_depth);
        self.reachable = outer_reachable;
        result
    }

    fn lower_slice_match_inner(
        &mut self,
        mut scrutinee: ir::ExprId,
        arms: &[BorrowedMatchArm<'_>],
        expectation: Expectation<'_>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        let mut source = self.flow(scrutinee);
        // A reference to an array is matched through its slice view; the
        // statically known length still decides which arms can apply.
        let mut known_len = None;
        if let Some((kind, element, len)) = source.ty.array_ref_element_len() {
            known_len = Some(len);
            let target = ir::Ty::Ref {
                kind,
                referent: Arc::new(ir::Ty::Slice {
                    element: Arc::new(element.clone()),
                }),
            };
            scrutinee = self.coerce_shared_array_to_slice(scrutinee, &target)?;
            source = self.flow(scrutinee);
        }
        let (kind, _) = source
            .ty
            .slice_ref_element()
            .ok_or("contract: slice pattern scrutinee")?;
        // Matching a `&mut [T]` place inspects it in place: the carrier is
        // reborrowed for the match, never moved out of its local.
        if kind == ir::BorrowKind::Mut
            && let ir::ExprKind::Local(local) = self.exprs[scrutinee.0].kind
        {
            scrutinee = self.borrow_expression(
                kind,
                ir::Place {
                    local,
                    projection: vec![ir::Projection::Deref],
                },
            )?;
        }
        let mut shapes = Vec::with_capacity(arms.len());
        for arm in arms {
            if arm.guarded {
                return Err("unsupported: matches! guards with slice patterns".into());
            }
            shapes.push(arm.pattern.map(arm_shape).transpose()?.flatten());
        }
        let usize_ty = ir::Ty::Int(ir::IntTy::Usize);
        let target = self.enter_loop(true);
        let frame = self.loops.last_mut().unwrap();
        frame.is_block = true;
        frame.synthetic = true;
        let slice = self.temporary(ir::SLICE_PATTERN_SCRUTINEE, source.ty.clone(), false)?;
        let len = self.temporary("$slice_pattern_len", usize_ty.clone(), false)?;
        let view = self.slice_carrier(slice, ir::BorrowKind::Shared)?;
        let length = self.for_expr(usize_ty.clone(), ir::ExprKind::SliceLen { slice: view })?;
        self.charge_node(std::mem::size_of::<ir::TypedStmt>() * (arms.len() + 2))?;
        let mut statements = vec![
            ir::TypedStmt::Let {
                local: slice,
                value: scrutinee,
            },
            ir::TypedStmt::Let {
                local: len,
                value: length,
            },
        ];
        // Lengths matched unconditionally so far: exact ones and `>= min`.
        let mut exact = std::collections::HashSet::new();
        let mut at_least: Option<usize> = None;
        let mut exhausted = false;
        let mut refutable = false;
        let entered = self.reachable && source.completes;
        for (arm, shape) in arms.iter().zip(&shapes) {
            let mut steps = Vec::new();
            let mut total = arm.guard.is_none();
            let mut condition = None;
            match shape {
                None => {
                    if let Some(pattern @ Pattern::Binding { .. }) = arm.pattern {
                        steps.push(SliceStep {
                            kind: StepKind::Whole,
                            pattern,
                            simple: true,
                        });
                    }
                    exhausted |= total;
                }
                Some(shape) => {
                    let pattern = arm.pattern.ok_or("contract: slice arm pattern")?;
                    let (front, back) = elements(pattern);
                    self.signatures
                        .plan()?
                        .budget
                        .charge_work(front.len() + back.len() + 1)?;
                    for (index, field) in front.iter().enumerate() {
                        steps.extend(element_step(StepKind::Front(index), field)?);
                    }
                    if let Some(rest) = &shape.rest {
                        steps.push(SliceStep {
                            kind: StepKind::Rest {
                                front: shape.front,
                                back: shape.back,
                            },
                            pattern: rest,
                            simple: true,
                        });
                    }
                    for (index, field) in back.iter().enumerate() {
                        steps.extend(element_step(StepKind::Back(back.len() - index), field)?);
                    }
                    total &= steps.iter().all(|step| step.simple);
                    refutable |= steps.iter().any(|step| !step.simple);
                    // Element places of one `&mut [T]` are disjoint, but the
                    // loans here are runtime-indexed and cannot prove it.
                    if kind == ir::BorrowKind::Mut
                        && steps.iter().filter(|step| binds(step.pattern)).count() > 1
                    {
                        return Err("unsupported: a `&mut` slice pattern with more than one binding element".into());
                    }
                    let min = shape.front + shape.back;
                    if let Some(known) = known_len {
                        if (shape.variable && min > known) || (!shape.variable && min != known) {
                            return Err(format!(
                                "E0527: pattern requires {min} elements but array has {known}"
                            ));
                        }
                        exhausted |= total;
                    } else {
                        if total {
                            if shape.variable {
                                at_least = Some(at_least.map_or(min, |old| old.min(min)));
                            } else {
                                exact.insert(min);
                            }
                            exhausted |=
                                at_least.is_some_and(|min| (0..min).all(|n| exact.contains(&n)));
                        }
                        // The arm that completes coverage is selected by
                        // elimination: every other length left earlier.
                        if !(total && exhausted) && !(shape.variable && min == 0) {
                            let left = self.for_expr(usize_ty.clone(), ir::ExprKind::Local(len))?;
                            let right = self.slice_usize(min)?;
                            condition = Some(self.for_expr(
                                ir::Ty::Bool,
                                ir::ExprKind::Binary {
                                    op: if shape.variable {
                                        BinaryOp::Ge
                                    } else {
                                        BinaryOp::Eq
                                    },
                                    left,
                                    right,
                                },
                            )?);
                        }
                    }
                }
            }
            let chain = SliceChain {
                target,
                slice,
                len,
                kind,
                steps: &steps,
                guard: arm.guard,
                body: &arm.body,
                expectation,
            };
            let reachable = entered
                && statements.iter().all(|statement| match statement {
                    ir::TypedStmt::Expr(id) => {
                        self.exprs[id.0].completion == ir::Completion::Returns
                    }
                    _ => true,
                });
            let selected = self.slice_chain_body(&chain, reachable)?;
            let statement = match condition {
                None => selected,
                Some(condition) => self.slice_wrap(
                    ir::ExprKind::If {
                        condition,
                        then_block: ir::TypedBlock {
                            statements: Vec::new(),
                            tail: Some(selected),
                        },
                        else_block: ir::TypedBlock {
                            statements: Vec::new(),
                            tail: None,
                        },
                    },
                    &Flow::value(ir::Ty::Unit),
                )?,
            };
            statements.push(ir::TypedStmt::Expr(statement));
        }
        if !exhausted {
            // Length coverage is exact. Coverage that depends on element
            // tests is not analysed, and must not be reported as invalid.
            return Err(if refutable {
                "unsupported: slice pattern exhaustiveness through refutable element patterns; add a catch-all arm".into()
            } else {
                format!("E0004: non-exhaustive match on {:?}", source.ty)
            });
        }
        let frame = self.loops.pop().ok_or("contract: slice match frame")?;
        Ok((
            ir::ExprKind::LabeledBlock {
                id: target,
                body: ir::TypedBlock {
                    statements,
                    tail: None,
                },
            },
            Flow {
                ty: frame.break_ty.unwrap_or(ir::Ty::Never),
                completes: source.completes && frame.can_exit,
            },
        ))
    }
}
