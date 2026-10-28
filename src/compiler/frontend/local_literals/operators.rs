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

impl Operation {
    /// Indexing and binary operators may wait on a lazy element coercion.
    /// Others keep the conservative quiescence barrier.
    pub(super) fn lazy_frontier_inputs(&self) -> Option<(Term, Option<Term>)> {
        matches!(
            self.kind,
            Kind::Index(_) | Kind::Binary(_) | Kind::Unary(_) | Kind::Deref | Kind::Assign(_)
        )
        .then_some((self.left, self.right))
    }

    /// The value produced by a pending index or deref, which only the lazy
    /// coercion of its receiver can reveal.
    pub(super) fn index_result(&self) -> Option<Term> {
        matches!(self.kind, Kind::Index(_) | Kind::Deref).then_some(self.result)
    }
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
                let index = right.ok_or("index constraint omitted operand")?;
                if self.unify(index, usize).is_err() {
                    return Err(self.user_error(
                        "E0277: the type cannot be indexed by this integer; builtin indexes must be `usize`",
                    ));
                }
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
                // `map[&7]`: a borrowed unsuffixed literal takes its type from
                // the unique `Index<&Int>` impl of the receiver.
                if matches!(kind, Kind::Index(_))
                    && let Domain::Ref(ir::BorrowKind::Shared, inner) = self.domain(right)
                    && matches!(self.domain(inner), Domain::Integer | Domain::Float)
                    && let Some(argument) = self.checker.numeric_operator_rhs_hint_at(
                        &receiver,
                        declaration,
                        &method,
                        module,
                        matches!(self.domain(inner), Domain::Float),
                        true,
                    )?
                {
                    let formal = self.fixed(argument)?;
                    self.unify(inner, formal)?;
                }
                // `map[&7]` with `impl<K: Borrow<Q>, Q> Index<&Q> for
                // HashMap<K, V>`: the unique structural impl contributes its
                // argument pattern and where-clauses as equations, so `Q`
                // is fixed by `K: Borrow<Q>` before literal defaulting.
                // Selection below still proves the final application.
                if matches!(kind, Kind::Index(_))
                    && let Domain::Ref(_, inner) = self.domain(right)
                    && matches!(self.domain(inner), Domain::Integer | Domain::Float)
                    && let Some(header) = self.structural_impl(left, declaration)?
                    && header.trait_ref.args.len() == 1
                {
                    let params = self.structural_impl_parameters(left, &header, 0)?;
                    let pattern = self.schema(&header.trait_ref.args[0], &params, 0)?;
                    self.unify(pattern, right)?;
                    // `K: Borrow<Q>` with `Q` an unsuffixed literal: for a
                    // numeric `K` only the reflexive `Borrow<T> for T`
                    // applies, so stock fixes `Q = K`.
                    let (borrow, _) = self.checker.operator_trait("borrow", "Borrow", "borrow")?;
                    for predicate in header.predicates.iter() {
                        if let ir::Predicate::Implements(t) = predicate
                            && t.declaration == borrow
                            && let ir::Ty::Param(k) = &t.self_ty
                            && let Some(ir::Ty::Param(q)) = t.args.first()
                            && let (Some(&k), Some(&q)) = (params.get(k), params.get(q))
                            && matches!(self.domain(q), Domain::Integer | Domain::Float)
                            && matches!(
                                self.resolve(k, 0)?,
                                Some(ir::Ty::Int(_) | ir::Ty::Float(_))
                            )
                        {
                            self.unify(q, k)?;
                        }
                    }
                    return Ok(false);
                }
                if matches!(self.domain(right), Domain::Integer | Domain::Float) {
                    // `guard[1]`: the literal's type comes from the `Index`
                    // impl of the first autoderef step that has one, exactly
                    // as the selection loop below walks the receiver.
                    let mut hinted = receiver.clone();
                    for _ in 0..8 {
                        if let Some(argument) = self.checker.numeric_operator_rhs_hint(
                            &hinted,
                            declaration,
                            &method,
                            module,
                            matches!(self.domain(right), Domain::Float),
                        )? {
                            let formal = self.fixed(argument)?;
                            self.coerce(right, formal)?;
                            break;
                        }
                        if !matches!(kind, Kind::Index(_)) {
                            break;
                        }
                        hinted = match &hinted {
                            ir::Ty::Ref { referent, .. } => referent.as_ref().clone(),
                            other => match self.checker.deref_target_hint(other)? {
                                Some(next) => next,
                                None => break,
                            },
                        };
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
                let index = right.ok_or("index constraint omitted operand")?;
                if self.unify(index, usize).is_err() {
                    return Err(self.user_error(
                        "E0277: the type cannot be indexed by this integer; builtin indexes must be `usize`",
                    ));
                }
                let element = self.fixed(element)?;
                self.unify(result, element)?;
                return Ok(true);
            }
            receiver = next;
        }
        let Some((_, signature, _)) = selected else {
            // Stock reports an unsatisfied `Index` bound as E0277; the
            // other operators use E0369.
            if matches!(kind, Kind::Index(_)) {
                return Err(
                    self.user_error("E0277: the type cannot be indexed by this operand type")
                );
            }
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
