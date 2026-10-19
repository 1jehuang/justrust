use justrust_engine::compiler::{self, cfg};
#[test]
fn raw_slice_constructors_remain_per_function_facts_without_codegen() {
    let source = "unsafe fn shared<'a>(p:*const i32,n:usize)->&'a[i32]{unsafe{__justrust_slice_from_raw_parts!(p,n)}}unsafe fn unique<'a>(p:*mut i32,n:usize)->&'a mut[i32]{unsafe{__justrust_slice_from_raw_parts_mut!(p,n)}}fn main(){let s=unsafe{shared(8usize as *const i32,0)};let n=s.len();println!(\"{}\",n);let u=unsafe{unique(8usize as *mut i32,0)};let m=u.len();println!(\"{}\",m);}";
    for workers in [1, 8] {
        let output = compiler::compile(source.into(), workers, true).unwrap();
        assert!(output.executable.is_none());
        assert!(output.report.capture_complete);
        assert!(!output.report.published_kind(103));
        assert!(output.analyses.iter().all(|a| a.passed()));
        assert_eq!(
            output
                .control_flow
                .iter()
                .flat_map(|f| f.blocks.iter())
                .flat_map(|b| b.statements.iter())
                .filter(|s| matches!(
                    s.value,
                    cfg::Statement::Assign {
                        value: cfg::Rvalue::RawSlice { .. },
                        ..
                    }
                ))
                .count(),
            2
        );
    }
}
