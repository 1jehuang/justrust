//! External checked structural utility contract, not Rust lexing or macro conformance.
use justrust_engine::compiler::token_primitives::*;
use std::sync::Arc;

fn map(text: &str) -> SourceMap {
    SourceMap {
        files: vec![SourceFile {
            lexical_path: "original/tokens.rs".into(),
            contents: Arc::from(text),
            line_starts: std::iter::once(0)
                .chain(
                    text.bytes()
                        .enumerate()
                        .filter_map(|(i, b)| (b == b'\n').then_some((i + 1) as u32)),
                )
                .collect(),
        }],
    }
}
fn snapshot(text: &str) -> SourceSnapshot {
    SourceSnapshot::new(map(text)).unwrap()
}
fn session() -> TokenSession {
    TokenSession::standalone(
        snapshot("x(y)"),
        ExpansionBudget::new(ExpansionLimits::default()),
    )
    .unwrap()
    .0
}
fn span(s: &mut TokenSession, lo: u32, hi: u32) -> SpanId {
    let raw = s
        .sources()
        .span(s.sources().file(0).unwrap(), lo, hi)
        .unwrap();
    s.original_span(raw).unwrap()
}
fn punct(span: SpanId, ch: char) -> InputToken {
    InputToken::Leaf(Token {
        span,
        kind: TokenKind::Punct {
            ch,
            spacing: Spacing::Alone,
        },
    })
}
fn expansion(s: &mut TokenSession, anchor: SpanId, parent: Option<ExpansionId>) -> ExpansionId {
    let defining_crate = s.register_crate_root(s.sources().file(0).unwrap()).unwrap();
    s.add_expansion(ExpansionRecord {
        definition: anchor,
        call: anchor,
        parent,
        defining_crate,
        edition: Edition::E2021,
    })
    .unwrap()
}

#[test]
fn snapshots_are_owned_utf8_checked_and_eof_aware() {
    let source = snapshot("α🙂\r\nz");
    let file = source.file(0).unwrap();
    assert_eq!(source.text(file).unwrap(), "α🙂\r\nz");
    let tail = source.span(file, 8, 9).unwrap();
    let (path, line, column) = source.location(tail).unwrap();
    assert_eq!(path.to_str(), Some("original/tokens.rs"));
    assert_eq!((line, column), (2, 1));
    assert!(source.span(file, 9, 9).is_ok());
    assert!(matches!(
        source.span(file, 1, 2),
        Err(TokenError::InvalidUtf8Boundary)
    ));
    assert!(matches!(
        source.span(file, 2, 3),
        Err(TokenError::InvalidUtf8Boundary)
    ));
    assert!(matches!(
        source.span(file, 4, 2),
        Err(TokenError::InvalidRange)
    ));
    assert!(matches!(
        source.span(file, 9, 10),
        Err(TokenError::InvalidRange)
    ));
    assert!(matches!(source.file(1), Err(TokenError::InvalidHandle)));
    assert_eq!(
        source.clone().text(file).unwrap(),
        source.text(file).unwrap()
    );
    let independent = snapshot("α🙂\r\nz");
    assert!(matches!(
        independent.text(file),
        Err(TokenError::ForeignHandle)
    ));
    assert!(matches!(
        independent.render_span(tail),
        Err(TokenError::ForeignHandle)
    ));
}

#[test]
fn snapshot_seals_caller_owned_map_clones() {
    let mut caller = map("α\r\nz");
    let sealed = SourceSnapshot::new(caller.clone()).unwrap();
    let file = sealed.file(0).unwrap();
    let original = sealed.span(file, 4, 5).unwrap();
    caller.files[0].contents = Arc::from("replacement");
    caller.files[0].lexical_path = "changed.rs".into();
    caller.files[0].line_starts = vec![99];
    caller.files.clear();
    assert_eq!(sealed.text(file).unwrap(), "α\r\nz");
    let (path, line, column) = sealed.location(original).unwrap();
    assert_eq!(path.to_str(), Some("original/tokens.rs"));
    assert_eq!((line, column), (2, 1));
    assert!(sealed.file(0).is_ok());
    assert!(sealed.file(1).is_err());
}

#[test]
fn snapshot_file_limit_is_exact_and_precedes_line_validation() {
    const LIMIT: usize = 1024;
    let empty = map("").files.remove(0);
    let exact = SourceSnapshot::new(SourceMap {
        files: vec![empty.clone(); LIMIT],
    })
    .unwrap();
    assert!(exact.file(LIMIT - 1).is_ok());
    assert!(matches!(exact.file(LIMIT), Err(TokenError::InvalidHandle)));
    let mut files = vec![empty; LIMIT + 1];
    files[0].line_starts.clear();
    assert!(
        matches!(SourceSnapshot::new(SourceMap { files }), Err(TokenError::SourceLimit {
        resource: SourceLimitResource::Files, used: 0, requested, limit: LIMIT,
    }) if requested == LIMIT + 1)
    );
}

#[test]
fn snapshot_byte_limit_is_exact_for_single_and_aggregate_contents() {
    const LIMIT: usize = 8 * 1024 * 1024;
    let exact_file = SourceFile {
        lexical_path: "exact.rs".into(),
        contents: Arc::from("x".repeat(LIMIT)),
        line_starts: vec![0],
    };
    let exact = SourceSnapshot::new(SourceMap {
        files: vec![exact_file],
    })
    .unwrap();
    let file = exact.file(0).unwrap();
    assert!(exact.span(file, LIMIT as u32, LIMIT as u32).is_ok());
    let oversized = SourceFile {
        lexical_path: "oversized.rs".into(),
        contents: Arc::from("x".repeat(LIMIT + 1)),
        line_starts: vec![],
    };
    assert!(
        matches!(SourceSnapshot::new(SourceMap { files: vec![oversized] }), Err(TokenError::SourceLimit {
        resource: SourceLimitResource::Bytes, used: 0, requested, limit: LIMIT,
    }) if requested == LIMIT + 1)
    );
    let half = SourceFile {
        lexical_path: "half.rs".into(),
        contents: Arc::from("x".repeat(LIMIT / 2)),
        line_starts: vec![0],
    };
    assert!(
        SourceSnapshot::new(SourceMap {
            files: vec![half.clone(), half.clone()]
        })
        .is_ok()
    );
    let mut files = vec![half.clone(), half, map("x").files.remove(0)];
    files[0].line_starts.clear();
    assert!(matches!(
        SourceSnapshot::new(SourceMap { files }),
        Err(TokenError::SourceLimit {
            resource: SourceLimitResource::Bytes,
            used: LIMIT,
            requested: 1,
            limit: LIMIT,
        })
    ));
}

#[test]
fn snapshot_rejects_forged_line_tables() {
    for starts in [vec![], vec![1], vec![0, 1], vec![0, 2, 2]] {
        let mut source = map("x\ny");
        source.files[0].line_starts = starts;
        assert!(matches!(
            SourceSnapshot::new(source),
            Err(TokenError::InvalidInput(_))
        ));
    }
}

#[test]
fn sessions_do_not_share_symbol_span_or_context_authority() {
    let sources = snapshot("x");
    let mut a = TokenSession::standalone(
        sources.clone(),
        ExpansionBudget::new(ExpansionLimits::default()),
    )
    .unwrap()
    .0;
    let mut b = TokenSession::standalone(sources, ExpansionBudget::new(ExpansionLimits::default()))
        .unwrap()
        .0;
    let symbol = a.intern_symbol("not a Rust identifier!").unwrap();
    assert_eq!(a.intern_symbol("not a Rust identifier!").unwrap(), symbol);
    assert_eq!(a.symbol(symbol).unwrap(), "not a Rust identifier!");
    assert!(matches!(b.symbol(symbol), Err(TokenError::ForeignHandle)));
    let anchor = span(&mut a, 0, 1);
    assert!(matches!(b.span(anchor), Err(TokenError::ForeignHandle)));
    assert!(matches!(
        b.context(a.root_context()),
        Err(TokenError::ForeignHandle)
    ));
    let own = span(&mut b, 0, 1);
    assert!(matches!(
        b.with_context(own, a.root_context(), None),
        Err(TokenError::ForeignHandle)
    ));
    let alien = snapshot("x");
    let raw = alien.span(alien.file(0).unwrap(), 0, 1).unwrap();
    assert!(matches!(
        a.original_span(raw),
        Err(TokenError::ForeignHandle)
    ));
}

#[test]
fn contexts_and_expansions_validate_nested_handles() {
    let mut s = session();
    let anchor = span(&mut s, 0, 1);
    let exp = expansion(&mut s, anchor, None);
    let root = s.root_context();
    let marked = s.apply_mark(root, exp, Transparency::Opaque).unwrap();
    assert_eq!(
        marked,
        s.apply_mark(root, exp, Transparency::Opaque).unwrap()
    );
    assert_ne!(
        marked,
        s.apply_mark(root, exp, Transparency::Transparent).unwrap()
    );
    let contextual = s.with_context(anchor, marked, Some(exp)).unwrap();
    assert_eq!(s.span(anchor).unwrap().context, root);
    assert_eq!(s.span(contextual).unwrap().context, marked);
    assert_eq!(s.span(contextual).unwrap().expansion, Some(exp));
    let mut other = session();
    let other_span = span(&mut other, 0, 1);
    let other_exp = expansion(&mut other, other_span, None);
    assert!(matches!(
        s.apply_mark(root, other_exp, Transparency::Opaque),
        Err(TokenError::ForeignHandle)
    ));
    assert!(matches!(
        s.with_context(anchor, root, Some(other_exp)),
        Err(TokenError::ForeignHandle)
    ));
    let foreign_crate = other
        .register_crate_root(other.sources().file(0).unwrap())
        .unwrap();
    assert!(matches!(
        s.crate_root(foreign_crate),
        Err(TokenError::ForeignHandle)
    ));
    assert!(matches!(
        s.add_expansion(ExpansionRecord {
            definition: anchor,
            call: other_span,
            parent: None,
            defining_crate: foreign_crate,
            edition: Edition::E2024
        }),
        Err(TokenError::ForeignHandle)
    ));
}

#[test]
fn token_validation_is_structural_not_a_lexer() {
    let mut s = session();
    let anchor = span(&mut s, 0, 1);
    let symbol = s.intern_symbol("arbitrary spelling, not lexed").unwrap();
    let root = s.root_context();
    let ident = Token {
        span: anchor,
        kind: TokenKind::Ident(Ident {
            symbol,
            raw: true,
            context: root,
        }),
    };
    assert!(ident.validate(&s).is_ok());
    let literal = Token {
        span: anchor,
        kind: TokenKind::Literal {
            kind: LiteralKind::Integer,
            spelling: symbol,
        },
    };
    assert!(literal.validate(&s).is_ok());
    let exp = expansion(&mut s, anchor, None);
    let marked = s.apply_mark(root, exp, Transparency::Opaque).unwrap();
    let incoherent = Token {
        span: anchor,
        kind: TokenKind::Ident(Ident {
            symbol,
            raw: false,
            context: marked,
        }),
    };
    assert!(matches!(
        incoherent.validate(&s),
        Err(TokenError::InvalidInput(_))
    ));
    for ch in ['(', ')', '[', ']', '{', '}', 'α', 'a'] {
        let InputToken::Leaf(token) = punct(anchor, ch) else {
            unreachable!()
        };
        assert!(
            matches!(token.validate(&s), Err(TokenError::InvalidInput(_))),
            "{ch}"
        );
    }
    let mut other = session();
    let foreign = other.intern_symbol("x").unwrap();
    assert!(matches!(
        (Token {
            span: anchor,
            kind: TokenKind::Literal {
                kind: LiteralKind::String,
                spelling: foreign
            }
        })
        .validate(&s),
        Err(TokenError::ForeignHandle)
    ));
}

#[test]
fn groups_keep_genuine_delimiters_order_and_stream_ownership() {
    let mut s = session();
    let open = span(&mut s, 1, 2);
    let close = span(&mut s, 3, 4);
    let leaf = span(&mut s, 2, 3);
    let input = [
        InputToken::Open {
            kind: Delimiter::Parenthesis,
            span: open,
        },
        punct(leaf, '+'),
        InputToken::Close {
            kind: Delimiter::Parenthesis,
            span: close,
        },
    ];
    let tree = group(&s, &input, GroupPolicy::OriginalSource).unwrap();
    let roots = tree.roots(&s).unwrap();
    let ids = tree.children(&s, &roots).unwrap();
    assert_eq!(ids.len(), 1);
    let root = ids[0];
    let Node::Delimited {
        kind,
        open: actual_open,
        children,
        close: actual_close,
    } = tree.node(&s, root).unwrap()
    else {
        panic!("expected real group")
    };
    assert_eq!(
        (*kind, *actual_open, *actual_close),
        (Delimiter::Parenthesis, open, close)
    );
    let child_ids = tree.children(&s, children).unwrap();
    assert_eq!(child_ids.len(), 1);
    assert!(matches!(
        tree.node(&s, child_ids[0]).unwrap(),
        Node::Leaf(Token {
            kind: TokenKind::Punct { ch: '+', .. },
            ..
        })
    ));
    assert!(tree.clone().node(&s, root).is_ok());
    let independent = group(&s, &input, GroupPolicy::OriginalSource).unwrap();
    assert!(matches!(
        independent.node(&s, root),
        Err(TokenError::ForeignHandle)
    ));
    assert!(matches!(
        independent.children(&s, &roots),
        Err(TokenError::ForeignHandle)
    ));
    assert!(matches!(
        tree.roots(&session()),
        Err(TokenError::ForeignHandle)
    ));
    let counters = s.budget().snapshot().counters;
    assert_eq!(counters[Counter::InputTokens as usize], 0);
    assert_eq!(counters[Counter::OutputTokens as usize], 0);
}

#[test]
fn malformed_groups_refund_storage_without_poisoning() {
    let mut s = session();
    let anchor = span(&mut s, 0, 1);
    let before = s.budget().snapshot();
    let malformed = [
        vec![InputToken::Open {
            kind: Delimiter::Brace,
            span: anchor,
        }],
        vec![InputToken::Close {
            kind: Delimiter::Brace,
            span: anchor,
        }],
        vec![
            InputToken::Open {
                kind: Delimiter::Brace,
                span: anchor,
            },
            InputToken::Close {
                kind: Delimiter::Bracket,
                span: anchor,
            },
        ],
    ];
    for input in malformed {
        assert!(matches!(
            group(&s, &input, GroupPolicy::OriginalSource),
            Err(TokenError::UnbalancedDelimiter { .. })
        ));
        let after = s.budget().snapshot();
        assert_eq!(after.retained_bytes, before.retained_bytes);
        assert_eq!(after.temporary_bytes, 0);
        assert!(after.failure.is_none());
    }
    assert!(group(&s, &[], GroupPolicy::OriginalSource).is_ok());
}

#[test]
fn stream_clones_hold_storage_until_last_drop() {
    let mut s = session();
    let anchor = span(&mut s, 0, 1);
    let before = s.budget().snapshot();
    let tree = group(&s, &[punct(anchor, '+')], GroupPolicy::OriginalSource).unwrap();
    let during = s.budget().snapshot();
    assert!(during.retained_bytes > before.retained_bytes);
    assert_eq!(during.temporary_bytes, 0);
    let clone = tree.clone();
    drop(tree);
    assert_eq!(s.budget().snapshot().retained_bytes, during.retained_bytes);
    drop(clone);
    assert_eq!(s.budget().snapshot().retained_bytes, before.retained_bytes);
    assert!(
        s.budget().snapshot().counters[Counter::Work as usize]
            > before.counters[Counter::Work as usize]
    );
}

#[test]
fn original_and_generated_group_depth_limits_are_distinct() {
    let mut limits = ExpansionLimits::default();
    limits.depths[Depth::OriginalGroups as usize] = 1;
    limits.depths[Depth::GeneratedGroups as usize] = 0;
    let budget = ExpansionBudget::new(limits);
    let mut s = TokenSession::standalone(snapshot("x"), budget.clone())
        .unwrap()
        .0;
    let anchor = span(&mut s, 0, 1);
    let pair = [
        InputToken::Open {
            kind: Delimiter::Brace,
            span: anchor,
        },
        InputToken::Close {
            kind: Delimiter::Brace,
            span: anchor,
        },
    ];
    assert!(group(&s, &pair, GroupPolicy::OriginalSource).is_ok());
    assert!(matches!(
        group(&s, &pair, GroupPolicy::Generated),
        Err(TokenError::Budget(BudgetFailure {
            resource: BudgetResource::Depth(Depth::GeneratedGroups),
            kind: BudgetFailureKind::Limit,
            ..
        }))
    ));
    assert_eq!(budget.snapshot().temporary_bytes, 0);
    assert!(matches!(s.span(anchor), Err(TokenError::Budget(_))));
}

#[test]
fn synthetic_origins_and_provenance_truncation_are_explicit() {
    let limits = ExpansionLimits {
        backtrace_frames: 1,
        ..ExpansionLimits::default()
    };
    let budget = ExpansionBudget::new(limits);
    let mut s = TokenSession::standalone(snapshot("x"), budget.clone())
        .unwrap()
        .0;
    let anchor = span(&mut s, 0, 1);
    let synthetic = s
        .synthetic_span(anchor, SyntheticReason::ExpansionToken)
        .unwrap();
    assert!(
        matches!(s.span(synthetic).unwrap().origin,SpanOrigin::Synthetic {anchor:a,reason:SyntheticReason::ExpansionToken} if a==anchor)
    );
    assert!(matches!(
        s.original(synthetic, 1),
        Err(TokenError::TraceTruncated)
    ));
    assert_eq!(
        s.original(synthetic, 2).unwrap(),
        s.original(anchor, 1).unwrap()
    );
    let first = expansion(&mut s, anchor, None);
    let second = expansion(&mut s, anchor, Some(first));
    let mut trace = s.trace(Some(second), 100).unwrap();
    assert_eq!(trace.next().unwrap().unwrap().expansion, second);
    assert!(trace.next().is_none());
    assert!(trace.truncated());
    let mut zero = s.trace(Some(second), 0).unwrap();
    assert!(zero.next().is_none());
    assert!(zero.truncated());
    assert!(!s.trace(None, 0).unwrap().truncated());
    assert!(budget.check().is_ok());
    let mut foreign = session();
    let alien = span(&mut foreign, 0, 1);
    let alien_exp = expansion(&mut foreign, alien, None);
    assert!(matches!(
        s.original(alien, 0),
        Err(TokenError::ForeignHandle)
    ));
    assert!(matches!(
        s.trace(Some(alien_exp), 0),
        Err(TokenError::ForeignHandle)
    ));
}

#[test]
fn every_raised_limit_prepoisons_instead_of_clamping() {
    let default = ExpansionLimits::default();
    let mut candidates = Vec::new();
    for i in 0..default.counters.len() {
        let mut l = default;
        l.counters[i] += 1;
        candidates.push(l);
    }
    for i in 0..default.depths.len() {
        let mut l = default;
        l.depths[i] += 1;
        candidates.push(l);
    }
    let mut l = default;
    l.bytes += 1;
    candidates.push(l);
    let mut l = default;
    l.matcher_states += 1;
    candidates.push(l);
    let mut l = default;
    l.invocation_output += 1;
    candidates.push(l);
    let mut l = default;
    l.backtrace_frames += 1;
    candidates.push(l);
    for limits in candidates {
        let budget = ExpansionBudget::new(limits);
        let failure = budget.check().unwrap_err();
        assert_eq!(failure.resource, BudgetResource::Limits);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.charge(Counter::Work, 0).unwrap_err(), failure);
    }
}

#[test]
fn zero_limits_are_valid_but_do_not_grant_positive_resources() {
    let zero = ExpansionLimits {
        counters: [0; 10],
        depths: [0; 3],
        bytes: 0,
        matcher_states: 0,
        invocation_output: 0,
        backtrace_frames: 0,
    };
    let budget = ExpansionBudget::new(zero);
    assert!(budget.check().is_ok());
    assert!(budget.charge(Counter::Work, 0).is_ok());
    assert!(budget.check_depth(Depth::Expansion, 0).is_ok());
    assert!(budget.reserve(Storage::Temporary, 0).is_ok());
    assert!(matches!(
        budget.charge(Counter::Work, 1),
        Err(BudgetFailure {
            kind: BudgetFailureKind::Limit,
            resource: BudgetResource::Counter(Counter::Work),
            ..
        })
    ));
    assert!(TokenSession::standalone(snapshot(""), ExpansionBudget::new(zero)).is_err());
}

#[test]
fn counter_clones_share_a_monotonic_first_failure() {
    for counter in Counter::ALL {
        let mut limits = ExpansionLimits::default();
        limits.counters[counter as usize] = 2;
        let budget = ExpansionBudget::new(limits);
        let clone = budget.clone();
        budget.charge(counter, 1).unwrap();
        clone.charge(counter, 1).unwrap();
        let error = clone.charge(counter, 1).unwrap_err();
        assert_eq!(error.resource, BudgetResource::Counter(counter));
        assert_eq!(budget.snapshot().counters[counter as usize], 2);
        assert_eq!(budget.reserve(Storage::Temporary, 0).unwrap_err(), error);
        assert_eq!(budget.check().unwrap_err(), error);
    }
}

#[test]
fn storage_leases_resize_promote_and_release_even_after_failure() {
    let limits = ExpansionLimits {
        bytes: 20,
        ..ExpansionLimits::default()
    };
    let budget = ExpansionBudget::new(limits);
    let mut lease = budget.reserve(Storage::Temporary, 8).unwrap();
    lease.resize(12).unwrap();
    assert_eq!(lease.bytes(), 12);
    lease.promote_retained().unwrap();
    assert_eq!(
        (
            budget.snapshot().retained_bytes,
            budget.snapshot().temporary_bytes
        ),
        (12, 0)
    );
    let temporary = budget.reserve(Storage::Temporary, 8).unwrap();
    assert_eq!(budget.snapshot().peak_bytes, 20);
    let error = lease.resize(13).unwrap_err();
    assert_eq!(error.resource, BudgetResource::Bytes);
    assert_eq!(lease.bytes(), 12);
    drop(temporary);
    drop(lease);
    assert_eq!(
        (
            budget.snapshot().retained_bytes,
            budget.snapshot().temporary_bytes
        ),
        (0, 0)
    );
    assert_eq!(budget.snapshot().peak_bytes, 20);
    assert_eq!(budget.check().unwrap_err(), error);
}

#[test]
fn invocation_clones_share_output_caps_and_commit_atomically() {
    let limits = ExpansionLimits {
        invocation_output: 3,
        ..ExpansionLimits::default()
    };
    let budget = ExpansionBudget::new(limits);
    let invocation = budget.begin_invocation().unwrap();
    let clone = invocation.clone();
    invocation.emit(2).unwrap();
    clone.emit(1).unwrap();
    let error = clone.emit(1).unwrap_err();
    assert_eq!(error.resource, BudgetResource::InvocationOutput);
    assert_eq!(
        budget.snapshot().counters[Counter::OutputTokens as usize],
        3
    );
    assert_eq!(budget.snapshot().counters[Counter::Invocations as usize], 1);
    assert_eq!(invocation.emit(0).unwrap_err(), error);
}

#[test]
fn matcher_state_leases_release_live_storage_not_logical_work() {
    let limits = ExpansionLimits {
        matcher_states: 2,
        ..ExpansionLimits::default()
    };
    let budget = ExpansionBudget::new(limits);
    let invocation = budget.begin_invocation().unwrap();
    let clone = invocation.clone();
    let states = invocation.reserve_states(2, 8).unwrap();
    assert_eq!(budget.snapshot().temporary_bytes, 16);
    assert_eq!(budget.snapshot().counters[Counter::Work as usize], 2);
    drop(states);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
    let states = clone.reserve_states(2, 8).unwrap();
    let before = budget.snapshot();
    assert_eq!(
        clone.reserve_states(1, 8).unwrap_err().resource,
        BudgetResource::MatcherStates
    );
    assert_eq!(budget.snapshot().temporary_bytes, before.temporary_bytes);
    assert_eq!(budget.snapshot().counters[Counter::Work as usize], 4);
    drop(states);
    assert_eq!(budget.snapshot().temporary_bytes, 0);
}

#[test]
fn overflow_and_allocation_failures_are_typed_and_latched() {
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let error = budget
        .reserve_array(Storage::Temporary, u64::MAX, 2)
        .unwrap_err();
    assert_eq!(
        (error.kind, error.resource),
        (BudgetFailureKind::Overflow, BudgetResource::Bytes)
    );
    assert_eq!(budget.snapshot().temporary_bytes, 0);
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let error = budget.checked_u32(u64::from(u32::MAX) + 1).unwrap_err();
    assert_eq!(
        (error.kind, error.resource),
        (BudgetFailureKind::Overflow, BudgetResource::Index)
    );
    assert_eq!(budget.poison_allocation(), error);
    let budget = ExpansionBudget::new(ExpansionLimits::default());
    let error = budget.poison_allocation();
    assert_eq!(error.kind, BudgetFailureKind::Allocation);
    assert_eq!(budget.check().unwrap_err(), error);
}
