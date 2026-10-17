use justrust_engine::compiler;

#[test]
fn labeled_blocks_and_outer_exits_share_validated_cfg_facts() {
    let source = "struct Token{x:i32}fn take()->i32{let token=Token{x:7};let moved='out: loop {let local=token; 'inner: loop {break 'out local;}};moved.x}fn main(){let mut n=1;let value='exit:{let token=Token{x:9};let r=&n;if *r==1{break 'exit token;}token};n=2;if value.x==9&&n==2&&take()==7{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.into(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 2);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for cfg in &compiled.control_flow {
                cfg.validate().unwrap();
            }
        }
    }
}

#[test]
fn labeled_transfers_do_not_bypass_move_or_loan_checks() {
    for source in [
        "struct T{x:i32}fn bad(){let token=T{x:1};let moved='exit:{break 'exit token;};let x=token.x;}fn main(){}",
        "fn bad(){let mut x=1;let r=&x;let n='exit:{x=2;break 'exit 7;};let old=*r;}fn main(){}",
        "fn bad(){let mut x=1;let r=&x;'out: loop {x=2;break 'out;}let old=*r;}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
                assert!(
                    failure
                        .analyses
                        .iter()
                        .any(|(_, analysis)| !analysis.passed()),
                    "rejection must reach ownership analysis: {}",
                    failure.error
                );
            }
        }
    }
}

#[test]
fn contextual_float_break_values_keep_declared_width() {
    let source = "fn value()->f32{'outer: loop {break 'outer 1.5;}}fn main(){let a:f32='block:{break 'block 2.5;};let b:f32=loop{break 3.5;};if a+b+value()==7.5f32{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.into(), workers, check).unwrap();
            assert_eq!(
                compiled.control_flow[0].result,
                compiler::ir::Ty::Float(compiler::ir::FloatTy::F32)
            );
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        }
    }
}
