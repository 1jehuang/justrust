use crate::{Diagnostic, DiagnosticCode, DiagnosticsContext};

// Diagnostic: closure-kind-mismatch
//
// This diagnostic is triggered when a closure is required to implement a `Fn*` or `AsyncFn*`
// trait that its closure kind does not allow, e.g. `Fn` for a closure that moves out of what it
// captures.
pub(crate) fn closure_kind_mismatch(
    ctx: &DiagnosticsContext<'_, '_>,
    d: &hir::ClosureKindMismatch,
) -> Diagnostic {
    Diagnostic::new_with_syntax_node_ptr(
        ctx,
        DiagnosticCode::RustcHardError("E0525"),
        format!(
            "expected a closure that implements the `{}` trait, but this closure only implements `{}`",
            d.expected, d.found
        ),
        d.span.map(Into::into),
    )
}

#[cfg(test)]
mod tests {
    use crate::tests::check_diagnostics;

    #[test]
    fn fn_once_closure_bound_to_a_variable_where_fn_is_required() {
        check_diagnostics(
            r#"
//- minicore: fn
struct S;
fn needs_fn(_: impl Fn()) {}

fn f() {
    let s = S;
    let c = move || drop(s);
    needs_fn(c);
 // ^^^^^^^^ error: expected a closure that implements the `Fn` trait, but this closure only implements `FnOnce`
}

fn g() {
    let s = S;
    needs_fn(move || drop(s));
                        //^ error: cannot move out of `s` which is behind a shared reference
}

fn drop<T>(_: T) {}
"#,
        );
    }

    #[test]
    fn closure_with_other_arguments_is_not_a_kind_mismatch() {
        check_diagnostics(
            r#"
//- minicore: fn
fn needs_fn(_: impl Fn(u32)) {}

fn f() {
    let c = || ();
    needs_fn(c);
 // ^^^^^^^^ error: the trait bound `impl Fn(): Fn<(u32,)>` is not satisfied
 // ^^^^^^^^ error: the trait bound `impl Fn(): FnOnce<(u32,)>` is not satisfied
}
"#,
        );
    }
}
