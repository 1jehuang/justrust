//! Reject malformed raw descriptor construction and preserve source authority.
use justrust_engine::compiler::{self, cfg, frontend, ir};
use std::sync::Arc;

#[test]
fn raw_constructor_metadata_is_not_mutability_or_origin_authority() {
    let program=frontend::parse("unsafe fn raw<'a>(p:*const i32,n:usize,q:*const i32,m:usize)->&'a[i32]{let first=unsafe{__justrust_slice_from_raw_parts!(p,n)};unsafe{__justrust_slice_from_raw_parts!(q,m)}}unsafe fn other<'a>(p:*const i32,n:usize)->&'a[i32]{unsafe{__justrust_slice_from_raw_parts!(p,n)}}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = (0..plan.callable_count())
        .map(|i| plan.callable_decl(i).unwrap())
        .find(|d| {
            let function = plan.callable_body(&program, *d).unwrap();
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == "raw"
        })
        .unwrap();
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let index = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::RawSlice { .. }))
        .unwrap();
    for mode in 0..3 {
        let mut forged = checked.clone();
        let ir::ExprKind::RawSlice { kind, args } = &mut forged.exprs[index].kind else {
            unreachable!()
        };
        match mode {
            0 => *kind = ir::BorrowKind::Mut,
            1 => {
                args.pop();
            }
            _ => args.swap(0, 1),
        }
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err(),
            "IR mode{mode}"
        );
    }
    let template = cfg::lower_template(Arc::new(checked), facts.clone(), &budget).unwrap();
    let other = (0..plan.callable_count())
        .map(|i| plan.callable_decl(i).unwrap())
        .find(|d| {
            let f = plan.callable_body(&program, *d).unwrap();
            f.name.len() == 1 && program.names.spelling(f.name.segments()[0]).unwrap() == "other"
        })
        .unwrap();
    let other_checked = frontend::type_callable_with_plan(&program, other, &plan, &budget).unwrap();
    let other_facts = plan.callable_facts(other).unwrap();
    let other_template =
        cfg::lower_template(Arc::new(other_checked), other_facts, &budget).unwrap();
    let raw_sites: Vec<_> = template
        .function()
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(b, block)| {
            block
                .statements
                .iter()
                .enumerate()
                .filter_map(move |(s, located)| {
                    matches!(
                        located.value,
                        cfg::Statement::Assign {
                            value: cfg::Rvalue::RawSlice { .. },
                            ..
                        }
                    )
                    .then_some((b, s))
                })
        })
        .collect();
    assert_eq!(raw_sites.len(), 2);
    for mode in 0..8 {
        let mut forged = template.function().as_ref().clone();
        let (other_block, other_statement) = raw_sites[1];
        let other_site = forged.blocks[other_block].statements[other_statement].clone();
        let cfg::Statement::Assign {
            value: cfg::Rvalue::RawSlice {
                args: other_args, ..
            },
            ..
        } = &other_site.value
        else {
            unreachable!()
        };
        if mode == 6 {
            forged.lowering_proof = None;
        }
        if mode == 7 {
            forged.lowering_proof = other_template.function().lowering_proof.clone();
        }
        let located = forged
            .blocks
            .iter_mut()
            .flat_map(|b| b.statements.iter_mut())
            .find(|s| {
                matches!(
                    s.value,
                    cfg::Statement::Assign {
                        value: cfg::Rvalue::RawSlice { .. },
                        ..
                    }
                )
            })
            .unwrap();
        if mode == 2 {
            located.origin = None;
        } else if mode == 3 {
            located.origin = other_site.origin;
        } else {
            let cfg::Statement::Assign {
                value: cfg::Rvalue::RawSlice { kind, args },
                ..
            } = &mut located.value
            else {
                unreachable!()
            };
            match mode {
                0 => *kind = ir::BorrowKind::Mut,
                1 => {
                    args.pop();
                }
                4 => args[0] = other_args[0].clone(),
                5 => args[1] = other_args[1].clone(),
                _ => {}
            }
        }
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err(),
            "CFG mode{mode}"
        );
    }
}

#[test]
fn unsafe_pointer_and_length_requirements_are_checked() {
    for source in [
        "fn bad<'a>(p:*const i32,n:usize)->&'a[i32]{__justrust_slice_from_raw_parts!(p,n)}fn main(){}",
        "unsafe fn bad<'a>(p:*const i32,n:usize)->&'a mut[i32]{unsafe{__justrust_slice_from_raw_parts_mut!(p,n)}}fn main(){}",
        "unsafe fn bad<'a>(p:*const i32)->&'a[i32]{unsafe{__justrust_slice_from_raw_parts!(p,true)}}fn main(){}",
        "fn view(p:&*const i32)->&[i32]{unsafe{std::slice::from_raw_parts::<i32>(*p,0)}}fn bad<'a>()->&'a[i32]{let p=8usize as *const i32;view(&p)}fn main(){}",
        "fn view(p:&mut *mut i32)->&mut[i32]{unsafe{std::slice::from_raw_parts_mut::<i32>(*p,0)}}fn main(){let mut p=8usize as *mut i32;let a=view(&mut p);let b=view(&mut p);let n=a.len()+b.len();println!(\"{}\",n);}",
    ] {
        for workers in [1, 8] {
            let error = compiler::compile(source.into(), workers, true)
                .err()
                .expect("invalid constructor accepted")
                .to_string();
            assert!(
                !error.to_ascii_lowercase().contains("unsupported"),
                "{error}"
            );
        }
    }
}
