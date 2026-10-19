//! Postfix `?` on the prelude `Option`/`Result`.
//!
//! `e?` lowers directly to the checked match it means:
//! `match e { Some(v) | Ok(v) => v, None => return None, Err(x) => return Err(x') }`
//! where `x'` is `x` when the error types are equal, or `From::from(x)` via the
//! trait registry otherwise. No source AST is synthesized.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Carrier {
    Option,
    Result,
}

fn carrier(ty: &ir::Ty) -> Option<(ir::EnumTy, Carrier)> {
    let ir::Ty::Enum(enumeration) = ty else {
        return None;
    };
    let def = enumeration.def();
    let names: Vec<&str> = def.variants.iter().map(|v| v.name.as_str()).collect();
    let std = crate::compiler::source::source_prelude::PRELUDE_MODULE;
    let option = format!("crate::{std}::option::Option");
    let result = format!("crate::{std}::result::Result");
    let kind = match names.as_slice() {
        ["None", "Some"] if def.name == option => Carrier::Option,
        ["Ok", "Err"] if def.name == result => Carrier::Result,
        _ => return None,
    };
    Some((enumeration.clone(), kind))
}

fn variant(ty: &ir::EnumTy, name: &str) -> Result<usize, String> {
    ty.def()
        .variants
        .iter()
        .position(|v| v.name == name)
        .ok_or_else(|| format!("contract: `?` carrier lacks variant {name}"))
}

impl Checker<'_> {
    fn try_expr(
        &mut self,
        ty: ir::Ty,
        completion: ir::Completion,
        kind: ir::ExprKind,
    ) -> Result<ir::ExprId, String> {
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion,
            kind,
        });
        Ok(id)
    }

    #[inline(never)]
    pub(super) fn lower_try(&mut self, operand: &Expr) -> Result<(ir::ExprKind, Flow), String> {
        let scrutinee = self.expr(operand)?;
        let source = self.flow(scrutinee);
        let Some((input, kind)) = carrier(&source.ty) else {
            return Err(format!(
                "E0277: the `?` operator can only be applied to values of type Option or Result, found {:?}",
                source.ty
            ));
        };
        let result = self.result.clone();
        let Some((output, output_kind)) = carrier(&result) else {
            return Err(
                "E0277: the `?` operator can only be used in a function that returns Option or Result"
                    .into(),
            );
        };
        if output_kind != kind {
            return Err(format!(
                "E0277: the `?` operator on {} cannot be used in a function returning {:?}",
                if kind == Carrier::Option {
                    "Option"
                } else {
                    "Result"
                },
                result
            ));
        }
        let (success, failure) = match kind {
            Carrier::Option => ("Some", "None"),
            Carrier::Result => ("Ok", "Err"),
        };
        let success_index = variant(&input, success)?;
        let failure_index = variant(&input, failure)?;
        let payload = input.def().variants[success_index].fields[0].ty.clone();
        let value = self.temporary("$try_value", payload.clone(), false)?;
        let read = self.try_expr(
            payload.clone(),
            ir::Completion::Returns,
            ir::ExprKind::Local(value),
        )?;
        let downcast = |index: usize| {
            ir::Projection::Downcast(ir::VariantId {
                owner: input.def().id,
                index,
            })
        };
        let success_arm = ir::MatchArm {
            guard: None,
            pattern: ir::MatchPattern::EnumVariant {
                ty: input.clone(),
                variant: success_index,
                bindings: vec![ir::PatternBinding {
                    local: value,
                    projection: vec![downcast(success_index), ir::Projection::Field(0)],
                    borrow: None,
                }],
            },
            body: read,
        };
        let out_failure = variant(&output, failure)?;
        let (bindings, fields) = match kind {
            Carrier::Option => (Vec::new(), Vec::new()),
            Carrier::Result => {
                let error_ty = input.def().variants[failure_index].fields[0].ty.clone();
                let target_ty = output.def().variants[out_failure].fields[0].ty.clone();
                let error = self.temporary("$try_error", error_ty.clone(), false)?;
                let read = self.try_expr(
                    error_ty.clone(),
                    ir::Completion::Returns,
                    ir::ExprKind::Local(error),
                )?;
                let converted = if error_ty == target_ty {
                    read
                } else {
                    self.try_from_conversion(read, &error_ty, &target_ty)?
                };
                (
                    vec![ir::PatternBinding {
                        local: error,
                        projection: vec![downcast(failure_index), ir::Projection::Field(0)],
                        borrow: None,
                    }],
                    vec![(0, converted)],
                )
            }
        };
        let residual = self.try_expr(
            result.clone(),
            ir::Completion::Returns,
            ir::ExprKind::EnumVariant {
                ty: output.clone(),
                variant: out_failure,
                fields,
            },
        )?;
        let exit = self.try_expr(
            ir::Ty::Never,
            ir::Completion::Never,
            ir::ExprKind::Block(ir::Block {
                statements: vec![ir::Stmt::Return(Some(residual))],
                tail: None,
            }),
        )?;
        let failure_arm = ir::MatchArm {
            guard: None,
            pattern: ir::MatchPattern::EnumVariant {
                ty: input.clone(),
                variant: failure_index,
                bindings,
            },
            body: exit,
        };
        Ok((
            ir::ExprKind::Match {
                scrutinee,
                arms: vec![success_arm, failure_arm],
            },
            Flow {
                ty: payload,
                completes: source.completes,
            },
        ))
    }

    /// `From::from(error)` for a differing residual error type, resolved
    /// through the ordinary trait registry. Unresolvable conversions are a
    /// type error, never an approximate cast.
    fn try_from_conversion(
        &mut self,
        error: ir::ExprId,
        from: &ir::Ty,
        to: &ir::Ty,
    ) -> Result<ir::ExprId, String> {
        Err(format!(
            "E0277: `?` couldn't convert the error to {to:?}: `From<{from:?}>` conversion is unsupported (error {:?})",
            error.0
        ))
    }
}
