//! `deny(unconditional_recursion)` over one checked callable body.
//!
//! Port of the rule in rustc `rustc_mir_transform/src/check_call_recursion.rs`
//! (MIT OR Apache-2.0, Rust Project Contributors), evaluated on the typed
//! body instead of MIR: the lint fires when no path from entry reaches a
//! return, a loop back edge, a diverging non-recursive call or panic, or an
//! unreachable `match` remainder without first passing a recursive call,
//! and at least one recursive call is reachable. Edges after a recursive
//! call are ignored. Unwind edges of calls, drops and runtime checks with a
//! normal successor are ignored, as in stock.
//!
//! A few MIR shapes are not recoverable from the typed tree (whether a match
//! leaves an unreachable `otherwise` block, and whether a trait call on a
//! generic impl resolves back to this impl). Those are classified both ways;
//! when the verdict depends on them the check is explicitly unsupported.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Uncertain shapes never return to the caller.
    Strict,
    /// Uncertain shapes may return to the caller.
    Lenient,
}

#[derive(Default)]
struct Flow {
    /// Execution may continue after the expression without recursing.
    normal: bool,
    /// Some path reaches a non-recursive exit (return, back edge, unwind).
    escape: bool,
    /// Loops/labeled blocks broken out of without recursing.
    breaks: Vec<ir::LoopId>,
}

impl Flow {
    fn normal() -> Self {
        Flow {
            normal: true,
            ..Flow::default()
        }
    }
    fn escaped() -> Self {
        Flow {
            escape: true,
            ..Flow::default()
        }
    }
    /// `self` followed by `next` (only reached on normal completion).
    fn then(mut self, next: impl FnOnce() -> Result<Flow, String>) -> Result<Flow, String> {
        if !self.normal {
            return Ok(self);
        }
        let next = next()?;
        self.normal = next.normal;
        self.escape |= next.escape;
        self.breaks.extend(next.breaks);
        Ok(self)
    }
    /// Either `self` or `other`.
    fn or(mut self, other: Flow) -> Flow {
        self.normal |= other.normal;
        self.escape |= other.escape;
        self.breaks.extend(other.breaks);
        self
    }
}

pub(super) enum CallClass {
    Recursive,
    NotRecursive,
    Uncertain,
}

pub(super) struct Classifier {
    pub(super) declaration: ir::DeclId,
    /// Trait item implemented (or defaulted) by this callable.
    pub(super) trait_item: Option<ir::DeclId>,
    pub(super) self_ty: Option<ir::Ty>,
    /// The callable or its impl has generic parameters.
    pub(super) generic: bool,
}

impl Classifier {
    fn classify(&self, target: &ir::Callee) -> CallClass {
        match target {
            ir::Callee::Direct(call) if call.declaration == self.declaration => {
                CallClass::Recursive
            }
            ir::Callee::TraitItem(call) if Some(call.item) == self.trait_item => {
                let callee_self = &call.trait_ref.self_ty;
                if Some(callee_self) == self.self_ty.as_ref() {
                    CallClass::Recursive
                } else if matches!(callee_self, ir::Ty::Param(_) | ir::Ty::AssociatedType(_))
                    || !self.generic
                {
                    // A rigid parameter never resolves to this impl, and a
                    // different closed self type selects another impl.
                    CallClass::NotRecursive
                } else {
                    CallClass::Uncertain
                }
            }
            _ => CallClass::NotRecursive,
        }
    }
}

struct Walk<'a> {
    function: &'a ir::CheckedFunction,
    classifier: &'a Classifier,
    mode: Mode,
    recursive_calls: usize,
    budget: &'a ir::PlanBudget,
}

/// Err(message) when the denied lint fires or the verdict is undecidable.
pub(super) fn check(
    function: &ir::CheckedFunction,
    classifier: &Classifier,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let verdict = |mode| -> Result<bool, String> {
        let mut walk = Walk {
            function,
            classifier,
            mode,
            recursive_calls: 0,
            budget,
        };
        let flow = walk.block(&function.body)?;
        Ok(!flow.normal && !flow.escape && walk.recursive_calls > 0)
    };
    match (verdict(Mode::Strict)?, verdict(Mode::Lenient)?) {
        (false, false) => Ok(()),
        (true, true) => Err(format!(
            "function cannot return without recursing (`unconditional_recursion` is denied) in `{}`",
            function.name
        )),
        _ => Err(format!(
            "unsupported: `deny(unconditional_recursion)` cannot decide whether `{}` recurses unconditionally",
            function.name
        )),
    }
}

impl<'a> Walk<'a> {
    fn uncertain(&self) -> Flow {
        match self.mode {
            Mode::Strict => Flow::default(),
            Mode::Lenient => Flow::escaped(),
        }
    }
    fn expr_ref(&self, id: ir::ExprId) -> Result<&'a ir::TypedExpr, String> {
        self.budget.charge_work(1)?;
        let function: &'a ir::CheckedFunction = self.function;
        function
            .exprs
            .get(id.0)
            .ok_or_else(|| "invalid recursion-check expression".into())
    }
    fn seq(&mut self, ids: &[ir::ExprId]) -> Result<Flow, String> {
        let mut flow = Flow::normal();
        for id in ids {
            flow = flow.then(|| self.expr(*id))?;
            if !flow.normal {
                break;
            }
        }
        Ok(flow)
    }
    fn block(&mut self, block: &ir::TypedBlock) -> Result<Flow, String> {
        let mut flow = Flow::normal();
        for statement in &block.statements {
            if !flow.normal {
                return Ok(flow);
            }
            let next = self.statement(statement)?;
            flow = flow.then(|| Ok(next))?;
        }
        match block.tail {
            Some(tail) => flow.then(|| self.expr(tail)),
            None => Ok(flow),
        }
    }
    fn statement(&mut self, statement: &ir::TypedStmt) -> Result<Flow, String> {
        use ir::TypedStmt as S;
        Ok(match statement {
            S::Declare { .. } => Flow::normal(),
            S::Let { value, .. }
            | S::LetPattern { value, .. }
            | S::Assign { value, .. }
            | S::AssignPlace { value, .. }
            | S::AssignOp { value, .. }
            | S::Expr(value) => self.expr(*value)?,
            S::AssignIndex { index, value, .. } | S::AssignIndexOp { index, value, .. } => {
                self.seq(&[*index, *value])?
            }
            S::LetElse {
                value, else_branch, ..
            } => {
                let value = self.expr(*value)?;
                if !value.normal {
                    return Ok(value);
                }
                let otherwise = self.expr(*else_branch)?;
                value.or(otherwise)
            }
            S::Return(value) => {
                let flow = match value {
                    Some(value) => self.expr(*value)?,
                    None => Flow::normal(),
                };
                let reached = flow.normal;
                let mut flow = flow;
                flow.normal = false;
                flow.escape |= reached;
                flow
            }
            S::While {
                id,
                condition,
                body,
            } => {
                let condition = self.expr(*condition)?;
                if !condition.normal {
                    return Ok(condition);
                }
                let body = self.block(body)?;
                self.close_loop(*id, body).or(condition)
            }
        })
    }
    /// Normal loop-body completion is a back edge; breaks continue after it.
    fn close_loop(&self, id: ir::LoopId, body: Flow) -> Flow {
        let broken = body.breaks.contains(&id);
        Flow {
            normal: broken,
            escape: body.escape || body.normal,
            breaks: body.breaks.into_iter().filter(|b| *b != id).collect(),
        }
    }
    fn expr(&mut self, id: ir::ExprId) -> Result<Flow, String> {
        use ir::ExprKind as E;
        let expr = self.expr_ref(id)?;
        Ok(match &expr.kind {
            E::Call {
                target,
                args,
                signature,
                ..
            } => {
                let flow = self.seq(args)?;
                if !flow.normal {
                    return Ok(flow);
                }
                let mut flow = flow;
                match self.classifier.classify(target) {
                    CallClass::Recursive => {
                        self.recursive_calls += 1;
                        flow.normal = false;
                    }
                    CallClass::Uncertain => {
                        self.recursive_calls += 1;
                        let uncertain = self.uncertain();
                        flow.normal = uncertain.escape;
                    }
                    CallClass::NotRecursive => {
                        if signature.result == ir::Ty::Never {
                            // Only the unwind edge leaves a diverging call.
                            flow.normal = false;
                            flow.escape = true;
                        }
                    }
                }
                flow
            }
            E::Panic(_) => Flow::escaped(),
            E::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.expr(*condition)?;
                if !condition.normal {
                    return Ok(condition);
                }
                let then_flow = self.block(then_block)?;
                let else_flow = self.block(else_block)?;
                let mut flow = then_flow.or(else_flow);
                flow.escape |= condition.escape;
                flow.breaks.extend(condition.breaks);
                flow
            }
            E::Binary {
                op: BinaryOp::And | BinaryOp::Or,
                left,
                right,
            } => {
                let left = self.expr(*left)?;
                if !left.normal {
                    return Ok(left);
                }
                let right = self.expr(*right)?;
                left.or(right)
            }
            E::Match {
                scrutinee,
                arms,
                scrutinee_scope,
            } => {
                let scrutinee = self.expr(*scrutinee)?;
                if !scrutinee.normal {
                    return Ok(scrutinee);
                }
                let mut flow = Flow::default();
                for arm in arms {
                    let arm_flow = match arm.guard {
                        Some(guard) => {
                            let guard = self.expr(guard)?;
                            if guard.normal {
                                let body = self.expr(arm.body)?;
                                let mut body = body;
                                body.escape |= guard.escape;
                                body.breaks.extend(guard.breaks);
                                body
                            } else {
                                guard
                            }
                        }
                        None => self.expr(arm.body)?,
                    };
                    flow = flow.or(arm_flow);
                }
                // `if let` (edition 2024 scrutinee scope) keeps its else
                // branch; only a full `match` may leave an unreachable remainder.
                if !*scrutinee_scope {
                    let remainder = self.match_remainder(arms);
                    flow = flow.or(remainder);
                }
                flow.escape |= scrutinee.escape;
                flow.breaks.extend(scrutinee.breaks);
                flow
            }
            E::Block(block) => self.block(block)?,
            E::LabeledBlock { id, body } => {
                let mut flow = self.block(body)?;
                if flow.breaks.contains(id) {
                    flow.normal = true;
                    flow.breaks.retain(|b| b != id);
                }
                flow
            }
            E::Loop { id, body } => {
                let body = self.block(body)?;
                self.close_loop(*id, body)
            }
            E::Break { target, value } => {
                let flow = match value {
                    Some(value) => self.expr(*value)?,
                    None => Flow::normal(),
                };
                let mut flow = flow;
                if flow.normal {
                    flow.breaks.push(*target);
                }
                flow.normal = false;
                flow
            }
            E::Continue { .. } => Flow::escaped(),
            kind => {
                let children = children(kind);
                let flow = self.seq(&children)?;
                if flow.normal
                    && expr.completion == ir::Completion::Never
                    && children_complete(self, &children)?
                {
                    // A leaf that cannot complete (for example a process exit).
                    let mut flow = flow;
                    flow.normal = false;
                    flow.escape = true;
                    return Ok(flow);
                }
                flow
            }
        })
    }
    /// Whether a full `match` leaves stock's unreachable `otherwise` block
    /// reachable from its last candidate's failed test.
    fn match_remainder(&self, arms: &[ir::TypedMatchArm]) -> Flow {
        let Some(last) = arms.last() else {
            return Flow::escaped();
        };
        if last.guard.is_some() {
            return self.uncertain();
        }
        match &last.pattern {
            ir::MatchPattern::Wildcard
            | ir::MatchPattern::Binding(_)
            | ir::MatchPattern::Unit
            | ir::MatchPattern::Aggregate { .. }
            | ir::MatchPattern::Bool(_) => Flow::default(),
            ir::MatchPattern::Decision { tests, .. } if tests.is_empty() => Flow::default(),
            ir::MatchPattern::Decision { tests, .. }
                if tests
                    .iter()
                    .all(|t| matches!(t.kind, ir::PatternTestKind::Bool(_))) =>
            {
                Flow::default()
            }
            ir::MatchPattern::EnumVariant { ty, .. } => {
                if ty.def().variants.len() <= 1 {
                    Flow::default()
                } else {
                    Flow::escaped()
                }
            }
            ir::MatchPattern::Int(_) => Flow::escaped(),
            ir::MatchPattern::Decision { tests, .. }
                if tests.len() == 1
                    && tests[0].projection.is_empty()
                    && matches!(tests[0].kind, ir::PatternTestKind::Int(_)) =>
            {
                Flow::escaped()
            }
            _ => self.uncertain(),
        }
    }
}

/// Children complete normally, so a `Never` completion is the node itself.
fn children_complete(walk: &Walk<'_>, children: &[ir::ExprId]) -> Result<bool, String> {
    for child in children {
        if walk.expr_ref(*child)?.completion == ir::Completion::Never {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Operand expressions of the remaining (straight-line) kinds, in
/// evaluation order.
fn children(kind: &ir::ExprKind) -> Vec<ir::ExprId> {
    use ir::ExprKind as E;
    match kind {
        E::CellNew { value, .. }
        | E::ManuallyDrop { value, .. }
        | E::ManuallyDropIntoInner { value, .. }
        | E::ManuallyDropDrop { value, .. }
        | E::OpaqueWrap { value, .. }
        | E::LeadingZeros { value }
        | E::CharToInt { value, .. }
        | E::U8ToChar { value }
        | E::PtrCast { value }
        | E::IntCast { value, .. }
        | E::FloatCast { value, .. }
        | E::Deref { value }
        | E::Repeat { value, .. }
        | E::ParamRepeat { value, .. }
        | E::Unary { value, .. } => vec![*value],
        E::CellGet { receiver, .. } => vec![*receiver],
        E::CellSet {
            receiver, value, ..
        } => vec![*receiver, *value],
        E::NativeSyscall(args)
        | E::RawSlice { args, .. }
        | E::Intrinsic { args, .. }
        | E::Array(args)
        | E::Tuple(args) => args.clone(),
        E::SliceDataPointer { slice } | E::SliceLen { slice } => vec![*slice],
        E::BoundsCheck { index, .. } => vec![*index],
        E::DynamicIndex { base, index } => vec![*base, *index],
        E::ArrayToSlice { reference } | E::RefToDyn { reference } => vec![*reference],
        E::SliceIndex { slice, index } => vec![*slice, *index],
        E::Subslice {
            slice, start, end, ..
        } => std::iter::once(*slice).chain(*start).chain(*end).collect(),
        E::Index { base, .. } | E::Field { base, .. } => vec![*base],
        E::EnumVariant { fields, .. } | E::Struct { fields, .. } => {
            fields.iter().map(|(_, id)| *id).collect()
        }
        E::Binary { left, right, .. } => vec![*left, *right],
        E::Format { arguments, .. } => arguments.clone(),
        _ => Vec::new(),
    }
}
