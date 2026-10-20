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
        self.delayed_results.push(result);
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
        );
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
            return Ok(false);
        };
        let spec = match kind {
            Kind::Binary(op) => super::super::operators::binary_trait(op),
            Kind::Unary(UnaryOp::Neg) => Some(("ops", "Neg", "neg")),
            Kind::Unary(UnaryOp::Not) => Some(("ops", "Not", "not")),
            Kind::Assign(op) => super::super::operators::assign_trait(op)
                .map(|(name, method)| ("ops", name, method)),
            Kind::Index(false) => Some(("ops", "Index", "index")),
            Kind::Index(true) => Some(("ops", "IndexMut", "index_mut")),
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
        let trait_ref = ir::TraitRef {
            declaration,
            self_ty: receiver,
            args: arguments.into(),
        };
        let Some((_, signature, _)) = plan.select_trait(
            &trait_ref,
            &method,
            &[],
            self.checker.facts(),
            false,
            &plan.budget,
        )?
        else {
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
        let output = if matches!(kind, Kind::Index(_)) {
            let ir::Ty::Ref { referent, .. } = signature.result else {
                return Err("invalid metadata: index method result is not a reference".into());
            };
            self.fixed((*referent).clone())?
        } else {
            self.fixed(signature.result)?
        };
        self.unify(result, output)?;
        Ok(true)
    }
}
