//! Mutable range authority and source-lifetime checks.
use justrust_engine::compiler::{self, frontend, ir};

#[test]
fn mutable_ranges_reject_alias_escape_and_strengthening() {
    for source in [
        "fn bad(s:&[i32]){let r=&mut s[..];r[0]=1;}fn main(){}",
        "fn bad<'a>()->&'a mut[i32]{let mut a=[1,2];&mut a[..]}fn main(){}",
        "fn main(){let mut a=[1,2,3];let x=&mut a[..2];let y=&mut a[1..];x[0]=8;y[0]=9;}",
        "fn main(){let mut a=[1,2,3];let x=&a[..2];let y=&mut a[1..];let n=x.len();y[0]=9;println!(\"{}\",n);}",
        "fn bad(s:&mut[i32]){let moved=s;let r=&mut s[..];r[0]=1;let n=moved.len();}fn main(){}",
        "fn bad(s:&mut[i32]){let r=&mut s[true..];}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let error = match compiler::compile(source.into(), workers, check) {
                    Ok(_) => panic!("invalid source accepted: {source}"),
                    Err(error) => error.to_string(),
                };
                assert!(
                    !error.to_ascii_lowercase().contains("unsupported"),
                    "{source}: {error}"
                );
            }
        }
    }
}

#[test]
fn mutable_range_rejects_structurally_wrong_result_kind() {
    let source =
        "fn view<'a>(s:&'a mut[i32],lo:usize,hi:usize)->&'a mut[i32]{&mut s[lo..hi]}fn main(){}";
    let program = frontend::parse(source).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = (0..plan.callable_count())
        .map(|i| plan.callable_decl(i).unwrap())
        .find(|d| {
            let f = plan.callable_body(&program, *d).unwrap();
            f.name.len() == 1 && program.names.spelling(f.name.segments()[0]).unwrap() == "view"
        })
        .unwrap();
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    let index = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::Subslice { .. }))
        .unwrap();
    // Structural validation checks kind/layout, while exact source bound
    // identity is tested through the canonical publication in the unit suite.
    let mut forged = checked.clone();
    let ir::Ty::Ref { kind, .. } = &mut forged.exprs[index].ty else {
        unreachable!()
    };
    *kind = ir::BorrowKind::Shared;
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
}
