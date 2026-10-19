//! Public malformed-IR controls for the primitive, including unused rows.
use justrust_engine::compiler::{cfg, ir};
use std::sync::Arc;

fn source() -> ir::CheckedFunction {
    ir::CheckedFunction {
        call_evidence: Arc::from([]),
        body_substitution: None,
        ref_contract: Default::default(),
        name: "syscall_authority".into(),
        params: vec![],
        result: ir::Ty::Int(ir::IntTy::Usize),
        locals: vec![],
        exprs: vec![
            ir::TypedExpr {
                ty: ir::Ty::Int(ir::IntTy::Usize),
                completion: ir::Completion::Returns,
                kind: ir::ExprKind::Int(ir::IntValue {
                    ty: ir::IntTy::Usize,
                    bits: 39,
                }),
            },
            ir::TypedExpr {
                ty: ir::Ty::Int(ir::IntTy::Usize),
                completion: ir::Completion::Returns,
                kind: ir::ExprKind::NativeSyscall(vec![ir::ExprId(0); 7]),
            },
        ],
        body: ir::Block {
            statements: vec![],
            tail: Some(ir::ExprId(1)),
        },
    }
}

#[test]
fn syscall_ir_rejects_wrong_arity_ids_types_and_completion_even_if_unused() {
    source().validate().unwrap();
    for used in [true, false] {
        for control in 0..5 {
            let mut invalid = source();
            if !used {
                invalid.body.tail = Some(ir::ExprId(0));
            }
            match control {
                0 => invalid.exprs[1].kind = ir::ExprKind::NativeSyscall(vec![ir::ExprId(0); 6]),
                1 => invalid.exprs[1].kind = ir::ExprKind::NativeSyscall(vec![ir::ExprId(99); 7]),
                2 => invalid.exprs[0].ty = ir::Ty::Bool,
                3 => invalid.exprs[1].ty = ir::Ty::Int(ir::IntTy::U64),
                _ => invalid.exprs[1].completion = ir::Completion::Never,
            }
            assert!(invalid.validate().is_err(), "used={used} control={control}");
        }
    }
}

#[test]
fn syscall_cfg_rejects_wrong_arity_and_operand_type() {
    let cfg = cfg::lower(&source()).unwrap();
    cfg.validate().unwrap();
    for control in 0..2 {
        let mut invalid = cfg.clone();
        let mut changed = false;
        for block in &mut invalid.blocks {
            for statement in &mut block.statements {
                if let cfg::Statement::Assign {
                    value: cfg::Rvalue::NativeSyscall(arguments),
                    ..
                } = &mut statement.value
                {
                    if control == 0 {
                        arguments.pop();
                    } else {
                        arguments[0] = cfg::Operand::Constant(cfg::Constant::Bool(true));
                    }
                    changed = true;
                }
            }
        }
        assert!(changed);
        assert!(invalid.validate().is_err(), "control={control}");
    }
}
