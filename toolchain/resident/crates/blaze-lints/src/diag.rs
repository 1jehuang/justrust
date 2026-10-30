//! A lint's diagnostic as data: what rustc's `DiagInner` holds of one, which is what
//! [`crate::emit`] writes as rustc does. It crosses the daemon's socket as it is, so the source
//! it points into is read where it is written, by its path.

use serde::{Deserialize, Serialize};
use syntax::TextRange;

use crate::Level;

/// A range of a file: bytes from 0. `file` is the path as rustc shows it, from the root of the
/// workspace when the file is below it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    pub lo: u32,
    pub hi: u32,
    /// The macro whose expansion it is in, when it is in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<Box<Expansion>>,
}

/// A macro's expansion a span is in: rustc's `ExpnData`, as much of it as diagnostics show.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Expansion {
    /// Where the macro was called: its call, or for a derive the path in `#[derive]`.
    pub call_site: Span,
    pub kind: MacroKind,
    pub name: String,
    /// Where the macro is defined, when rustc says.
    pub def_site: Option<Span>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MacroKind {
    Bang,
    Attr,
    Derive,
}

impl MacroKind {
    /// As rustc's notes name it.
    pub fn descr(self) -> &'static str {
        match self {
            MacroKind::Bang => "macro",
            MacroKind::Attr => "attribute macro",
            MacroKind::Derive => "derive macro",
        }
    }
}

impl Expansion {
    /// As rustc's JSON names it: `name!`, `#[name]`, `#[derive(Name)]`.
    pub fn decl_name(&self) -> String {
        match self.kind {
            MacroKind::Bang => format!("{}!", self.name),
            MacroKind::Attr => format!("#[{}]", self.name),
            MacroKind::Derive => format!("#[derive({})]", self.name),
        }
    }
}

impl Span {
    pub fn new(file: &str, range: TextRange) -> Span {
        Span {
            file: file.to_owned(),
            lo: range.start().into(),
            hi: range.end().into(),
            expansion: None,
        }
    }

    pub fn to(&self, other: &Span) -> Span {
        Span { lo: self.lo.min(other.lo), hi: self.hi.max(other.hi), ..self.clone() }
    }

    pub fn with_lo(&self, lo: u32) -> Span {
        Span { lo, ..self.clone() }
    }

    pub fn with_hi(&self, hi: u32) -> Span {
        Span { hi, ..self.clone() }
    }

    pub fn shrink_to_lo(&self) -> Span {
        Span { hi: self.lo, ..self.clone() }
    }

    pub fn shrink_to_hi(&self) -> Span {
        Span { lo: self.hi, ..self.clone() }
    }

    pub fn is_empty(&self) -> bool {
        self.lo == self.hi
    }
}

/// rustc's `MultiSpan`: the places a diagnostic is of, and the labels on places.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MultiSpan {
    pub primary: Vec<Span>,
    pub labels: Vec<(Span, String)>,
}

/// A span with whether it is one of the primary ones and its label: rustc's `SpanLabel`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpanLabel<'a> {
    pub span: &'a Span,
    pub is_primary: bool,
    pub label: Option<&'a str>,
}

impl From<Span> for MultiSpan {
    fn from(span: Span) -> MultiSpan {
        MultiSpan::of(span)
    }
}

impl MultiSpan {
    pub fn of(span: Span) -> MultiSpan {
        MultiSpan { primary: vec![span], labels: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.primary.is_empty() && self.labels.is_empty()
    }

    pub fn primary_span(&self) -> Option<&Span> {
        self.primary.first()
    }

    /// rustc's `MultiSpan::span_labels`: the labelled spans in the order they were labelled,
    /// then the primary spans that have no label.
    pub fn span_labels(&self) -> Vec<SpanLabel<'_>> {
        let mut labels: Vec<SpanLabel<'_>> = self
            .labels
            .iter()
            .map(|(span, label)| SpanLabel {
                span,
                is_primary: self.primary.contains(span),
                label: Some(label),
            })
            .collect();
        for span in &self.primary {
            if !labels.iter().any(|it| it.span == span) {
                labels.push(SpanLabel { span, is_primary: true, label: None });
            }
        }
        labels
    }
}

/// The level of a part of a diagnostic: rustc's `Sublevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sublevel {
    Note,
    Help,
    Warning,
}

impl Sublevel {
    pub fn name(self) -> &'static str {
        match self {
            Sublevel::Note => "note",
            Sublevel::Help => "help",
            Sublevel::Warning => "warning",
        }
    }
}

/// A note, a help or a warning under a diagnostic: rustc's `Subdiag`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Child {
    pub level: Sublevel,
    pub message: String,
    pub span: MultiSpan,
    /// rustc's `note_once` and `help_once`: written with the first diagnostic of a crate it is
    /// part of, and not again in that crate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,
}

/// How sure a suggestion is: rustc's `Applicability`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Applicability {
    MachineApplicable,
    MaybeIncorrect,
    HasPlaceholders,
    Unspecified,
}

impl Applicability {
    pub fn name(self) -> &'static str {
        match self {
            Applicability::MachineApplicable => "MachineApplicable",
            Applicability::MaybeIncorrect => "MaybeIncorrect",
            Applicability::HasPlaceholders => "HasPlaceholders",
            Applicability::Unspecified => "Unspecified",
        }
    }
}

/// How a suggestion is shown: rustc's `SuggestionStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionStyle {
    /// Its message only, never its code.
    HideCodeInline,
    HideCodeAlways,
    /// Only for tools: not written.
    CompletelyHidden,
    /// Inline when it is short, else as a patch.
    ShowCode,
    /// Always as a patch.
    ShowAlways,
}

impl SuggestionStyle {
    pub fn hide_inline(self) -> bool {
        !matches!(self, SuggestionStyle::ShowCode)
    }
}

/// A suggested change: one or more ways to substitute parts of the source: rustc's
/// `CodeSuggestion`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub message: String,
    /// Each way to substitute, each a set of disjoint parts.
    pub substitutions: Vec<Vec<(Span, String)>>,
    pub applicability: Applicability,
    pub style: SuggestionStyle,
}

/// A lint's diagnostic, at the level it was found at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// As rustc's JSON names it: `clippy::needless_bool`, `unfulfilled_lint_expectations`.
    pub lint: String,
    /// `Warn`, `ForceWarn`, `Deny` or `Forbid`: the levels a diagnostic is written at.
    pub level: Level,
    pub message: String,
    pub span: MultiSpan,
    pub children: Vec<Child>,
    pub suggestions: Vec<Suggestion>,
    /// rustc turns off the suggestions of a lint in code of another crate's macro.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suggestions_disabled: bool,
}

impl Diagnostic {
    /// Whether it is written as an error.
    pub fn is_error(&self) -> bool {
        self.level >= Level::Deny
    }

    /// Whether cargo counts it as one `cargo clippy --fix` applies: it has a suggestion rustfix
    /// takes as it is.
    pub fn is_fixable(&self) -> bool {
        !self.suggestions_disabled
            && self
                .suggestions
                .iter()
                .any(|it| it.applicability == Applicability::MachineApplicable)
    }

    pub fn note(&mut self, message: impl Into<String>) -> &mut Diagnostic {
        self.sub(Sublevel::Note, message, MultiSpan::default(), false)
    }

    pub fn help(&mut self, message: impl Into<String>) -> &mut Diagnostic {
        self.sub(Sublevel::Help, message, MultiSpan::default(), false)
    }

    pub fn span_note(&mut self, span: Span, message: impl Into<String>) -> &mut Diagnostic {
        self.sub(Sublevel::Note, message, MultiSpan::of(span), false)
    }

    pub fn span_help(&mut self, span: Span, message: impl Into<String>) -> &mut Diagnostic {
        self.sub(Sublevel::Help, message, MultiSpan::of(span), false)
    }

    pub fn sub(
        &mut self,
        level: Sublevel,
        message: impl Into<String>,
        span: MultiSpan,
        once: bool,
    ) -> &mut Diagnostic {
        self.children.push(Child { level, message: message.into(), span, once });
        self
    }

    /// A label on `span`, which is primary when it is one of the primary spans.
    pub fn span_label(&mut self, span: Span, label: impl Into<String>) -> &mut Diagnostic {
        self.span.labels.push((span, label.into()));
        self
    }

    /// rustc's `span_suggestion`: shown inline when it is short.
    pub fn suggestion(
        &mut self,
        span: Span,
        message: impl Into<String>,
        code: impl Into<String>,
        applicability: Applicability,
    ) -> &mut Diagnostic {
        self.suggestion_with_style(
            vec![(span, code.into())],
            message,
            applicability,
            SuggestionStyle::ShowCode,
        )
    }

    /// rustc's `multipart_suggestion`: the parts are one change, shown as a patch even when
    /// there is one.
    pub fn multipart_suggestion(
        &mut self,
        message: impl Into<String>,
        parts: Vec<(Span, String)>,
        applicability: Applicability,
    ) -> &mut Diagnostic {
        self.suggestion_with_style(parts, message, applicability, SuggestionStyle::ShowAlways)
    }

    pub fn suggestion_with_style(
        &mut self,
        mut parts: Vec<(Span, String)>,
        message: impl Into<String>,
        applicability: Applicability,
        style: SuggestionStyle,
    ) -> &mut Diagnostic {
        let mut seen = rustc_hash::FxHashSet::default();
        parts.retain(|(span, code)| seen.insert((span.lo, span.hi, code.clone())));
        self.suggestions.push(Suggestion {
            message: message.into(),
            substitutions: vec![parts],
            applicability,
            style,
        });
        self
    }
}
