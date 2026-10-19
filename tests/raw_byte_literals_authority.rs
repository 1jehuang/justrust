//! Byte payload typing cannot be forged into string, slice, mutable or wrong-extent IR.
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

#[test]
fn byte_string_ir_requires_exact_array_extent_and_returning_completion() {
    let program = frontend::parse(r#"fn bytes()->&'static [u8;2]{b"\0\xff"}fn main(){}"#).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    let checked =
        frontend::type_callable_with_plan(&program, declaration, &plan, facts.budget()).unwrap();
    let index = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::ByteStr(_)))
        .unwrap();
    assert_eq!(checked.exprs[index].ty, ir::Ty::byte_string_ty(2));
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    for ty in [
        ir::Ty::shared_str_ty(),
        ir::Ty::byte_string_ty(1),
        ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(ir::Ty::Array {
                element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
                len: 2,
            }),
        },
        ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Slice {
                element: Arc::new(ir::Ty::Int(ir::IntTy::U8)),
            }),
        },
        ir::Ty::Int(ir::IntTy::U8),
    ] {
        let mut forged = checked.clone();
        forged.exprs[index].ty = ty;
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err()
        );
        assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
    }
    let mut forged = checked.clone();
    forged.exprs[index].completion = ir::Completion::Never;
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
    let mut forged = checked;
    let ir::ExprKind::ByteStr(bytes) = &mut forged.exprs[index].kind else {
        unreachable!()
    };
    bytes.push(0);
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
}
