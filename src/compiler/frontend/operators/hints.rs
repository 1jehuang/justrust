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
            let Some((_, signature, _)) =
                plan.select_trait(application, method, &[], self.facts(), false, &plan.budget)?
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
        for args in candidates {
            let trait_ref = ir::TraitRef {
                declaration,
                self_ty: receiver.clone(),
                args: args.into(),
            };
            if let Some((_, signature, _)) =
                plan.select_trait(&trait_ref, &method, &[], self.facts(), false, &plan.budget)?
            {
                return Ok(Some(signature.result));
            }
        }
        Err(
            "E0369: no implementation of the canonical operator trait for these operand types"
                .into(),
        )
    }
}
