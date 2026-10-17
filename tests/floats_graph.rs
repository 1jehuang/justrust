use justrust_engine::{Key, compiler};

#[test]
fn malformed_float_constants_and_cast_endpoints_fail_cfg_validation() {
    use compiler::{cfg, ir};
    let compiled = compiler::compile("fn f()->f32{1.5f32}fn main(){}".into(), 1, true).unwrap();
    let original = compiled.control_flow[0].as_ref();
    let mut found = false;
    for (block, body) in original.blocks.iter().enumerate() {
        for (statement, located) in body.statements.iter().enumerate() {
            if let cfg::Statement::Assign {
                value: cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::Float(_))),
                ..
            } = &located.value
            {
                found = true;
                let mut invalid = original.clone();
                if let cfg::Statement::Assign { value, .. } =
                    &mut invalid.blocks[block].statements[statement].value
                {
                    *value = cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::Float(
                        ir::FloatValue {
                            ty: ir::FloatTy::F32,
                            bits: 1u64 << 32,
                        },
                    )));
                }
                assert!(invalid.validate().unwrap_err().contains("noncanonical"));
                let mut invalid = original.clone();
                if let cfg::Statement::Assign { value, .. } =
                    &mut invalid.blocks[block].statements[statement].value
                {
                    *value = cfg::Rvalue::FloatCast {
                        value: cfg::Operand::Constant(cfg::Constant::Int(ir::IntValue::i32(1))),
                        to: ir::NumTy::Int(ir::IntTy::I32),
                    };
                }
                assert!(invalid.validate().unwrap_err().contains("float endpoint"));
            }
        }
    }
    assert!(found, "float literal missing from retained CFG");
}

#[test]
fn float_values_share_checked_cfg_and_ownership_facts() {
    let source = "fn widen(x:f32)->f64{x as f64}fn trunc(x:f64)->u64{x as u64}fn main(){let mut a=[1.5f32,2.5f32];a[1usize]+=0.5f32;if widen(a[1usize])==3.0f64&&trunc(-1.0f64)==0u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 3);
            assert_eq!(compiled.analyses.len(), 3);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(
                compiled.control_flow[0].result,
                compiler::ir::Ty::Float(compiler::ir::FloatTy::F64)
            );
            for index in 0..3 {
                for kind in [102, 108, 109] {
                    let key = Key::new(kind, vec![index]);
                    let evaluated: Vec<_> = compiled
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(actual, _)| *actual == key)
                        .collect();
                    assert_eq!(evaluated.len(), 1);
                    assert_eq!(evaluated[0].1, 1);
                }
            }
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                if check { 0 } else { 3 }
            );
        }
    }
}

#[test]
fn unused_float_loan_violation_blocks_publication() {
    let source = "fn bad(p:&mut f64){let s=&*p;*p=2.0f64;let n=*s as u64;}fn main(){println!(\"must not publish\");}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            assert!(
                failure
                    .analyses
                    .iter()
                    .any(|(_, analysis)| !analysis.passed())
            );
            assert!(failure.report.unwrap().capture_complete);
        }
    }
}

#[test]
fn float_typed_noncompletion_never_materializes_a_cast() {
    let source =
        "fn f()->f64{({return 7.0f64;0.0f32}) as f64}fn main(){if f()==7.0f64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(
                compiled.control_flow[0].result,
                compiler::ir::Ty::Float(compiler::ir::FloatTy::F64)
            );
        }
    }
}

#[test]
fn unused_invalid_float_operations_fail_in_check_and_build() {
    for source in [
        "fn bad()->f64{1.0f32}fn main(){}",
        "fn bad()->f32{1.0f32&2.0f32}fn main(){}",
        "fn bad()->f32{1.0f32<<2u8}fn main(){}",
        "fn bad()->f32{!1.0f32}fn main(){}",
        "fn bad()->u8{true as f64 as u8}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
            }
        }
    }
}
