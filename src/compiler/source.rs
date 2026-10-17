//! Bounded configured source preparation and original-file byte provenance.
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[path = "source_attrs.rs"]
pub mod source_attrs;
#[path = "source_cfg.rs"]
pub mod source_cfg;
#[path = "source_map.rs"]
pub mod source_map;
pub use source_cfg::CfgSet;
pub use source_map::{FileId, OriginSegment, SourceMap, SourceSpan};
const MAX_FILE_DEPTH: usize = 64;
const MAX_MODULE_DEPTH: usize = 128;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILES: usize = 1024;
const MAX_ORIGINS: usize = 1024 * 1024;
#[derive(Clone, Debug)]
/// Native x86_64 Linux configuration plus bounded user predicates.
/// The default pins abort/checked-debug profile facts, not complete Rust panic semantics.
pub struct SourceOptions {
    pub cfg: CfgSet,
}
impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            cfg: CfgSet::native_x86_64_linux(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceErrorKind {
    Io,
    Syntax,
    Unsupported,
    Limit,
    Cycle,
    Module,
}
#[derive(Clone, Debug)]
pub struct SourceLocation {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}
#[derive(Clone, Debug)]
pub struct SourceError {
    pub kind: SourceErrorKind,
    pub primary: Option<SourceSpan>,
    pub message: String,
    pub location: Option<SourceLocation>,
}
impl SourceError {
    pub fn syntax(primary: Option<SourceSpan>, message: impl Into<String>) -> Self {
        Self {
            kind: SourceErrorKind::Syntax,
            primary,
            message: message.into(),
            location: None,
        }
    }
    pub fn with_sources(mut self, sources: &SourceMap) -> Self {
        if let Some((path, line, column)) = self.primary.and_then(|s| sources.location(s)) {
            self.location = Some(SourceLocation {
                path: path.to_owned(),
                line,
                column,
            });
        }
        self
    }
}
impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(l) = &self.location {
            write!(f, "{}:{}:{}: ", l.path.display(), l.line, l.column)?;
        }
        if self.kind == SourceErrorKind::Unsupported {
            f.write_str("unsupported: ")?;
        }
        f.write_str(&self.message)
    }
}
impl std::error::Error for SourceError {}
#[derive(Clone, Debug)]
pub struct PreparedAttribute {
    pub span: SourceSpan,
    pub name: String,
    pub effective: bool,
}
#[derive(Clone, Debug, Default)]
pub struct PreparedSource {
    pub text: String,
    pub sources: SourceMap,
    pub origins: Vec<OriginSegment>,
    pub attributes: Vec<PreparedAttribute>,
    pub source_bytes: usize,
    pub files: usize,
}
impl PreparedSource {
    pub fn map_range(&self, r: Range<usize>) -> Option<SourceSpan> {
        if r.end > self.text.len()
            || r.start > r.end
            || !self.text.is_char_boundary(r.start)
            || !self.text.is_char_boundary(r.end)
        {
            return None;
        }
        let span = source_map::map_range(&self.origins, r.clone())?;
        let file = self.sources.files.get(span.file.0 as usize)?;
        let original = file.contents.get(span.lo as usize..span.hi as usize)?;
        let index = self
            .origins
            .partition_point(|s| s.prepared.start <= r.start)
            .checked_sub(1)?;
        let segment = self.origins.get(index)?;
        if !segment.synthetic && original != &self.text[r] {
            return None;
        }
        Some(span)
    }
}
#[derive(Debug)]
pub struct LoadedSource {
    pub text: String,
    pub source_bytes: usize,
    pub files: usize,
}
pub fn load(path: &Path) -> Result<LoadedSource, String> {
    prepare_file(path, &SourceOptions::default())
        .map(|p| LoadedSource {
            text: p.text,
            source_bytes: p.source_bytes,
            files: p.files,
        })
        .map_err(|e| e.to_string())
}
pub fn prepare_file(path: &Path, options: &SourceOptions) -> Result<PreparedSource, SourceError> {
    prepare_with_reader(path, options, &mut FsReader)
}
fn prepare_with_reader(
    path: &Path,
    options: &SourceOptions,
    reader: &mut dyn SourceReader,
) -> Result<PreparedSource, SourceError> {
    options
        .cfg
        .validate_native()
        .map_err(|message| Loader::error(SourceErrorKind::Unsupported, None, message))?;
    let mut loader = Loader::new(options, Some(reader));
    loader
        .file(
            path,
            path.parent().unwrap_or_else(|| Path::new(".")),
            0,
            None,
        )
        .map_err(|e| e.with_sources(&loader.prepared.sources))?;
    Ok(loader.prepared)
}
/// No filesystem capability exists in text preparation, even for path-like names.
pub fn prepare_text(
    name: impl Into<PathBuf>,
    text: &str,
    options: &SourceOptions,
) -> Result<PreparedSource, SourceError> {
    options
        .cfg
        .validate_native()
        .map_err(|message| Loader::error(SourceErrorKind::Unsupported, None, message))?;
    if text.len() > MAX_BYTES {
        return Err(Loader::error(
            SourceErrorKind::Limit,
            None,
            format!("source size limit ({MAX_BYTES} bytes) exceeded"),
        ));
    }
    let mut loader = Loader::new(options, None);
    loader
        .snapshot(name.into(), text.to_owned(), Path::new("."), 0)
        .map_err(|e| e.with_sources(&loader.prepared.sources))?;
    Ok(loader.prepared)
}
trait SourceReader {
    fn canonicalize(&mut self, p: &Path) -> std::io::Result<PathBuf>;
    fn is_file(&mut self, p: &Path) -> std::io::Result<bool>;
    fn exists(&mut self, p: &Path) -> std::io::Result<bool>;
    fn read(&mut self, p: &Path, limit: usize) -> std::io::Result<Vec<u8>>;
}
struct FsReader;
impl SourceReader for FsReader {
    fn canonicalize(&mut self, p: &Path) -> std::io::Result<PathBuf> {
        p.canonicalize()
    }
    fn is_file(&mut self, p: &Path) -> std::io::Result<bool> {
        Ok(std::fs::metadata(p)?.is_file())
    }
    fn exists(&mut self, p: &Path) -> std::io::Result<bool> {
        p.try_exists()
    }
    fn read(&mut self, p: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
        let file = File::open(p)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("source must be a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(limit as u64).read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}
struct Loader<'a> {
    active: Vec<PathBuf>,
    options: &'a SourceOptions,
    reader: Option<&'a mut dyn SourceReader>,
    prepared: PreparedSource,
}
impl<'a> Loader<'a> {
    fn new(options: &'a SourceOptions, reader: Option<&'a mut dyn SourceReader>) -> Self {
        Self {
            active: Vec::new(),
            options,
            reader,
            prepared: PreparedSource::default(),
        }
    }
    fn error(
        kind: SourceErrorKind,
        primary: Option<SourceSpan>,
        message: impl Into<String>,
    ) -> SourceError {
        SourceError {
            kind,
            primary,
            message: message.into(),
            location: None,
        }
    }
    fn span(file: FileId, r: Range<usize>) -> SourceSpan {
        SourceSpan {
            file,
            lo: r.start as u32,
            hi: r.end as u32,
        }
    }
    fn file(
        &mut self,
        path: &Path,
        directory: &Path,
        depth: usize,
        anchor: Option<SourceSpan>,
    ) -> Result<(), SourceError> {
        if self.active.len() > MAX_FILE_DEPTH {
            return Err(Self::error(
                SourceErrorKind::Limit,
                anchor,
                format!("module file depth limit ({MAX_FILE_DEPTH}) exceeded"),
            ));
        }
        if self.prepared.files >= MAX_FILES {
            return Err(Self::error(
                SourceErrorKind::Limit,
                anchor,
                format!("module file limit ({MAX_FILES}) exceeded"),
            ));
        }
        let io_error = |e| {
            Self::error(
                SourceErrorKind::Io,
                anchor,
                format!("{}: {e}", path.display()),
            )
        };
        let reader = self.reader.as_deref_mut().ok_or_else(|| {
            Self::error(
                SourceErrorKind::Unsupported,
                anchor,
                "external modules are unsupported in text input",
            )
        })?;
        let canonical = reader.canonicalize(path).map_err(io_error)?;
        if self.active.contains(&canonical) {
            return Err(Self::error(
                SourceErrorKind::Cycle,
                anchor,
                format!(
                    "module inclusion cycle: {} -> {}",
                    self.active
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" -> "),
                    canonical.display()
                ),
            ));
        }
        if !reader.is_file(path).map_err(io_error)? {
            return Err(Self::error(
                SourceErrorKind::Io,
                anchor,
                format!("{}: source must be a regular file", path.display()),
            ));
        }
        let bytes = reader
            .read(path, MAX_BYTES - self.prepared.source_bytes + 1)
            .map_err(io_error)?;
        let text = String::from_utf8(bytes).map_err(|e| {
            Self::error(
                SourceErrorKind::Syntax,
                anchor,
                format!("{}: source is not UTF-8: {e}", path.display()),
            )
        })?;
        self.active.push(canonical);
        let result = self.snapshot(path.to_owned(), text, directory, depth);
        self.active.pop();
        result
    }
    fn snapshot(
        &mut self,
        path: PathBuf,
        text: String,
        directory: &Path,
        depth: usize,
    ) -> Result<(), SourceError> {
        if text.len() > MAX_BYTES - self.prepared.source_bytes {
            return Err(Self::error(
                SourceErrorKind::Limit,
                None,
                format!("source size limit ({MAX_BYTES} bytes) exceeded"),
            ));
        }
        self.prepared.source_bytes += text.len();
        self.prepared.files += 1;
        let original: Arc<str> = text.into();
        let id = self.prepared.sources.add(path, original.clone());
        let filtered = source_attrs::prepare(&original, &self.options.cfg).map_err(|e| {
            Self::error(
                match e.kind {
                    source_attrs::AttributeErrorKind::Syntax => SourceErrorKind::Syntax,
                    source_attrs::AttributeErrorKind::Unsupported => SourceErrorKind::Unsupported,
                    source_attrs::AttributeErrorKind::Limit => SourceErrorKind::Limit,
                },
                Some(Self::span(id, e.range)),
                e.message,
            )
        })?;
        // Disabled syntax is checked without name resolution, typing or child IO.
        // Legal Rust outside this parser's bounded grammar remains Unsupported,
        // not a claim that the disabled program is invalid or full cfg is supported.
        for inactive in filtered.inactive_items {
            let syntax = inactive_syntax(&inactive.text).map_err(|message| {
                Self::error(
                    SourceErrorKind::Syntax,
                    Some(Self::span(id, inactive.range.clone())),
                    message,
                )
            })?;
            super::frontend::parse_syntax_unprepared(&syntax).map_err(|e| {
                Self::error(
                    e.kind,
                    e.has_location.then(|| {
                        Self::span(
                            id,
                            inactive.range.start + e.bytes.start
                                ..inactive.range.start + e.bytes.end,
                        )
                    }),
                    format!("inactive item: {}", e.message),
                )
            })?;
        }
        for a in filtered.attributes {
            self.prepared.attributes.push(PreparedAttribute {
                span: Self::span(id, a.range),
                name: a.name,
                effective: a.effective,
            });
        }
        self.expand(&filtered.text, &original, id, 0, directory, depth)
    }
    fn append(
        &mut self,
        text: &str,
        origin: SourceSpan,
        synthetic: bool,
    ) -> Result<(), SourceError> {
        if text.len() > MAX_BYTES - self.prepared.text.len() {
            return Err(Self::error(
                SourceErrorKind::Limit,
                Some(origin),
                format!("expanded source size limit ({MAX_BYTES} bytes) exceeded"),
            ));
        }
        let start = self.prepared.text.len();
        self.prepared.text.push_str(text);
        if let Some(last) = self.prepared.origins.last_mut()
            && !synthetic
            && !last.synthetic
            && last.prepared.end == start
            && last.origin.file == origin.file
            && last.origin.hi == origin.lo
        {
            last.prepared.end += text.len();
            last.origin.hi = origin.hi;
            return Ok(());
        }
        if self.prepared.origins.len() >= MAX_ORIGINS {
            return Err(Self::error(
                SourceErrorKind::Limit,
                Some(origin),
                "source provenance segment limit exceeded",
            ));
        }
        self.prepared.origins.push(OriginSegment {
            prepared: start..start + text.len(),
            origin,
            synthetic,
        });
        Ok(())
    }
    fn copy(
        &mut self,
        text: &str,
        original: &str,
        id: FileId,
        base: usize,
        r: Range<usize>,
    ) -> Result<(), SourceError> {
        let mut start = r.start;
        while start < r.end {
            let synthetic = text.as_bytes()[start] != original.as_bytes()[base + start];
            let mut end = start + 1;
            while end < r.end
                && (text.as_bytes()[end] != original.as_bytes()[base + end]) == synthetic
            {
                end += 1;
            }
            self.append(
                &text[start..end],
                Self::span(id, base + start..base + end),
                synthetic,
            )?;
            start = end;
        }
        if r.is_empty() && self.prepared.origins.is_empty() {
            self.append("", Self::span(id, base + r.start..base + r.end), false)?;
        }
        Ok(())
    }
    fn expand(
        &mut self,
        text: &str,
        original: &str,
        id: FileId,
        base: usize,
        directory: &Path,
        depth: usize,
    ) -> Result<(), SourceError> {
        let anchor = Some(Self::span(id, base..base + text.len()));
        if depth > MAX_MODULE_DEPTH {
            return Err(Self::error(
                SourceErrorKind::Limit,
                anchor,
                format!("module nesting exceeds limit {MAX_MODULE_DEPTH}"),
            ));
        }
        let tokens = tokens(text).map_err(|e| Self::error(SourceErrorKind::Syntax, anchor, e))?;
        let mut closing = Vec::new();
        for t in &tokens {
            match t.text(text) {
                "{" => closing.push("}"),
                "[" => closing.push("]"),
                "(" => closing.push(")"),
                c @ ("}" | "]" | ")") => {
                    if closing.pop() != Some(c) {
                        return Err(Self::error(
                            SourceErrorKind::Syntax,
                            anchor,
                            "unbalanced delimiter in source file",
                        ));
                    }
                }
                _ => {}
            }
        }
        if !closing.is_empty() {
            return Err(Self::error(
                SourceErrorKind::Syntax,
                anchor,
                "unclosed delimiter in source file",
            ));
        }
        let mut copied = 0;
        let mut i = 0;
        while i < tokens.len() {
            let mut head = i;
            if tokens[head].text(text) == "pub" {
                head += 1;
                if tokens.get(head).is_some_and(|t| t.text(text) == "(") {
                    head = close_group(&tokens, text, head) + 1;
                }
            }
            if tokens.get(head).is_some_and(|t| t.text(text) == "mod")
                && head + 2 < tokens.len()
                && syntactic_identifier(tokens[head + 1].text(text))
            {
                let name = tokens[head + 1].text(text);
                let delimiter = tokens[head + 2].text(text);
                if delimiter == ";" {
                    let decl =
                        Self::span(id, base + tokens[head].start..base + tokens[head + 2].end);
                    let reader = self.reader.as_deref_mut().ok_or_else(|| {
                        Self::error(
                            SourceErrorKind::Unsupported,
                            Some(decl),
                            "external modules are unsupported in text input",
                        )
                    })?;
                    let flat = directory.join(format!("{name}.rs"));
                    let nested = directory.join(name).join("mod.rs");
                    let a = reader.exists(&flat).map_err(|e| {
                        Self::error(
                            SourceErrorKind::Io,
                            Some(decl),
                            format!("{}: {e}", flat.display()),
                        )
                    })?;
                    let b = reader.exists(&nested).map_err(|e| {
                        Self::error(
                            SourceErrorKind::Io,
                            Some(decl),
                            format!("{}: {e}", nested.display()),
                        )
                    })?;
                    let path = match (a, b) {
                        (true, false) => flat,
                        (false, true) => nested,
                        (true, true) => {
                            return Err(Self::error(
                                SourceErrorKind::Module,
                                Some(decl),
                                format!(
                                    "ambiguous module {name}: both {} and {} exist",
                                    flat.display(),
                                    nested.display()
                                ),
                            ));
                        }
                        (false, false) => {
                            return Err(Self::error(
                                SourceErrorKind::Module,
                                Some(decl),
                                format!(
                                    "module {name} not found: expected {} or {}",
                                    flat.display(),
                                    nested.display()
                                ),
                            ));
                        }
                    };
                    self.copy(text, original, id, base, copied..tokens[head + 2].start)?;
                    self.append("{\n", decl, true)?;
                    self.file(&path, &directory.join(name), depth + 1, Some(decl))?;
                    self.append("\n}", decl, true)?;
                    copied = tokens[head + 2].end;
                    i = head + 3;
                    continue;
                }
                if delimiter == "{" {
                    let end = close_group(&tokens, text, head + 2);
                    self.copy(text, original, id, base, copied..tokens[head + 2].end)?;
                    self.expand(
                        &text[tokens[head + 2].end..tokens[end].start],
                        original,
                        id,
                        base + tokens[head + 2].end,
                        &directory.join(name),
                        depth + 1,
                    )?;
                    copied = tokens[end].start;
                    i = end + 1;
                    continue;
                }
            }
            // Skip the complete non-module item, including signature and macro token trees.
            while i < tokens.len() {
                match tokens[i].text(text) {
                    ";" => {
                        i += 1;
                        break;
                    }
                    "{" => {
                        i = close_group(&tokens, text, i) + 1;
                        break;
                    }
                    "(" | "[" => i = close_group(&tokens, text, i) + 1,
                    _ => i += 1,
                }
            }
        }
        self.copy(text, original, id, base, copied..text.len())
    }
}
fn close_group(tokens: &[Token], text: &str, start: usize) -> usize {
    let mut depth = 0;
    for (i, t) in tokens.iter().enumerate().skip(start) {
        match t.text(text) {
            "{" | "[" | "(" => depth += 1,
            "}" | "]" | ")" => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

/// Strip structurally complete external declarations from inactive syntax only.
/// Their grammar is validated here, but no module resolution or IO is possible.
fn inactive_syntax(text: &str) -> Result<String, String> {
    let tokens = tokens(text)?;
    let mut closing = Vec::new();
    for t in &tokens {
        match t.text(text) {
            "{" => closing.push("}"),
            "[" => closing.push("]"),
            "(" => closing.push(")"),
            c @ ("}" | "]" | ")") => {
                if closing.pop() != Some(c) {
                    return Err("unbalanced delimiter in inactive item".into());
                }
            }
            _ => {}
        }
    }
    if !closing.is_empty() {
        return Err("unclosed delimiter in inactive item".into());
    }
    fn items(
        tokens: &[Token],
        text: &str,
        start: usize,
        end: usize,
        depth: usize,
        out: &mut [u8],
    ) -> Result<(), String> {
        if depth > MAX_MODULE_DEPTH {
            return Err(format!("module nesting exceeds limit {MAX_MODULE_DEPTH}"));
        }
        let mut i = start;
        while i < end {
            let mut head = i;
            if tokens[head].text(text) == "pub" {
                head += 1;
                if tokens.get(head).is_some_and(|t| t.text(text) == "(") {
                    head = close_group(tokens, text, head) + 1;
                }
                inactive_visibility(tokens, text, i, head)?;
                if head > i + 1 {
                    for b in &mut out[tokens[i + 1].start..tokens[head - 1].end] {
                        if *b != b'\n' && *b != b'\r' {
                            *b = b' ';
                        }
                    }
                }
            }
            if head + 2 < end
                && tokens[head].text(text) == "mod"
                && syntactic_identifier(tokens[head + 1].text(text))
            {
                if tokens[head + 2].text(text) == ";" {
                    inactive_visibility(tokens, text, i, head)?;
                    for b in &mut out[tokens[i].start..tokens[head + 2].end] {
                        if *b != b'\n' && *b != b'\r' {
                            *b = b' ';
                        }
                    }
                    i = head + 3;
                    continue;
                }
                if tokens[head + 2].text(text) == "{" {
                    let close = close_group(tokens, text, head + 2);
                    items(tokens, text, head + 3, close, depth + 1, out)?;
                    i = close + 1;
                    continue;
                }
            }
            while i < end {
                match tokens[i].text(text) {
                    ";" => {
                        i += 1;
                        break;
                    }
                    "{" => {
                        i = close_group(tokens, text, i) + 1;
                        break;
                    }
                    "(" | "[" => i = close_group(tokens, text, i) + 1,
                    _ => i += 1,
                }
            }
        }
        Ok(())
    }
    let mut out = text.as_bytes().to_vec();
    // The boundary scanner above already checked raw-string termination. Inactive
    // literals carry no values into typing, but still occupy one expression token.
    // Normalize only their transport spelling for the bounded syntax parser.
    for token in &tokens {
        let spelling = token.text(text);
        if spelling.starts_with("r\"")
            || spelling.starts_with("r#")
            || spelling.starts_with("br\"")
            || spelling.starts_with("br#")
        {
            for b in &mut out[token.start..token.end] {
                if *b != b'\n' && *b != b'\r' {
                    *b = b' ';
                }
            }
            out[token.start] = b'0';
        }
    }
    items(&tokens, text, 0, tokens.len(), 0, &mut out)?;
    Ok(String::from_utf8(out).expect("masking complete token ranges preserves UTF-8"))
}

/// Check only the visibility grammar. Disabled paths must not be resolved or
/// constrained to the active module's ancestors.
fn inactive_visibility(
    tokens: &[Token],
    text: &str,
    start: usize,
    head: usize,
) -> Result<(), String> {
    if head == start || head == start + 1 {
        return Ok(());
    }
    let restriction = &tokens[start + 2..head - 1];
    if restriction.len() == 1 && matches!(restriction[0].text(text), "crate" | "self" | "super") {
        return Ok(());
    }
    if restriction.first().is_none_or(|t| t.text(text) != "in") {
        return Err(
            "incorrect visibility restriction: expected crate, self, super, or in path".into(),
        );
    }
    let mut i = 1;
    if restriction.get(i).is_some_and(|t| t.text(text) == ":")
        && restriction.get(i + 1).is_some_and(|t| t.text(text) == ":")
    {
        i += 2;
    }
    loop {
        if restriction.get(i).is_none_or(|t| {
            !matches!(t.text(text), "crate" | "self" | "super")
                && !syntactic_identifier(t.text(text))
        }) {
            return Err("expected identifier in visibility restriction".into());
        }
        i += 1;
        if i == restriction.len() {
            return Ok(());
        }
        if restriction.get(i).is_none_or(|t| t.text(text) != ":")
            || restriction.get(i + 1).is_none_or(|t| t.text(text) != ":")
        {
            return Err("expected path separator in visibility restriction".into());
        }
        i += 2;
    }
}

// Match the existing bounded frontend's identifier contract before stripping any
// inactive declaration. Lexical ASCII shape alone must not hide reserved keywords.
fn syntactic_identifier(s: &str) -> bool {
    identifier(s)
        && s != "_"
        && !matches!(
            s,
            "fn" | "let"
                | "mut"
                | "if"
                | "else"
                | "while"
                | "return"
                | "true"
                | "false"
                | "pub"
                | "const"
                | "static"
                | "loop"
                | "break"
                | "continue"
                | "for"
                | "in"
                | "match"
                | "struct"
                | "enum"
                | "use"
                | "mod"
                | "impl"
                | "trait"
                | "self"
                | "Self"
                | "as"
                | "async"
                | "await"
                | "move"
                | "unsafe"
                | "where"
                | "type"
                | "ref"
                | "extern"
                | "crate"
                | "super"
                | "dyn"
                | "abstract"
                | "become"
                | "box"
                | "do"
                | "final"
                | "macro"
                | "override"
                | "priv"
                | "typeof"
                | "unsized"
                | "virtual"
                | "yield"
                | "try"
                | "gen"
        )
}

fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
struct Token {
    start: usize,
    end: usize,
}
impl Token {
    fn text<'a>(&self, s: &'a str) -> &'a str {
        &s[self.start..self.end]
    }
}

/// A boundary scanner only. The frontend still owns syntax and semantics.
fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1usize;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth != 0 {
                return Err("unterminated block comment".into());
            }
            continue;
        }
        // Raw strings must hide all their braces and apparent module tokens.
        let raw_start = if b[i] == b'r' {
            Some(i + 1)
        } else if b[i..].starts_with(b"br") {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut quote) = raw_start {
            let hashes_start = quote;
            while quote < b.len() && b[quote] == b'#' {
                quote += 1;
            }
            if quote < b.len() && b[quote] == b'"' {
                let hashes = quote - hashes_start;
                i = quote + 1;
                loop {
                    if i >= b.len() {
                        return Err("unterminated raw string".into());
                    }
                    if b[i] == b'"'
                        && b.get(i + 1..i + 1 + hashes)
                            .is_some_and(|s| s.iter().all(|c| *c == b'#'))
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                out.push(Token { start, end: i });
                continue;
            }
        }
        // A named lifetime has no closing quote. Keep the apostrophe in the
        // token so names such as 'mod cannot masquerade as module keywords.
        // A closing quote immediately after the identifier instead denotes a
        // character literal, whose delimiters must remain hidden below.
        if b[i] == b'\''
            && b.get(i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            let mut end = i + 2;
            while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
                end += 1;
            }
            if b.get(end) != Some(&b'\'') {
                i = end;
                out.push(Token { start, end: i });
                continue;
            }
        }
        if b[i] == b'"' || b[i] == b'\'' {
            let quote = b[i];
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                } else if b[i] == quote {
                    i += 1;
                    closed = true;
                    break;
                } else {
                    i += 1;
                }
            }
            if !closed {
                return Err("unterminated string or character literal".into());
            }
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        } else {
            i += text[i..].chars().next().unwrap().len_utf8();
        }
        out.push(Token { start, end: i });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Default)]
    struct RecordingReader {
        calls: Vec<(&'static str, PathBuf)>,
        root: String,
        child: Option<String>,
    }
    impl SourceReader for RecordingReader {
        fn canonicalize(&mut self, p: &Path) -> std::io::Result<PathBuf> {
            self.calls.push(("canonicalize", p.to_owned()));
            Ok(p.to_owned())
        }
        fn is_file(&mut self, p: &Path) -> std::io::Result<bool> {
            self.calls.push(("stat", p.to_owned()));
            Ok(true)
        }
        fn exists(&mut self, p: &Path) -> std::io::Result<bool> {
            self.calls.push(("exists", p.to_owned()));
            Ok(self.child.is_some() && p == Path::new("child.rs"))
        }
        fn read(&mut self, p: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
            self.calls.push(("open", p.to_owned()));
            self.calls.push(("stat-opened", p.to_owned()));
            self.calls.push(("read", p.to_owned()));
            let text = if p == Path::new("main.rs") {
                &self.root
            } else {
                self.child.as_ref().unwrap()
            };
            Ok(text.as_bytes()[..text.len().min(limit)].to_vec())
        }
    }

    #[test]
    fn disabled_external_modules_make_zero_child_observations() {
        for root in [
            "#[cfg(any())] mod missing; fn main(){}",
            "#[cfg(any())] #[path=\"other.rs\"] mod missing; fn main(){}",
            "mod inline { #![cfg(any())] mod missing; } fn main(){}",
            "#![cfg(any())] mod missing;",
            "#[cfg(any())] mod inline { mod missing; } fn main(){}",
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                ..Default::default()
            };
            let prepared =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap();
            assert_eq!(prepared.files, 1);
            assert!(
                reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")),
                "{root}: {:?}",
                reader.calls
            );
            assert_eq!(
                reader
                    .calls
                    .iter()
                    .map(|(kind, _)| *kind)
                    .collect::<Vec<_>>(),
                ["canonicalize", "stat", "open", "stat-opened", "read"]
            );
        }
    }

    #[test]
    fn inactive_external_file_inner_cfg_does_not_observe_descendants() {
        let mut reader = RecordingReader {
            root: "mod child; fn main(){}".into(),
            child: Some("#![cfg(any())] mod missing;".into()),
            ..Default::default()
        };
        let prepared =
            prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                .unwrap();
        assert_eq!(prepared.files, 2);
        assert!(
            !reader
                .calls
                .iter()
                .any(|(_, p)| p.to_string_lossy().contains("missing"))
        );
        assert_eq!(
            reader
                .calls
                .iter()
                .filter(|(kind, _)| *kind == "exists")
                .count(),
            2
        );
    }

    #[test]
    fn invalid_options_observe_no_files() {
        let mut options = SourceOptions::default();
        options.cfg.names.insert("windows".into());
        let mut reader = RecordingReader::default();
        assert_eq!(
            prepare_with_reader(Path::new("main.rs"), &options, &mut reader)
                .unwrap_err()
                .kind,
            SourceErrorKind::Unsupported
        );
        assert!(reader.calls.is_empty());
    }

    #[test]
    fn disabled_external_visibility_checks_syntax_but_not_path_resolution() {
        for visibility in [
            "pub(crate)",
            "pub(in ::foo)",
            "pub(in foo)",
            "pub(in crate::missing)",
        ] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {visibility} mod child; fn main(){{}}"),
                ..Default::default()
            };
            prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                .unwrap();
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
        for visibility in ["pub(bogus)", "pub(in)"] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {visibility} mod child; fn main(){{}}"),
                ..Default::default()
            };
            let error =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{error}");
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
    }

    #[test]
    fn disabled_reserved_module_and_visibility_names_are_syntax_errors_without_child_io() {
        for declaration in ["mod fn;", "pub(in fn) mod child;"] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {declaration} fn main(){{}}"),
                ..Default::default()
            };
            let error =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            assert_eq!(
                error.kind,
                SourceErrorKind::Syntax,
                "{declaration}: {error}"
            );
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
    }

    #[test]
    fn disabled_leading_visibility_on_all_items_is_syntax_only() {
        for declaration in [
            "pub(in foo) fn unused(){}",
            "pub(in crate::missing) mod inline{mod child;}",
        ] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {declaration} fn main(){{}}"),
                ..Default::default()
            };
            prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                .unwrap();
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
        for declaration in [
            "pub(bogus) fn unused(){}",
            "pub(in fn) fn unused(){}",
            "pub(bogus) mod inline{}",
            "pub(in fn) mod inline{}",
        ] {
            let mut reader = RecordingReader {
                root: format!("#[cfg(any())] {declaration} fn main(){{}}"),
                ..Default::default()
            };
            let error =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader)
                    .unwrap_err();
            assert_eq!(
                error.kind,
                SourceErrorKind::Syntax,
                "{declaration}: {error}"
            );
            assert!(reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")));
        }
    }

    #[test]
    fn malformed_unicode_cfg_preserves_original_span_and_location_in_text_and_child() {
        let text = "// α🙂\r\n#[cfg(a β)] pub fn item(){}";
        let options = SourceOptions::default();
        let tree = Tree::new();
        tree.put("main.rs", "mod child; fn main(){}");
        tree.put("child.rs", text);
        for (error, path) in [
            (
                prepare_text("unicode.rs", text, &options).unwrap_err(),
                PathBuf::from("unicode.rs"),
            ),
            (
                prepare_file(&tree.0.join("main.rs"), &options).unwrap_err(),
                tree.0.join("child.rs"),
            ),
        ] {
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{error}");
            let span = error.primary.unwrap();
            assert!(text.is_char_boundary(span.lo as usize));
            assert!(text.is_char_boundary(span.hi as usize));
            assert_eq!(&text[span.lo as usize..span.hi as usize], "β");
            let location = error
                .location
                .as_ref()
                .expect("original Unicode error location");
            assert_eq!(location.path, path);
            assert_eq!((location.line, location.column), (2, 9));
            assert!(error.to_string().contains(":2:9:"));
        }
    }

    #[test]
    fn inactive_invalid_grammar_rejects_but_unused_semantics_do_not_run() {
        let options = SourceOptions::default();
        assert!(
            prepare_text(
                "test.rs",
                "#[cfg(any())] fn dead(){let = 1;} fn main(){}",
                &options
            )
            .is_err()
        );
        assert!(
            prepare_text(
                "test.rs",
                "#[cfg(any())] fn dead(){unknown();} fn main(){}",
                &options
            )
            .is_ok()
        );
        let raw = "#[cfg(any())] fn dead(){let text=r###\"mod missing; } #[unknown_attr]\"###;} fn main(){}";
        assert!(prepare_text("test.rs", raw, &options).is_ok());
    }

    #[test]
    fn original_bytes_and_synthetic_module_braces_are_distinct() {
        let tree = Tree::new();
        tree.put("main.rs", "// α🙂\r\nmod child; fn main(){}\r\n");
        let child = "// α🙂\r\npub fn child(){}\r\n";
        tree.put("child.rs", child);
        let prepared = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap();
        let start = prepared.text.find("pub fn child").unwrap();
        let span = prepared.map_range(start..start + 3).unwrap();
        assert_eq!(span.file, FileId(1));
        assert_eq!(
            &prepared.sources.files[1].contents[span.lo as usize..span.hi as usize],
            "pub"
        );
        assert_eq!(prepared.sources.location(span).unwrap().1, 2);
        let brace = prepared.text.find("{\n").unwrap();
        let anchor = prepared.map_range(brace..brace + 1).unwrap();
        assert_eq!(anchor.file, FileId(0));
        assert_eq!(
            &prepared.sources.files[0].contents[anchor.lo as usize..anchor.hi as usize],
            "mod child;"
        );
        assert!(prepared.map_range(brace..start + 1).is_none());
        assert!(
            prepared
                .map_range(prepared.text.len()..prepared.text.len())
                .is_some()
        );
        let empty = prepare_text("empty.rs", "", &SourceOptions::default()).unwrap();
        assert_eq!(
            empty.map_range(0..0),
            Some(SourceSpan {
                file: FileId(0),
                lo: 0,
                hi: 0
            })
        );
    }

    #[test]
    fn macro_token_trees_do_not_discover_phantom_modules() {
        for root in [
            "m!(mod missing;); fn main(){}",
            "m![mod missing;]; fn main(){}",
            "m!{mod missing;} fn main(){}",
        ] {
            let mut reader = RecordingReader {
                root: root.into(),
                ..Default::default()
            };
            let _ =
                prepare_with_reader(Path::new("main.rs"), &SourceOptions::default(), &mut reader);
            assert!(
                reader.calls.iter().all(|(_, p)| p == Path::new("main.rs")),
                "{root}"
            );
        }
    }

    #[test]
    fn text_and_file_preparation_apply_identical_options_and_attribute_policy() {
        let tree = Tree::new();
        let text = "#[cfg(feature=\"chosen\")] #[inline(always)] fn selected(){} #[cfg(not(feature=\"chosen\"))] fn absent(){}";
        tree.put("main.rs", text);
        let mut options = SourceOptions::default();
        options.cfg.insert_user("feature=\"chosen\"").unwrap();
        let file = prepare_file(&tree.0.join("main.rs"), &options).unwrap();
        let memory = prepare_text("test.rs", text, &options).unwrap();
        assert_eq!(file.text, memory.text);
        assert_eq!(file.attributes.len(), memory.attributes.len());
        assert!(file.text.contains("fn selected"));
        assert!(!file.text.contains("fn absent"));
        assert_eq!(file.sources.files[0].contents.as_ref(), text);
        assert_eq!(memory.sources.files[0].contents.as_ref(), text);
    }

    #[test]
    fn inline_module_128_boundary_is_uniform_for_files_and_text() {
        let tree = Tree::new();
        for depth in [MAX_MODULE_DEPTH, MAX_MODULE_DEPTH + 1] {
            let text = format!("{}fn f(){{}}{}", "mod a{".repeat(depth), "}".repeat(depth));
            tree.put("main.rs", &text);
            let file = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default());
            let memory = prepare_text("test.rs", &text, &SourceOptions::default());
            if depth == MAX_MODULE_DEPTH {
                assert!(file.is_ok(), "{file:?}");
                assert!(memory.is_ok(), "{memory:?}");
            } else {
                assert_eq!(file.unwrap_err().kind, SourceErrorKind::Limit);
                assert_eq!(memory.unwrap_err().kind, SourceErrorKind::Limit);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn repeated_canonical_file_has_distinct_lexical_occurrence_provenance() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a; mod b;");
        tree.put("shared.rs", "pub fn value(){}");
        std::os::unix::fs::symlink(tree.0.join("shared.rs"), tree.0.join("a.rs")).unwrap();
        std::os::unix::fs::symlink(tree.0.join("shared.rs"), tree.0.join("b.rs")).unwrap();
        let source = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap();
        let occurrences: Vec<_> = source
            .text
            .match_indices("pub fn value")
            .map(|(i, _)| source.map_range(i..i + 3).unwrap())
            .collect();
        assert_eq!(occurrences.len(), 2);
        assert_ne!(occurrences[0].file, occurrences[1].file);
        assert!(
            source.sources.files[occurrences[0].file.0 as usize]
                .lexical_path
                .ends_with("a.rs")
        );
        assert!(
            source.sources.files[occurrences[1].file.0 as usize]
                .lexical_path
                .ends_with("b.rs")
        );
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "jr-source-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn put(&self, name: &str, text: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        fn load(&self) -> Result<LoadedSource, String> {
            load(&self.0.join("main.rs"))
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn mixed_layout_preserves_visibility_and_counts() {
        let tree = Tree::new();
        let root = "pub mod a; mod b; fn main(){}";
        let a = "pub mod inline { mod child; }";
        let child = "fn private(){}";
        let b = "pub fn public(){}";
        tree.put("main.rs", root);
        tree.put("a.rs", a);
        tree.put("a/inline/child/mod.rs", child);
        tree.put("b/mod.rs", b);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.files, 4);
        assert_eq!(
            loaded.source_bytes,
            root.len() + a.len() + child.len() + b.len()
        );
        assert!(loaded.text.contains("pub mod a{"));
        assert!(loaded.text.contains("mod child{"));
        assert!(loaded.text.contains("fn private(){}"));
        assert!(loaded.text.contains("pub fn public(){}"));
    }

    #[test]
    fn comments_and_literals_do_not_load_phantom_files() {
        let tree = Tree::new();
        let root = r###"/* mod phantom; /* } */ */ // mod ghost;
fn main(){println!("mod absent; }"); let x = r##"mod raw; {"##; let c = '}';}
"###;
        tree.put("main.rs", root);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.text, root);
        assert_eq!(loaded.files, 1);
    }

    #[test]
    fn named_lifetimes_survive_external_module_expansion() {
        let tree = Tree::new();
        let root = "mod child; fn id<'a>(x:&'a i32)->&'a i32{x} fn main(){}";
        let child = "pub fn shorten<'long:'short,'short>(x:&'long i32)->&'short i32{x}";
        tree.put("main.rs", root);
        tree.put("child.rs", child);
        let loaded = tree.load().unwrap();
        assert_eq!(loaded.files, 2);
        assert_eq!(loaded.source_bytes, root.len() + child.len());
        assert!(loaded.text.contains("fn id<'a>(x:&'a i32)->&'a i32{x}"));
        assert!(loaded.text.contains(child));
        super::super::frontend::parse(&loaded.text).unwrap();
    }

    #[test]
    fn lifetime_tokens_do_not_hide_delimiters_or_expose_literal_contents() {
        let text = "fn f<'mod>(x:&'mod i32){let a='}';let b='a';let c='\\'';}";
        let actual: Vec<_> = tokens(text).unwrap().iter().map(|t| t.text(text)).collect();
        assert_eq!(actual.iter().filter(|t| **t == "'mod").count(), 2);
        assert_eq!(actual.iter().filter(|t| **t == "}").count(), 1);
        assert!(actual.contains(&"'}'"));
        assert!(actual.contains(&"'a'"));
        assert!(actual.contains(&"'\\''"));
    }

    #[test]
    fn missing_ambiguous_and_path_attribute_are_explicit() {
        let tree = Tree::new();
        tree.put("main.rs", "mod child;");
        assert!(tree.load().unwrap_err().contains("not found"));
        tree.put("child.rs", "");
        tree.put("child/mod.rs", "");
        assert!(tree.load().unwrap_err().contains("ambiguous"));
        tree.put("main.rs", "#[path = \"other.rs\"] mod child;");
        let error = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{error}");
        assert!(error.message.contains("path"), "{error}");
        tree.put(
            "main.rs",
            "#[cfg_attr(any(), path = \"other.rs\")] mod child;",
        );
        assert!(tree.load().unwrap_err().contains("ambiguous"));
    }

    #[test]
    fn nested_flat_file_uses_stem_directory() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        tree.put("a.rs", "mod b;");
        tree.put("a/b.rs", "mod c;");
        tree.put("a/b/c.rs", "fn leaf(){}");
        assert_eq!(tree.load().unwrap().files, 4);
    }

    #[test]
    fn depth_and_size_are_bounded() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        for depth in 0..=MAX_FILE_DEPTH {
            tree.put(&format!("{}a.rs", "a/".repeat(depth)), "mod a;");
        }
        let error = prepare_file(&tree.0.join("main.rs"), &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Limit, "{error}");
        assert!(error.message.contains("limit"), "{error}");
        tree.put("main.rs", &" ".repeat(MAX_BYTES + 1));
        assert!(tree.load().unwrap_err().contains("size limit"));
        let oversized_text = " ".repeat(MAX_BYTES + 1);
        let error =
            prepare_text("oversized.rs", &oversized_text, &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Limit);
        assert!(error.message.contains("size limit"));
        let options = SourceOptions::default();
        let mut reader = FsReader;
        let mut loader = Loader::new(&options, Some(&mut reader));
        loader.prepared.files = MAX_FILES;
        assert!(
            loader
                .file(&tree.0.join("main.rs"), &tree.0, 0, None)
                .unwrap_err()
                .to_string()
                .contains("file limit")
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonical_active_stack_rejects_symlink_cycle() {
        let tree = Tree::new();
        tree.put("main.rs", "mod a;");
        std::os::unix::fs::symlink(tree.0.join("main.rs"), tree.0.join("a.rs")).unwrap();
        assert!(tree.load().unwrap_err().contains("cycle"));
    }

    #[test]
    fn nonregular_sources_are_rejected() {
        let tree = Tree::new();
        assert!(load(&tree.0).unwrap_err().contains("regular file"));
        tree.put("main.rs", "mod child;");
        std::fs::create_dir(tree.0.join("child.rs")).unwrap();
        assert!(tree.load().unwrap_err().contains("regular file"));
    }

    #[test]
    fn malformed_and_non_utf8_sources_fail() {
        let tree = Tree::new();
        for text in [
            "/*",
            "fn main(){",
            "fn main(){\"unterminated}",
            "fn main(){r###\"unfinished}",
        ] {
            tree.put("main.rs", text);
            assert!(tree.load().is_err(), "{text}");
        }
        std::fs::write(tree.0.join("main.rs"), [0xff]).unwrap();
        assert!(tree.load().unwrap_err().contains("UTF-8"));
    }
}
