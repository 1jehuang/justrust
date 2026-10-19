use justrust_engine::compiler::{self, cfg, ir};

#[test]
fn mutable_slice_checks_use_per_function_facts_without_codegen() {
    let source = "fn bump(s:&mut[i32]){s[0]=4;s[1]+=2;}fn main(){let mut a=[1,2];bump(&mut a);}";
    for workers in [1, 8] {
        let result = compiler::compile(source.into(), workers, true).unwrap();
        assert!(result.executable.is_none());
        assert!(result.report.capture_complete);
        assert!(result.analyses.iter().all(|a| a.passed()));
        assert!(!result.report.published_kind(103));
        let indexed_writes = result.control_flow.iter().flat_map(|f| f.blocks.iter())
            .flat_map(|b| b.statements.iter())
            .filter(|s| matches!(&s.value, cfg::Statement::IndexAssign{array,..} | cfg::Statement::IndexAssignOp{array,..} if array.projection.last()==Some(&ir::Projection::Deref)))
            .count();
        assert_eq!(indexed_writes, 2);
    }
}
