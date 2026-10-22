use justrust_engine::compiler::{self, cfg, ir};

#[test]
fn array_len_uses_real_borrow_and_unsize_without_codegen() {
    let source = "fn f(a:&mut[i32;2])->usize{a.len()}fn main(){let mut a=[1,2];let n=f(&mut a);}";
    for workers in [1, 8] {
        let result = compiler::compile(source.into(), workers, true).unwrap();
        assert!(result.executable.is_none());
        assert!(result.report.capture_complete);
        assert!(!result.report.published_kind(103));
        assert!(result.report.published_kind(112));
        assert!(!result.report.published_kind(113));
        // CHECK reports SOURCE_ADMISSION CFGs (606e7aed7) but never demands LOWER.
        assert!(!result.report.published_kind(108));
        let result = compiler::compile(source.into(), workers, false).unwrap();
        assert!(result.executable.is_some());
        assert!(result.analyses.iter().all(|a| a.passed()));
        assert!(
            result
                .control_flow
                .iter()
                .flat_map(|f| &f.loan_sites)
                .any(|loan| loan.kind == ir::BorrowKind::Shared
                    && loan.origin.is_some()
                    && matches!(loan.place.projection.last(), Some(ir::Projection::Deref)))
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
                        value: cfg::Rvalue::ArrayToSlice { .. },
                        ..
                    }
                ))
        );
    }
}

#[test]
fn explicit_mutable_array_receiver_keeps_both_loan_origins() {
    let source =
        "fn f()->usize{let mut a=[1,2];let n=(&mut a).len();a[0]=3;n}fn main(){let n=f();}";
    for workers in [1, 8] {
        let result = compiler::compile(source.into(), workers, true).unwrap();
        assert!(result.executable.is_none());
        assert!(!result.report.published_kind(103));
        assert!(result.report.published_kind(112));
        assert!(!result.report.published_kind(113));
        // CHECK reports SOURCE_ADMISSION CFGs (606e7aed7) but never demands LOWER.
        assert!(!result.report.published_kind(108));
        let result = compiler::compile(source.into(), workers, false).unwrap();
        assert!(result.executable.is_some());
        assert!(result.analyses.iter().all(|a| a.passed()));
        let loans: Vec<_> = result
            .control_flow
            .iter()
            .flat_map(|f| &f.loan_sites)
            .collect();
        assert!(
            loans
                .iter()
                .any(|loan| loan.kind == ir::BorrowKind::Mut && loan.origin.is_some())
        );
        assert!(loans.iter().any(|loan| loan.kind == ir::BorrowKind::Shared
            && loan.origin.is_some()
            && matches!(loan.place.projection.last(), Some(ir::Projection::Deref))));
    }
}
