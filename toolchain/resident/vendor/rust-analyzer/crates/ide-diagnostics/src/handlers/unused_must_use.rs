use crate::{Diagnostic, DiagnosticCode, DiagnosticsContext};

// Diagnostic: unused-must-use
//
// This diagnostic is triggered when a value with the `#[must_use]` attribute
// is dropped without being used.
pub(crate) fn unused_must_use<'db>(
    ctx: &DiagnosticsContext<'_, 'db>,
    d: &hir::UnusedMustUse<'db>,
) -> Diagnostic {
    let message = match d.message {
        Some(message) => format!("unused return value that must be used: {message}"),
        None => "unused return value that must be used".to_owned(),
    };
    Diagnostic::new_with_syntax_node_ptr(
        ctx,
        DiagnosticCode::RustcLint("unused_must_use"),
        message,
        d.expr.map(Into::into),
    )
    .stable()
}

#[cfg(test)]
mod tests {
    use crate::tests::check_diagnostics;

    #[test]
    fn unused_must_use_function_call() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    produces();
  //^^^^^^^^^^ warn: unused return value that must be used
}
"#,
        );
    }

    #[test]
    fn unused_must_use_method_call() {
        check_diagnostics(
            r#"
struct S;
impl S {
    #[must_use]
    fn produces(&self) -> i32 { 0 }
}
fn main() {
    let s = S;
    s.produces();
  //^^^^^^^^^^^^ warn: unused return value that must be used
}
"#,
        );
    }

    #[test]
    fn with_message() {
        check_diagnostics(
            r#"
struct S;
impl S {
    #[must_use = "custom message"]
    fn produces(&self) -> i32 { 0 }
}
fn main() {
    let s = S;
    s.produces();
  //^^^^^^^^^^^^ warn: unused return value that must be used: custom message
}
"#,
        );
    }

    #[test]
    fn unused_must_use_type() {
        check_diagnostics(
            r#"
#[must_use]
struct Important;
fn produces() -> Important { Important }
fn main() {
    produces();
  //^^^^^^^^^^ warn: unused return value that must be used
}
"#,
        );
    }

    #[test]
    fn no_warning_when_value_used() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    let _x = produces();
}
"#,
        );
    }

    #[test]
    fn no_warning_when_no_must_use_attribute() {
        check_diagnostics(
            r#"
fn ordinary() -> i32 { 0 }
fn main() {
    ordinary();
}
"#,
        );
    }

    #[test]
    fn no_warning_when_value_assigned() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    let x;
    x = produces();
    let _ = x;
}
"#,
        );
    }

    #[test]
    fn block_tail_expression_in_stmt_position() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    {
        produces()
      //^^^^^^^^^^ warn: unused return value that must be used
    };
}
"#,
        );
    }

    #[test]
    fn unsafe_block_tail_expression_in_stmt_position() {
        check_diagnostics(
            r#"
#[must_use]
unsafe fn produces() -> i32 { 0 }
fn main() {
    unsafe {
        produces()
      //^^^^^^^^^^ warn: unused return value that must be used
    };
}
"#,
        );
    }

    #[test]
    fn nested_block_tail_expression() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    {
        {
            produces()
          //^^^^^^^^^^ warn: unused return value that must be used
        }
    };
}
"#,
        );
    }

    #[test]
    fn no_warning_when_block_tail_is_bound() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    let _x = {
        produces()
    };
}
"#,
        );
    }

    #[test]
    fn if_branches_in_stmt_position_are_not_linted() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    if true {
        produces()
    } else {
        produces()
    };
}
"#,
        );
    }

    #[test]
    fn match_arms_in_stmt_position_are_not_linted() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    match 0 {
        0 => produces(),
        _ => produces(),
    };
}
"#,
        );
    }

    #[test]
    fn const_block_in_stmt_position_is_not_linted() {
        check_diagnostics(
            r#"
#[must_use]
const fn produces() -> i32 { 0 }
fn main() {
    const {
        produces()
    };
}
"#,
        );
    }

    #[test]
    fn must_use_type_through_block() {
        check_diagnostics(
            r#"
#[must_use]
struct Important;
fn produces() -> Important { Important }
fn main() {
    {
        produces()
      //^^^^^^^^^^ warn: unused return value that must be used
    };
}
"#,
        );
    }

    #[test]
    fn result_and_control_flow_with_an_uninhabited_side_are_what_they_hold() {
        check_diagnostics(
            r#"
//- minicore: result, try
#[must_use]
struct Important;
enum Never {}
fn unit() -> Result<(), Never> { Ok(()) }
fn important() -> Result<Important, Never> { Ok(Important) }
fn flow() -> core::ops::ControlFlow<Never, Important> { core::ops::ControlFlow::Continue(Important) }
fn never() -> Never { loop {} }
fn main() {
    unit();
    important();
  //^^^^^^^^^^^ warn: unused return value that must be used
    flow();
  //^^^^^^ warn: unused return value that must be used
}
"#,
        );
    }

    #[test]
    fn a_local_holding_a_function_is_not_linted() {
        check_diagnostics(
            r#"
#[must_use]
fn produces() -> i32 { 0 }
fn main() {
    let local = produces;
    local();
}
"#,
        );
    }
}
