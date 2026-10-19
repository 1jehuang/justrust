//! Invalid return lifetime and mutability must fail even in unused functions.
use justrust_engine::compiler;

#[test]
fn forged_slice_result_kind_and_element_are_rejected() {
    use justrust_engine::compiler::{cfg, frontend, ir};
    use std::sync::Arc;
    let program = frontend::parse("fn same(s:&[i32])->&[i32]{s}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = (0..plan.callable_count())
        .map(|i| plan.callable_decl(i).unwrap())
        .find(|d| {
            let f = plan.callable_body(&program, *d).unwrap();
            f.name.len() == 1 && program.names.spelling(f.name.segments()[0]).unwrap() == "same"
        })
        .unwrap();
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let template = cfg::lower_template(Arc::new(checked.clone()), facts.clone(), &budget).unwrap();
    for replacement in [
        ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(ir::Ty::Slice {
                element: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
            }),
        },
        ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Slice {
                element: Arc::new(ir::Ty::Bool),
            }),
        },
    ] {
        let mut forged = checked.clone();
        forged.result = replacement.clone();
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err()
        );
        let mut forged_cfg = template.function().as_ref().clone();
        forged_cfg.result = replacement;
        assert!(
            forged_cfg
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err()
        );
    }
}

#[test]
fn returned_slices_do_not_escape_storage_or_authorize_aliasing() {
    for source in [
        "fn bad<'a>()->&'a[i32]{let a=[1,2];&a}fn main(){}",
        "fn bad<'a>()->&'a mut[i32]{let mut a=[1,2];&mut a}fn main(){}",
        "fn same(s:&mut[i32])->&mut[i32]{s}fn main(){let mut a=[1,2];let r=same(&mut a);a[0]=7;let n=r.len();println!(\"{}\",n);}",
        "fn bad<'a,'b>(s:&'a[i32])->&'b[i32]{s}fn main(){}",
        "fn bad(s:&[i32])->&mut[i32]{s}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let error = match compiler::compile(source.into(), workers, check) {
                    Ok(_) => panic!("accepted workers={workers}, check={check}: {source}"),
                    Err(error) => error.to_string(),
                };
                assert!(
                    !error.to_ascii_lowercase().contains("unsupported"),
                    "invalid source hit unsupported boundary instead: {error}"
                );
            }
        }
    }
}
