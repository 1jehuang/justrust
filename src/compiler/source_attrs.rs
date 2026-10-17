//! Bounded source configuration. Only explicit module/free-item attachment is supported.
//! This is a structural filter, not a Rust parser or a macro expander.
use super::source_cfg::{
    CfgSet, Meta, MetaError, MetaValue, evaluate, parse_meta, validate_string_literal,
};
use super::source_probe::VerifiedIdentityContext;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributeErrorKind {
    Syntax,
    Unsupported,
    Limit,
}
#[derive(Clone, Debug)]
pub struct AttributeError {
    pub kind: AttributeErrorKind,
    pub range: Range<usize>,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct AttributeRecord {
    pub range: Range<usize>,
    pub name: String,
    pub effective: bool,
}
#[derive(Clone, Debug)]
pub struct InactiveItem {
    pub range: Range<usize>,
    pub text: String,
}
#[derive(Clone, Debug)]
pub struct AttributeOutput {
    pub text: String,
    pub attributes: Vec<AttributeRecord>,
    pub inactive_items: Vec<InactiveItem>,
}
impl From<MetaError> for AttributeError {
    fn from(e: MetaError) -> Self {
        Self {
            kind: if e.limit {
                AttributeErrorKind::Limit
            } else {
                AttributeErrorKind::Syntax
            },
            range: e.range,
            message: e.message,
        }
    }
}
fn error(kind: AttributeErrorKind, range: Range<usize>, message: &str) -> AttributeError {
    AttributeError {
        kind,
        range,
        message: message.into(),
    }
}
const MAX_TOKENS: usize = 1_000_000;
const MAX_DEPTH: usize = 64;
const MAX_WORK: usize = 16_384;

#[derive(Clone, Debug)]
struct Token {
    range: Range<usize>,
    close: Option<usize>,
    doc: Option<bool>,
}
impl Token {
    fn text<'a>(&self, text: &'a str) -> &'a str {
        &text[self.range.clone()]
    }
}
#[derive(Clone)]
struct Attr {
    range: Range<usize>,
    content: Range<usize>,
    inner: bool,
    doc: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Crate,
    Module,
    Function,
    FreeItem,
    Unsupported,
}

pub fn prepare(text: &str, cfg: &CfgSet) -> Result<AttributeOutput, AttributeError> {
    prepare_with_context(text, cfg, None)
}

/// A verified source snapshot may retain these specific inert metadata forms.
/// This does not register any language item, intrinsic, diagnostic provider, or
/// const capability. Ordinary source preparation never receives this context.
pub(super) fn prepare_verified_identity(
    text: &str,
    cfg: &CfgSet,
    context: &VerifiedIdentityContext,
) -> Result<AttributeOutput, AttributeError> {
    prepare_with_context(text, cfg, Some(context))
}

fn prepare_with_context(
    text: &str,
    cfg: &CfgSet,
    identity_metadata: Option<&VerifiedIdentityContext>,
) -> Result<AttributeOutput, AttributeError> {
    if text.len() > 8 * 1024 * 1024 {
        return Err(error(
            AttributeErrorKind::Limit,
            0..text.len(),
            "source attribute input byte limit exceeded",
        ));
    }
    let tokens = scan(text)?;
    let mut filter = Filter {
        text,
        cfg,
        identity_metadata,
        tokens,
        bytes: text.as_bytes().to_vec(),
        records: Vec::new(),
        inactive_items: Vec::new(),
        work: 0,
    };
    filter.scope(0, filter.tokens.len(), 0..text.len(), Owner::Crate, 0)?;
    Ok(AttributeOutput {
        text: String::from_utf8(filter.bytes).expect("masking preserves UTF-8"),
        attributes: filter.records,
        inactive_items: filter.inactive_items,
    })
}
struct Filter<'a> {
    text: &'a str,
    cfg: &'a CfgSet,
    identity_metadata: Option<&'a VerifiedIdentityContext>,
    tokens: Vec<Token>,
    bytes: Vec<u8>,
    records: Vec<AttributeRecord>,
    inactive_items: Vec<InactiveItem>,
    work: usize,
}
impl Filter<'_> {
    fn mask(&mut self, range: Range<usize>) {
        for byte in &mut self.bytes[range] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
    }
    fn inactive(&mut self, range: Range<usize>) -> Result<(), AttributeError> {
        let mut bytes = self.text.as_bytes()[range.clone()].to_vec();
        let mut i = self.tokens.partition_point(|t| t.range.end <= range.start);
        while i < self.tokens.len() {
            if self.tokens[i].range.start < range.start {
                i += 1;
                continue;
            }
            if self.tokens[i].range.start >= range.end {
                break;
            }
            if let Some((a, next)) = self.attr(i, self.tokens.len())? {
                for b in &mut bytes[a.range.start - range.start..a.range.end - range.start] {
                    if !matches!(*b, b'\n' | b'\r') {
                        *b = b' ';
                    }
                }
                i = next;
            } else {
                i += 1;
            }
        }
        self.inactive_items.push(InactiveItem {
            range,
            text: String::from_utf8(bytes).expect("masked UTF-8"),
        });
        Ok(())
    }
    fn attr(&self, i: usize, end: usize) -> Result<Option<(Attr, usize)>, AttributeError> {
        let t = &self.tokens[i];
        if let Some(inner) = t.doc {
            return Ok(Some((
                Attr {
                    range: t.range.clone(),
                    content: t.range.clone(),
                    inner,
                    doc: true,
                },
                i + 1,
            )));
        }
        if t.text(self.text) != "#" {
            return Ok(None);
        }
        let inner = i + 1 < end && self.tokens[i + 1].text(self.text) == "!";
        let bracket = i + 1 + usize::from(inner);
        if bracket >= end || self.tokens[bracket].text(self.text) != "[" {
            return Err(error(
                AttributeErrorKind::Syntax,
                t.range.clone(),
                "expected attribute brackets",
            ));
        }
        let close = self.tokens[bracket].close.unwrap();
        outer_shape(
            self.text,
            self.tokens[bracket].range.end..self.tokens[close].range.start,
        )?;
        Ok(Some((
            Attr {
                range: t.range.start..self.tokens[close].range.end,
                content: self.tokens[bracket].range.end..self.tokens[close].range.start,
                inner,
                doc: false,
            },
            close + 1,
        )))
    }
    fn tick(&mut self, range: Range<usize>, depth: usize) -> Result<(), AttributeError> {
        self.work += 1;
        if self.work > MAX_WORK || depth > MAX_DEPTH {
            return Err(error(
                AttributeErrorKind::Limit,
                range,
                "attribute expansion work/depth limit exceeded",
            ));
        }
        Ok(())
    }
    // Expansion is deliberately lazy: inactive cfg_attr payloads are not effective metadata.
    fn expand(
        &mut self,
        range: Range<usize>,
        depth: usize,
        out: &mut Vec<Meta>,
        keep: &mut bool,
    ) -> Result<(), AttributeError> {
        self.tick(range.clone(), depth)?;
        outer_shape(self.text, range.clone())?;
        let source = &self.text[range.clone()];
        let ts = scan(source).map_err(|mut e| {
            e.range = range.start + e.range.start..range.start + e.range.end;
            e
        })?;
        let name = ts.first().map(|t| t.text(source)).unwrap_or("");
        if name.is_empty()
            || !name
                .chars()
                .next()
                .is_some_and(|c| c == '_' || c.is_alphabetic())
        {
            return Err(error(
                AttributeErrorKind::Syntax,
                range,
                "expected attribute name",
            ));
        }
        self.records.push(AttributeRecord {
            range: range.clone(),
            name: name.into(),
            effective: true,
        });
        if name == "cfg_attr" {
            if ts.len() < 3 || ts[1].text(source) != "(" || ts[1].close != Some(ts.len() - 1) {
                return Err(error(
                    AttributeErrorKind::Syntax,
                    range,
                    "cfg_attr requires a parenthesized predicate and attributes",
                ));
            }
            let mut pieces = Vec::new();
            let mut begin = ts[1].range.end;
            let mut i = 2;
            while i < ts.len() - 1 {
                if ts[i].text(source) == "," {
                    pieces.push(begin..ts[i].range.start);
                    begin = ts[i].range.end;
                }
                if let Some(close) = ts[i].close {
                    i = close + 1;
                } else {
                    i += 1;
                }
            }
            let tail = begin..ts.last().unwrap().range.start;
            if !source[tail.clone()].trim().is_empty() {
                pieces.push(tail);
            }
            if pieces.len() < 2 {
                return Err(error(
                    AttributeErrorKind::Syntax,
                    range,
                    "cfg_attr requires predicate and at least one attribute",
                ));
            }
            let pred = &pieces[0];
            let enabled = evaluate(
                &parse_meta(&source[pred.clone()], range.start + pred.start)?,
                self.cfg,
            )?;
            for p in pieces.into_iter().skip(1) {
                let p = range.start + p.start..range.start + p.end;
                if enabled && *keep {
                    self.expand(p, depth + 1, out, keep)?;
                } else {
                    self.tick(p.clone(), depth + 1)?;
                    outer_shape(self.text, p.clone())?;
                    let payload = &self.text[p.clone()];
                    let pt = scan(payload).map_err(|mut e| {
                        e.range = p.start + e.range.start..p.start + e.range.end;
                        e
                    })?;
                    if pt.is_empty()
                        || !pt[0]
                            .text(payload)
                            .chars()
                            .next()
                            .is_some_and(|c| c == '_' || c.is_alphabetic())
                        || (pt.len() == 2 && pt[1].text(payload) == "=")
                    {
                        return Err(error(
                            AttributeErrorKind::Syntax,
                            p,
                            "malformed cfg_attr attribute payload",
                        ));
                    }
                    // Record the inactive spelling without interpreting its nested semantic grammar.
                    let name = self.text[p.clone()]
                        .trim()
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or("")
                        .to_string();
                    self.records.push(AttributeRecord {
                        range: p,
                        name,
                        effective: false,
                    });
                }
            }
        } else {
            if name != "cfg" {
                out.push(Meta {
                    name: name.into(),
                    value: MetaValue::Word,
                    range,
                });
                return Ok(());
            }
            let meta = parse_meta(source, range.start)?;
            if meta.name == "cfg" {
                let MetaValue::List(args) = &meta.value else {
                    return Err(error(
                        AttributeErrorKind::Syntax,
                        meta.range,
                        "cfg requires exactly one predicate",
                    ));
                };
                if args.len() != 1 {
                    return Err(error(
                        AttributeErrorKind::Syntax,
                        meta.range,
                        "cfg requires exactly one predicate",
                    ));
                }
                *keep = evaluate(&args[0], self.cfg)?;
            } else {
                out.push(meta);
            }
        }
        Ok(())
    }
    fn attributes(
        &mut self,
        attrs: &[Attr],
        owner: Owner,
        item: Option<Range<usize>>,
    ) -> Result<bool, AttributeError> {
        let record_start = self.records.len();
        self.work = self.work.saturating_add(attrs.len());
        if self.work > MAX_WORK {
            return Err(error(
                AttributeErrorKind::Limit,
                attrs[0].range.clone(),
                "attribute count limit exceeded",
            ));
        }
        let mut effective = Vec::new();
        let mut keep = true;
        for attr in attrs {
            if !keep {
                self.records.push(AttributeRecord {
                    range: attr.range.clone(),
                    name: if attr.doc {
                        "doc".into()
                    } else {
                        self.text[attr.content.clone()]
                            .trim()
                            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .next()
                            .unwrap_or("")
                            .into()
                    },
                    effective: false,
                });
                continue;
            }
            if attr.doc {
                self.records.push(AttributeRecord {
                    range: attr.range.clone(),
                    name: "doc".into(),
                    effective: true,
                });
            } else {
                self.expand(attr.content.clone(), 0, &mut effective, &mut keep)?;
            }
        }
        if !keep {
            for r in &mut self.records[record_start..] {
                r.effective = false;
            }
            return Ok(false);
        }
        if owner == Owner::Unsupported && !attrs.is_empty() {
            return Err(error(
                AttributeErrorKind::Unsupported,
                attrs[0].range.clone(),
                "attributes at this attachment position are unsupported",
            ));
        }
        let mut seen_identity_metadata = std::collections::BTreeSet::new();
        for raw in effective {
            if self.identity_metadata.is_some()
                && matches!(
                    raw.name.as_str(),
                    "stable" | "rustc_const_stable" | "rustc_diagnostic_item"
                )
            {
                let permitted = owner == Owner::Function
                    && item.as_ref().is_some_and(|item| {
                        self.identity_metadata.is_some_and(|context| {
                            context.permits_metadata(self.text, raw.range.clone(), item.clone())
                        })
                    });
                if !permitted
                    || !valid_identity_metadata(&parse_meta(
                        &self.text[raw.range.clone()],
                        raw.range.start,
                    )?)
                    || !seen_identity_metadata.insert(raw.name.clone())
                {
                    return Err(error(
                        AttributeErrorKind::Unsupported,
                        raw.range,
                        "identity probe metadata requires an exact verified source owner and supported inert form",
                    ));
                }
                continue;
            }
            let meta = if matches!(raw.name.as_str(), "doc" | "inline") {
                parse_meta(&self.text[raw.range.clone()], raw.range.start).map_err(|e| {
                    if raw.name == "doc" && !e.limit {
                        error(
                            AttributeErrorKind::Unsupported,
                            raw.range.clone(),
                            "unsupported doc metadata form",
                        )
                    } else {
                        e.into()
                    }
                })?
            } else {
                raw
            };
            match meta.name.as_str() {
                "doc" => match &meta.value {
                    MetaValue::String(_) => {}
                    MetaValue::List(a)
                        if a.len() == 1
                            && a[0].name == "hidden"
                            && matches!(a[0].value, MetaValue::Word) => {}
                    _ => {
                        return Err(error(
                            AttributeErrorKind::Unsupported,
                            meta.range,
                            "only doc string and doc(hidden) metadata are supported",
                        ));
                    }
                },
                "inline" => {
                    if owner != Owner::Function {
                        return Err(error(
                            AttributeErrorKind::Syntax,
                            meta.range,
                            "inline requires a function owner",
                        ));
                    }
                    match &meta.value {
                        MetaValue::Word => {}
                        MetaValue::List(a)
                            if a.len() == 1
                                && matches!(a[0].name.as_str(), "always" | "never")
                                && matches!(a[0].value, MetaValue::Word) => {}
                        _ => {
                            return Err(error(
                                AttributeErrorKind::Syntax,
                                meta.range,
                                "expected inline, inline(always), or inline(never)",
                            ));
                        }
                    }
                }
                _ => {
                    return Err(error(
                        if known_semantic_attribute(&meta.name) {
                            AttributeErrorKind::Unsupported
                        } else {
                            AttributeErrorKind::Syntax
                        },
                        meta.range,
                        &if known_semantic_attribute(&meta.name) {
                            format!("active attribute `{}` is unsupported", meta.name)
                        } else {
                            format!("unknown unregistered attribute `{}`", meta.name)
                        },
                    ));
                }
            }
        }
        for attr in attrs {
            self.mask(attr.range.clone());
        }
        Ok(true)
    }
    fn scope(
        &mut self,
        start: usize,
        end: usize,
        region: Range<usize>,
        owner: Owner,
        depth: usize,
    ) -> Result<(), AttributeError> {
        if depth > 128 {
            return Err(error(
                AttributeErrorKind::Limit,
                region,
                "module nesting exceeds limit 128",
            ));
        }
        let mut i = start;
        let mut inner = Vec::new();
        while i < end {
            let Some((attr, next)) = self.attr(i, end)? else {
                break;
            };
            if !attr.inner {
                break;
            }
            inner.push(attr);
            i = next;
        }
        if !self.attributes(&inner, owner, None)? {
            self.inactive(region.clone())?;
            self.mask(region);
            return Ok(());
        }
        while i < end {
            if self.tokens[i].text(self.text) == ";" {
                i += 1;
                continue;
            }
            let first = i;
            let mut attrs = Vec::new();
            while i < end {
                let Some((attr, next)) = self.attr(i, end)? else {
                    break;
                };
                if attr.inner {
                    return Err(error(
                        AttributeErrorKind::Unsupported,
                        attr.range,
                        "inner attribute must be at the beginning of a crate or module",
                    ));
                }
                attrs.push(attr);
                i = next;
            }
            if i == end {
                if let Some(a) = attrs.first() {
                    return Err(error(
                        AttributeErrorKind::Syntax,
                        a.range.clone(),
                        "outer attribute has no item owner",
                    ));
                }
                break;
            }
            let item_start = i;
            let (kind, mut cursor) = self.item_kind(i, end);
            let mut body = None;
            let mut stop = end;
            while cursor < end {
                let token = self.tokens[cursor].text(self.text);
                if token == ";" {
                    stop = cursor + 1;
                    break;
                }
                if token == "{" {
                    let close = self.tokens[cursor].close.unwrap();
                    if matches!(
                        kind,
                        "fn" | "mod"
                            | "struct"
                            | "enum"
                            | "impl"
                            | "trait"
                            | "union"
                            | "extern"
                            | "macro_rules"
                            | "unknown"
                    ) {
                        body = Some((cursor, close));
                        stop = close + 1;
                        if stop < end && self.tokens[stop].text(self.text) == ";" {
                            stop += 1;
                        }
                        break;
                    }
                }
                if let Some(close) = self.tokens[cursor].close {
                    cursor = close + 1;
                } else {
                    cursor += 1;
                }
            }
            let item_owner = match kind {
                "fn" => Owner::Function,
                "mod" => Owner::Module,
                "struct" | "enum" | "impl" | "trait" | "union" | "extern" | "use" | "const"
                | "static" | "type" => Owner::FreeItem,
                _ => Owner::Unsupported,
            };
            if !self.attributes(
                &attrs,
                item_owner,
                Some(self.tokens[item_start].range.start..self.tokens[stop - 1].range.end),
            )? {
                self.inactive(
                    self.tokens[item_start].range.start..self.tokens[stop - 1].range.end,
                )?;
                self.mask(self.tokens[first].range.start..self.tokens[stop - 1].range.end);
            } else {
                // Only modules introduce supported nested item attachment positions.
                let header_end = body.map_or(stop, |(open, _)| open);
                self.reject_nested(item_start, header_end)?;
                if let Some((open, close)) = body {
                    if kind == "mod" {
                        self.scope(
                            open + 1,
                            close,
                            self.tokens[open].range.end..self.tokens[close].range.start,
                            Owner::Module,
                            depth + 1,
                        )?;
                    } else if kind != "macro_rules" {
                        self.reject_nested(open + 1, close)?;
                    }
                }
            }
            i = stop;
        }
        Ok(())
    }
    fn item_kind(&self, mut i: usize, end: usize) -> (&'static str, usize) {
        if self.tokens[i].text(self.text) == "pub" {
            i += 1;
            if i < end && self.tokens[i].text(self.text) == "(" {
                i = self.tokens[i].close.unwrap() + 1;
            }
        }
        while i < end
            && matches!(
                self.tokens[i].text(self.text),
                "unsafe" | "async" | "default"
            )
        {
            i += 1;
        }
        if i + 1 < end
            && self.tokens[i].text(self.text) == "const"
            && self.tokens[i + 1].text(self.text) == "fn"
        {
            i += 1;
        }
        let kind = if i < end {
            match self.tokens[i].text(self.text) {
                "fn" => "fn",
                "mod" => "mod",
                "struct" => "struct",
                "enum" => "enum",
                "impl" => "impl",
                "trait" => "trait",
                "union" => "union",
                "extern" => "extern",
                "use" => "use",
                "const" => "const",
                "static" => "static",
                "type" => "type",
                "macro_rules" => "macro_rules",
                _ => "unknown",
            }
        } else {
            "unknown"
        };
        (kind, i)
    }
    fn reject_nested(&self, start: usize, end: usize) -> Result<(), AttributeError> {
        for i in start..end {
            if let Some((a, _)) = self.attr(i, end)? {
                return Err(error(
                    AttributeErrorKind::Unsupported,
                    a.range,
                    "attributes on fields, parameters, associated items, statements, or expressions are unsupported",
                ));
            }
        }
        Ok(())
    }
}

/// Validate only source metadata spelling. This helper returns no compiler
/// authority and cannot authorize an owner or a source snapshot.
fn valid_identity_metadata(meta: &Meta) -> bool {
    match (meta.name.as_str(), &meta.value) {
        ("stable" | "rustc_const_stable", MetaValue::List(args)) if args.len() == 2 => {
            let expected_feature = if meta.name == "stable" {
                "convert_id"
            } else {
                "const_identity"
            };
            args[0].name == "feature"
                && matches!(&args[0].value, MetaValue::String(value) if value == expected_feature)
                && args[1].name == "since"
                && matches!(&args[1].value, MetaValue::String(value) if value == "1.33.0")
        }
        ("rustc_diagnostic_item", MetaValue::String(value)) => value == "convert_identity",
        _ => false,
    }
}

fn known_semantic_attribute(name: &str) -> bool {
    name.starts_with("rustc_")
        || matches!(
            name,
            "path"
                | "repr"
                | "lang"
                | "target_feature"
                | "track_caller"
                | "naked"
                | "link"
                | "link_name"
                | "link_section"
                | "no_mangle"
                | "export_name"
                | "unsafe"
                | "derive"
                | "feature"
                | "no_std"
                | "no_core"
                | "allow"
                | "warn"
                | "deny"
                | "forbid"
                | "expect"
                | "stable"
                | "unstable"
                | "deprecated"
                | "must_use"
                | "cold"
                | "test"
                | "bench"
                | "ignore"
                | "should_panic"
                | "global_allocator"
                | "panic_handler"
                | "alloc_error_handler"
                | "proc_macro"
                | "proc_macro_attribute"
                | "proc_macro_derive"
                | "automatically_derived"
                | "non_exhaustive"
                | "macro_export"
                | "macro_use"
                | "recursion_limit"
                | "type_length_limit"
                | "crate_name"
                | "crate_type"
                | "no_main"
        )
}

fn reserved_attribute_ident(name: &str) -> bool {
    // `unsafe` is an explicit attribute wrapper. Simple paths also admit
    // crate/self/super, so they are not ordinary-keyword rejections here.
    matches!(
        name,
        "_" | "as"
            | "break"
            | "const"
            | "continue"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "Self"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
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
    )
}

fn outer_shape(text: &str, range: Range<usize>) -> Result<(), AttributeError> {
    let source = &text[range.clone()];
    let ts = scan(source).map_err(|mut e| {
        e.range = range.start + e.range.start..range.start + e.range.end;
        e
    })?;
    let mut numeric_end = 0;
    for (index, token) in ts.iter().enumerate() {
        let spelling = token.text(source);
        if token.range.start >= numeric_end
            && spelling.as_bytes().first().is_some_and(u8::is_ascii_digit)
        {
            let mut end = token.range.end;
            for next in &ts[index + 1..] {
                if next.range.start != end
                    || !next
                        .text(source)
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
                {
                    break;
                }
                end = next.range.end;
            }
            numeric_end = end;
            if !source[token.range.start..end]
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'_')
            {
                return Err(error(
                    AttributeErrorKind::Unsupported,
                    range.start + token.range.start..range.start + end,
                    "non-decimal numeric literal grammar in attributes is unsupported",
                ));
            }
        }
        let unsupported_literal = spelling.starts_with("b\"")
            || spelling.starts_with("c\"")
            || spelling.starts_with("br\"")
            || spelling.starts_with("br#")
            || spelling.starts_with("cr\"")
            || spelling.starts_with("cr#")
            || spelling.starts_with("b'")
            || (spelling.starts_with('\'') && spelling.ends_with('\'') && spelling.len() >= 2);
        if unsupported_literal {
            return Err(error(
                AttributeErrorKind::Unsupported,
                range.start + token.range.start..range.start + token.range.end,
                "byte, C, and character literal grammar in attributes is unsupported",
            ));
        }
        if spelling.starts_with('"') || spelling.starts_with("r\"") || spelling.starts_with("r#") {
            validate_string_literal(spelling, range.start + token.range.start)?;
        }
    }
    let ident = |i: usize| {
        ts.get(i).is_some_and(|t| {
            let mut chars = t.text(source).chars();
            !reserved_attribute_ident(t.text(source))
                && chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
                && chars.all(|c| c == '_' || c.is_alphanumeric())
        })
    };
    let invalid = || {
        error(
            AttributeErrorKind::Syntax,
            range.clone(),
            "malformed attribute syntax",
        )
    };
    if !ident(0) {
        return Err(invalid());
    }
    let mut i = 1;
    while i + 2 < ts.len()
        && ts[i].text(source) == ":"
        && ts[i + 1].text(source) == ":"
        && ident(i + 2)
    {
        i += 3;
    }
    if i == ts.len() {
        return Ok(());
    }
    if ts[i].text(source) == "=" {
        if i + 1 == ts.len() {
            return Err(invalid());
        }
        return match parse_meta(source, range.start) {
            Ok(meta) => {
                if matches!(meta.value, MetaValue::OtherLiteral) {
                    let rhs = source[ts[i].range.end..].trim();
                    let decimal = rhs.as_bytes().first().is_some_and(u8::is_ascii_digit)
                        && rhs.bytes().all(|b| b.is_ascii_digit() || b == b'_');
                    if !decimal && !matches!(rhs, "true" | "false") {
                        return Err(error(
                            AttributeErrorKind::Unsupported,
                            range,
                            "non-decimal attribute literal grammar is unsupported",
                        ));
                    }
                }
                Ok(())
            }
            Err(e) => {
                let rhs = &ts[i + 1..];
                // Nonliteral expressions require a real expression parser/macro pipeline.
                // Never treat their nonempty spelling as proof of valid grammar.
                let nonliteral = rhs.iter().any(|t| t.text(source) == "!")
                    || rhs.first().is_some_and(|t| {
                        matches!(t.text(source), "(" | "{" | "[" | "-")
                            || t.text(source).starts_with('\'')
                            || t.text(source).starts_with("b\"")
                            || t.text(source).starts_with("b'")
                    });
                if nonliteral && !e.limit {
                    Err(error(
                        AttributeErrorKind::Unsupported,
                        range,
                        "nonliteral attribute RHS grammar is unsupported",
                    ))
                } else {
                    Err(e.into())
                }
            }
        };
    }
    if ts[i].text(source) == "(" && ts[i].close == Some(ts.len() - 1) {
        return Ok(());
    }
    Err(invalid())
}

/// Lexical boundaries only. Strings, chars and non-doc comments are opaque.
fn scan(text: &str) -> Result<Vec<Token>, AttributeError> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut out: Vec<Token> = Vec::new();
    let mut stack: Vec<(usize, u8)> = Vec::new();
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if b[i..].starts_with(b"//") {
            let doc = (b[i..].starts_with(b"///") && !b[i..].starts_with(b"////"))
                || b[i..].starts_with(b"//!");
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            if doc {
                out.push(Token {
                    range: start..i,
                    close: None,
                    doc: Some(b[start + 2] == b'!'),
                });
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let doc = (b[i..].starts_with(b"/**")
                && !b[i..].starts_with(b"/***")
                && !b[i..].starts_with(b"/**/"))
                || b[i..].starts_with(b"/*!");
            i += 2;
            let mut nesting = 1;
            while i < b.len() && nesting > 0 {
                if b[i..].starts_with(b"/*") {
                    nesting += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    nesting -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if nesting != 0 {
                return Err(error(
                    AttributeErrorKind::Syntax,
                    start..i,
                    "unterminated block comment",
                ));
            }
            if doc {
                out.push(Token {
                    range: start..i,
                    close: None,
                    doc: Some(b[start + 2] == b'!'),
                });
            }
            continue;
        }
        let raw = if b[i] == b'r' {
            Some(i + 1)
        } else if b[i..].starts_with(b"br") || b[i..].starts_with(b"cr") {
            Some(i + 2)
        } else {
            None
        };
        let mut literal = false;
        if let Some(mut q) = raw {
            let hashes_start = q;
            while b.get(q) == Some(&b'#') {
                q += 1;
            }
            if b.get(q) == Some(&b'"') {
                let hashes = q - hashes_start;
                i = q + 1;
                loop {
                    if i >= b.len() {
                        return Err(error(
                            AttributeErrorKind::Syntax,
                            start..i,
                            "unterminated raw string",
                        ));
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
                literal = true;
            }
        }
        if !literal {
            let q = if matches!(b[i], b'b' | b'c') && matches!(b.get(i + 1), Some(b'"' | b'\'')) {
                i + 1
            } else {
                i
            };
            let mut lifetime = false;
            if b[q] == b'\''
                && text[q + 1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c == '_' || c.is_alphabetic())
            {
                let mut end = q + 1;
                while let Some(c) = text[end..].chars().next() {
                    if !(c == '_' || c.is_alphanumeric()) {
                        break;
                    }
                    end += c.len_utf8();
                }
                if b.get(end) != Some(&b'\'') {
                    i = end;
                    lifetime = true;
                }
            }
            if !lifetime && matches!(b[q], b'"' | b'\'') {
                let quote = b[q];
                i = q + 1;
                loop {
                    if i >= b.len() {
                        return Err(error(
                            AttributeErrorKind::Syntax,
                            start..i,
                            "unterminated quoted literal",
                        ));
                    }
                    if b[i] == b'\\' {
                        i = (i + 2).min(b.len());
                    } else if b[i] == quote {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            } else if !lifetime {
                let c = text[i..].chars().next().unwrap();
                i += c.len_utf8();
                if c == '_' || c.is_alphabetic() {
                    while let Some(c) = text[i..].chars().next() {
                        if !(c == '_' || c.is_alphanumeric()) {
                            break;
                        }
                        i += c.len_utf8();
                    }
                }
            }
        }
        let n = out.len();
        if i == start + 1 {
            match b[start] {
                b'(' | b'[' | b'{' => {
                    if stack.len() >= 512 {
                        return Err(error(
                            AttributeErrorKind::Limit,
                            start..i,
                            "source delimiter nesting limit exceeded",
                        ));
                    }
                    stack.push((n, b[start]));
                }
                b')' | b']' | b'}' => {
                    let expected = match b[start] {
                        b')' => b'(',
                        b']' => b'[',
                        _ => b'{',
                    };
                    let Some((open, kind)) = stack.pop() else {
                        return Err(error(
                            AttributeErrorKind::Syntax,
                            start..i,
                            "unbalanced delimiter",
                        ));
                    };
                    if kind != expected {
                        return Err(error(
                            AttributeErrorKind::Syntax,
                            start..i,
                            "mismatched delimiter",
                        ));
                    }
                    out[open].close = Some(n);
                }
                _ => {}
            }
        }
        out.push(Token {
            range: start..i,
            close: None,
            doc: None,
        });
        if out.len() > MAX_TOKENS {
            return Err(error(
                AttributeErrorKind::Limit,
                start..i,
                "source token limit exceeded",
            ));
        }
    }
    if let Some((open, _)) = stack.last() {
        return Err(error(
            AttributeErrorKind::Syntax,
            out[*open].range.clone(),
            "unclosed delimiter",
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prep(s: &str) -> Result<AttributeOutput, AttributeError> {
        prepare(s, &CfgSet::default())
    }
    #[test]
    fn pinned_identity_metadata_forms_are_exact_and_inert() {
        for text in [
            r#"stable(feature="convert_id",since="1.33.0")"#,
            r#"rustc_const_stable(feature="const_identity",since="1.33.0")"#,
            r#"rustc_diagnostic_item="convert_identity""#,
        ] {
            assert!(
                valid_identity_metadata(&parse_meta(text, 0).unwrap()),
                "{text}"
            );
        }
        // Validation of source spelling alone does not activate the probe path.
        assert_eq!(prep(r#"#[stable(feature="convert_id",since="1.33.0")] pub const fn identity<T>(x:T)->T{x}"#).unwrap_err().kind, AttributeErrorKind::Unsupported);
    }
    #[test]
    fn identity_metadata_does_not_have_an_ignore_private_attributes_bucket() {
        for text in [
            r#"stable(feature="other",since="1.33.0")"#,
            r#"stable(feature="convert_id",since="1.34.0")"#,
            r#"stable(since="1.33.0",feature="convert_id")"#,
            r#"stable(feature="convert_id",feature="convert_id")"#,
            r#"stable(feature="convert_id",since="1.33.0",extra="x")"#,
            r#"rustc_const_stable(feature="convert_id",since="1.33.0")"#,
            r#"rustc_const_stable(feature="const_identity")"#,
            r#"rustc_diagnostic_item="other""#,
            "rustc_diagnostic_item(convert_identity)",
            "rustc_intrinsic",
            "lang=\"identity\"",
            "feature(rustc_attrs)",
        ] {
            assert!(
                !valid_identity_metadata(&parse_meta(text, 0).unwrap()),
                "{text}"
            );
        }
    }
    #[test]
    fn ordinary_source_cannot_self_activate_identity_metadata_policy() {
        for attr in [
            r#"stable(feature="convert_id",since="1.33.0")"#,
            r#"rustc_const_stable(feature="const_identity",since="1.33.0")"#,
            r#"rustc_diagnostic_item="convert_identity""#,
        ] {
            for name in ["identity", "unrelated"] {
                let source = format!("/// Kept docs\n#[{attr}] #[inline(always)] fn {name}(){{}}");
                assert_eq!(
                    prep(&source).unwrap_err().kind,
                    AttributeErrorKind::Unsupported,
                    "{source}"
                );
            }
        }
        assert_eq!(
            prep("#[verified_identity] fn identity(){}")
                .unwrap_err()
                .kind,
            AttributeErrorKind::Syntax
        );
    }
    #[test]
    fn disabled_module_and_nested_cfg_attr_do_not_activate_path() {
        let s =
            "#[cfg(any())] mod missing; #[cfg_attr(all(),cfg_attr(any(),path=\"x\"))] mod real;";
        let out = prep(s).unwrap();
        assert!(!out.text.contains("missing"));
        assert!(out.text.contains("mod real;"));
        assert!(!out.text.contains('#'));
        assert!(
            out.attributes
                .iter()
                .any(|a| a.name == "path" && !a.effective)
        );
        assert_eq!(
            prep("#[cfg_attr(all(),path=\"x\")] mod real;")
                .unwrap_err()
                .kind,
            AttributeErrorKind::Unsupported
        );
    }
    #[test]
    fn sequential_cfg_short_circuit_and_deferred_unknown_policy() {
        for s in [
            "#[unknown] #[cfg(any())] fn f() {}",
            "#[cfg(any())] #[unknown] fn f() {}",
            "#[cfg(any())] #[cfg(not())] fn f() {}",
        ] {
            assert!(prep(s).is_ok(), "{s}");
        }
        assert_eq!(
            prep("#[cfg(not())] #[cfg(any())] fn f() {}")
                .unwrap_err()
                .kind,
            AttributeErrorKind::Syntax
        );
        assert_eq!(
            prep("#[unknown] fn unused() {}").unwrap_err().kind,
            AttributeErrorKind::Syntax
        );
    }
    #[test]
    fn explicit_attachment_positions_fail_closed() {
        for s in [
            "struct S { #[cfg(any())] x:i32 }",
            "fn f(#[doc=\"x\"] x:i32) {}",
            "fn f() { #[cfg(any())] let x=1; }",
            "impl S { #[inline] fn f() {} }",
            "fn f(){} #![doc=\"x\"]",
        ] {
            assert_eq!(
                prep(s).unwrap_err().kind,
                AttributeErrorKind::Unsupported,
                "{s}"
            );
        }
        assert_eq!(
            prep("#[inline] struct S {}").unwrap_err().kind,
            AttributeErrorKind::Syntax
        );
    }
    #[test]
    fn inner_cfg_removes_scope_before_descendants() {
        let out = prep("mod x { #![cfg(any())] #[unknown] mod missing; } fn main(){}").unwrap();
        assert!(!out.text.contains("missing"));
        assert!(out.text.contains("fn main"));
        assert!(
            prep("#![cfg(any())] #[unknown] mod missing;")
                .unwrap()
                .text
                .trim()
                .is_empty()
        );
    }
    #[test]
    fn docs_inline_and_byte_preservation() {
        let s = "//! β\r\n#[doc=r#\"α\"#]\n#[doc(hidden)] #[inline(always)] fn f() {}";
        let out = prep(s).unwrap();
        assert_eq!(out.text.len(), s.len());
        for (a, b) in out.text.bytes().zip(s.bytes()) {
            if matches!(b, b'\n' | b'\r') {
                assert_eq!(a, b);
            }
        }
        assert_eq!(out.attributes.iter().filter(|a| a.name == "doc").count(), 3);
        for s in [
            "#[inline()] fn f(){}",
            "#[inline(sometimes)] fn f(){}",
            "#[inline=\"x\"] fn f(){}",
        ] {
            assert_eq!(prep(s).unwrap_err().kind, AttributeErrorKind::Syntax);
        }
        assert_eq!(
            prep("#[doc(unknown)] fn f(){}").unwrap_err().kind,
            AttributeErrorKind::Unsupported
        );
    }
    #[test]
    fn literal_comment_module_spellings_are_inert() {
        prep("// #[path=\"x\"] mod absent;\nfn f(){ let a=r###\"#[unknown] mod x;\"###; let c='}'; /* #[unknown] */ }").unwrap();
    }
    #[test]
    fn inactive_payload_semantics_not_checked() {
        for s in [
            "#[cfg_attr(any(),inline(sometimes))] fn f(){}",
            "#[cfg_attr(any(),cfg(not()))] fn f(){}",
            "#[cfg_attr(any(),unknown_attr(foo =))] fn f(){}",
        ] {
            assert!(prep(s).is_ok(), "{s}");
        }
        assert!(prep("#[cfg_attr(all(),cfg(not()))] fn f(){}").is_err());
    }
    #[test]
    fn semantic_forms_deferred_but_outer_syntax_always_checked() {
        for attr in ["doc=3", "inline(3)", "inline(sometimes)"] {
            for s in [
                format!("#[{attr}] #[cfg(any())] fn f(){{}}"),
                format!("#[cfg(any())] #[{attr}] fn f(){{}}"),
            ] {
                assert!(prep(&s).is_ok(), "{s}");
            }
        }
        for s in [
            "#[unknown =] #[cfg(any())] fn f(){}",
            "#[cfg(any())] #[doc =] fn f(){}",
            "#[cfg_attr(any(),doc =)] fn f(){}",
        ] {
            assert_eq!(prep(s).unwrap_err().kind, AttributeErrorKind::Syntax, "{s}");
        }
        assert_eq!(
            prep("#[doc=42] fn f(){}").unwrap_err().kind,
            AttributeErrorKind::Unsupported
        );
        assert_eq!(
            prep("#[repr(C)] struct S{}").unwrap_err().kind,
            AttributeErrorKind::Unsupported
        );
    }
    #[test]
    fn inactive_fragments_keep_original_byte_coordinates() {
        let s = "// β\n#[cfg(any())] fn f(){ #[unknown] let x=1; } fn g(){}";
        let out = prep(s).unwrap();
        let item = &out.inactive_items[0];
        assert_eq!(item.text.len(), item.range.len());
        assert!(item.text.starts_with("fn f"));
        assert!(!item.text.contains('#'));
        assert_eq!(&s[item.range.clone()][..4], "fn f");
    }
    #[test]
    fn disabled_unknown_envelope_cannot_launder_following_item() {
        let s = "#[cfg(any())] unexpanded! {} #[unknown] fn f(){}";
        assert_eq!(prep(s).unwrap_err().kind, AttributeErrorKind::Syntax);
    }
    #[test]
    fn reserved_attribute_names_are_syntax_even_when_disabled() {
        for attr in ["fn", "_"] {
            for source in [
                format!("#[{attr}] #[cfg(any())] fn f(){{}}"),
                format!("#[cfg(any())] #[{attr}] fn f(){{}}"),
            ] {
                assert_eq!(
                    prep(&source).unwrap_err().kind,
                    AttributeErrorKind::Syntax,
                    "{source}"
                );
            }
        }
        assert_eq!(
            prep("#[unsafe(no_mangle)] fn f(){}").unwrap_err().kind,
            AttributeErrorKind::Unsupported
        );
    }
    #[test]
    fn inactive_attribute_numeric_grammar_is_fail_closed() {
        for literal in ["0b2", "0xff", "42u8", "1.5"] {
            let source = format!("#[cfg_attr(any(),unknown({literal}))] fn f(){{}}");
            assert_eq!(
                prep(&source).unwrap_err().kind,
                AttributeErrorKind::Unsupported,
                "{source}"
            );
        }
        prep("#[doc=3] #[cfg(any())] fn f(){}").unwrap();
        prep("#[cfg_attr(any(),unknown(42))] fn f(){}").unwrap();
    }
    #[test]
    fn inactive_attribute_token_trees_do_not_skip_unsupported_literals() {
        for literal in [
            r#"b"\q""#,
            r#"c"\q""#,
            r#"br"value""#,
            r#"cr"value""#,
            r"'\q'",
            r"b'\q'",
        ] {
            let source = format!("#[cfg_attr(any(), unknown({literal}))] fn f(){{}}");
            assert_eq!(
                prep(&source).unwrap_err().kind,
                AttributeErrorKind::Unsupported,
                "{source}"
            );
        }
        assert_eq!(
            prep(r#"#[cfg_attr(any(),unknown("\q"))] fn f(){}"#)
                .unwrap_err()
                .kind,
            AttributeErrorKind::Syntax
        );
        prep(r##"#[cfg_attr(any(),unknown("valid",r#"raw"#))] fn f(){}"##).unwrap();
    }
    #[test]
    fn malformed_string_escape_is_lexical_even_after_false_cfg() {
        let attr = r#"#[cfg(mode="\u{_41}")]"#;
        for source in [
            format!("{attr} fn f(){{}}"),
            format!("{attr} #[cfg(any())] fn f(){{}}"),
            format!("#[cfg(any())] {attr} fn f(){{}}"),
        ] {
            let error = prep(&source).unwrap_err();
            assert_eq!(error.kind, AttributeErrorKind::Syntax, "{source}");
            assert!(error.message.contains("Unicode escape"));
        }
    }
    #[test]
    fn nonempty_invalid_rhs_is_never_suppressed() {
        for attr in ["doc = ;", "unknown = +"] {
            for source in [
                format!("#[{attr}] #[cfg(any())] fn f(){{}}"),
                format!("#[cfg(any())] #[{attr}] fn f(){{}}"),
            ] {
                assert_eq!(
                    prep(&source).unwrap_err().kind,
                    AttributeErrorKind::Syntax,
                    "{source}"
                );
            }
        }
        assert_eq!(
            prep("#[doc = concat!(\"x\")] #[cfg(any())] fn f(){}")
                .unwrap_err()
                .kind,
            AttributeErrorKind::Unsupported
        );
    }
    #[test]
    fn inline_module_depth_is_separate_from_cfg_attr_depth() {
        for depth in [128, 129, 300] {
            let s = format!("{}fn f(){{}}{}", "mod m{".repeat(depth), "}".repeat(depth));
            if depth == 128 {
                prep(&s).unwrap();
            } else {
                assert_eq!(
                    prep(&s).unwrap_err().message,
                    "module nesting exceeds limit 128"
                );
            }
        }
    }
    #[test]
    fn nested_expansion_is_bounded() {
        let s = format!(
            "#[{}inline{}] fn f(){{}}",
            "cfg_attr(all(),".repeat(70),
            ")".repeat(70)
        );
        assert_eq!(prep(&s).unwrap_err().kind, AttributeErrorKind::Limit);
    }
}
