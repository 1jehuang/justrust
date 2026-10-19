//! Indexed format typing must not grant reference or writable authority.
use justrust_engine::compiler::{frontend, ir};

#[test]
fn indexed_format_rejects_forged_operand_kinds() {
    let source = "fn main(){let a=[1,2];let s:&[i32]=&a;println!(\"{}\",s[0]);}";
    let program = frontend::parse(source).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.callable_decl(0).unwrap();
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let argument = checked
        .exprs
        .iter()
        .find_map(|e| {
            if let ir::ExprKind::Format { arguments, .. } = &e.kind {
                arguments.first().copied()
            } else {
                None
            }
        })
        .unwrap();
    for mode in 0..2 {
        let mut forged = checked.clone();
        if mode == 0 {
            let ir::Ty::Ref { kind, .. } = &mut forged.exprs[argument.0].ty else {
                panic!("checked shared place borrow expected")
            };
            *kind = ir::BorrowKind::Mut;
        } else {
            forged.exprs[argument.0].ty = ir::Ty::Bool;
        }
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err(),
            "mode={mode}"
        );
    }
}
