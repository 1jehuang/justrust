//! Scalar evaluator validation only. Compiler acceptance has separate source
//! workflow and graph integrity gates.
use justrust_engine::compiler::const_eval::*;
use justrust_engine::compiler::ir::{self, DeclId, IntTy, IntValue, PlanBudget, SnapshotId};
use std::{
    future::{Future, ready},
    pin::pin,
    task::{Context, Poll, Waker},
};

fn run<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("standalone test resolver must be immediately ready"),
    }
}
fn id(index: u32) -> DeclId {
    DeclId {
        snapshot: snapshot(),
        index,
    }
}
fn snapshot() -> SnapshotId {
    // IDs compared by a test are built from its explicitly supplied owner below.
    SnapshotId::allocate().unwrap()
}
fn int(ty: IntTy, bits: u128) -> ScalarValue {
    ScalarValue::Int(IntValue {
        ty,
        bits: bits & ty.mask(),
    })
}
fn lit(value: ScalarValue) -> ScalarExpr {
    ScalarExpr {
        ty: value.ty(),
        kind: ScalarExprKind::Literal(value),
    }
}
fn body(exprs: Vec<ScalarExpr>) -> ConstBodyFact {
    let root = exprs.len() - 1;
    ConstBodyFact::new(id(0), exprs, root, &PlanBudget::default()).unwrap()
}
fn eval(body: &ConstBodyFact) -> Result<ConstValueFact, ConstError> {
    run(evaluate(body, &PlanBudget::default(), |_| {
        ready(Err(ConstError::new(
            ErrorKind::InvalidMetadata,
            "unexpected reference",
        )))
    }))
}
fn bin(op: Binary, l: ScalarValue, r: ScalarValue) -> Result<ScalarValue, ConstError> {
    let ty = if matches!(
        op,
        Binary::Eq
            | Binary::Ne
            | Binary::Lt
            | Binary::Le
            | Binary::Gt
            | Binary::Ge
            | Binary::And
            | Binary::Or
    ) {
        ScalarTy::Bool
    } else {
        l.ty()
    };
    eval(&body(vec![
        lit(l),
        lit(r),
        ScalarExpr {
            ty,
            kind: ScalarExprKind::Binary {
                op,
                left: 0,
                right: 1,
            },
        },
    ]))
    .map(ConstValueFact::value)
}
fn unary(op: Unary, value: ScalarValue) -> Result<ScalarValue, ConstError> {
    eval(&body(vec![
        lit(value),
        ScalarExpr {
            ty: value.ty(),
            kind: ScalarExprKind::Unary { op, value: 0 },
        },
    ]))
    .map(ConstValueFact::value)
}

#[test]
fn original_node_inputs_are_typed_and_not_declaration_aliases() {
    let input = body(vec![ScalarExpr {
        ty: ScalarTy::Int(IntTy::U16),
        kind: ScalarExprKind::Input(37),
    }]);
    let evaluate = |value| {
        run(evaluate_with_inputs(
            &input,
            &PlanBudget::default(),
            |_| {
                ready(Err::<ConstValueFact, &'static str>(
                    "unexpected declaration read",
                ))
            },
            |node| {
                assert_eq!(node, 37);
                ready(Ok(value))
            },
        ))
    };
    assert_eq!(
        evaluate(Some(int(IntTy::U16, 513))).unwrap().value(),
        int(IntTy::U16, 513)
    );
    assert!(matches!(evaluate(None), Err(EvaluateError::Scalar(_))));
    assert!(matches!(
        evaluate(Some(int(IntTy::U8, 1))),
        Err(EvaluateError::Scalar(_))
    ));
    assert!(eval(&input).is_err());
    let failed = run(evaluate_with_inputs(
        &input,
        &PlanBudget::default(),
        |_| {
            ready(Err::<ConstValueFact, &'static str>(
                "unexpected declaration read",
            ))
        },
        |_| ready(Err("original member graph dependency failed")),
    ));
    assert!(matches!(
        failed,
        Err(EvaluateError::Dependency(
            "original member graph dependency failed"
        ))
    ));
}

#[test]
fn fixed_width_signed_and_unsigned_boundaries() {
    for ty in [IntTy::I8, IntTy::I16, IntTy::I32, IntTy::I64, IntTy::I128] {
        let sign = 1u128 << (ty.bits() - 1);
        let max = int(ty, sign - 1);
        let min = int(ty, sign);
        let minus_one = int(ty, ty.mask());
        let one = int(ty, 1);
        for result in [
            bin(Binary::Add, max, one),
            bin(Binary::Sub, min, one),
            bin(Binary::Div, min, minus_one),
            bin(Binary::Rem, min, minus_one),
            unary(Unary::Neg, min),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.kind, ErrorKind::Evaluation);
            assert_eq!(error.code(), Some("E0080"));
        }
        assert_eq!(bin(Binary::Add, min, one).unwrap(), int(ty, sign + 1));
        assert_eq!(unary(Unary::Neg, max).unwrap(), int(ty, sign + 1));
        assert_eq!(bin(Binary::Lt, min, max).unwrap(), ScalarValue::Bool(true));
    }
    for ty in [IntTy::U8, IntTy::U16, IntTy::U32, IntTy::U64, IntTy::U128] {
        let max = int(ty, ty.mask());
        let one = int(ty, 1);
        let zero = int(ty, 0);
        assert_eq!(
            bin(Binary::Add, max, one).unwrap_err().kind,
            ErrorKind::Evaluation
        );
        assert_eq!(
            bin(Binary::Sub, zero, one).unwrap_err().kind,
            ErrorKind::Evaluation
        );
        assert_eq!(
            bin(Binary::Mul, max, int(ty, 2)).unwrap_err().kind,
            ErrorKind::Evaluation
        );
        assert_eq!(unary(Unary::Not, zero).unwrap(), max);
        assert_eq!(bin(Binary::Sub, max, one).unwrap(), int(ty, ty.mask() - 1));
    }
}

#[test]
fn checked_i8_arithmetic_matches_bootstrap_integer_semantics() {
    let samples = [i8::MIN, -127, -64, -3, -1, 0, 1, 2, 3, 63, 126, i8::MAX];
    for l in samples {
        for r in samples {
            for (op, expected) in [
                (Binary::Add, l.checked_add(r)),
                (Binary::Sub, l.checked_sub(r)),
                (Binary::Mul, l.checked_mul(r)),
                (Binary::Div, l.checked_div(r)),
                (Binary::Rem, l.checked_rem(r)),
            ] {
                let actual = bin(
                    op,
                    int(IntTy::I8, l as u8 as u128),
                    int(IntTy::I8, r as u8 as u128),
                );
                match expected {
                    Some(value) => assert_eq!(
                        actual.unwrap(),
                        int(IntTy::I8, value as u8 as u128),
                        "{l} {op:?} {r}"
                    ),
                    None => assert_eq!(actual.unwrap_err().kind, ErrorKind::Evaluation),
                }
            }
        }
    }
}

#[test]
fn shifts_use_rhs_type_check_count_and_preserve_rust_bit_semantics() {
    assert_eq!(
        bin(Binary::Shl, int(IntTy::U8, 128), int(IntTy::U8, 1)).unwrap(),
        int(IntTy::U8, 0)
    );
    assert_eq!(
        bin(Binary::Shl, int(IntTy::I8, 64), int(IntTy::U8, 1)).unwrap(),
        int(IntTy::I8, 128)
    );
    assert_eq!(
        bin(Binary::Shl, int(IntTy::U8, 255), int(IntTy::U128, 1)).unwrap(),
        int(IntTy::U8, 254)
    );
    assert_eq!(
        bin(Binary::Shr, int(IntTy::I8, 128), int(IntTy::U16, 1)).unwrap(),
        int(IntTy::I8, 192)
    );
    assert_eq!(
        bin(
            Binary::Shr,
            int(IntTy::U128, u128::MAX),
            int(IntTy::I8, 127)
        )
        .unwrap(),
        int(IntTy::U128, 1)
    );
    for rhs in [
        int(IntTy::I8, 255),
        int(IntTy::U8, 8),
        int(IntTy::U128, u128::MAX),
    ] {
        assert_eq!(
            bin(Binary::Shl, int(IntTy::U8, 1), rhs).unwrap_err().code(),
            Some("E0080")
        );
    }
}

#[test]
fn comparisons_and_bitwise_boolean_and_integer_operations() {
    for (op, expected) in [
        (Binary::Eq, false),
        (Binary::Ne, true),
        (Binary::Lt, true),
        (Binary::Le, true),
        (Binary::Gt, false),
        (Binary::Ge, false),
    ] {
        assert_eq!(
            bin(op, int(IntTy::I128, u128::MAX), int(IntTy::I128, 0)).unwrap(),
            ScalarValue::Bool(expected)
        );
    }
    for (op, expected) in [
        (Binary::BitAnd, 0x08),
        (Binary::BitOr, 0xef),
        (Binary::BitXor, 0xe7),
    ] {
        assert_eq!(
            bin(op, int(IntTy::U8, 0xa9), int(IntTy::U8, 0x4e)).unwrap(),
            int(IntTy::U8, expected)
        );
    }
    for op in [
        Binary::And,
        Binary::Or,
        Binary::BitAnd,
        Binary::BitOr,
        Binary::BitXor,
        Binary::Eq,
        Binary::Ne,
        Binary::Lt,
        Binary::Le,
        Binary::Gt,
        Binary::Ge,
    ] {
        let expected = match op {
            Binary::And | Binary::BitAnd => false,
            Binary::Or | Binary::BitOr | Binary::BitXor | Binary::Ne | Binary::Lt | Binary::Le => {
                true
            }
            _ => false,
        };
        assert_eq!(
            bin(op, ScalarValue::Bool(false), ScalarValue::Bool(true)).unwrap(),
            ScalarValue::Bool(expected)
        );
    }
    assert_eq!(
        unary(Unary::Not, ScalarValue::Bool(false)).unwrap(),
        ScalarValue::Bool(true)
    );
}

#[test]
fn resolver_must_return_exact_declaration_and_type() {
    let snapshot = snapshot();
    let owner = DeclId { snapshot, index: 0 };
    let dependency = DeclId { snapshot, index: 1 };
    let leaf = ConstBodyFact::new(
        dependency,
        vec![lit(int(IntTy::U32, 42))],
        0,
        &PlanBudget::default(),
    )
    .unwrap();
    let value = eval(&leaf).unwrap();
    assert_eq!(leaf.declaration(), dependency);
    assert_eq!(leaf.ty(), ScalarTy::Int(IntTy::U32));
    assert_eq!(value.declaration(), dependency);
    let caller = ConstBodyFact::new(
        owner,
        vec![ScalarExpr {
            ty: ScalarTy::Int(IntTy::U32),
            kind: ScalarExprKind::Reference(dependency),
        }],
        0,
        &PlanBudget::default(),
    )
    .unwrap();
    let mut demanded = Vec::new();
    let result = run(evaluate(&caller, &PlanBudget::default(), |id| {
        demanded.push(id);
        ready(Ok(value))
    }))
    .unwrap();
    assert_eq!(demanded, [dependency]);
    assert_eq!(result.value(), int(IntTy::U32, 42));
    let wrong = eval(&body(vec![lit(int(IntTy::U32, 99))])).unwrap();
    assert_eq!(
        run(evaluate(&caller, &PlanBudget::default(), |_| ready(Ok(
            wrong
        ))))
        .unwrap_err()
        .kind,
        ErrorKind::InvalidMetadata
    );
    let wrong_type = ConstBodyFact::new(
        dependency,
        vec![lit(ScalarValue::Bool(true))],
        0,
        &PlanBudget::default(),
    )
    .unwrap();
    let wrong_type = eval(&wrong_type).unwrap();
    assert_eq!(
        run(evaluate(&caller, &PlanBudget::default(), |_| ready(Ok(
            wrong_type
        ))))
        .unwrap_err()
        .kind,
        ErrorKind::Type
    );
}

#[test]
fn short_circuit_skips_value_dependency_but_not_body_validation() {
    for (op, l) in [(Binary::And, false), (Binary::Or, true)] {
        let snapshot = snapshot();
        let owner = DeclId { snapshot, index: 0 };
        let other = DeclId { snapshot, index: 1 };
        let exprs = vec![
            lit(ScalarValue::Bool(l)),
            ScalarExpr {
                ty: ScalarTy::Bool,
                kind: ScalarExprKind::Reference(other),
            },
            ScalarExpr {
                ty: ScalarTy::Bool,
                kind: ScalarExprKind::Binary {
                    op,
                    left: 0,
                    right: 1,
                },
            },
        ];
        let caller = ConstBodyFact::new(owner, exprs.clone(), 2, &PlanBudget::default()).unwrap();
        assert_eq!(eval(&caller).unwrap().value(), ScalarValue::Bool(l));
        let mut invalid = exprs;
        invalid[1].kind = ScalarExprKind::RuntimeCall;
        let error = ConstBodyFact::new(owner, invalid, 2, &PlanBudget::default()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::NonConstCall);
        assert_eq!(error.code(), Some("E0015"));
    }
}

#[test]
fn malformed_arenas_and_cross_snapshot_refs_are_not_evaluation_errors() {
    let owner = id(0);
    let bad_ref = ScalarExpr {
        ty: ScalarTy::Bool,
        kind: ScalarExprKind::Reference(id(0)),
    };
    let bad_bits = ScalarExpr {
        ty: ScalarTy::Int(IntTy::U8),
        kind: ScalarExprKind::Literal(ScalarValue::Int(IntValue {
            ty: IntTy::U8,
            bits: 256,
        })),
    };
    let forward = ScalarExpr {
        ty: ScalarTy::Bool,
        kind: ScalarExprKind::Unary {
            op: Unary::Not,
            value: 0,
        },
    };
    for expr in [bad_ref, bad_bits, forward] {
        let error = ConstBodyFact::new(owner, vec![expr], 0, &PlanBudget::default()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidMetadata);
        assert_eq!(error.code(), None);
    }
    assert_eq!(
        ConstBodyFact::new(owner, vec![], 0, &PlanBudget::default())
            .unwrap_err()
            .kind,
        ErrorKind::InvalidMetadata
    );
    for kind in [
        ScalarExprKind::UnsupportedConstCall,
        // usize is admitted on the pinned 64-bit native target; isize is
        // the remaining target-sized Unsupported boundary.
        ScalarExprKind::Literal(ScalarValue::Int(IntValue {
            ty: IntTy::Isize,
            bits: 1,
        })),
    ] {
        let ty = match kind {
            ScalarExprKind::Literal(v) => v.ty(),
            _ => ScalarTy::Bool,
        };
        let error = ConstBodyFact::new(
            owner,
            vec![ScalarExpr { ty, kind }],
            0,
            &PlanBudget::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert_eq!(error.code(), None);
    }
}

#[test]
fn type_errors_are_distinct_from_ctfe_failures() {
    for exprs in [
        vec![
            lit(ScalarValue::Bool(true)),
            ScalarExpr {
                ty: ScalarTy::Bool,
                kind: ScalarExprKind::Unary {
                    op: Unary::Neg,
                    value: 0,
                },
            },
        ],
        vec![
            lit(int(IntTy::U8, 1)),
            ScalarExpr {
                ty: ScalarTy::Int(IntTy::U8),
                kind: ScalarExprKind::Unary {
                    op: Unary::Neg,
                    value: 0,
                },
            },
        ],
        vec![
            lit(int(IntTy::U8, 1)),
            lit(int(IntTy::U16, 1)),
            ScalarExpr {
                ty: ScalarTy::Int(IntTy::U8),
                kind: ScalarExprKind::Binary {
                    op: Binary::Add,
                    left: 0,
                    right: 1,
                },
            },
        ],
    ] {
        let root = exprs.len() - 1;
        assert_eq!(
            ConstBodyFact::new(id(0), exprs, root, &PlanBudget::default())
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
}

#[test]
fn task_budget_exhaustion_remains_resource_not_e0080() {
    let budget = PlanBudget::new(ir::PlanLimits {
        work: 0,
        ..Default::default()
    });
    let error =
        ConstBodyFact::new(id(0), vec![lit(ScalarValue::Bool(true))], 0, &budget).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Resource);
    assert_eq!(error.code(), None);
    let caller = body(vec![lit(ScalarValue::Bool(true))]);
    let error = run(evaluate(&caller, &budget, |_| {
        ready(Err(ConstError::new(
            ErrorKind::InvalidMetadata,
            "unexpected",
        )))
    }))
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Resource);
}

#[test]
fn deep_initializers_use_iterative_local_state() {
    let mut exprs = vec![lit(ScalarValue::Bool(true))];
    for i in 0..4096 {
        exprs.push(ScalarExpr {
            ty: ScalarTy::Bool,
            kind: ScalarExprKind::Unary {
                op: Unary::Not,
                value: i,
            },
        });
    }
    assert_eq!(eval(&body(exprs)).unwrap().value(), ScalarValue::Bool(true));
}

#[test]
fn body_admission_pays_incoming_capacity_and_arc_conversion_peak() {
    let bytes = 2 * std::mem::size_of::<ScalarExpr>() + 2 * std::mem::size_of::<usize>();
    let budget = || {
        PlanBudget::new(ir::PlanLimits {
            bytes,
            ..Default::default()
        })
    };
    ConstBodyFact::new(id(0), vec![lit(ScalarValue::Bool(true))], 0, &budget()).unwrap();
    let mut spare_capacity = Vec::with_capacity(64);
    spare_capacity.push(lit(ScalarValue::Bool(true)));
    let error = ConstBodyFact::new(id(0), spare_capacity, 0, &budget()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Resource);
    let too_small = PlanBudget::new(ir::PlanLimits {
        bytes: bytes - 1,
        ..Default::default()
    });
    let error =
        ConstBodyFact::new(id(0), vec![lit(ScalarValue::Bool(true))], 0, &too_small).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Resource);
}

#[test]
fn asynchronous_dependency_suspends_without_repeating_read() {
    let snapshot = snapshot();
    let owner = DeclId { snapshot, index: 0 };
    let dependency = DeclId { snapshot, index: 1 };
    let leaf = ConstBodyFact::new(
        dependency,
        vec![lit(int(IntTy::U8, 7))],
        0,
        &PlanBudget::default(),
    )
    .unwrap();
    let value = eval(&leaf).unwrap();
    let caller = ConstBodyFact::new(
        owner,
        vec![ScalarExpr {
            ty: ScalarTy::Int(IntTy::U8),
            kind: ScalarExprKind::Reference(dependency),
        }],
        0,
        &PlanBudget::default(),
    )
    .unwrap();
    let budget = PlanBudget::default();
    let mut calls = 0;
    let future = evaluate(&caller, &budget, |requested| {
        calls += 1;
        assert_eq!(requested, dependency);
        let mut first = true;
        std::future::poll_fn(move |cx| {
            if first {
                first = false;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(Ok(value))
            }
        })
    });
    {
        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        let Poll::Ready(result) = future.as_mut().poll(&mut cx) else {
            panic!("dependency did not resume");
        };
        assert_eq!(result.unwrap().value(), int(IntTy::U8, 7));
    }
    assert_eq!(calls, 1);
    fn immutable_graph_fact<T: Send + Sync>() {}
    immutable_graph_fact::<ConstBodyFact>();
    immutable_graph_fact::<ConstValueFact>();
}

#[test]
fn logical_short_circuit_skips_evaluation_errors_but_not_type_errors() {
    let exprs = vec![
        lit(ScalarValue::Bool(false)),
        lit(int(IntTy::U8, 1)),
        lit(int(IntTy::U8, 0)),
        ScalarExpr {
            ty: ScalarTy::Int(IntTy::U8),
            kind: ScalarExprKind::Binary {
                op: Binary::Div,
                left: 1,
                right: 2,
            },
        },
        ScalarExpr {
            ty: ScalarTy::Bool,
            kind: ScalarExprKind::Binary {
                op: Binary::Eq,
                left: 3,
                right: 1,
            },
        },
        ScalarExpr {
            ty: ScalarTy::Bool,
            kind: ScalarExprKind::Binary {
                op: Binary::And,
                left: 0,
                right: 4,
            },
        },
    ];
    assert_eq!(
        eval(&body(exprs.clone())).unwrap().value(),
        ScalarValue::Bool(false)
    );
    let mut invalid = exprs;
    invalid[3].ty = ScalarTy::Bool;
    assert_eq!(
        ConstBodyFact::new(id(0), invalid, 5, &PlanBudget::default())
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

#[test]
fn ctfe_edge_expectations_match_external_stock_oracle_only() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let scratch = Scratch(base.join(format!(
        "jr-const-eval-oracle-{}-{:?}",
        std::process::id(),
        snapshot()
    )));
    std::fs::create_dir_all(&scratch.0).unwrap();
    // This checks the semantic expectations against stock, not native compiler
    // const-item acceptance. No stock compiler is invoked by the evaluator.
    for (source, error) in [
        (
            "const X:u8=128u8<<1; const Y:i8=64i8<<1; const MIN:i8=-128; const _:()=assert!(X==0 && Y == -128 && MIN == -128); fn main(){}",
            None,
        ),
        (
            "const X:bool=false && (1u8/0==1); const Y:bool=true || (1u8/0==1); const _:()=assert!(!X && Y); fn main(){}",
            None,
        ),
        (
            "const fn id(x:u8)->u8{x} const X:u8=id(1); fn main(){}",
            None,
        ),
        ("const BAD:i8=i8::MIN/-1;fn main(){}", Some("E0080")),
        ("const BAD:i8=i8::MIN%-1;fn main(){}", Some("E0080")),
        ("const BAD:u8=255u8+1;fn main(){}", Some("E0080")),
        ("const BAD:u8=1u8<<8;fn main(){}", Some("E0080")),
        (
            "fn runtime()->bool{true}const BAD:bool=false&&runtime();fn main(){}",
            Some("E0015"),
        ),
    ] {
        let mut child = Command::new("rustc")
            .env_remove("RUSTC_BOOTSTRAP")
            .args([
                "--edition=2024",
                "--crate-name=const_scalar_oracle",
                "-Cpanic=abort",
                "--emit=metadata",
                "--color=never",
                "-Awarnings",
                "-",
            ])
            .arg("-o")
            .arg(scratch.0.join("oracle.rmeta"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("stock rustc required for CTFE edge oracle");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        match error {
            None => assert!(output.status.success(), "stock rejected {source}: {stderr}"),
            Some(code) => assert!(
                !output.status.success() && stderr.contains(code),
                "stock expected {code} for {source}: {stderr}"
            ),
        }
    }
}
