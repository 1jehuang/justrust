//! Stock-measured lexical classifications and checked native ingress tests.
//! Lexical token admission is not parser, macro expansion, or full Rust support.
//! Stock subprocesses below are an independent oracle, never a native fallback.
use justrust_engine::compiler::source::{
    FileId, OriginSegment, PreparedLexInput, PreparedMode, PreparedSource, SourceErrorKind,
    SourceSpan,
};
use justrust_engine::compiler::token_primitives::{
    BudgetFailureKind, BudgetResource, Counter, DocStyle, Edition, ExpansionBudget,
    ExpansionLimits, InputToken, LexError, LiteralKind, SourceFile, SourceMap, Spacing, TokenError,
    TokenKind, TokenSession, decode_normal_string, lex_prepared,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// First measured before native expectations on 2026-10-17. The actual compiler
// used by every rerun is printed independently, including hosted pinned nightly.
const BASELINE_STOCK: &str = "rustc 1.98.1; commit 48a229ceaefd4985c50990b14116b6d856af0985; 2026-09-01; x86_64-unknown-linux-gnu; LLVM 22.1.8";

struct Case {
    name: &'static str,
    source: &'static str,
    stock_valid: bool,
}

fn source_file(name: &str, text: &str) -> SourceFile {
    SourceFile {
        lexical_path: name.into(),
        contents: Arc::from(text),
        line_starts: std::iter::once(0)
            .chain(
                text.bytes()
                    .enumerate()
                    .filter_map(|(i, b)| (b == b'\n').then_some((i + 1) as u32)),
            )
            .collect(),
    }
}
fn prepared_identity(text: &str) -> PreparedSource {
    let mut prepared = PreparedSource::default();
    prepared.text = text.into();
    prepared.sources = SourceMap {
        files: vec![source_file("original.rs", text)],
    };
    prepared.origins = vec![OriginSegment {
        prepared: 0..text.len(),
        origin: SourceSpan {
            file: FileId(0),
            lo: 0,
            hi: text.len() as u32,
        },
        synthetic: false,
    }];
    prepared.source_bytes = text.len();
    prepared.files = 1;
    prepared
}

#[test]
fn identity_projection_keeps_unicode_crlf_and_eof_original_locations() {
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let input = PreparedLexInput::identity(
        Path::new("original/utf8.rs"),
        "// α🙂\r\nx",
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(input.mode(), PreparedMode::SyntaxOnly);
    let span = input.project(11..12).unwrap();
    let location = input.location(span).unwrap();
    assert_eq!(
        (location.path.as_path(), location.line, location.column),
        (Path::new("original/utf8.rs"), 2, 1)
    );
    let eof = input.render(input.eof().unwrap()).unwrap();
    assert_eq!((eof.lo, eof.hi), (12, 12));
    assert_eq!(
        input.project(4..5).unwrap_err().kind,
        SourceErrorKind::Contract
    );
    assert_eq!(
        input.project(12..13).unwrap_err().kind,
        SourceErrorKind::Contract
    );
    let other = PreparedLexInput::identity(
        Path::new("original/utf8.rs"),
        "// α🙂\r\nx",
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    assert_ne!(input.view_id(), other.view_id());
    assert_eq!(
        other.render(span).unwrap_err().kind,
        SourceErrorKind::Contract
    );
}

#[test]
fn raw_identity_does_not_apply_configuration_or_touch_paths() {
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let text = "#[cfg(any())] mod absent;\n/// exact docs\n";
    let input = PreparedLexInput::identity(
        Path::new("/does/not/exist/raw.rs"),
        text,
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(input.text(), text);
    assert_eq!(input.mode(), PreparedMode::SyntaxOnly);
    assert!(!input.is_synthetic(0..1).unwrap());
}

#[test]
fn prepared_projection_rejects_cross_file_logical_tokens() {
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let mut prepared = prepared_identity("abcd");
    prepared.sources.files = vec![source_file("left.rs", "ab"), source_file("right.rs", "cd")];
    prepared.files = 2;
    prepared.origins = vec![
        OriginSegment {
            prepared: 0..2,
            origin: SourceSpan {
                file: FileId(0),
                lo: 0,
                hi: 2,
            },
            synthetic: false,
        },
        OriginSegment {
            prepared: 2..4,
            origin: SourceSpan {
                file: FileId(1),
                lo: 0,
                hi: 2,
            },
            synthetic: false,
        },
    ];
    let input = PreparedLexInput::prepared(
        &prepared,
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(input.mode(), PreparedMode::Configured);
    assert_eq!(
        input.location(input.project(0..2).unwrap()).unwrap().path,
        Path::new("left.rs")
    );
    assert_eq!(
        input.location(input.project(2..4).unwrap()).unwrap().path,
        Path::new("right.rs")
    );
    assert_eq!(
        input.project(0..4).unwrap_err().kind,
        SourceErrorKind::Contract
    );
    let eof = input.render(input.eof().unwrap()).unwrap();
    assert_eq!((eof.file, eof.lo, eof.hi), (FileId(1), 2, 2));
}

#[test]
fn malformed_prepared_provenance_is_contract_not_lexical_syntax() {
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    for variant in 0..4 {
        let mut prepared = prepared_identity("abcd");
        match variant {
            0 => prepared.sources.files[0].contents = Arc::from("abxd"),
            1 => prepared.origins[0].origin.file = FileId(10),
            2 => prepared.origins[0].prepared = 1..4,
            _ => prepared.origins[0].origin.hi = 3,
        }
        let input = PreparedLexInput::prepared(
            &prepared,
            &budget,
            justrust_engine::compiler::owner_path::standalone_issued(
                &budget,
                justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            input.project(0..4).unwrap_err().kind,
            SourceErrorKind::Contract,
            "variant{variant}"
        );
        assert!(budget.check().is_ok());
    }
}

#[test]
fn synthetic_transport_is_an_explicit_anchor_not_original_spelling() {
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let mut prepared = prepared_identity(";");
    prepared.text = "{".into();
    prepared.origins[0].synthetic = true;
    let input = PreparedLexInput::prepared(
        &prepared,
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    let projected = input.projected(0..1).unwrap();
    assert!(projected.synthetic);
    let raw = input.render(projected.original).unwrap();
    assert_eq!((raw.lo, raw.hi), (0, 1));
    assert_eq!(
        input
            .snapshot()
            .text(input.snapshot().file(0).unwrap())
            .unwrap(),
        ";"
    );
}

#[test]
fn ingress_storage_failure_is_a_typed_limit_not_syntax() {
    let budget = ExpansionBudget::new(ExpansionLimits {
        bytes: 0,
        ..ExpansionLimits::default()
    });
    let error = PreparedLexInput::identity(
        Path::new("bounded.rs"),
        "x",
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind, SourceErrorKind::Limit);
    assert!(budget.check().is_err());
}

#[test]
fn native_lexical_categories_do_not_confuse_expression_suffix_semantics() {
    let mut accepted = 0;
    let mut rejected = 0;
    for case in CASES {
        let budget = ExpansionBudget::new(ExpansionLimits::default());
        let input = PreparedLexInput::identity(
            Path::new("corpus.rs"),
            case.source,
            &budget,
            justrust_engine::compiler::owner_path::standalone_issued(
                &budget,
                justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let mut session = TokenSession::standalone(input.snapshot().clone(), budget)
            .unwrap()
            .0;
        let lexical_valid = case.stock_valid
            || matches!(
                case.name,
                "expression_float_suffix" | "expression_integer_suffix"
            );
        match lex_prepared(&input, Edition::E2024, &mut session) {
            Ok(tokens) => {
                assert!(lexical_valid, "{} unexpectedly lexed", case.name);
                tokens.validate_input(&session, &input).unwrap();
                assert!(!tokens.events().is_empty());
                assert_eq!(tokens.eof().prepared, case.source.len()..case.source.len());
                accepted += 1;
            }
            Err(LexError::Source { kind, prepared, .. }) => {
                assert!(!lexical_valid, "{} unexpectedly rejected", case.name);
                assert_eq!(kind, SourceErrorKind::Syntax, "{}", case.name);
                assert!(
                    case.source.get(prepared).is_some(),
                    "{} split UTF8 diagnostic",
                    case.name
                );
                rejected += 1;
            }
            Err(error) => panic!("{}: wrong lexical failure category: {error:?}", case.name),
        }
    }
    assert_eq!((accepted, rejected), (23, 31));
}

#[test]
fn valid_lexical_classes_remain_explicit_unsupported_parser_ingress() {
    // These original stock-accepted sources are legal lexical inputs. A bounded
    // parser refusing them is an Unsupported boundary, not invalid Rust proof.
    let names = [
        "unicode_ident",
        "unicode_char",
        "raw_string",
        "raw_ident",
        "byte_hex",
        "bytestring",
        "raw_bytestring",
        "char",
        "cstring",
        "combining_continuation",
    ];
    for name in names {
        let case = CASES.iter().find(|case| case.name == name).unwrap();
        assert!(case.stock_valid);
        let prepared = prepared_identity(case.source);
        if matches!(
            name,
            "unicode_char" | "char" | "raw_string" | "byte_hex" | "bytestring" | "raw_bytestring"
        ) {
            justrust_engine::compiler::frontend::parse_prepared(&prepared)
                .unwrap_or_else(|error| panic!("{name}: character ingress rejected: {error}"));
            continue;
        }
        let error = justrust_engine::compiler::frontend::parse_prepared(&prepared)
            .err()
            .unwrap_or_else(|| panic!("{name}: unexpectedly admitted parser boundary"));
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{name}: {error}");
        assert!(
            error.primary.is_some(),
            "{name}: missing original source anchor"
        );
    }
}

#[test]
fn literal_spellings_suffix_ranges_and_raw_hashes_are_preserved() {
    let text = r###"r#type β á _ 1.5e+2f64 0xffu8 r##"x"## b'a' c"x" "a\n""###;
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let input = PreparedLexInput::identity(
        Path::new("literals.rs"),
        text,
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    let mut session = TokenSession::standalone(input.snapshot().clone(), budget.clone())
        .unwrap()
        .0;
    let tokens = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
    assert_eq!(tokens.events().len(), 10);
    let first = &tokens.events()[0];
    let InputToken::Leaf(token) = first.token else {
        panic!("expected raw identifier")
    };
    let TokenKind::Ident(ident) = token.kind else {
        panic!("expected identifier")
    };
    assert!(ident.raw);
    assert_eq!(session.symbol(ident.symbol).unwrap(), "type");
    for (index, kind, suffix) in [
        (4, LiteralKind::Float, "f64"),
        (5, LiteralKind::Integer, "u8"),
        (6, LiteralKind::RawString, ""),
        (7, LiteralKind::Byte, ""),
        (8, LiteralKind::CString, ""),
        (9, LiteralKind::String, ""),
    ] {
        let event = &tokens.events()[index];
        let InputToken::Leaf(token) = event.token else {
            panic!("expected literal")
        };
        let TokenKind::Literal {
            kind: actual,
            spelling,
        } = token.kind
        else {
            panic!("expected literal kind")
        };
        assert_eq!(actual, kind);
        assert_eq!(
            session.symbol(spelling).unwrap(),
            &text[event.prepared.clone()]
        );
        let info = event.literal.as_ref().unwrap();
        assert_eq!(&text[info.suffix.clone()], suffix);
        assert!(event.prepared.start <= info.body.start && info.body.end <= event.prepared.end);
    }
    assert_eq!(
        tokens.events()[6].literal.as_ref().unwrap().raw_hashes,
        Some(2)
    );
    assert_eq!(
        &text[tokens.events()[6].literal.as_ref().unwrap().body.clone()],
        "x"
    );
    let decoded = decode_normal_string(r#""a\n""#, &budget).unwrap();
    assert_eq!(decoded.as_str(), "a\n");
}

#[test]
fn spacing_lifetime_and_tuple_dot_transport_are_exact() {
    let text = "'a :: >>= + /* gap */ = .0.1";
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let input = PreparedLexInput::identity(
        Path::new("spacing.rs"),
        text,
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    let mut session = TokenSession::standalone(input.snapshot().clone(), budget)
        .unwrap()
        .0;
    let tokens = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
    let punctuation: Vec<_> = tokens
        .events()
        .iter()
        .filter_map(|event| match event.token {
            InputToken::Leaf(justrust_engine::compiler::token_primitives::Token {
                kind: TokenKind::Punct { ch, spacing },
                ..
            }) => Some((ch, spacing)),
            _ => None,
        })
        .collect();
    assert_eq!(
        punctuation,
        vec![
            ('\'', Spacing::Joint),
            (':', Spacing::Joint),
            (':', Spacing::Alone),
            ('>', Spacing::Joint),
            ('>', Spacing::Joint),
            ('=', Spacing::Alone),
            ('+', Spacing::Alone),
            ('=', Spacing::Alone),
            ('.', Spacing::Alone)
        ]
    );
    let tuple = tokens.events().last().unwrap();
    let InputToken::Leaf(token) = tuple.token else {
        panic!("expected tuple numeric atom")
    };
    assert!(matches!(
        token.kind,
        TokenKind::Literal {
            kind: LiteralKind::Float,
            ..
        }
    ));
    assert_eq!(&text[tuple.prepared.clone()], "0.1");
    assert_eq!(
        tuple.literal.as_ref().unwrap().decimal_point,
        Some(text.len() - 2)
    );
}

#[test]
fn documentation_is_audited_once_without_reintroducing_attributes() {
    let text = "/// α\r\n//! inner\n/** outer */\n/*! inner */\n//// ordinary\n/*** ordinary */\n/**/\nfn main(){}";
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let input = PreparedLexInput::identity(
        Path::new("docs.rs"),
        text,
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    let mut session = TokenSession::standalone(input.snapshot().clone(), budget.clone())
        .unwrap()
        .0;
    let tokens = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
    assert_eq!(tokens.docs().len(), 4);
    assert_eq!(tokens.docs().iter().filter(|doc| doc.inner).count(), 2);
    assert_eq!(
        tokens
            .docs()
            .iter()
            .filter(|doc| doc.style == DocStyle::Line)
            .count(),
        2
    );
    for doc in tokens.docs() {
        assert!(doc.prepared.start <= doc.body.start && doc.body.end <= doc.prepared.end);
        let original = session.original(doc.span, 2).unwrap();
        let raw = input.render(original).unwrap();
        assert_eq!(
            (raw.lo as usize, raw.hi as usize),
            (doc.prepared.start, doc.prepared.end)
        );
    }
    assert_eq!(&text[tokens.events()[0].prepared.clone()], "fn");
    assert_eq!(
        budget.snapshot().counters[Counter::InputTokens as usize],
        (tokens.events().len() + tokens.docs().len()) as u64
    );
}

#[test]
fn lexed_output_rejects_other_session_or_prepared_view() {
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let input = PreparedLexInput::identity(
        Path::new("same.rs"),
        "fn main(){}",
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    let mut session = TokenSession::standalone(input.snapshot().clone(), budget.clone())
        .unwrap()
        .0;
    let tokens = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
    let other_session = TokenSession::standalone(input.snapshot().clone(), budget.clone())
        .unwrap()
        .0;
    assert!(matches!(
        tokens.validate_input(&other_session, &input),
        Err(LexError::Transport(TokenError::ForeignHandle))
    ));
    let other_view = PreparedLexInput::identity(
        Path::new("same.rs"),
        "fn main(){}",
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        tokens.validate_input(&session, &other_view),
        Err(LexError::Projection(error)) if error.kind == SourceErrorKind::Contract
    ));
    let failure = budget.poison_allocation();
    assert_eq!(failure.kind, BudgetFailureKind::Allocation);
    assert_eq!(failure.resource, BudgetResource::Allocation);
    let error = tokens.validate_input(&session, &input).unwrap_err();
    let LexError::Projection(error) = error else {
        panic!("expected setup projection of allocation poison, got {error:?}")
    };
    assert_eq!(error.kind, SourceErrorKind::Infrastructure);
    assert_eq!(
        error.message,
        TokenError::Budget(failure.clone()).to_string()
    );
    assert!(error.primary.is_none());
    assert!(error.location.is_none());
    assert_eq!(budget.snapshot().failure, Some(failure));
}

#[test]
fn late_invalid_literal_beats_earlier_bad_projection() {
    let mut prepared = prepared_identity(r#"fn main(){ let _ = "ok"; let _ = "\q"; }"#);
    prepared.origins[0].origin.file = FileId(99);
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let input = PreparedLexInput::prepared(
        &prepared,
        &budget,
        justrust_engine::compiler::owner_path::standalone_issued(
            &justrust_engine::compiler::token_primitives::ExpansionBudget::default(),
            justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
        )
        .unwrap(),
    )
    .unwrap();
    let mut session = TokenSession::standalone(input.snapshot().clone(), budget)
        .unwrap()
        .0;
    assert!(matches!(
        lex_prepared(&input, Edition::E2024, &mut session),
        Err(LexError::Source {
            kind: SourceErrorKind::Syntax,
            ..
        })
    ));
}

#[test]
fn invalid_byte_unicode_has_full_character_original_child_span() {
    let text = "// α🙂\r\nfn main(){ let _ = b\"β\"; }";
    let mut prepared = prepared_identity(text);
    prepared.sources.files[0].lexical_path = "children/bad.rs".into();
    let error = justrust_engine::compiler::frontend::parse_prepared(&prepared)
        .err()
        .unwrap();
    assert_eq!(error.kind, SourceErrorKind::Syntax);
    let primary = error.primary.unwrap();
    assert_eq!(&text[primary.lo as usize..primary.hi as usize], "β");
    let location = error.location.unwrap();
    assert_eq!(
        (location.path.as_path(), location.line, location.column),
        (Path::new("children/bad.rs"), 2, 22)
    );
}

#[test]
fn guarded_strings_have_explicit_edition_specific_lexical_outcomes() {
    for text in ["#\"abc\"#", "##"] {
        for edition in [Edition::E2024, Edition::E2021] {
            let budget = ExpansionBudget::new(ExpansionLimits::default());
            let input = PreparedLexInput::identity(
                Path::new("edition.rs"),
                text,
                &budget,
                justrust_engine::compiler::owner_path::standalone_issued(
                    &budget,
                    justrust_engine::compiler::owner_path::OwnerDomain::SourceSnapshot,
                )
                .unwrap(),
            )
            .unwrap();
            let mut session = TokenSession::standalone(input.snapshot().clone(), budget)
                .unwrap()
                .0;
            let result = lex_prepared(&input, edition, &mut session);
            if edition == Edition::E2024 {
                assert!(matches!(
                    result,
                    Err(LexError::Source {
                        kind: SourceErrorKind::Syntax,
                        ..
                    })
                ));
            } else {
                assert!(result.is_ok());
            }
        }
    }
}
const CASES: &[Case] = &[
    Case {
        name: "unicode_ident",
        source: "fn main(){ let β = 1; let _ = β; }",
        stock_valid: true,
    },
    Case {
        name: "unicode_string",
        source: "fn main(){ let _ = \"β🙂\"; }",
        stock_valid: true,
    },
    Case {
        name: "unicode_char",
        source: "fn main(){ let _ = 'β'; }",
        stock_valid: true,
    },
    Case {
        name: "unicode_escape",
        source: r#"fn main(){ let _ = "\u{1F642}"; }"#,
        stock_valid: true,
    },
    Case {
        name: "unicode_leading_underscore",
        source: r#"fn main(){ let _ = "\u{_41}"; }"#,
        stock_valid: false,
    },
    Case {
        name: "unicode_surrogate",
        source: r#"fn main(){ let _ = "\u{D800}"; }"#,
        stock_valid: false,
    },
    Case {
        name: "unicode_out_of_range",
        source: r#"fn main(){ let _ = "\u{110000}"; }"#,
        stock_valid: false,
    },
    Case {
        name: "unicode_empty",
        source: r#"fn main(){ let _ = "\u{}"; }"#,
        stock_valid: false,
    },
    Case {
        name: "unicode_missing_brace",
        source: r#"fn main(){ let _ = "\u1234"; }"#,
        stock_valid: false,
    },
    Case {
        name: "string_escape",
        source: r#"fn main(){ let _ = "\q"; }"#,
        stock_valid: false,
    },
    Case {
        name: "raw_string",
        source: r###"fn main(){ let _ = r##"β\q"##; }"###,
        stock_valid: true,
    },
    Case {
        name: "unclosed_raw",
        source: r###"fn main(){ let _ = r##"abc"#; }"###,
        stock_valid: false,
    },
    Case {
        name: "raw_ident_crate",
        source: "fn main(){ let r#crate = 1; }",
        stock_valid: false,
    },
    Case {
        name: "raw_ident",
        source: "fn main(){ let r#type = 1; let _ = r#type; }",
        stock_valid: true,
    },
    Case {
        name: "byte_unicode",
        source: "fn main(){ let _ = b'β'; }",
        stock_valid: false,
    },
    Case {
        name: "bytestring_unicode",
        source: "fn main(){ let _ = b\"β\"; }",
        stock_valid: false,
    },
    Case {
        name: "raw_bytestring_unicode",
        source: "fn main(){ let _ = br\"β\"; }",
        stock_valid: false,
    },
    Case {
        name: "byte_escape",
        source: r#"fn main(){ let _ = b"\q"; }"#,
        stock_valid: false,
    },
    Case {
        name: "byte_unicode_escape",
        source: r#"fn main(){ let _ = b"\u{41}"; }"#,
        stock_valid: false,
    },
    Case {
        name: "byte_hex",
        source: r#"fn main(){ let _ = b"\xFF"; }"#,
        stock_valid: true,
    },
    Case {
        name: "bytestring",
        source: r#"fn main(){ let _ = b"hi\n"; }"#,
        stock_valid: true,
    },
    Case {
        name: "raw_bytestring",
        source: r##"fn main(){ let _ = br#"\q"#; }"##,
        stock_valid: true,
    },
    Case {
        name: "char",
        source: "fn main(){ let _ = 'x'; }",
        stock_valid: true,
    },
    Case {
        name: "multichar",
        source: "fn main(){ let _ = 'ab'; }",
        stock_valid: false,
    },
    Case {
        name: "empty_char",
        source: "fn main(){ let _ = ''; }",
        stock_valid: false,
    },
    Case {
        name: "cstring",
        source: "fn main(){ let _ = c\"abc\"; }",
        stock_valid: true,
    },
    Case {
        name: "cstring_nul",
        source: r#"fn main(){ let _ = c"a\0b"; }"#,
        stock_valid: false,
    },
    Case {
        name: "binary_digit",
        source: "fn main(){ let _ = 0b2; }",
        stock_valid: false,
    },
    Case {
        name: "float",
        source: "fn main(){ let _ = 1.5e+2f64; }",
        stock_valid: true,
    },
    Case {
        name: "tuple_dots",
        source: "fn main(){ let _ = ((1,2),(3,4)).0.1; }",
        stock_valid: true,
    },
    Case {
        name: "doc_crlf",
        source: "/// β\r\nfn main(){}\r\n",
        stock_valid: true,
    },
    Case {
        name: "combining_continuation",
        source: "fn main(){ let a\u{301} = 1; let _ = a\u{301}; }",
        stock_valid: true,
    },
    Case {
        name: "combining_start",
        source: "fn main(){ let \u{301}a = 1; }",
        stock_valid: false,
    },
    Case {
        name: "emoji_ident",
        source: "fn main(){ let 🙂 = 1; }",
        stock_valid: false,
    },
    Case {
        name: "underscore_pattern",
        source: "fn main(){ let _ = 1; }",
        stock_valid: true,
    },
    Case {
        name: "raw_underscore",
        source: "fn main(){ let r#_ = 1; }",
        stock_valid: false,
    },
    Case {
        name: "raw_self",
        source: "fn main(){ let r#self = 1; }",
        stock_valid: false,
    },
    Case {
        name: "raw_super",
        source: "fn main(){ let r#super = 1; }",
        stock_valid: false,
    },
    Case {
        name: "raw_Self",
        source: "fn main(){ let r#Self = 1; }",
        stock_valid: false,
    },
    Case {
        name: "lifetime",
        source: "fn id<'a>(x: &'a i32) -> &'a i32 { x } fn main(){}",
        stock_valid: true,
    },
    Case {
        name: "float_suffix",
        source: "fn main(){ let _ = 1.5f32; }",
        stock_valid: true,
    },
    // These two sources are expression-invalid, but their suffix spellings are
    // lexically valid. The swallowed-token-tree controls below pin the phase.
    Case {
        name: "expression_float_suffix",
        source: "fn main(){ let _ = 1.5wat; }",
        stock_valid: false,
    },
    Case {
        name: "expression_integer_suffix",
        source: "fn main(){ let _ = 1u8x; }",
        stock_valid: false,
    },
    Case {
        name: "radix_empty",
        source: "fn main(){ let _ = 0x; }",
        stock_valid: false,
    },
    Case {
        name: "octal_digit",
        source: "fn main(){ let _ = 0o8; }",
        stock_valid: false,
    },
    Case {
        name: "tuple_variable_dots",
        source: "fn main(){ let t = ((1,2),(3,4)); let _ = t.0.1; }",
        stock_valid: true,
    },
    Case {
        name: "guarded_string_edition",
        source: "macro_rules! accept {($($t:tt)*)=>{}} accept!(#\"abc\"#); fn main(){}",
        stock_valid: false,
    },
    Case {
        name: "reserved_hashes_edition",
        source: "macro_rules! accept {($($t:tt)*)=>{}} accept!(##); fn main(){}",
        stock_valid: false,
    },
    Case {
        name: "suffix_macro_integer",
        source: "macro_rules! accept {($($t:tt)*)=>{}} accept!(1u8x); fn main(){}",
        stock_valid: true,
    },
    Case {
        name: "suffix_macro_float",
        source: "macro_rules! accept {($($t:tt)*)=>{}} accept!(1.5wat); fn main(){}",
        stock_valid: true,
    },
    Case {
        name: "doc_bare_cr_line",
        source: "/// x\ry\nfn main(){}",
        stock_valid: false,
    },
    Case {
        name: "doc_bare_cr_block",
        source: "/** x\ry */fn main(){}",
        stock_valid: false,
    },
    Case {
        name: "hex_float_macro",
        source: "macro_rules! accept {($($t:tt)*)=>{}} accept!(0x1.0); fn main(){}",
        stock_valid: false,
    },
    Case {
        name: "grave",
        source: "fn main(){ ` }",
        stock_valid: false,
    },
];
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let path = base.join(format!(
                "jr-lexical-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("scratch creation: {e}"),
            }
        }
        panic!("scratch allocation exhausted")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, text: &str) {
        let mut file = new_file(&self.path(name));
        file.write_all(text.as_bytes()).unwrap();
        drop(file);
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap()
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Captured {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    let mut child = Running(command.spawn().unwrap());
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= 65536,
                "oracle output limit"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "oracle timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(status.code().is_some(), "oracle terminated by signal");
    Captured {
        success: status.success(),
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
#[test]
fn stock_first_lexical_classification_table() {
    let scratch = Scratch::new();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(Command::new(&rustc).arg("-vV"), &scratch);
    assert!(
        version.success,
        "{}",
        String::from_utf8_lossy(&version.stderr)
    );
    let version = String::from_utf8(version.stdout).unwrap();
    assert!(version.contains("commit-hash:"));
    eprintln!("Lexical baseline stock: {BASELINE_STOCK}\nActual stock oracle:\n{version}");
    assert_eq!(CASES.iter().filter(|case| case.stock_valid).count(), 21);
    assert_eq!(CASES.len(), 54);
    // Rust2024 is the production target. Rust2021 remains a separately measured
    // comparison, not an implicit default or claim of broad edition support.
    for edition in ["2024", "2021"] {
        for case in CASES {
            let expected = case.stock_valid
                || (edition == "2021"
                    && matches!(
                        case.name,
                        "guarded_string_edition" | "reserved_hashes_edition"
                    ));
            let source = format!("{}-{edition}.rs", case.name);
            let metadata = scratch.path(&format!("{}-{edition}.rmeta", case.name));
            scratch.write(&source, case.source);
            let result = capture(
                Command::new(&rustc)
                    .args([
                        "--crate-type=lib",
                        "--crate-name=lexical_oracle",
                        "--emit=metadata",
                        "-Cpanic=abort",
                    ])
                    .arg(format!("--edition={edition}"))
                    .arg(scratch.path(&source))
                    .arg("-o")
                    .arg(&metadata),
                &scratch,
            );
            assert_eq!(
                result.success,
                expected,
                "{} edition{edition}: {}",
                case.name,
                String::from_utf8_lossy(&result.stderr)
            );
            if expected {
                assert!(
                    fs::metadata(metadata).unwrap().len() > 0,
                    "{}: missing stock artifact",
                    case.name
                );
            } else {
                assert!(
                    !metadata.exists(),
                    "{}: invalid stock source produced artifact",
                    case.name
                );
                assert!(String::from_utf8_lossy(&result.stderr).contains("error"));
            }
        }
    }
}
