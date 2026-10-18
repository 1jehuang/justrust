use super::*;

fn checked(source: &str, index: usize) -> ir::CheckedFunction {
    let program = parse(source).unwrap();
    let signatures = signature_table(&program).unwrap();
    let checked = type_function_with_signatures(&program, index, &signatures).unwrap();
    checked.validate().unwrap();
    checked
}

#[test]
fn char_literals_types_aggregates_and_scalar_comparisons() {
    let body = checked(
        r#"struct Pair{a:char,b:char}fn identity(x:char)->char{x}
        fn main(){let a:char='水';let b=identity('\u{1_F980}');
        let p=Pair{a:a,b:b};let t=(p.a,p.b);let values=[t.0,t.1];
        let same=values[0usize]=='水';let order=a<b;let other=a!=b;
        let le=a<=b;let gt=b>a;let ge=b>=a;}"#,
        1,
    );
    assert_eq!(body.locals[0].ty, ir::Ty::Char);
    assert_eq!(body.locals[1].ty, ir::Ty::Char);
    assert!(body.exprs.iter().any(|expression| {
        matches!(expression.kind, ir::ExprKind::Char(value) if value.value == '水' as u32)
    }));
    assert_eq!(
        body.exprs
            .iter()
            .filter(|expression| { matches!(expression.kind, ir::ExprKind::Binary { .. }) })
            .count(),
        6
    );
}

#[test]
fn char_tokens_keep_source_identity_and_macro_literal_transport() {
    let source = r#"fn main(){macro_rules! relay{($x:literal)=>{$x}}
        let scalar:char=relay!('\u{10ffff}');}"#;
    let program = parse(source).unwrap();
    program.validate_name_handles().unwrap();
    preflight_program_clone(&program, &ir::PlanBudget::default()).unwrap();
    let statement = &program.functions[0].body.statements[0];
    assert!(matches!(
        statement,
        Stmt::Let {
            ty: Type::Char,
            value: Expr::Char('\u{10ffff}'),
            ..
        }
    ));
    recipes::inventory(&program, &ir::PlanBudget::default()).unwrap();
    checked(source, 0);
}

#[test]
fn char_is_not_an_integer_or_arithmetic_domain() {
    for expression in [
        "'a' == 97",
        "97 == 'a'",
        "'a' < 97",
        "97 > 'a'",
        "'a' + 'b'",
        "'a' - 'b'",
        "'a' * 'b'",
        "'a' / 'b'",
        "'a' % 'b'",
        "'a' & 'b'",
        "'a' | 'b'",
        "'a' ^ 'b'",
        "'a' << 1",
        "!'a'",
        "-'a'",
    ] {
        let source = format!("fn main(){{let bad={expression};}}");
        let program = parse(&source).unwrap();
        let signatures = signature_table(&program).unwrap();
        assert!(
            type_function_with_signatures(&program, 0, &signatures).is_err(),
            "{expression}"
        );
    }
}

#[test]
fn char_wildcard_binding_patterns_and_repeat_are_admitted() {
    checked(
        "fn identity(x:char)->char{match x{value=>value}}fn main(){let values=['水';2];let [first,..]=values;let _=identity(first);}",
        1,
    );
    // Character literal patterns are admitted since SCALAR-PATTERNS.md.
    parse_syntax_unprepared("fn main(){let x='a';match x{'a'=>{},_=>{}}}").unwrap();
}

#[test]
fn malformed_chars_keep_lexical_syntax_diagnostics() {
    for literal in [
        "''",
        "'ab'",
        r"'\u{d800}'",
        r"'\u{110000}'",
        r"'\u{_1}'",
        r"'\u{}'",
        r"'\u{0000000}'",
        r"'\x80'",
        r"'\q'",
        "'a'u32",
    ] {
        let source = format!("fn main(){{let x={literal};}}");
        let error = parse_syntax_unprepared(&source).err().unwrap();
        assert_eq!(
            error.kind,
            super::super::source::SourceErrorKind::Syntax,
            "{literal}: {error}"
        );
        assert!(error.has_location, "{literal}: {error}");
    }
}

#[test]
fn character_casts_keep_exact_types_and_scalar_truncation() {
    for (name, ty) in [
        ("u8", ir::IntTy::U8),
        ("u16", ir::IntTy::U16),
        ("u32", ir::IntTy::U32),
        ("u64", ir::IntTy::U64),
        ("u128", ir::IntTy::U128),
        ("usize", ir::IntTy::Usize),
        ("i8", ir::IntTy::I8),
        ("i16", ir::IntTy::I16),
        ("i32", ir::IntTy::I32),
        ("i64", ir::IntTy::I64),
        ("i128", ir::IntTy::I128),
        ("isize", ir::IntTy::Isize),
    ] {
        for scalar in [0u32, 0x7f, 0xff, 0x100, 0x6c34, 0x1f980, 0x10ffff] {
            let source = format!("fn f()->{name}{{'\\u{{{scalar:x}}}' as {name}}}fn main(){{}}");
            let body = checked(&source, 0);
            let root = body.body.tail.unwrap();
            let ir::ExprKind::CharToInt { value, to } = body.exprs[root.0].kind else {
                panic!("character conversion was not preserved: {source}");
            };
            assert_eq!(to, ty);
            assert_eq!(body.exprs[root.0].ty, ir::Ty::Int(ty));
            assert_eq!(body.exprs[value.0].ty, ir::Ty::Char);
            let CountValue::Known(value) = CountEvaluator::new(&body.exprs)
                .unwrap()
                .evaluate(root)
                .unwrap()
            else {
                panic!("constant character cast was not evaluated");
            };
            assert_eq!(value.ty, ty);
            assert_eq!(value.bits, u128::from(scalar) & ty.mask());
        }
    }
    for byte in [0u8, 127, 128, 255] {
        let body = checked(&format!("fn f()->char{{{byte}u8 as char}}fn main(){{}}"), 0);
        let root = body.body.tail.unwrap();
        assert!(matches!(
            body.exprs[root.0].kind,
            ir::ExprKind::U8ToChar { .. }
        ));
        assert!(matches!(
            CountEvaluator::new(&body.exprs).unwrap().evaluate(root).unwrap(),
            CountValue::Char(value) if value == u32::from(byte)
        ));
    }
}

#[test]
fn character_cast_rejections_do_not_acquire_numeric_coercions() {
    for expression in [
        "0 as char",
        "0i8 as char",
        "0i16 as char",
        "0i32 as char",
        "0i64 as char",
        "0i128 as char",
        "0isize as char",
        "0u16 as char",
        "0u32 as char",
        "0u64 as char",
        "0u128 as char",
        "0usize as char",
        "0.0f32 as char",
        "0.0f64 as char",
        "true as char",
        "'a' as char",
        "'a' as f32",
        "'a' as f64",
        "'a' as bool",
        "'a' as &u8",
        "{let x=0u8; (&x) as char}",
        "{let x='a'; (&x) as u32}",
    ] {
        for source in [
            format!("fn main(){{let _={expression};}}"),
            format!("fn main(){{return;let _={expression};}}"),
        ] {
            let program = parse(&source).unwrap();
            let signatures = signature_table(&program).unwrap();
            assert!(
                type_function_with_signatures(&program, 0, &signatures).is_err(),
                "invalid live/dead cast accepted: {source}"
            );
        }
    }
}

#[test]
fn character_casts_evaluate_operands_once_and_preserve_dead_completion() {
    for (source, index, result) in [
        (
            "fn source()->char{'水'}fn f()->u8{source() as u8}fn main(){}",
            1,
            ir::Ty::Int(ir::IntTy::U8),
        ),
        (
            "fn source()->u8{255u8}fn f()->char{source() as char}fn main(){}",
            1,
            ir::Ty::Char,
        ),
    ] {
        let body = checked(source, index);
        assert_eq!(
            body.exprs
                .iter()
                .filter(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        assert_eq!(body.exprs[body.body.tail.unwrap().0].ty, result);
    }
    for source in [
        "fn f()->u8{({return 1u8;'水'}) as u8}fn main(){}",
        "fn f()->char{({return 'a';255u8}) as char}fn main(){}",
    ] {
        let body = checked(source, 0);
        let root = body.body.tail.unwrap();
        let cast = &body.exprs[root.0];
        let value = match cast.kind {
            ir::ExprKind::CharToInt { value, .. } | ir::ExprKind::U8ToChar { value } => value,
            _ => panic!("missing dead character cast"),
        };
        assert_eq!(cast.completion, ir::Completion::Never);
        assert_eq!(body.exprs[value.0].completion, ir::Completion::Never);
        assert!(matches!(
            CountEvaluator::new(&body.exprs)
                .unwrap()
                .evaluate(root)
                .unwrap(),
            CountValue::Dynamic
        ));
    }
}

#[test]
fn character_cast_metadata_rejects_forged_domains_and_completion() {
    for source in [
        "fn f()->u16{'水' as u16}fn main(){}",
        "fn f()->char{255u8 as char}fn main(){}",
    ] {
        let body = checked(source, 0);
        let root = body.body.tail.unwrap();
        let value = match body.exprs[root.0].kind {
            ir::ExprKind::CharToInt { value, .. } | ir::ExprKind::U8ToChar { value } => value,
            _ => unreachable!(),
        };
        let mut bad = body.clone();
        bad.exprs[root.0].ty = ir::Ty::Bool;
        assert!(bad.validate().is_err());
        let mut bad = body.clone();
        bad.exprs[value.0].ty = ir::Ty::Bool;
        assert!(bad.validate().is_err());
        let mut bad = body;
        bad.exprs[root.0].completion = ir::Completion::Never;
        assert!(bad.validate().is_err());
    }
}

#[test]
fn character_cast_static_shift_counts_use_truncated_values() {
    checked(
        "fn main(){let _=1u8<<('\\u{100}' as u8);let _=1u8<<(7u8 as char as u8);}",
        0,
    );
    for expression in ["1u8<<('水' as u8)", "1u8<<(8u8 as char as u8)"] {
        let program = parse(&format!("fn main(){{let _={expression};}}")).unwrap();
        let signatures = signature_table(&program).unwrap();
        assert!(type_function_with_signatures(&program, 0, &signatures).is_err());
    }
}

#[test]
fn character_casts_obey_frontend_and_count_resource_limits() {
    let program = parse("fn main(){let _='水' as u8;let _=255u8 as char;}").unwrap();
    recipes::inventory(&program, &ir::PlanBudget::default()).unwrap();
    for limits in [
        ir::PlanLimits {
            work: 0,
            ..Default::default()
        },
        ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        },
    ] {
        let budget = ir::PlanBudget::new(limits);
        assert!(recipes::inventory(&program, &budget).is_err());
    }
    let body = checked("fn f()->u8{'水' as u8}fn main(){}", 0);
    let root = body.body.tail.unwrap();
    let mut evaluator = CountEvaluator::new(&body.exprs).unwrap();
    evaluator.work = COUNT_EVAL_WORK;
    assert!(evaluator.evaluate(root).unwrap_err().contains("work"));

    let mut exprs = vec![ir::TypedExpr {
        kind: ir::ExprKind::Char(ir::CharValue { value: 255 }),
        ty: ir::Ty::Char,
        completion: ir::Completion::Returns,
    }];
    for index in 0..130 {
        let (kind, ty) = if index % 2 == 0 {
            (
                ir::ExprKind::CharToInt {
                    value: ir::ExprId(index),
                    to: ir::IntTy::U8,
                },
                ir::Ty::Int(ir::IntTy::U8),
            )
        } else {
            (
                ir::ExprKind::U8ToChar {
                    value: ir::ExprId(index),
                },
                ir::Ty::Char,
            )
        };
        exprs.push(ir::TypedExpr {
            kind,
            ty,
            completion: ir::Completion::Returns,
        });
    }
    let mut evaluator = CountEvaluator::new(&exprs).unwrap();
    evaluator.evaluate(ir::ExprId(100)).unwrap();
    assert!(
        evaluator
            .evaluate(ir::ExprId(130))
            .unwrap_err()
            .contains("depth")
    );
}
