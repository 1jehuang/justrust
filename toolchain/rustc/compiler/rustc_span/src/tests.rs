use super::*;

#[test]
fn resident_revision_keeps_symbols_and_interned_spans_but_resets_metavars() {
    let mut globals = SessionGlobals::new(edition::DEFAULT_EDITION, &[], None);
    let symbol_name = format!("resident-owned-symbol-{}", std::process::id());
    let (symbol, span) = globals.enter_resident_revision(|| {
        let symbol = Symbol::intern(&symbol_name);
        // The length exceeds the inline span format, exercising the real interner.
        let span = Span::with_root_ctxt(BytePos(1), BytePos(100_001));
        with_metavar_spans(|map| {
            assert!(map.insert(span, DUMMY_SP));
            assert_eq!(map.get(span), Some(DUMMY_SP));
            assert_eq!(map.freeze_and_get_read_spans().len(), 1);
        });
        (symbol, span)
    });
    globals.enter_resident_revision(|| {
        assert_eq!(Symbol::intern(&symbol_name), symbol);
        assert_eq!(symbol.as_str(), symbol_name.as_str());
        assert_eq!((span.lo(), span.hi()), (BytePos(1), BytePos(100_001)));
        let next_target = Span::with_root_ctxt(BytePos(3), BytePos(7));
        with_metavar_spans(|map| {
            assert_eq!(map.get(span), None);
            assert!(map.insert(span, next_target));
            assert_eq!(map.get(span), Some(next_target));
        });
    });
}

#[test]
fn resident_revision_unwind_releases_global_scope_and_allows_reentry() {
    let mut globals = SessionGlobals::new(edition::DEFAULT_EDITION, &[], None);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        globals.enter_resident_revision(|| {
            with_metavar_spans(|map| {
                let _ = map.freeze_and_get_read_spans();
            });
            panic!("discard this revision");
        });
    }));
    assert!(failed.is_err());
    globals.enter_resident_revision(|| {
        let name = format!("after-revision-unwind-{}", std::process::id());
        assert_eq!(Symbol::intern(&name).as_str(), name.as_str());
        with_metavar_spans(|map| assert!(map.insert(DUMMY_SP, DUMMY_SP)));
    });
}

#[test]
fn test_lookup_line() {
    let source = "abcdefghijklm\nabcdefghij\n...".to_owned();
    let mut sf = SourceFile::new(
        FileName::Anon(Hash64::ZERO),
        source,
        SourceFileHashAlgorithm::Sha256,
        Some(SourceFileHashAlgorithm::Sha256),
    )
    .unwrap();
    sf.start_pos = BytePos(3);
    assert_eq!(sf.lines(), &[RelativeBytePos(0), RelativeBytePos(14), RelativeBytePos(25)]);

    assert_eq!(sf.lookup_line(RelativeBytePos(0)), Some(0));
    assert_eq!(sf.lookup_line(RelativeBytePos(1)), Some(0));

    assert_eq!(sf.lookup_line(RelativeBytePos(13)), Some(0));
    assert_eq!(sf.lookup_line(RelativeBytePos(14)), Some(1));
    assert_eq!(sf.lookup_line(RelativeBytePos(15)), Some(1));

    assert_eq!(sf.lookup_line(RelativeBytePos(25)), Some(2));
    assert_eq!(sf.lookup_line(RelativeBytePos(26)), Some(2));
}

#[test]
fn test_normalize_newlines() {
    fn check(before: &str, after: &str, expected_positions: &[u32]) {
        let mut actual = before.to_string();
        let mut actual_positions = vec![];
        normalize_newlines(&mut actual, &mut actual_positions);
        let actual_positions: Vec<_> = actual_positions.into_iter().map(|nc| nc.pos.0).collect();
        assert_eq!(actual.as_str(), after);
        assert_eq!(actual_positions, expected_positions);
    }
    check("", "", &[]);
    check("\n", "\n", &[]);
    check("\r", "\r", &[]);
    check("\r\r", "\r\r", &[]);
    check("\r\n", "\n", &[1]);
    check("hello world", "hello world", &[]);
    check("hello\nworld", "hello\nworld", &[]);
    check("hello\r\nworld", "hello\nworld", &[6]);
    check("\r\nhello\r\nworld\r\n", "\nhello\nworld\n", &[1, 7, 13]);
    check("\r\r\n", "\r\n", &[2]);
    check("hello\rworld", "hello\rworld", &[]);
}

#[test]
fn test_trim() {
    let span = |lo: usize, hi: usize| {
        Span::new(BytePos::from_usize(lo), BytePos::from_usize(hi), SyntaxContext::root(), None)
    };

    // Various positions, named for their relation to `start` and `end`.
    let well_before = 1;
    let before = 3;
    let start = 5;
    let mid = 7;
    let end = 9;
    let after = 11;
    let well_after = 13;

    // The resulting span's context should be that of `self`, not `other`.
    let other = span(start, end).with_ctxt(SyntaxContext::from_u32(999));

    // Test cases for `trim_end`.

    assert_eq!(span(well_before, before).trim_end(other), Some(span(well_before, before)));
    assert_eq!(span(well_before, start).trim_end(other), Some(span(well_before, start)));
    assert_eq!(span(well_before, mid).trim_end(other), Some(span(well_before, start)));
    assert_eq!(span(well_before, end).trim_end(other), Some(span(well_before, start)));
    assert_eq!(span(well_before, after).trim_end(other), Some(span(well_before, start)));

    assert_eq!(span(start, mid).trim_end(other), None);
    assert_eq!(span(start, end).trim_end(other), None);
    assert_eq!(span(start, after).trim_end(other), None);

    assert_eq!(span(mid, end).trim_end(other), None);
    assert_eq!(span(mid, after).trim_end(other), None);

    assert_eq!(span(end, after).trim_end(other), None);

    assert_eq!(span(after, well_after).trim_end(other), None);

    // Test cases for `trim_start`.

    assert_eq!(span(after, well_after).trim_start(other), Some(span(after, well_after)));
    assert_eq!(span(end, well_after).trim_start(other), Some(span(end, well_after)));
    assert_eq!(span(mid, well_after).trim_start(other), Some(span(end, well_after)));
    assert_eq!(span(start, well_after).trim_start(other), Some(span(end, well_after)));
    assert_eq!(span(before, well_after).trim_start(other), Some(span(end, well_after)));

    assert_eq!(span(mid, end).trim_start(other), None);
    assert_eq!(span(start, end).trim_start(other), None);
    assert_eq!(span(before, end).trim_start(other), None);

    assert_eq!(span(start, mid).trim_start(other), None);
    assert_eq!(span(before, mid).trim_start(other), None);

    assert_eq!(span(before, start).trim_start(other), None);

    assert_eq!(span(well_before, before).trim_start(other), None);
}

#[test]
fn test_unnormalized_source_length() {
    let source = "\u{feff}hello\r\nferries\r\n".to_owned();
    let sf = SourceFile::new(
        FileName::Anon(Hash64::ZERO),
        source,
        SourceFileHashAlgorithm::Sha256,
        Some(SourceFileHashAlgorithm::Sha256),
    )
    .unwrap();
    assert_eq!(sf.unnormalized_source_len, 19);
    assert_eq!(sf.normalized_source_len.0, 14);
}
