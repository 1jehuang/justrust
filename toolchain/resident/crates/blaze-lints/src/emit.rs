//! Diagnostics written as rustc writes them: in text, as its `annotate-snippets` emitter does,
//! and as its `--error-format=json` does, which cargo passes on as `compiler-message`s.
//!
//! Follows rustc's `compiler/rustc_errors/src/annotate_snippet_emitter_writer.rs`,
//! `emitter.rs` (`primary_span_formatted`, `is_different`, `detect_confusion_type`,
//! `normalize_whitespace`) and `json.rs` at rust-lang/rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0
//! (MIT or Apache-2.0), with the same version of `annotate-snippets` doing the layout.

use std::{collections::BTreeMap, sync::Arc};

use annotate_snippets::{AnnotationKind, Group, Patch, Renderer, Snippet};
use serde::Serialize;

use crate::{
    Level,
    diag::{
        Diagnostic, Expansion, MacroKind, MultiSpan, Span, Sublevel, Suggestion, SuggestionStyle,
    },
};

/// rustc's `MAX_SUGGESTIONS`: the most ways to substitute that one suggestion shows.
const MAX_SUGGESTIONS: usize = 4;

/// The width rustc lays diagnostics out for when it is not told one, as under cargo.
pub const WIDTH: usize = annotate_snippets::renderer::DEFAULT_TERM_WIDTH;

/// The text of the files diagnostics point into, by the path they name them by.
pub struct Sources {
    files: BTreeMap<String, Option<Arc<File>>>,
    read: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

/// A file's text and where its lines start.
pub struct File {
    pub text: String,
    lines: Vec<usize>,
}

impl File {
    pub fn new(text: String) -> File {
        let mut lines = vec![0];
        lines.extend(text.match_indices('\n').map(|(at, _)| at + 1).filter(|it| *it < text.len()));
        File { text, lines }
    }

    /// The 0-based line `at` is on.
    fn line_of(&self, at: usize) -> usize {
        self.lines.partition_point(|it| *it <= at).saturating_sub(1)
    }

    /// The bytes of 0-based line `line`, from its start to the start of the next one.
    fn line_bounds(&self, line: usize) -> std::ops::Range<usize> {
        let start = self.lines.get(line).copied().unwrap_or(self.text.len());
        let end = self.lines.get(line + 1).copied().unwrap_or(self.text.len());
        start..end
    }

    /// The text of 0-based line `line`, without its end.
    fn line_text(&self, line: usize) -> &str {
        let bounds = self.line_bounds(line);
        let text = &self.text[bounds];
        text.strip_suffix('\n').map(|it| it.strip_suffix('\r').unwrap_or(it)).unwrap_or(text)
    }

    /// The 1-based line and the 0-based column in chars of byte `at`.
    fn position(&self, at: usize) -> (usize, usize) {
        let at = floor(&self.text, at);
        let line = self.line_of(at);
        (line + 1, self.text[self.lines[line]..at].chars().count())
    }

    fn snippet(&self, span: &Span) -> Option<&str> {
        self.text.get(span.lo as usize..span.hi as usize)
    }
}

impl Sources {
    /// Files read with `read`, once each.
    pub fn new(read: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Sources {
        Sources { files: BTreeMap::new(), read: Box::new(read) }
    }

    /// Files whose text is given.
    pub fn of(files: impl IntoIterator<Item = (String, String)>) -> Sources {
        let mut sources = Sources::new(|_| None);
        for (name, text) in files {
            sources.files.insert(name, Some(Arc::new(File::new(text))));
        }
        sources
    }

    pub fn file(&mut self, name: &str) -> Option<Arc<File>> {
        if let Some(file) = self.files.get(name) {
            return file.clone();
        }
        let file = (self.read)(name).map(|it| Arc::new(File::new(it)));
        self.files.insert(name.to_owned(), file.clone());
        file
    }

    fn snippet(&mut self, span: &Span) -> Option<String> {
        Some(self.file(&span.file)?.snippet(span)?.to_owned())
    }
}

/// How the text of a diagnostic is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    /// `--error-format=short`: a line of where it is and what it says.
    pub short: bool,
    /// In colour, as to a terminal.
    pub ansi: bool,
    /// The columns it is laid out in: cargo passes rustc its terminal's, else rustc takes 140.
    pub width: usize,
}

impl Default for Style {
    fn default() -> Style {
        Style { short: false, ansi: false, width: WIDTH }
    }
}

/// `diagnostic` as rustc writes it, without the empty line it writes after one.
pub fn rendered(diagnostic: &Diagnostic, sources: &mut Sources, style: Style) -> String {
    let mut span = diagnostic.span.clone();
    let mut suggestions: Vec<Suggestion> = match diagnostic.suggestions_disabled {
        true => Vec::new(),
        false => diagnostic.suggestions.clone(),
    };
    primary_span_formatted(&mut span, &mut suggestions, sources);
    let mut children = diagnostic.children.clone();
    macro_backtrace(&mut span);
    for child in &mut children {
        macro_backtrace(&mut child.span);
    }
    let renderer = match style.ansi {
        true => Renderer::styled(),
        false => Renderer::plain(),
    };
    let renderer = renderer
        .term_width(style.width)
        .decor_style(annotate_snippets::renderer::DecorStyle::Ascii)
        .short_message(style.short);
    let level = level_of(diagnostic.level);
    let mut report: Vec<Group<'_>> = Vec::new();
    let mut group = Group::with_title(level.clone().primary_title(diagnostic.message.clone()));

    for (file, annotations) in primary_first(collect_annotations(&span), &span) {
        if let Some(snippet) = annotated_snippet(&file, annotations, sources) {
            group = group.element(snippet);
        }
    }

    for child in &children {
        let level = sublevel_of(child.level);
        if child.span.is_empty() {
            group = group.element(level.message(child.message.clone()));
            continue;
        }
        report.push(std::mem::replace(
            &mut group,
            Group::with_title(level.secondary_title(child.message.clone())),
        ));
        for (file, annotations) in primary_first(collect_annotations(&child.span), &child.span) {
            if let Some(snippet) = annotated_snippet(&file, annotations, sources) {
                group = group.element(snippet);
            }
        }
    }

    for suggestion in suggestions {
        match suggestion.style {
            SuggestionStyle::CompletelyHidden => {}
            SuggestionStyle::HideCodeAlways => {
                group = group.element(annotate_snippets::Level::HELP.message(suggestion.message));
            }
            SuggestionStyle::HideCodeInline
            | SuggestionStyle::ShowCode
            | SuggestionStyle::ShowAlways => {
                let substitutions: Vec<Vec<(Span, String)>> = suggestion
                    .substitutions
                    .into_iter()
                    .filter_map(|mut parts| {
                        parts.sort_by_key(|(span, _)| span.lo);
                        let first = parts.first()?.0.file.clone();
                        if parts.iter().any(|(span, _)| span.file != first) {
                            return None;
                        }
                        sources.file(&first)?;
                        parts.retain(|(span, snippet)| is_different(sources, snippet, span));
                        (!parts.is_empty()).then_some(parts)
                    })
                    .collect();
                if substitutions.is_empty() {
                    continue;
                }
                let mut message = suggestion.message;
                let file_name = substitutions[0][0].0.file.clone();
                let Some(file) = sources.file(&file_name) else { continue };
                let others = substitutions.len().saturating_sub(MAX_SUGGESTIONS);
                let mut subs = Vec::new();
                for parts in substitutions.into_iter().take(MAX_SUGGESTIONS) {
                    let mut confusion = Confusion::None;
                    for (span, snippet) in &parts {
                        confusion =
                            confusion.combine(detect_confusion_type(sources, snippet, span));
                    }
                    if confusion != Confusion::None {
                        message.push_str(confusion.label_text());
                    }
                    let mut parts: Vec<(Span, String)> = parts
                        .into_iter()
                        .filter(|(span, snippet)| is_different(sources, snippet, span))
                        .collect();
                    if parts.is_empty() {
                        continue;
                    }
                    let fold = !attribute_suggestion(&file, &mut parts);
                    let spans: Vec<Span> = parts.iter().map(|(span, _)| span.clone()).collect();
                    let Some((lo, source, line)) = shrink_file(&file, &spans) else { continue };
                    subs.push(
                        Snippet::source(source)
                            .line_start(line)
                            .path(file_name.clone())
                            .fold(fold)
                            .patches(parts.into_iter().map(move |(span, replacement)| {
                                let from = (span.lo as usize).saturating_sub(lo);
                                let to = (span.hi as usize).saturating_sub(lo);
                                Patch::new(from..to, replacement)
                            })),
                    );
                }
                if !subs.is_empty() {
                    report.push(std::mem::replace(
                        &mut group,
                        Group::with_title(annotate_snippets::Level::HELP.secondary_title(message)),
                    ));
                    group = group.elements(subs);
                    if others > 0 {
                        let plural = if others == 1 { "" } else { "s" };
                        group = group.element(
                            annotate_snippets::Level::NOTE
                                .no_name()
                                .message(format!("and {others} other candidate{plural}")),
                        );
                    }
                }
            }
        }
    }
    if !group.is_empty() {
        report.push(group);
    }
    renderer.render(&report)
}

/// `rendered`'s text as rustc writes it to its output: with the empty line after it.
pub fn written(diagnostic: &Diagnostic, sources: &mut Sources, style: Style) -> String {
    let mut text = rendered(diagnostic, sources, style);
    text.push('\n');
    if !style.short {
        text.push('\n');
    }
    text
}

fn level_of(level: Level) -> annotate_snippets::Level<'static> {
    match level {
        Level::Deny | Level::Forbid => annotate_snippets::Level::ERROR,
        _ => annotate_snippets::Level::WARNING,
    }
}

fn sublevel_of(level: Sublevel) -> annotate_snippets::Level<'static> {
    match level {
        Sublevel::Note => annotate_snippets::Level::NOTE,
        Sublevel::Help => annotate_snippets::Level::HELP,
        Sublevel::Warning => annotate_snippets::Level::WARNING,
    }
}

struct Annotation {
    kind: AnnotationKind,
    span: Span,
    label: Option<String>,
}

/// rustc's `collect_annotations`: the labels of `span` by file, each file's by line.
fn collect_annotations(span: &MultiSpan) -> Vec<(String, Vec<Annotation>)> {
    let mut output: Vec<(String, Vec<Annotation>)> = Vec::new();
    for label in span.span_labels() {
        let kind = if label.is_primary { AnnotationKind::Primary } else { AnnotationKind::Context };
        let annotation = Annotation {
            kind,
            span: label.span.clone(),
            label: label.label.map(normalize_whitespace),
        };
        match output.iter_mut().find(|(file, _)| *file == label.span.file) {
            Some((_, annotations)) => annotations.push(annotation),
            None => output.push((label.span.file.clone(), vec![annotation])),
        }
    }
    output
}

/// The file of the primary span first, found as rustc finds it: by a binary search of the
/// files in the order they were met.
fn primary_first(
    mut files: Vec<(String, Vec<Annotation>)>,
    span: &MultiSpan,
) -> Vec<(String, Vec<Annotation>)> {
    if let Some(primary) = span.primary_span()
        && let Ok(at) = files.binary_search_by(|(file, _)| file.as_str().cmp(&primary.file))
    {
        files.swap(0, at);
    }
    files
}

fn annotated_snippet<'a>(
    file_name: &str,
    mut annotations: Vec<Annotation>,
    sources: &mut Sources,
) -> Option<Snippet<'a, annotate_snippets::Annotation<'a>>> {
    let file = sources.file(file_name)?;
    annotations.sort_by_key(|it| file.line_of(it.span.lo as usize));
    let spans: Vec<Span> = annotations.iter().map(|it| it.span.clone()).collect();
    let (lo, source, line) = shrink_file(&file, &spans)?;
    Some(Snippet::source(source).line_start(line).path(file_name.to_owned()).annotations(
        annotations.into_iter().map(move |it| {
            let from = (it.span.lo as usize).saturating_sub(lo);
            let to = (it.span.hi as usize).saturating_sub(lo);
            let annotation = it.kind.span(from..to);
            match it.label {
                Some(label) => annotation.label(label),
                None => annotation,
            }
        }),
    ))
}

/// rustc's `shrink_file`: the whole lines `spans` are on, where they start, and the line
/// number of the first.
fn shrink_file(file: &File, spans: &[Span]) -> Option<(usize, String, usize)> {
    let lo = spans.iter().map(|it| it.lo as usize).min()?;
    let hi = spans.iter().map(|it| it.hi as usize).max()?;
    let (lo_line, hi_line) = (file.line_of(lo), file.line_of(hi));
    let start = file.line_bounds(lo_line).start;
    let end = file.line_bounds(hi_line).end;
    Some((start, file.text.get(start..end)?.to_owned(), lo_line + 1))
}

/// rustc's attribute special case: a suggestion that adds `#[...]` on a line of its own is
/// moved to the start of the line, with its indentation, and shown with the line after.
fn attribute_suggestion(file: &File, parts: &mut [(Span, String)]) -> bool {
    let [(span, snippet)] = parts else { return false };
    let trimmed = snippet.trim();
    if !(trimmed.starts_with("#[") && trimmed.ends_with(']') && snippet.ends_with('\n')) {
        return false;
    }
    if span.lo != span.hi {
        return false;
    }
    let before = &file.text[..span.lo as usize];
    let before = before.rsplit_once('\n').map_or(before, |(_, it)| it);
    if !before.trim().is_empty() {
        return false;
    }
    if !before.is_empty() && !snippet.ends_with(before) {
        snippet.insert_str(0, before);
        span.lo -= before.len() as u32;
        span.hi = span.lo;
    }
    true
}

/// rustc's `render_multispan_macro_backtrace` without `-Z macro-backtrace`: a span in a macro's
/// expansion that is not in its call gets a label on the outermost call.
fn macro_backtrace(span: &mut MultiSpan) {
    let mut labels: Vec<(Span, String)> = Vec::new();
    for primary in &span.primary {
        let backtrace: Vec<&Expansion> =
            std::iter::successors(primary.expansion.as_deref(), |it| {
                it.call_site.expansion.as_deref()
            })
            .collect();
        let Some(outermost) = backtrace.last() else { continue };
        let call = &outermost.call_site;
        let contains = call.file == primary.file && call.lo <= primary.lo && primary.hi <= call.hi;
        if contains {
            continue;
        }
        let what = match outermost.kind {
            MacroKind::Attr => "this attribute macro expansion",
            MacroKind::Derive => "this derive macro expansion",
            MacroKind::Bang => "this macro invocation",
        };
        let label = (call.clone(), format!("in {what}"));
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    span.labels.extend(labels);
}

/// rustc's `primary_span_formatted`: a lone suggestion that is short is shown as a label on
/// what it replaces, and not as a patch.
fn primary_span_formatted(
    span: &mut MultiSpan,
    suggestions: &mut Vec<Suggestion>,
    sources: &mut Sources,
) {
    let [suggestion] = &suggestions[..] else { return };
    let [substitution] = &suggestion.substitutions[..] else { return };
    let [(part_span, snippet)] = &substitution[..] else { return };
    let inline = suggestion.message.split_whitespace().count() < 10
        && !snippet.contains('\n')
        && !matches!(
            suggestion.style,
            SuggestionStyle::HideCodeAlways
                | SuggestionStyle::CompletelyHidden
                | SuggestionStyle::ShowAlways
        );
    if !inline {
        return;
    }
    let snippet = snippet.trim();
    let label = if snippet.is_empty() || suggestion.style.hide_inline() {
        format!("help: {}", suggestion.message)
    } else {
        let confusion = detect_confusion_type(sources, snippet, part_span);
        format!("help: {}{}: `{snippet}`", suggestion.message, confusion.label_text())
    };
    span.labels.push((part_span.clone(), label));
    suggestions.clear();
}

/// Whether `suggested` is not what is at `span` already.
fn is_different(sources: &mut Sources, suggested: &str, span: &Span) -> bool {
    sources.snippet(span).is_none_or(|found| found != suggested)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confusion {
    None,
    Case,
    DigitLetter,
    Both,
}

impl Confusion {
    fn label_text(self) -> &'static str {
        match self {
            Confusion::None => "",
            Confusion::Case => " (notice the capitalization)",
            Confusion::DigitLetter => " (notice the digit/letter confusion)",
            Confusion::Both => " (notice the capitalization and digit/letter confusion)",
        }
    }

    fn combine(self, other: Confusion) -> Confusion {
        match (self, other) {
            (Confusion::None, other) => other,
            (this, Confusion::None) => this,
            (Confusion::Both, _) | (_, Confusion::Both) => Confusion::Both,
            (Confusion::Case, Confusion::DigitLetter)
            | (Confusion::DigitLetter, Confusion::Case) => Confusion::Both,
            (this, _) => this,
        }
    }
}

/// rustc's `detect_confusion_type`: whether the suggestion differs from what is there only in
/// letters that look alike.
fn detect_confusion_type(sources: &mut Sources, suggested: &str, span: &Span) -> Confusion {
    let Some(found) = sources.snippet(span) else { return Confusion::None };
    if found.len() != suggested.len() {
        return Confusion::None;
    }
    const ASCII_CONFUSABLES: &[char] =
        &['c', 'f', 'i', 'k', 'o', 's', 'u', 'v', 'w', 'x', 'y', 'z'];
    const DIGIT_LETTER: [(char, char); 5] =
        [('0', 'O'), ('1', 'l'), ('5', 'S'), ('8', 'B'), ('9', 'g')];
    let (mut case, mut digit_letter, mut other) = (false, false, false);
    for (f, s) in std::iter::zip(found.chars(), suggested.chars()) {
        if f == s {
            continue;
        }
        if f.eq_ignore_ascii_case(&s) {
            if ASCII_CONFUSABLES.contains(&f) || ASCII_CONFUSABLES.contains(&s) {
                case = true;
            } else {
                other = true;
            }
        } else if DIGIT_LETTER.contains(&(f, s)) || DIGIT_LETTER.contains(&(s, f)) {
            digit_letter = true;
        } else {
            other = true;
        }
    }
    let differs = found != suggested;
    match (case && !other && differs, digit_letter && !other && differs) {
        (true, true) => Confusion::Both,
        (true, false) => Confusion::Case,
        (false, true) => Confusion::DigitLetter,
        (false, false) => Confusion::None,
    }
}

/// rustc's `OUTPUT_REPLACEMENTS`, in its order.
const OUTPUT_REPLACEMENTS: &[(char, &str)] = &[
    ('\0', "␀"),
    ('\u{0001}', "␁"),
    ('\u{0002}', "␂"),
    ('\u{0003}', "␃"),
    ('\u{0004}', "␄"),
    ('\u{0005}', "␅"),
    ('\u{0006}', "␆"),
    ('\u{0007}', "␇"),
    ('\u{0008}', "␈"),
    ('\t', "    "),
    ('\u{000b}', "␋"),
    ('\u{000c}', "␌"),
    ('\u{000d}', "␍"),
    ('\u{000e}', "␎"),
    ('\u{000f}', "␏"),
    ('\u{0010}', "␐"),
    ('\u{0011}', "␑"),
    ('\u{0012}', "␒"),
    ('\u{0013}', "␓"),
    ('\u{0014}', "␔"),
    ('\u{0015}', "␕"),
    ('\u{0016}', "␖"),
    ('\u{0017}', "␗"),
    ('\u{0018}', "␘"),
    ('\u{0019}', "␙"),
    ('\u{001a}', "␚"),
    ('\u{001b}', "␛"),
    ('\u{001c}', "␜"),
    ('\u{001d}', "␝"),
    ('\u{001e}', "␞"),
    ('\u{001f}', "␟"),
    ('\u{007f}', "␡"),
    ('\u{200d}', ""),
    ('\u{202a}', "�"),
    ('\u{202b}', "�"),
    ('\u{202c}', "�"),
    ('\u{202d}', "�"),
    ('\u{202e}', "�"),
    ('\u{2066}', "�"),
    ('\u{2067}', "�"),
    ('\u{2068}', "�"),
    ('\u{2069}', "�"),
];

fn normalize_whitespace(text: &str) -> String {
    text.chars().fold(String::with_capacity(text.len()), |mut out, c| {
        match OUTPUT_REPLACEMENTS.binary_search_by_key(&c, |(it, _)| *it) {
            Ok(at) => out.push_str(OUTPUT_REPLACEMENTS[at].1),
            Err(_) => out.push(c),
        }
        out
    })
}

/// The last char boundary of `text` at or before `at`.
fn floor(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A diagnostic as rustc's JSON has it, with its keys in the order cargo writes them: cargo
/// reads rustc's message into its `rendered` and a sorted map of the rest.
#[derive(Serialize)]
pub struct Json {
    pub rendered: String,
    #[serde(rename = "$message_type")]
    message_type: &'static str,
    children: Vec<JsonChild>,
    level: &'static str,
    message: String,
    spans: Vec<JsonSpan>,
    code: Option<JsonCode>,
}

#[derive(Serialize)]
struct JsonChild {
    children: Vec<JsonChild>,
    code: Option<JsonCode>,
    level: &'static str,
    message: String,
    rendered: Option<String>,
    spans: Vec<JsonSpan>,
}

#[derive(Serialize)]
struct JsonCode {
    code: String,
    explanation: Option<String>,
}

#[derive(Serialize)]
struct JsonExpansion {
    def_site_span: Option<JsonSpan>,
    macro_decl_name: String,
    span: JsonSpan,
}

#[derive(Serialize)]
struct JsonSpan {
    byte_end: u32,
    byte_start: u32,
    column_end: usize,
    column_start: usize,
    expansion: Option<Box<JsonExpansion>>,
    file_name: String,
    is_primary: bool,
    label: Option<String>,
    line_end: usize,
    line_start: usize,
    suggested_replacement: Option<String>,
    suggestion_applicability: Option<&'static str>,
    text: Vec<JsonLine>,
}

#[derive(Serialize)]
struct JsonLine {
    highlight_end: usize,
    highlight_start: usize,
    text: String,
}

/// `diagnostic` in rustc's JSON, with `rendered` as its text.
pub fn json(diagnostic: &Diagnostic, sources: &mut Sources, rendered: String) -> Json {
    let suggestions = match diagnostic.suggestions_disabled {
        true => &[][..],
        false => &diagnostic.suggestions[..],
    };
    let children = diagnostic.children.iter().map(|child| JsonChild {
        children: Vec::new(),
        code: None,
        level: child.level.name(),
        message: child.message.clone(),
        rendered: None,
        spans: json_spans(&child.span, sources),
    });
    let children: Vec<JsonChild> = children.collect();
    let suggested = suggestions.iter().map(|suggestion| {
        let spans = suggestion.substitutions.iter().flatten().filter_map(|(span, replacement)| {
            let mut it = json_span(span, true, None, sources)?;
            it.suggested_replacement = Some(replacement.clone());
            it.suggestion_applicability = Some(suggestion.applicability.name());
            Some(it)
        });
        JsonChild {
            children: Vec::new(),
            code: None,
            level: "help",
            message: suggestion.message.clone(),
            rendered: None,
            spans: spans.collect(),
        }
    });
    Json {
        rendered,
        message_type: "diagnostic",
        children: children.into_iter().chain(suggested).collect(),
        level: if diagnostic.is_error() { "error" } else { "warning" },
        message: diagnostic.message.clone(),
        spans: json_spans(&diagnostic.span, sources),
        code: Some(JsonCode { code: diagnostic.lint.clone(), explanation: None }),
    }
}

fn json_spans(span: &MultiSpan, sources: &mut Sources) -> Vec<JsonSpan> {
    span.span_labels()
        .into_iter()
        .filter_map(|it| json_span(it.span, it.is_primary, it.label, sources))
        .collect()
}

fn json_span(
    span: &Span,
    is_primary: bool,
    label: Option<&str>,
    sources: &mut Sources,
) -> Option<JsonSpan> {
    let file = sources.file(&span.file)?;
    let (line_start, column_start) = file.position(span.lo as usize);
    let (line_end, column_end) = file.position(span.hi as usize);
    let text = (line_start..=line_end).map(|line| {
        let text = file.line_text(line - 1);
        let highlight_start = if line == line_start { column_start + 1 } else { 1 };
        let highlight_end =
            if line == line_end { column_end + 1 } else { text.chars().count() + 1 };
        JsonLine { highlight_end, highlight_start, text: text.to_owned() }
    });
    let text = text.collect();
    let expansion = span.expansion.as_deref().and_then(|it| {
        Some(Box::new(JsonExpansion {
            def_site_span: it
                .def_site
                .as_ref()
                .and_then(|def| json_span(def, false, None, sources)),
            macro_decl_name: it.decl_name(),
            span: json_span(&it.call_site, false, None, sources)?,
        }))
    });
    Some(JsonSpan {
        byte_end: span.hi,
        byte_start: span.lo,
        column_end: column_end + 1,
        column_start: column_start + 1,
        expansion,
        file_name: span.file.clone(),
        is_primary,
        label: label.map(str::to_owned),
        line_end,
        line_start,
        suggested_replacement: None,
        suggestion_applicability: None,
        text,
    })
}
