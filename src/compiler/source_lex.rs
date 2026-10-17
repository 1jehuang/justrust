//! Borrowed, policy-free lexical transport into one sealed original-source snapshot.
//!
//! Construction imports sources but does not project token spans. A lexer must
//! finish lexical validation before requesting projections. Configured input is
//! already filtered by source preparation, so this bridge never reinjects attrs
//! or documentation. Raw identity input performs no preparation or filesystem I/O.
use std::num::NonZeroU64;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;

use super::super::expansion_budget::{BudgetFailure, Counter, ExpansionBudget};
use super::super::token_session::{OriginalSpan, SourceSnapshot, TokenSession, allocate_identity};
use super::{
    MAX_BYTES, MAX_ORIGINS, OriginSegment, PreparedSource, SourceError, SourceErrorKind,
    SourceLocation, SourceSpan,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PreparedViewId(NonZeroU64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedMode {
    Configured,
    SyntaxOnly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectedSpan {
    pub original: OriginalSpan,
    /// Anchored transport whitespace/module delimiters, not original spelling.
    pub synthetic: bool,
}

#[derive(Clone)]
enum StructureFailure {
    Budget(BudgetFailure),
    Contract(&'static str),
}

pub struct PreparedLexInput<'a> {
    text: &'a str,
    origins: Option<&'a [OriginSegment]>,
    snapshot: SourceSnapshot,
    view: PreparedViewId,
    budget: ExpansionBudget,
    structure: OnceLock<Result<(), StructureFailure>>,
}
impl<'a> PreparedLexInput<'a> {
    pub fn prepared(
        prepared: &'a PreparedSource,
        budget: &ExpansionBudget,
    ) -> Result<Self, SourceError> {
        Self::new(
            &prepared.text,
            Some(&prepared.origins),
            SourceSnapshot::copy_borrowed(&prepared.sources, budget)
                .map_err(SourceError::from_token)?,
            budget,
        )
    }
    pub fn identity(
        name: &Path,
        text: &'a str,
        budget: &ExpansionBudget,
    ) -> Result<Self, SourceError> {
        Self::new(
            text,
            None,
            SourceSnapshot::from_text(name, text, budget).map_err(SourceError::from_token)?,
            budget,
        )
    }
    fn new(
        text: &'a str,
        origins: Option<&'a [OriginSegment]>,
        snapshot: SourceSnapshot,
        budget: &ExpansionBudget,
    ) -> Result<Self, SourceError> {
        budget
            .check()
            .map_err(|error| SourceError::from_token(error.into()))?;
        if text.len() > MAX_BYTES || origins.is_some_and(|segments| segments.len() > MAX_ORIGINS) {
            return Err(Self::error(
                SourceErrorKind::Limit,
                "lexical source transport limit exceeded",
            ));
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let view = PreparedViewId(allocate_identity(&NEXT).map_err(SourceError::from_token)?);
        Ok(Self {
            text,
            origins,
            snapshot,
            view,
            budget: budget.clone(),
            structure: OnceLock::new(),
        })
    }
    pub fn text(&self) -> &'a str {
        self.text
    }
    pub fn view_id(&self) -> PreparedViewId {
        self.view
    }
    pub fn mode(&self) -> PreparedMode {
        if self.origins.is_some() {
            PreparedMode::Configured
        } else {
            PreparedMode::SyntaxOnly
        }
    }
    pub fn snapshot(&self) -> &SourceSnapshot {
        &self.snapshot
    }
    /// Setup validation only. This does not inspect origins or project spelling,
    /// so callers can still complete lexical validation before any projection.
    pub fn validate_session(&self, session: &TokenSession) -> Result<(), SourceError> {
        if !self.snapshot.same_snapshot(session.sources())
            || !self.budget.same_authority(session.budget())
        {
            return Err(Self::contract(
                "lexical input and token session have foreign source or budget authority",
            ));
        }
        self.budget
            .check()
            .map_err(|error| SourceError::from_token(error.into()))
    }
    pub fn project(&self, range: Range<usize>) -> Result<OriginalSpan, SourceError> {
        Ok(self.projected(range)?.original)
    }
    pub fn is_synthetic(&self, range: Range<usize>) -> Result<bool, SourceError> {
        Ok(self.projected(range)?.synthetic)
    }
    pub fn eof(&self) -> Result<OriginalSpan, SourceError> {
        self.project(self.text.len()..self.text.len())
    }
    pub fn projected(&self, range: Range<usize>) -> Result<ProjectedSpan, SourceError> {
        self.budget
            .check()
            .map_err(|error| SourceError::from_token(error.into()))?;
        self.validate_structure()?;
        let spelling = self
            .text
            .get(range.clone())
            .ok_or_else(|| Self::contract("invalid prepared byte range or UTF-8 boundary"))?;
        self.budget
            .charge(Counter::Work, 1)
            .map_err(|error| SourceError::from_token(error.into()))?;
        let (file_index, lo, hi, synthetic) = if let Some(segments) = self.origins {
            let index = segments
                .partition_point(|segment| segment.prepared.start <= range.start)
                .checked_sub(1)
                .ok_or_else(|| Self::contract("prepared range has no original-source anchor"))?;
            let segment = &segments[index];
            if range.start < segment.prepared.start || range.end > segment.prepared.end {
                return Err(Self::contract(
                    "logical token crosses an original-source segment boundary",
                ));
            }
            let raw = super::source_map::map_range(segments, range.clone())
                .ok_or_else(|| Self::contract("invalid prepared origin projection"))?;
            (raw.file.0 as usize, raw.lo, raw.hi, segment.synthetic)
        } else {
            (
                0,
                u32::try_from(range.start)
                    .map_err(|_| Self::contract("prepared start exceeds source span range"))?,
                u32::try_from(range.end)
                    .map_err(|_| Self::contract("prepared end exceeds source span range"))?,
                false,
            )
        };
        let file = self
            .snapshot
            .file(file_index)
            .map_err(SourceError::from_token)?;
        let original = self
            .snapshot
            .span(file, lo, hi)
            .map_err(SourceError::from_token)?;
        if !synthetic {
            self.budget
                .charge(Counter::Work, spelling.len() as u64)
                .map_err(|error| SourceError::from_token(error.into()))?;
            let text = self.snapshot.text(file).map_err(SourceError::from_token)?;
            if text.get(lo as usize..hi as usize) != Some(spelling) {
                return Err(Self::contract(
                    "prepared spelling differs from sealed original source",
                ));
            }
        }
        Ok(ProjectedSpan {
            original,
            synthetic,
        })
    }
    fn validate_structure(&self) -> Result<(), SourceError> {
        self.structure
            .get_or_init(|| {
                let Some(segments) = self.origins else {
                    return Ok(());
                };
                self.budget
                    .charge(Counter::Work, segments.len() as u64)
                    .map_err(StructureFailure::Budget)?;
                if segments.is_empty() {
                    return Err(StructureFailure::Contract(
                        "prepared source has no origin segments",
                    ));
                }
                let mut end = 0;
                for segment in segments {
                    if segment.prepared.start != end
                        || segment.prepared.start > segment.prepared.end
                        || segment.prepared.end > self.text.len()
                        || !self.text.is_char_boundary(segment.prepared.start)
                        || !self.text.is_char_boundary(segment.prepared.end)
                        || segment.origin.lo > segment.origin.hi
                        || (!segment.synthetic
                            && segment.prepared.len()
                                != (segment.origin.hi - segment.origin.lo) as usize)
                    {
                        return Err(StructureFailure::Contract(
                            "invalid, overlapping or incomplete prepared origin segments",
                        ));
                    }
                    end = segment.prepared.end;
                }
                if end != self.text.len() {
                    return Err(StructureFailure::Contract(
                        "prepared origin segments do not cover the filtered source",
                    ));
                }
                Ok(())
            })
            .clone()
            .map_err(|error| match error {
                StructureFailure::Budget(error) => SourceError::from_token(error.into()),
                StructureFailure::Contract(message) => Self::contract(message),
            })
    }
    pub fn render(&self, span: OriginalSpan) -> Result<SourceSpan, SourceError> {
        self.snapshot
            .render_span(span)
            .map_err(SourceError::from_token)
    }
    pub fn location(&self, span: OriginalSpan) -> Result<SourceLocation, SourceError> {
        let (path, line, column) = self
            .snapshot
            .location(span)
            .map_err(SourceError::from_token)?;
        Ok(SourceLocation {
            path: path.to_owned(),
            line,
            column,
        })
    }
    pub fn source_error(
        &self,
        kind: SourceErrorKind,
        primary: Option<OriginalSpan>,
        message: impl Into<String>,
    ) -> SourceError {
        let Some(span) = primary else {
            return Self::error(kind, message);
        };
        let raw = match self.render(span) {
            Ok(raw) => raw,
            Err(error) => return error,
        };
        let location = match self.location(span) {
            Ok(location) => location,
            Err(error) => return error,
        };
        SourceError {
            kind,
            primary: Some(raw),
            message: message.into(),
            _diagnostic_lease: None,
            location: Some(location),
        }
    }
    fn contract(message: impl Into<String>) -> SourceError {
        Self::error(SourceErrorKind::Contract, message)
    }
    fn error(kind: SourceErrorKind, message: impl Into<String>) -> SourceError {
        SourceError {
            kind,
            primary: None,
            message: message.into(),
            _diagnostic_lease: None,
            location: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::{BudgetFailureKind, ExpansionLimits};
    use crate::compiler::source::{FileId, SourceOptions, prepare_text};
    use crate::compiler::token_session::TokenError;

    #[test]
    fn prepared_borrows_filtered_text_and_projects_unicode_original_locations() {
        let source = prepare_text(
            "original.rs",
            "// α🙂\r\nfn main() {}",
            &SourceOptions::default(),
        )
        .unwrap();
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::prepared(&source, &budget).unwrap();
        assert!(std::ptr::eq(input.text().as_ptr(), source.text.as_ptr()));
        let start = input.text().find("main").unwrap();
        let span = input.project(start..start + 4).unwrap();
        let location = input.location(span).unwrap();
        assert_eq!(
            (location.path.as_path(), location.line, location.column),
            (Path::new("original.rs"), 2, 4)
        );
        assert!(!input.is_synthetic(start..start + 4).unwrap());
        assert_eq!(input.mode(), PreparedMode::Configured);
    }
    #[test]
    fn identity_does_not_apply_attrs_cfg_or_io_and_has_distinct_views() {
        let text = "#[cfg(any())] mod absent; /// docs\n";
        let budget = ExpansionBudget::default();
        let a = PreparedLexInput::identity(Path::new("raw.rs"), text, &budget).unwrap();
        let b = PreparedLexInput::identity(Path::new("raw.rs"), text, &budget).unwrap();
        assert_eq!(a.text(), text);
        assert_eq!(a.mode(), PreparedMode::SyntaxOnly);
        assert_ne!(a.view_id(), b.view_id());
        let span = a.project(0..1).unwrap();
        assert_eq!(b.render(span).unwrap_err().kind, SourceErrorKind::Contract);
        let clone = a.snapshot().clone();
        assert!(clone.same_snapshot(a.snapshot()));
    }
    #[test]
    fn lexical_pairing_requires_both_source_and_shared_budget_identity() {
        let budget = ExpansionBudget::default();
        let input =
            PreparedLexInput::identity(Path::new("pair.rs"), "fn main(){}", &budget).unwrap();
        let matching = TokenSession::new(input.snapshot().clone(), budget.clone()).unwrap();
        input.validate_session(&matching).unwrap();
        let independent = ExpansionBudget::new(budget.limits());
        let split = TokenSession::new(input.snapshot().clone(), independent).unwrap();
        assert_eq!(
            input.validate_session(&split).unwrap_err().kind,
            SourceErrorKind::Contract
        );
        let other =
            PreparedLexInput::identity(Path::new("pair.rs"), input.text(), &budget).unwrap();
        let foreign = TokenSession::new(other.snapshot().clone(), budget).unwrap();
        assert_eq!(
            input.validate_session(&foreign).unwrap_err().kind,
            SourceErrorKind::Contract
        );
    }
    #[test]
    fn eof_and_empty_identity_are_explicit_checked_original_anchors() {
        let budget = ExpansionBudget::default();
        for text in ["", "α\r\n"] {
            let input = PreparedLexInput::identity(Path::new("empty.rs"), text, &budget).unwrap();
            let raw = input.render(input.eof().unwrap()).unwrap();
            assert_eq!((raw.lo, raw.hi), (text.len() as u32, text.len() as u32));
        }
        let source = prepare_text("empty.rs", "", &SourceOptions::default()).unwrap();
        let input = PreparedLexInput::prepared(&source, &budget).unwrap();
        assert_eq!(input.render(input.eof().unwrap()).unwrap().lo, 0);
    }
    #[test]
    fn malformed_ranges_mappings_and_spelling_are_contract_not_syntax() {
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::identity(Path::new("utf8.rs"), "α", &budget).unwrap();
        assert_eq!(
            input.project(1..2).unwrap_err().kind,
            SourceErrorKind::Contract
        );
        let mut source =
            prepare_text("forged.rs", "fn main() {}", &SourceOptions::default()).unwrap();
        source.text.replace_range(0..2, "xx");
        let input = PreparedLexInput::prepared(&source, &budget).unwrap();
        assert_eq!(
            input.project(0..2).unwrap_err().kind,
            SourceErrorKind::Contract
        );
        drop(input);
        source.origins[0].origin.file = FileId(u32::MAX);
        let input = PreparedLexInput::prepared(&source, &budget).unwrap();
        assert_eq!(
            input.project(0..2).unwrap_err().kind,
            SourceErrorKind::Contract
        );
    }
    #[test]
    fn cached_structure_errors_recreate_unleased_diagnostics_without_repeating_work() {
        let mut source =
            prepare_text("cache.rs", "fn main(){}", &SourceOptions::default()).unwrap();
        source.origins[0].prepared.start = 1;
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::prepared(&source, &budget).unwrap();
        let first = input.validate_structure().unwrap_err();
        let work = budget.snapshot().counters[Counter::Work as usize];
        let second = input.validate_structure().unwrap_err();
        assert_eq!(first.kind, SourceErrorKind::Contract);
        assert_eq!(first.kind, second.kind);
        assert_eq!(first.message, second.message);
        assert!(first._diagnostic_lease.is_none());
        assert!(second._diagnostic_lease.is_none());
        assert_eq!(budget.snapshot().counters[Counter::Work as usize], work);
    }

    #[test]
    fn cached_structure_budget_failure_keeps_the_exact_latched_cause() {
        let source = prepare_text("cache.rs", "fn main(){}", &SourceOptions::default()).unwrap();
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::prepared(&source, &budget).unwrap();
        let failure = budget
            .charge(
                Counter::Work,
                budget.limits().counters[Counter::Work as usize] + 1,
            )
            .unwrap_err();
        let first = input.validate_structure().unwrap_err();
        let second = input.validate_structure().unwrap_err();
        assert_eq!(first.kind, SourceErrorKind::Limit);
        assert_eq!(
            first.message,
            TokenError::Budget(failure.clone()).to_string()
        );
        assert_eq!(second.message, first.message);
        assert_eq!(budget.check().unwrap_err(), failure);
    }
    #[test]
    fn complete_logical_tokens_cannot_cross_segments_even_with_equal_bytes() {
        let budget = ExpansionBudget::default();
        let mut source = prepare_text("split.rs", "abcd", &SourceOptions::default()).unwrap();
        source.origins = vec![
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
                    file: FileId(0),
                    lo: 2,
                    hi: 4,
                },
                synthetic: false,
            },
        ];
        let input = PreparedLexInput::prepared(&source, &budget).unwrap();
        assert_eq!(
            input.project(1..3).unwrap_err().kind,
            SourceErrorKind::Contract
        );
        let raw = input.render(input.project(2..2).unwrap()).unwrap();
        assert_eq!(raw.lo, 2);
    }
    #[test]
    fn synthetic_delimiters_remain_explicit_anchors_not_original_spelling() {
        let budget = ExpansionBudget::default();
        let source =
            prepare_text("module.rs", "mod child;", &SourceOptions::default()).unwrap_err();
        // Text preparation correctly has no external-module capability.
        assert_eq!(source.kind, SourceErrorKind::Unsupported);
        let mut prepared = prepare_text("anchor.rs", ";", &SourceOptions::default()).unwrap();
        prepared.text = "{".into();
        prepared.origins[0].synthetic = true;
        let input = PreparedLexInput::prepared(&prepared, &budget).unwrap();
        let projected = input.projected(0..1).unwrap();
        assert!(projected.synthetic);
        let raw = input.render(projected.original).unwrap();
        assert_eq!((raw.lo, raw.hi), (0, 1));
    }
    #[test]
    fn structured_token_failures_keep_honest_categories_and_display_prefixes() {
        assert_eq!(
            SourceError::from_token(TokenError::InvalidRange).kind,
            SourceErrorKind::Contract
        );
        assert!(
            SourceError::from_token(TokenError::InvalidRange)
                .to_string()
                .starts_with("contract: ")
        );
        assert_eq!(
            SourceError::from_token(TokenError::IdExhausted).kind,
            SourceErrorKind::Infrastructure
        );
        assert!(
            SourceError::from_token(TokenError::IdExhausted)
                .to_string()
                .starts_with("infrastructure: ")
        );
        let budget = ExpansionBudget::default();
        let failure = budget.poison_allocation();
        assert_eq!(failure.kind, BudgetFailureKind::Allocation);
        assert_eq!(
            SourceError::from_token(failure.into()).kind,
            SourceErrorKind::Infrastructure
        );
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes: 0,
            ..ExpansionLimits::default()
        });
        assert_eq!(
            PreparedLexInput::identity(Path::new("limit.rs"), "x", &budget)
                .err()
                .unwrap()
                .kind,
            SourceErrorKind::Limit
        );
    }
    #[test]
    fn original_errors_retain_path_line_column_and_primary_span() {
        let budget = ExpansionBudget::default();
        let input =
            PreparedLexInput::identity(Path::new("scalar.rs"), "α🙂\r\nx", &budget).unwrap();
        let span = input.project(8..9).unwrap();
        let error = input.source_error(SourceErrorKind::Syntax, Some(span), "expected expression");
        assert_eq!(error.kind, SourceErrorKind::Syntax);
        assert_eq!(error.primary.unwrap().lo, 8);
        assert_eq!(error.to_string(), "scalar.rs:2:1: expected expression");
    }
    #[test]
    fn empty_client_plus_verified_provider_retains_checked_provenance() {
        use crate::compiler::source::{SourceProbeRequest, prepare_probe_file};
        use std::sync::atomic::Ordering;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let directory = std::env::temp_dir().join(format!(
            "justrust-empty-client-projection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let client = directory.join("client.rs");
        std::fs::write(&client, "").unwrap();
        let request = SourceProbeRequest {
            lock_path: Path::new(env!("CARGO_MANIFEST_DIR")).join("libraries/source-lock.json"),
            item: "core::convert::identity".into(),
        };
        let mut prepared =
            prepare_probe_file(&client, &SourceOptions::default(), &request).unwrap();
        assert_eq!(prepared.origins[0].prepared, 0..0);
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::prepared(&prepared, &budget).unwrap();
        let start = input.text().find("identity").unwrap();
        let projected = input.projected(start..start + 8).unwrap();
        assert!(!projected.synthetic);
        assert_eq!(input.render(projected.original).unwrap().file, FileId(1));
        assert!(
            input
                .location(projected.original)
                .unwrap()
                .path
                .ends_with("probes/identity/convert.rs")
        );
        // At the shared boundary the next segment, not the empty client, wins.
        assert_eq!(
            input.render(input.project(0..0).unwrap()).unwrap().file,
            FileId(1)
        );
        drop(input);
        prepared.origins[1].prepared.start += 1;
        let input = PreparedLexInput::prepared(&prepared, &budget).unwrap();
        assert_eq!(
            input.project(start..start + 8).unwrap_err().kind,
            SourceErrorKind::Contract
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn late_lexical_error_precedes_early_cross_segment_projection_failure() {
        use crate::compiler::lexer::{LexError, lex_prepared};
        use crate::compiler::token::Edition;
        use crate::compiler::token_session::TokenSession;
        let mut prepared =
            prepare_text("late.rs", "fn main() {}", &SourceOptions::default()).unwrap();
        let end = prepared.text.len();
        prepared.origins = vec![
            OriginSegment {
                prepared: 0..1,
                origin: SourceSpan {
                    file: FileId(0),
                    lo: 0,
                    hi: 1,
                },
                synthetic: false,
            },
            OriginSegment {
                prepared: 1..end,
                origin: SourceSpan {
                    file: FileId(0),
                    lo: 1,
                    hi: end as u32,
                },
                synthetic: false,
            },
        ];
        prepared.text.push('`');
        let budget = ExpansionBudget::default();
        let input = PreparedLexInput::prepared(&prepared, &budget).unwrap();
        let mut session = TokenSession::new(input.snapshot().clone(), budget).unwrap();
        let error = lex_prepared(&input, Edition::E2024, &mut session)
            .err()
            .unwrap();
        match error {
            LexError::Source { kind, prepared, .. } => {
                assert_eq!(kind, SourceErrorKind::Syntax);
                assert_eq!(prepared, end..end + 1);
            }
            error => panic!("lexical validation was masked by projection: {error}"),
        }
    }
}
