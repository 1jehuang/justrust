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
    let error = parse_syntax_unprepared("fn main(){let x='a';match x{'a'=>{},_=>{}}}")
        .err()
        .unwrap();
    assert_eq!(
        error.kind,
        super::super::source::SourceErrorKind::Unsupported
    );
    assert!(error.message.contains("character literal"));
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
