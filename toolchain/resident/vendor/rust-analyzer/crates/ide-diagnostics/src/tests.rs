#![allow(clippy::print_stderr)]

mod overly_long_real_world_cases;

use hir::setup_tracing;
use ide_db::{
    RootDatabase,
    assists::{AssistResolveStrategy, ExprFillDefaultMode},
    base_db::SourceDatabase,
    line_index,
};
use itertools::Itertools;
use stdx::trim_indent;
use test_fixture::WithFixture;
use test_utils::{MiniCore, assert_eq_text, extract_annotations};

use crate::{DiagnosticsConfig, Severity};

/// Compare every Debug-visible diagnostic field, including fixes and main-node pointers.
/// Diagnostic deliberately has no PartialEq implementation.
fn resident_semantic_subphases_compare(selection: usize, disabled: bool) {
    use std::{cell::RefCell, time::Instant};

    let (db, file_id) = RootDatabase::with_single_file(
        r#"
fn first() { let _: u32 = "first"; }
fn second() { let _: bool = 42; }
"#,
    );
    let file_id = file_id.file_id(&db);
    let mut config = DiagnosticsConfig::test_sample();
    if disabled {
        config.disabled.insert("E0308".into());
    }
    hir::attach_db(&db, || {
        let old_calls = RefCell::new(Vec::new());
        let old = super::semantic_diagnostics_of_bodies(
            &db,
            &config,
            &AssistResolveStrategy::All,
            file_id,
            None,
            &|body| {
                let mut calls = old_calls.borrow_mut();
                calls.push(body);
                match selection {
                    0 => false,
                    1 => true,
                    _ => calls.len() % 2 == 1,
                }
            },
        );
        let new_calls = RefCell::new(Vec::new());
        let callback_intervals = RefCell::new(Vec::new());
        let before = Instant::now();
        let mut intervals = vec![("sentinel", before, before)];
        let new = super::semantic_diagnostics_of_bodies_with_timings(
            &db,
            &config,
            &AssistResolveStrategy::All,
            file_id,
            None,
            &|body| {
                let start = Instant::now();
                let mut calls = new_calls.borrow_mut();
                calls.push(body);
                let selected = match selection {
                    0 => false,
                    1 => true,
                    _ => calls.len() % 2 == 1,
                };
                callback_intervals.borrow_mut().push((start, Instant::now()));
                selected
            },
            Some(&mut intervals),
        );
        let after = Instant::now();
        assert_eq!(format!("{old:#?}"), format!("{new:#?}"));
        assert_eq!(*old_calls.borrow(), *new_calls.borrow());
        assert!(!new_calls.borrow().is_empty());
        if selection == 1 && !disabled {
            assert!(new.iter().any(|diag| diag.code.as_str() == "E0308"));
        }
        if disabled {
            assert!(new.iter().all(|diag| diag.code.as_str() != "E0308"));
        }
        assert_eq!(intervals[0], ("sentinel", before, before));
        let intervals = &intervals[1..];
        assert_eq!(
            intervals.iter().map(|it| it.0).collect::<Vec<_>>(),
            [
                "diagnostics.semantic.prepare_parse",
                "diagnostics.semantic.syntax_walk",
                "diagnostics.semantic.module_context",
                "diagnostics.semantic.collect",
                "diagnostics.semantic.render_and_filter",
            ]
        );
        for &(_, start, end) in intervals {
            assert!(before <= start && start <= end && end <= after);
        }
        for pair in intervals.windows(2) {
            assert_eq!(pair[0].2, pair[1].1);
        }
        let (_, collect_start, collect_end) = intervals[3];
        assert!(collect_start < collect_end);
        for &(start, end) in callback_intervals.borrow().iter() {
            assert!(collect_start <= start && end <= collect_end);
        }
    });
}

#[test]
fn resident_semantic_subphases_all_bodies_exact() {
    resident_semantic_subphases_compare(1, false);
}

#[test]
fn resident_semantic_subphases_no_bodies_exact() {
    resident_semantic_subphases_compare(0, false);
}

#[test]
fn resident_semantic_subphases_selective_bodies_exact() {
    resident_semantic_subphases_compare(2, false);
}

#[test]
fn resident_semantic_subphases_disabled_diagnostic_exact() {
    resident_semantic_subphases_compare(1, true);
}

#[test]
fn resident_semantic_subphases_disabled_clockfree_api_exact() {
    let (db, file_id) = RootDatabase::with_single_file("fn f() { let _: u32 = true; }");
    let file_id = file_id.file_id(&db);
    let config = DiagnosticsConfig::test_sample();
    hir::attach_db(&db, || {
        let old = super::semantic_diagnostics_of_bodies(
            &db,
            &config,
            &AssistResolveStrategy::All,
            file_id,
            None,
            &|_| true,
        );
        let new = super::semantic_diagnostics_of_bodies_with_timings(
            &db,
            &config,
            &AssistResolveStrategy::All,
            file_id,
            None,
            &|_| true,
            None,
        );
        assert_eq!(format!("{old:#?}"), format!("{new:#?}"));
        assert!(new.iter().any(|diag| diag.code.as_str() == "E0308"));
    });
    let mut phases = super::SemanticDiagnosticPhases::new(None);
    assert!(phases.output.is_none() && phases.started.is_none());
    phases.next("diagnostics.semantic.syntax_walk");
    phases.next("diagnostics.semantic.module_context");
    phases.next("diagnostics.semantic.collect");
    phases.next("diagnostics.semantic.render_and_filter");
    assert!(phases.output.is_none() && phases.started.is_none());
    drop(phases);
}

/// Takes a multi-file input fixture with annotated cursor positions,
/// and checks that:
///  * a diagnostic is produced
///  * the first diagnostic fix trigger range touches the input cursor position
///  * that the contents of the file containing the cursor match `after` after the diagnostic fix is applied
#[track_caller]
pub(crate) fn check_fix(
    #[rust_analyzer::rust_fixture] ra_fixture_before: &str,
    #[rust_analyzer::rust_fixture] ra_fixture_after: &str,
) {
    check_nth_fix(0, ra_fixture_before, ra_fixture_after);
}
/// Takes a multi-file input fixture with annotated cursor positions,
/// and checks that:
///  * a diagnostic is produced
///  * every diagnostic fixes trigger range touches the input cursor position
///  * that the contents of the file containing the cursor match `after` after each diagnostic fix is applied
pub(crate) fn check_fixes(
    #[rust_analyzer::rust_fixture] ra_fixture_before: &str,
    ra_fixtures_after: Vec<&str>,
) {
    for (i, ra_fixture_after) in ra_fixtures_after.iter().enumerate() {
        check_nth_fix(i, ra_fixture_before, ra_fixture_after)
    }
}

#[track_caller]
fn check_nth_fix(
    nth: usize,
    #[rust_analyzer::rust_fixture] ra_fixture_before: &str,
    #[rust_analyzer::rust_fixture] ra_fixture_after: &str,
) {
    let mut config = DiagnosticsConfig::test_sample();
    config.expr_fill_default = ExprFillDefaultMode::Default;
    check_nth_fix_with_config(config, nth, ra_fixture_before, ra_fixture_after)
}

#[track_caller]
pub(crate) fn check_fix_with_disabled(
    #[rust_analyzer::rust_fixture] ra_fixture_before: &str,
    #[rust_analyzer::rust_fixture] ra_fixture_after: &str,
    disabled: &[&str],
) {
    let mut config = DiagnosticsConfig::test_sample();
    config.expr_fill_default = ExprFillDefaultMode::Default;
    config.disabled.extend(disabled.iter().map(|&disabled| disabled.to_owned()));
    check_nth_fix_with_config(config, 0, ra_fixture_before, ra_fixture_after)
}

#[track_caller]
fn check_nth_fix_with_config(
    config: DiagnosticsConfig,
    nth: usize,
    #[rust_analyzer::rust_fixture] ra_fixture_before: &str,
    #[rust_analyzer::rust_fixture] ra_fixture_after: &str,
) {
    let after = trim_indent(ra_fixture_after);

    let (db, file_position) = RootDatabase::with_position(ra_fixture_before);
    let diagnostic = hir::attach_db(&db, || {
        super::full_diagnostics(
            &db,
            &config,
            &AssistResolveStrategy::All,
            file_position.file_id.file_id(&db),
        )
        .pop()
        .expect("no diagnostics")
    });
    let fix = &diagnostic
        .fixes
        .unwrap_or_else(|| panic!("{:?} diagnostic misses fixes", diagnostic.code))[nth];
    let actual = {
        let source_change = fix.source_change.as_ref().unwrap();
        let file_id = *source_change.source_file_edits.keys().next().unwrap();
        let mut actual = db.file_text(file_id).text(&db).to_string();

        for (edit, snippet_edit) in source_change.source_file_edits.values() {
            edit.apply(&mut actual);
            if let Some(snippet_edit) = snippet_edit {
                snippet_edit.apply(&mut actual);
            }
        }
        actual
    };

    assert!(
        fix.target.contains_inclusive(file_position.offset),
        "diagnostic fix range {:?} does not touch cursor position {:?}",
        fix.target,
        file_position.offset
    );
    assert_eq_text!(&after, &actual);
}

pub(crate) fn check_fixes_unordered(
    #[rust_analyzer::rust_fixture] ra_fixture_before: &str,
    ra_fixtures_after: Vec<&str>,
) {
    for ra_fixture_after in ra_fixtures_after.iter() {
        check_has_fix(ra_fixture_before, ra_fixture_after)
    }
}

#[track_caller]
pub(crate) fn check_has_fix(
    #[rust_analyzer::rust_fixture] ra_fixture_before: &str,
    #[rust_analyzer::rust_fixture] ra_fixture_after: &str,
) {
    let after = trim_indent(ra_fixture_after);

    let (db, file_position) = RootDatabase::with_position(ra_fixture_before);
    let mut conf = DiagnosticsConfig::test_sample();
    conf.expr_fill_default = ExprFillDefaultMode::Default;
    let fix = hir::attach_db(&db, || {
        super::full_diagnostics(
            &db,
            &conf,
            &AssistResolveStrategy::All,
            file_position.file_id.file_id(&db),
        )
    })
    .into_iter()
    .find(|d| {
        d.fixes
            .as_ref()
            .and_then(|fixes| {
                fixes.iter().find(|fix| {
                    if !fix.target.contains_inclusive(file_position.offset) {
                        return false;
                    }
                    let actual = {
                        let source_change = fix.source_change.as_ref().unwrap();
                        let file_id = *source_change.source_file_edits.keys().next().unwrap();
                        let mut actual = db.file_text(file_id).text(&db).to_string();

                        for (edit, snippet_edit) in source_change.source_file_edits.values() {
                            edit.apply(&mut actual);
                            if let Some(snippet_edit) = snippet_edit {
                                snippet_edit.apply(&mut actual);
                            }
                        }
                        actual
                    };
                    after == actual
                })
            })
            .is_some()
    });
    assert!(fix.is_some(), "no diagnostic with desired fix");
}

/// Checks that there's a diagnostic *without* fix at `$0`.
pub(crate) fn check_no_fix(#[rust_analyzer::rust_fixture] ra_fixture: &str) {
    let (db, file_position) = RootDatabase::with_position(ra_fixture);
    let diagnostic = hir::attach_db(&db, || {
        super::full_diagnostics(
            &db,
            &DiagnosticsConfig::test_sample(),
            &AssistResolveStrategy::All,
            file_position.file_id.file_id(&db),
        )
    })
    .pop()
    .unwrap();
    assert!(diagnostic.fixes.is_none(), "got a fix when none was expected: {diagnostic:?}");
}

#[track_caller]
pub(crate) fn check_diagnostics(#[rust_analyzer::rust_fixture] ra_fixture: &str) {
    let mut config = DiagnosticsConfig::test_sample();
    config.disabled.insert("inactive-code".to_owned());
    check_diagnostics_with_config(config, ra_fixture)
}

#[track_caller]
pub(crate) fn check_diagnostics_with_disabled(
    #[rust_analyzer::rust_fixture] ra_fixture: &str,
    disabled: &[&str],
) {
    let mut config = DiagnosticsConfig::test_sample();
    config.disabled.extend(disabled.iter().map(|&s| s.to_owned()));
    check_diagnostics_with_config(config, ra_fixture)
}

#[track_caller]
pub(crate) fn check_diagnostics_with_config(
    config: DiagnosticsConfig,
    #[rust_analyzer::rust_fixture] ra_fixture: &str,
) {
    let _tracing = setup_tracing();

    let (db, files) = RootDatabase::with_many_files(ra_fixture);
    let mut annotations = files
        .iter()
        .copied()
        .flat_map(|file_id| {
            hir::attach_db(&db, || {
                super::full_diagnostics(
                    &db,
                    &config,
                    &AssistResolveStrategy::All,
                    file_id.file_id(&db),
                )
                .into_iter()
                .map(|d| {
                    let mut annotation = String::new();
                    if let Some(fixes) = &d.fixes {
                        assert!(!fixes.is_empty());
                        annotation.push_str("💡 ")
                    }
                    annotation.push_str(match d.severity {
                        Severity::Error => "error",
                        Severity::WeakWarning => "weak",
                        Severity::Warning => "warn",
                        Severity::Allow => "allow",
                    });
                    annotation.push_str(": ");
                    annotation.push_str(&d.message);
                    (d.range, annotation)
                })
            })
        })
        .map(|(diagnostic, annotation)| (diagnostic.file_id, (diagnostic.range, annotation)))
        .into_group_map();
    for file_id in files {
        let file_id = file_id.file_id(&db);
        let line_index = line_index(&db, file_id);

        let mut actual = annotations.remove(&file_id).unwrap_or_default();
        let mut expected = extract_annotations(db.file_text(file_id).text(&db));
        expected.sort_by_key(|(range, s)| (range.start(), s.clone()));
        actual.sort_by_key(|(range, s)| (range.start(), s.clone()));
        // FIXME: We should panic on duplicates instead, but includes currently cause us to report
        // diagnostics twice for the calling module when both files are queried.
        actual.dedup();
        // actual.iter().duplicates().for_each(|(range, msg)| {
        //     panic!("duplicate diagnostic at {:?}: {msg:?}", line_index.line_col(range.start()))
        // });
        if expected.is_empty() {
            // makes minicore smoke test debuggable
            for (e, _) in &actual {
                eprintln!(
                    "Code in range {e:?} = {}",
                    &db.file_text(file_id).text(&db)[usize::from(e.start())..usize::from(e.end())]
                )
            }
        }
        if expected != actual {
            let fneg = expected
                .iter()
                .filter(|x| !actual.contains(x))
                .map(|(range, s)| (line_index.line_col(range.start()), range, s))
                .collect::<Vec<_>>();
            let fpos = actual
                .iter()
                .filter(|x| !expected.contains(x))
                .map(|(range, s)| (line_index.line_col(range.start()), range, s))
                .collect::<Vec<_>>();

            panic!("Diagnostic test failed.\nFalse negatives: {fneg:?}\nFalse positives: {fpos:?}");
        }
    }
}

#[test]
fn test_disabled_diagnostics() {
    let mut config = DiagnosticsConfig::test_sample();
    config.disabled.insert("E0583".into());

    let (db, file_id) = RootDatabase::with_single_file(r#"mod foo;"#);
    let file_id = file_id.file_id(&db);

    let diagnostics = hir::attach_db(&db, || {
        super::full_diagnostics(&db, &config, &AssistResolveStrategy::All, file_id)
    });
    assert!(diagnostics.is_empty());

    let diagnostics = hir::attach_db(&db, || {
        super::full_diagnostics(
            &db,
            &DiagnosticsConfig::test_sample(),
            &AssistResolveStrategy::All,
            file_id,
        )
    });
    assert!(!diagnostics.is_empty());
}

#[test]
fn minicore_smoke_test() {
    if test_utils::skip_slow_tests() {
        return;
    }

    fn check(minicore: MiniCore) {
        let source = minicore.source_code(MiniCore::RAW_SOURCE);
        let mut config = DiagnosticsConfig::test_sample();
        // This should be ignored since we conditionally remove code which creates single item use with braces
        config.disabled.insert("unused_braces".to_owned());
        config.disabled.insert("unused_variables".to_owned());
        config.disabled.insert("remove-unnecessary-else".to_owned());
        check_diagnostics_with_config(config, &source);
    }

    // Checks that there is no diagnostic in minicore for each flag.
    for flag in MiniCore::available_flags(MiniCore::RAW_SOURCE) {
        if flag == "clone" {
            // Clone without copy has `moved-out-of-ref`, so ignoring.
            // FIXME: Maybe we should merge copy and clone in a single flag?
            continue;
        }
        eprintln!("Checking minicore flag {flag}");
        check(MiniCore::from_flags([flag]));
    }
    // And one time for all flags, to check codes which are behind multiple flags + prevent name collisions
    eprintln!("Checking all minicore flags");
    check(MiniCore::from_flags(MiniCore::available_flags(MiniCore::RAW_SOURCE)))
}
