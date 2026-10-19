//! Native format operands keep an ordinary checked borrow in the graph.
use justrust_engine::compiler::{self, cfg};

#[test]
fn indexed_format_check_retains_borrow_without_codegen() {
    let source = "fn main(){let a=[1,2];let s:&[i32]=&a;println!(\"{}\",s[0]);}";
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
                    loan.kind == justrust_engine::compiler::ir::BorrowKind::Shared
                        && matches!(
                            loan.place.projection.last(),
                            Some(justrust_engine::compiler::ir::Projection::DynamicIndex(_))
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
                        value: cfg::Rvalue::Borrow(_),
                        ..
                    }
                ))
        );
    }
}
