use hir::HirDisplay;
use syntax::ast::{ArithOp, BinaryOp};

use crate::{Diagnostic, DiagnosticCode, DiagnosticsContext};

// Diagnostic: binary-operator-cannot-be-applied
//
// This diagnostic is triggered if a binary operator (`+`, `==`, `+=`, ...) is applied
// to operands whose types do not implement the corresponding trait (`Add`, `PartialEq`,
// `AddAssign`, ...).
pub(crate) fn binary_operator_cannot_be_applied(
    ctx: &DiagnosticsContext<'_, '_>,
    d: &hir::BinaryOperatorCannotBeApplied<'_>,
) -> Diagnostic {
    let lhs = d.lhs.display(ctx.sema.db, ctx.display_target);
    let rhs = d.rhs.display(ctx.sema.db, ctx.display_target);
    let (code, message) = match d.op {
        BinaryOp::Assignment { op } => (
            "E0368",
            format!(
                "binary assignment operation `{}=` cannot be applied to type `{lhs}`",
                op.map(|op| op.to_string()).unwrap_or_default()
            ),
        ),
        BinaryOp::ArithOp(op) => (
            "E0369",
            match op {
                ArithOp::Add => format!("cannot add `{rhs}` to `{lhs}`"),
                ArithOp::Sub => format!("cannot subtract `{rhs}` from `{lhs}`"),
                ArithOp::Mul => format!("cannot multiply `{lhs}` by `{rhs}`"),
                ArithOp::Div => format!("cannot divide `{lhs}` by `{rhs}`"),
                ArithOp::Rem => {
                    format!("cannot calculate the remainder of `{lhs}` divided by `{rhs}`")
                }
                ArithOp::BitAnd
                | ArithOp::BitXor
                | ArithOp::BitOr
                | ArithOp::Shl
                | ArithOp::Shr => format!("no implementation for `{lhs} {op} {rhs}`"),
            },
        ),
        BinaryOp::CmpOp(_) | BinaryOp::LogicOp(_) => {
            ("E0369", format!("binary operation `{}` cannot be applied to type `{lhs}`", d.op))
        }
    };
    Diagnostic::new_with_syntax_node_ptr(
        ctx,
        DiagnosticCode::RustcHardError(code),
        message,
        d.expr.map(Into::into),
    )
    .stable()
}

#[cfg(test)]
mod tests {
    use crate::tests::check_diagnostics;

    #[test]
    fn eq_on_struct_without_impl() {
        check_diagnostics(
            r#"
//- minicore: eq
struct S;

fn f() {
    let _ = S == S;
          //^^^^^^ error: binary operation `==` cannot be applied to type `S`
}
"#,
        );
    }

    #[test]
    fn eq_on_fn_item_with_unknown_rhs() {
        check_diagnostics(
            r#"
//- minicore: eq
fn foo() {}

fn f() {
    let y;
    if foo == y {}
     //^^^^^^^^ error: binary operation `==` cannot be applied to type `fn foo()`
}
"#,
        );
    }

    #[test]
    fn add_on_struct_without_impl() {
        check_diagnostics(
            r#"
//- minicore: add
struct S;

fn f() {
    let _ = S + S;
          //^^^^^ error: cannot add `S` to `S`
}
"#,
        );
    }

    #[test]
    fn add_assign_on_struct_without_impl() {
        check_diagnostics(
            r#"
//- minicore: add
struct S;

fn f() {
    let mut s = S;
    s += S;
  //^^^^^^ error: binary assignment operation `+=` cannot be applied to type `S`
}
"#,
        );
    }

    #[test]
    fn allows_ops_with_impl() {
        check_diagnostics(
            r#"
//- minicore: add, eq, builtin_impls
struct S;

impl core::ops::Add for S {
    type Output = S;
    fn add(self, _: S) -> S { S }
}

impl PartialEq for S {
    fn eq(&self, _: &S) -> bool { true }
}

fn f() {
    let _ = S + S;
    let _ = S == S;
    let _ = 1 + 2;
    let _ = 1u8 == 2;
}
"#,
        );
    }

    #[test]
    fn no_error_when_an_operand_is_an_error() {
        check_diagnostics(
            r#"
//- minicore: eq
struct S;

fn f() {
    let _ = S == unknown;
               //^^^^^^^ error: no such value in this scope
}
"#,
        );
    }
}
