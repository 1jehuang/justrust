//! CHECK observes ordinary reborrow facts, never code generation.
use justrust_engine::compiler::{self, cfg};

#[test]
fn mutable_ranges_remain_per_function_checked_reborrows() {
    let source = "fn tail<'a>(s:&'a mut[i32])->&'a mut[i32]{&mut s[1..]}fn main(){let mut a=[1,2,3];let s=tail(&mut a);s[0]=7;}";
    for workers in [1, 8] {
        let result = compiler::compile(source.into(), workers, true).unwrap();
        assert!(result.executable.is_none());
        assert!(result.report.capture_complete);
        assert!(!result.report.published_kind(103));
        assert!(result.analyses.iter().all(|a| a.passed()));
        assert!(
            result
                .control_flow
                .iter()
                .flat_map(|f| &f.loan_sites)
                .any(|loan| {
                    loan.kind == justrust_engine::compiler::ir::BorrowKind::Mut
                        && matches!(
                            loan.place.projection.last(),
                            Some(justrust_engine::compiler::ir::Projection::Deref)
                        )
                        && loan.origin.is_some()
                })
        );
        assert!(
            result
                .control_flow
                .iter()
                .flat_map(|f| &f.blocks)
                .flat_map(|b| &b.statements)
                .any(|s| matches!(
                    s.value,
                    cfg::Statement::Assign {
                        value: cfg::Rvalue::Subslice { .. },
                        ..
                    }
                ))
        );
    }
}
