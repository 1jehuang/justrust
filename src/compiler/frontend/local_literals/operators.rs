//! Per-body operator constraints use the same canonical trait selection as lowering.
//! Receiver, RHS and associated Output are independent terms. No impl is invented
//! when a receiver is still unknown and no selected signature grants move authority.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Binary(BinaryOp),
    Unary(UnaryOp),
    Assign(BinaryOp),
    Index(bool),
    /// Explicit `*x`: builtin for references/raw pointers, else `Deref`.
    Deref,
}
pub(super) struct Operation {
    kind: Kind,
    left: Term,
    right: Option<Term>,
    result: Term,
}

fn comparison(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    )
}

impl Solver<'_, '_, '_> {
    pub(super) fn operator_index(
        &mut self,
        base: Term,
        index: Term,
        mutable: bool,
    ) -> Result<Term, String> {
        let base = self.peel_refs(base);
        self.queue_operator(Kind::Index(mutable), base, Some(index))
    }

    pub(super) fn operator_binary(
        &mut self,
        op: BinaryOp,
        left: Term,
        right: Term,
    ) -> Result<Term, String> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let boolean = self.fixed(ir::Ty::Bool)?;
            self.unify(left, boolean)?;
            self.unify(right, boolean)?;
            return Ok(boolean);
        }
        self.queue_operator(Kind::Binary(op), left, Some(right))
    }

    pub(super) fn operator_deref(&mut self, value: Term) -> Result<Term, String> {
        self.queue_operator(Kind::Deref, value, None)
    }

    pub(super) fn operator_unary(&mut self, op: UnaryOp, value: Term) -> Result<Term, String> {
        self.queue_operator(Kind::Unary(op), value, None)
    }

    pub(super) fn operator_assignment(
        &mut self,
        op: BinaryOp,
        left: Term,
        right: Term,
    ) -> Result<(), String> {
        self.queue_operator(Kind::Assign(op), left, Some(right))?;
        Ok(())
    }

    fn queue_operator(
        &mut self,
        kind: Kind,
        left: Term,
        right: Option<Term>,
    ) -> Result<Term, String> {
        let result = self.fresh()?;
        // A deref result is a place, never a coercion producer; keeping it
        // out of delayed results preserves the prior unconstrained behavior.
        if !matches!(kind, Kind::Deref) {
            self.delayed_results.push(result);
        }
        self.operators.push(Operation {
            kind,
            left,
            right,
            result,
        });
        Ok(result)
    }

    pub(super) fn solve_operators(&mut self) -> Result<bool, String> {
        let operations = std::mem::take(&mut self.operators);
        let mut progress = false;
        for operation in operations {
            if self.solve_operator(&operation)? {
                progress = true;
            } else {
                self.operators.push(operation);
            }
        }
        Ok(progress)
    }

    fn solve_operator(&mut self, operation: &Operation) -> Result<bool, String> {
        let Operation {
            kind,
            left,
            right,
            result,
        } = *operation;
        let left = if matches!(kind, Kind::Index(_)) {
            self.peel_refs(left)
        } else {
            left
        };
        if matches!(kind, Kind::Deref) {
            match self.domain(left) {
                // A carrier still open in the final round adds no evidence:
                // before d658dd6e1 this was an unconstrained fresh term.
                // Checked lowering still types and proves the dereference.
                Domain::Unknown => return Ok(self.projection_defaults),
                Domain::Ref(_, element) | Domain::RawPtr(_, element) => {
                    self.unify(result, element)?;
                    return Ok(true);
                }
                Domain::Fixed(
                    ir::Ty::Ref { referent, .. }
                    | ir::Ty::RawPtr {
                        pointee: referent, ..
                    },
                ) => {
                    let element = self.fixed(referent.as_ref().clone())?;
                    self.unify(result, element)?;
                    return Ok(true);
                }
                // Only nominal/parameter carriers may overload `Deref`. A
                // scalar, array or other builtin carrier has no target: leave
                // it unresolved for the checker's E0614, never `*x: typeof x`.
                Domain::Nominal(..) | Domain::Fixed(ir::Ty::Param(_)) => {}
                _ => return Ok(false),
            }
        }
        if matches!(self.domain(left), Domain::Unknown) {
            if matches!(kind, Kind::Index(_)) {
                return Ok(false);
            }
            // Only the final numeric-default round may infer a missing builtin
            // receiver from numeric RHS evidence. Later nominal uses win first.
            if !self.projection_defaults {
                return Ok(false);
            }
            match right.map(|term| self.domain(term)) {
                Some(
                    Domain::Integer
                    | Domain::Float
                    | Domain::Fixed(ir::Ty::Int(_) | ir::Ty::Float(_)),
                ) => self.unify(left, right.unwrap())?,
                _ => return Ok(false),
            }
        }
        let overloaded = matches!(
            self.domain(left),
            Domain::Nominal(..) | Domain::Tuple(..) | Domain::Fixed(ir::Ty::Param(_))
        ) || (matches!(kind, Kind::Binary(op) if comparison(op))
            && matches!(self.domain(left), Domain::Ref(..))
            && right.is_some_and(|right| matches!(self.domain(right), Domain::Nominal(..))));
        if !overloaded {
            if matches!(kind, Kind::Index(_)) {
                let element = match self.domain(left) {
                    Domain::Array(element, _) | Domain::Slice(element) => element,
                    Domain::Unknown => return Ok(false),
                    _ => return Err(self.user_error("E0608: cannot index this type")),
                };
                let usize = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                self.unify(right.ok_or("index constraint omitted operand")?, usize)?;
                self.unify(result, element)?;
                return Ok(true);
            }
            let (left, right) = match (kind, right) {
                // A still-open arithmetic operand may yet resolve to `&T`
                // (`i as u32 * x` with `x` from an iterator projection);
                // equating it with the scalar side now would be a guess.
                (Kind::Binary(op) | Kind::Assign(op), Some(right))
                    if !comparison(op)
                        && !self.projection_defaults
                        && matches!(self.domain(right), Domain::Unknown)
                        && matches!(
                            self.domain(left),
                            Domain::Fixed(ir::Ty::Int(_) | ir::Ty::Float(_))
                        ) =>
                {
                    return Ok(false);
                }
                (Kind::Binary(op), Some(right)) => {
                    let (left, right) = self.scalar_reference_operands(op, left, right);
                    (left, Some(right))
                }
                // `x += &y`: primitives implement `OpAssign<&T> for T`.
                (Kind::Assign(op), Some(right)) if !comparison(op) => {
                    let (_, right) = self.scalar_reference_operands(op, left, right);
                    (left, Some(right))
                }
                _ => (left, right),
            };
            if let Some(right) = right {
                let shift = matches!(
                    kind,
                    Kind::Binary(BinaryOp::Shl | BinaryOp::Shr)
                        | Kind::Assign(BinaryOp::Shl | BinaryOp::Shr)
                );
                if !shift {
                    self.unify(left, right)?;
                }
            }
            let output = match kind {
                Kind::Assign(_) => self.fixed(ir::Ty::Unit)?,
                Kind::Binary(op) if comparison(op) => self.fixed(ir::Ty::Bool)?,
                _ => left,
            };
            self.unify(result, output)?;
            return Ok(true);
        }
        let Some(receiver) = self.resolve(left, 0)? else {
            // An overloaded deref with an open receiver adds no evidence.
            // Leaving the result unconstrained matches the prior solver
            // behavior; checked lowering still selects and proves the call.
            return Ok(matches!(kind, Kind::Deref));
        };
        let spec = match kind {
            Kind::Binary(op) => super::super::operators::binary_trait(op),
            Kind::Unary(UnaryOp::Neg) => Some(("ops", "Neg", "neg")),
            Kind::Unary(UnaryOp::Not) => Some(("ops", "Not", "not")),
            Kind::Assign(op) => super::super::operators::assign_trait(op)
                .map(|(name, method)| ("ops", name, method)),
            Kind::Index(false) => Some(("ops", "Index", "index")),
            Kind::Index(true) => Some(("ops", "IndexMut", "index_mut")),
            Kind::Deref => Some(("ops", "Deref", "deref")),
        };
        let Some((module, name, method)) = spec else {
            return Ok(false);
        };
        let (declaration, method) = self.checker.operator_trait(module, name, method)?;
        let plan = self.checker.signatures.plan()?;
        let arity = plan.registry.declaration(declaration)?.param_env.arity();
        let arguments = match (arity, right) {
            (1, _) => Vec::new(),
            (2, Some(right)) => {
                if matches!(self.domain(right), Domain::Integer | Domain::Float) {
                    if let Some(argument) = self.checker.numeric_operator_rhs_hint(
                        &receiver,
                        declaration,
                        &method,
                        module,
                        matches!(self.domain(right), Domain::Float),
                    )? {
                        let formal = self.fixed(argument)?;
                        self.coerce(right, formal)?;
                    }
                    if !self.projection_defaults
                        && matches!(self.domain(right), Domain::Integer | Domain::Float)
                    {
                        return Ok(false);
                    }
                }
                let Some(right) = self.resolve(right, 0)? else {
                    return Ok(false);
                };
                vec![right]
            }
            _ => return Ok(false),
        };
        let arguments: std::sync::Arc<[ir::Ty]> = arguments.into();
        let mut receiver = receiver;
        let mut selected = None;
        // `guard[i]`/`boxed[i]`: stock autoderefs the indexed operand through
        // `Deref` until an `Index` impl applies. Bounded by the receiver depth.
        for _ in 0..8 {
            let trait_ref = ir::TraitRef {
                declaration,
                self_ty: receiver.clone(),
                args: arguments.clone(),
            };
            selected = plan.select_trait_with_scratch(
                &trait_ref,
                &method,
                &[],
                self.checker.facts(),
                false,
                &plan.budget,
                &self.checker.trait_scratch,
            )?;
            if selected.is_some() || !matches!(kind, Kind::Index(_)) {
                break;
            }
            let builtin = match &receiver {
                ir::Ty::Ref { referent, .. } => Some(referent.as_ref().clone()),
                _ => None,
            };
            let next = match builtin {
                Some(next) => next,
                None => {
                    let (deref, deref_method) =
                        self.checker.operator_trait("ops", "Deref", "deref")?;
                    let deref_ref = ir::TraitRef {
                        declaration: deref,
                        self_ty: receiver.clone(),
                        args: Vec::new().into(),
                    };
                    let Some((_, signature, _)) = plan.select_trait_with_scratch(
                        &deref_ref,
                        &deref_method,
                        &[],
                        self.checker.facts(),
                        false,
                        &plan.budget,
                        &self.checker.trait_scratch,
                    )?
                    else {
                        break;
                    };
                    match signature.result {
                        ir::Ty::Ref { referent, .. } => referent.as_ref().clone(),
                        _ => break,
                    }
                }
            };
            if matches!(next, ir::Ty::Array { .. } | ir::Ty::Slice { .. }) {
                let element = match &next {
                    ir::Ty::Array { element, .. } | ir::Ty::Slice { element, .. } => {
                        element.as_ref().clone()
                    }
                    _ => unreachable!(),
                };
                let usize = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                self.unify(right.ok_or("index constraint omitted operand")?, usize)?;
                let element = self.fixed(element)?;
                self.unify(result, element)?;
                return Ok(true);
            }
            receiver = next;
        }
        let Some((_, signature, _)) = selected else {
            return Err(self.user_error(
                "E0369: no implementation of the canonical operator trait for these operand types",
            ));
        };
        if let Some(right) = right
            && let Some(formal) = signature.params.get(1)
        {
            let formal = if module == "cmp" {
                match formal {
                    ir::Ty::Ref { referent, .. } => referent.as_ref(),
                    _ => formal,
                }
            } else {
                formal
            };
            let formal = self.fixed(formal.clone())?;
            self.coerce(right, formal)?;
        }
        let output = if matches!(kind, Kind::Index(_) | Kind::Deref) {
            let ir::Ty::Ref { referent, .. } = signature.result else {
                return Err("invalid metadata: place operator result is not a reference".into());
            };
            self.fixed((*referent).clone())?
        } else {
            self.fixed(signature.result)?
        };
        self.unify(result, output)?;
        Ok(true)
    }
}

impl Solver<'_, '_, '_> {
    /// Mirrors the checker's primitive forwarding impls: arithmetic accepts
    /// one shared reference layer per operand over a numeric referent, and
    /// comparisons peel equal reference depths. Only hints; the checker
    /// re-derives every accepted read.
    fn scalar_reference_operands(&self, op: BinaryOp, left: Term, right: Term) -> (Term, Term) {
        let numeric = |solver: &Self, term: Term| {
            matches!(
                solver.domain(term),
                Domain::Integer
                    | Domain::Float
                    | Domain::Fixed(ir::Ty::Int(_) | ir::Ty::Float(_) | ir::Ty::Bool)
            )
        };
        if comparison(op) {
            let (mut l, mut r) = (left, right);
            for _ in 0..8 {
                let (Domain::Ref(_, a), Domain::Ref(_, b)) = (self.domain(l), self.domain(r))
                else {
                    break;
                };
                l = a;
                r = b;
            }
            if (l, r) != (left, right) && numeric(self, l) && numeric(self, r) {
                return (l, r);
            }
            return (left, right);
        }
        let peel = |term: Term| match self.domain(term) {
            Domain::Ref(ir::BorrowKind::Shared, inner) if numeric(self, inner) => inner,
            _ => term,
        };
        (peel(left), peel(right))
    }
}
