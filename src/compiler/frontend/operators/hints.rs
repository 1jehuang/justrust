//! Immutable expression hints replay canonical operator selection, including Output.
//! No operand is lowered here and no hint supplies borrow or move authority.
use super::*;

impl type_check::Checker<'_> {
    /// Only a unique proved numeric formal constrains a numeric literal domain.
    /// This is an expectation, never call authority; lowering selects again.
    pub(in crate::compiler::frontend) fn numeric_operator_rhs_hint(
        &self,
        receiver: &ir::Ty,
        declaration: ir::DeclId,
        method: &AstIdent,
        module: &str,
        float: bool,
    ) -> Result<Option<ir::Ty>, String> {
        self.numeric_operator_rhs_hint_at(receiver, declaration, method, module, float, false)
    }

    /// `referenced`: the operand is `&{literal}` (`map[&7]`), so the formal
    /// must be a shared reference to the numeric type, which is returned.
    pub(in crate::compiler::frontend) fn numeric_operator_rhs_hint_at(
        &self,
        receiver: &ir::Ty,
        declaration: ir::DeclId,
        method: &AstIdent,
        module: &str,
        float: bool,
        referenced: bool,
    ) -> Result<Option<ir::Ty>, String> {
        let Some(facts) = self.facts().validated() else {
            return Ok(None);
        };
        let plan = self.signatures.plan()?;
        let (applications, deferred) =
            plan.inferred_trait_applications(receiver, declaration, facts, &plan.budget)?;
        if deferred {
            return Ok(None);
        }
        let mut unique = None;
        for application in applications.as_slice() {
            plan.budget.charge_work(1)?;
            let Some((_, signature, _)) = plan.select_trait_with_scratch(
                application,
                method,
                &[],
                self.facts(),
                false,
                &plan.budget,
                &self.trait_scratch,
            )?
            else {
                continue;
            };
            let Some(argument) = signature.params.get(1) else {
                continue;
            };
            let argument = if module == "cmp" {
                match argument {
                    ir::Ty::Ref { referent, .. } => referent.as_ref(),
                    _ => argument,
                }
            } else {
                argument
            };
            let argument = match (referenced, argument) {
                (false, argument) => argument,
                (
                    true,
                    ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent,
                    },
                ) => referent.as_ref(),
                (true, _) => continue,
            };
            if !matches!(
                (float, argument),
                (false, ir::Ty::Int(_)) | (true, ir::Ty::Float(_))
            ) {
                continue;
            }
            if let Some(previous) = &unique
                && previous != argument
            {
                return Ok(None);
            }
            unique = Some(argument.clone());
        }
        Ok(unique)
    }

    /// `s + &t` with `t: String`: when exactly one proved application of the
    /// binary operator trait exists for the receiver, stock checks the RHS
    /// as coercible to that impl's formal (`&String` to `&str`). Returns the
    /// unique formal; any ambiguity or open selection supplies nothing.
    pub(in crate::compiler::frontend) fn unique_operator_rhs_formal(
        &self,
        receiver: &ir::Ty,
        declaration: ir::DeclId,
        method: &AstIdent,
    ) -> Result<Option<ir::Ty>, String> {
        let Some(facts) = self.facts().validated() else {
            return Ok(None);
        };
        let plan = self.signatures.plan()?;
        let (applications, deferred) =
            plan.inferred_trait_applications(receiver, declaration, facts, &plan.budget)?;
        if deferred {
            return Ok(None);
        }
        let mut unique = None;
        for application in applications.as_slice() {
            plan.budget.charge_work(1)?;
            let Some((_, signature, _)) = plan.select_trait_with_scratch(
                application,
                method,
                &[],
                self.facts(),
                false,
                &plan.budget,
                &self.trait_scratch,
            )?
            else {
                continue;
            };
            let Some(argument) = signature.params.get(1) else {
                continue;
            };
            if unique.replace(argument.clone()).is_some() {
                return Ok(None);
            }
        }
        Ok(unique)
    }

    pub(in crate::compiler::frontend) fn unsuffixed_operator_rhs_hint(
        &self,
        receiver: &ir::Ty,
        declaration: ir::DeclId,
        method: &AstIdent,
        module: &str,
        rhs: &Expr,
    ) -> Result<Option<ir::Ty>, String> {
        match rhs {
            Expr::Integer(literal) if literal.suffix.is_none() => {
                self.numeric_operator_rhs_hint(receiver, declaration, method, module, false)
            }
            Expr::Float(literal) if literal.suffix.is_none() => {
                self.numeric_operator_rhs_hint(receiver, declaration, method, module, true)
            }
            Expr::BorrowValue {
                kind: ir::BorrowKind::Shared,
                value,
            } => match value.as_ref() {
                Expr::Integer(literal) if literal.suffix.is_none() => Ok(self
                    .numeric_operator_rhs_hint_at(
                        receiver,
                        declaration,
                        method,
                        module,
                        false,
                        true,
                    )?
                    .map(|ty| ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: ty.into(),
                    })),
                Expr::Float(literal) if literal.suffix.is_none() => Ok(self
                    .numeric_operator_rhs_hint_at(
                        receiver,
                        declaration,
                        method,
                        module,
                        true,
                        true,
                    )?
                    .map(|ty| ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: ty.into(),
                    })),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }
    pub(in crate::compiler::frontend) fn operator_result_hint(
        &self,
        expression: &Expr,
    ) -> Result<Option<ir::Ty>, String> {
        let (spec, value, rhs) = match expression {
            Expr::Binary {
                op, left, right, ..
            } => {
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                        | BinaryOp::And
                        | BinaryOp::Or
                ) {
                    return Ok(Some(ir::Ty::Bool));
                }
                let Some(spec) = binary_trait(*op) else {
                    return Ok(None);
                };
                (spec, left.as_ref(), Some(right.as_ref()))
            }
            Expr::Unary { op, value, .. } => {
                let spec = match op {
                    UnaryOp::Neg => ("ops", "Neg", "neg"),
                    UnaryOp::Not => ("ops", "Not", "not"),
                };
                (spec, value.as_ref(), None)
            }
            _ => return Ok(None),
        };
        let Some(receiver) = self.method_receiver_hint(value)? else {
            return match expression {
                Expr::Binary {
                    op: BinaryOp::Shl | BinaryOp::Shr,
                    ..
                } => Ok(None),
                Expr::Binary { right, .. } => self.scalar_hint(right),
                _ => Ok(None),
            };
        };
        if !overloadable(&receiver) {
            return Ok(Some(receiver));
        }
        let (declaration, method) = self.operator_trait(spec.0, spec.1, spec.2)?;
        let plan = self.signatures.plan()?;
        let candidates = match (
            plan.registry.declaration(declaration)?.param_env.arity(),
            rhs,
        ) {
            (1, _) => vec![Vec::new()],
            (2, Some(rhs)) => match self
                .unsuffixed_operator_rhs_hint(&receiver, declaration, &method, spec.0, rhs)?
                .or(self.method_receiver_hint(rhs)?)
            {
                Some(rhs) => vec![vec![rhs]],
                None => {
                    let fallback = if matches!(rhs, Expr::Float(_)) {
                        ir::Ty::Float(ir::FloatTy::F64)
                    } else {
                        ir::Ty::Int(ir::IntTy::I32)
                    };
                    if matches!(rhs, Expr::Integer(_) | Expr::Float(_)) {
                        vec![vec![fallback]]
                    } else {
                        vec![vec![receiver.clone()], vec![fallback]]
                    }
                }
            },
            _ => return Ok(None),
        };
        let mut candidates = candidates;
        if spec.0 == "ops"
            && rhs.is_some()
            && let Some(formal) =
                self.unique_operator_rhs_formal(&receiver, declaration, &method)?
            && !candidates.iter().any(|c| c.first() == Some(&formal))
        {
            candidates.push(vec![formal]);
        }
        for args in candidates {
            let trait_ref = ir::TraitRef {
                declaration,
                self_ty: receiver.clone(),
                args: args.into(),
            };
            if let Some((_, signature, _)) = plan.select_trait_with_scratch(
                &trait_ref,
                &method,
                &[],
                self.facts(),
                false,
                &plan.budget,
                &self.trait_scratch,
            )? {
                return Ok(Some(signature.result));
            }
        }
        Err(
            "E0369: no implementation of the canonical operator trait for these operand types"
                .into(),
        )
    }
}

impl type_check::Checker<'_> {
    /// Every proved operand formal of the operator trait for this receiver,
    /// without duplicates. `None` while selection is still open. These are
    /// expectations for literal typing only; lowering selects again.
    pub(in crate::compiler::frontend) fn operator_rhs_formals(
        &self,
        receiver: &ir::Ty,
        declaration: ir::DeclId,
        method: &AstIdent,
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        let Some(facts) = self.facts().validated() else {
            return Ok(None);
        };
        let plan = self.signatures.plan()?;
        let (applications, deferred) =
            plan.inferred_trait_applications(receiver, declaration, facts, &plan.budget)?;
        if deferred {
            return Ok(None);
        }
        let mut formals = Vec::new();
        for application in applications.as_slice() {
            plan.budget.charge_work(1)?;
            let Some((_, signature, _)) = plan.select_trait_with_scratch(
                application,
                method,
                &[],
                self.facts(),
                false,
                &plan.budget,
                &self.trait_scratch,
            )?
            else {
                continue;
            };
            if let Some(argument) = signature.params.get(1)
                && !formals.contains(argument)
            {
                formals.push(argument.clone());
            }
        }
        Ok(Some(formals))
    }
}

/// Could a hinted operand type whose unsuffixed literals were defaulted
/// (`i32`, `f64`) really be `formal`? Structure must otherwise agree.
fn defaulted_literal_fits(hinted: &ir::Ty, formal: &ir::Ty) -> bool {
    match (hinted, formal) {
        (ir::Ty::Int(ir::IntTy::I32), ir::Ty::Int(_)) => true,
        (ir::Ty::Float(ir::FloatTy::F64), ir::Ty::Float(_)) => true,
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(a, b)| defaulted_literal_fits(a, b))
        }
        (
            ir::Ty::Ref { kind, referent },
            ir::Ty::Ref {
                kind: formal_kind,
                referent: formal,
            },
        ) => kind == formal_kind && defaulted_literal_fits(referent, formal),
        (
            ir::Ty::Array { element, len },
            ir::Ty::Array {
                element: formal,
                len: formal_len,
            },
        ) => len == formal_len && defaulted_literal_fits(element, formal),
        // `Range<i32>` written as `(1..3)` against a `Range<usize>` formal.
        _ => match (nominal_parts(hinted), nominal_parts(formal)) {
            (Some((owner, actuals)), Some((formal_owner, formals))) => {
                owner == formal_owner
                    && actuals.len() == formals.len()
                    && actuals
                        .iter()
                        .zip(formals)
                        .all(|(a, b)| defaulted_literal_fits(a, b))
            }
            _ => hinted == formal,
        },
    }
}

impl type_check::Checker<'_> {
    /// `grid[(1, 0)]`: the operand hint defaulted its literals to `i32`, but
    /// exactly one proved operand formal has that shape. Only an expectation:
    /// the operand is still checked against the selected signature.
    pub(in crate::compiler::frontend) fn defaulted_operand_formal(
        &self,
        receiver: &ir::Ty,
        declaration: ir::DeclId,
        method: &AstIdent,
        hinted: &ir::Ty,
    ) -> Result<Option<ir::Ty>, String> {
        let Some(formals) = self.operator_rhs_formals(receiver, declaration, method)? else {
            return Ok(None);
        };
        let mut fitting = formals
            .into_iter()
            .filter(|formal| formal != hinted && defaulted_literal_fits(hinted, formal));
        let first = fitting.next();
        Ok(if fitting.next().is_some() {
            None
        } else {
            first
        })
    }
}

impl type_check::Checker<'_> {
    /// Could this written operand have type `formal`? Structure only: every
    /// leaf is still checked against the selected signature.
    fn operand_expr_fits(
        &self,
        operand: &Expr,
        formal: &ir::Ty,
        depth: usize,
    ) -> Result<bool, String> {
        self.infer_work(depth)?;
        Ok(match (operand, formal) {
            (Expr::Integer(_), _) => matches!(formal, ir::Ty::Int(_)),
            (Expr::Float(_), _) => matches!(formal, ir::Ty::Float(_)),
            (Expr::Tuple(fields), ir::Ty::Tuple(formals)) => {
                if fields.len() != formals.len() {
                    return Ok(false);
                }
                for (field, formal) in fields.iter().zip(formals.iter()) {
                    if !self.operand_expr_fits(field, formal, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            (Expr::Tuple(fields), _) if !fields.is_empty() => false,
            _ => match self.method_receiver_hint(operand)? {
                Some(hinted) => hinted == *formal || defaulted_literal_fits(&hinted, formal),
                None => true,
            },
        })
    }

    /// Operand type expectation for `owner[index]`. An operand with no
    /// independent type (a tuple of literals and locals) takes the unique
    /// `Index` formal of `owner` that its written shape fits; otherwise the
    /// builtin `usize` expectation stands. Never call authority.
    pub(in crate::compiler::frontend) fn index_operand_hint(
        &self,
        owner: &ir::Ty,
        index: &Expr,
    ) -> Result<ir::Ty, String> {
        if let Some(hinted) = self.method_receiver_hint(index)? {
            return Ok(hinted);
        }
        let fallback = ir::Ty::Int(ir::IntTy::Usize);
        let Ok((declaration, name)) = self.operator_trait("ops", "Index", "index") else {
            return Ok(fallback);
        };
        let Some(formals) = self.operator_rhs_formals(owner, declaration, &name)? else {
            return Ok(fallback);
        };
        let mut fitting = None;
        for formal in formals {
            if self.operand_expr_fits(index, &formal, 0)? && fitting.replace(formal).is_some() {
                return Ok(fallback);
            }
        }
        Ok(fitting.unwrap_or(fallback))
    }
}
