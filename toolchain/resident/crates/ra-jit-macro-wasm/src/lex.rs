//! Text a macro asks to be tokens of (`str::parse::<TokenStream>`, `Literal::from_str`),
//! lexed by the host as rust-analyzer's proc-macro server lexes it
//! (`proc-macro-srv/src/token_stream.rs`), each token at a span of `call_site`'s anchor and
//! context over its range of the text.

use intern::Symbol;
use ra_ap_rustc_lexer::{self as lexer, DocStyle, LiteralKind, TokenKind};
use span::{Span, TextRange, TextSize};
use tt::{DelimiterKind, IdentIsRaw, Leaf, LitKind, Spacing};

use crate::wire::Tree;

fn ranged(span: Span, range: std::ops::Range<usize>) -> Span {
    let range = TextRange::new(TextSize::new(range.start as u32), TextSize::new(range.end as u32));
    Span { range, ..span }
}

fn punct_char(kind: TokenKind) -> Option<char> {
    Some(match kind {
        TokenKind::Semi => ';',
        TokenKind::Comma => ',',
        TokenKind::Dot => '.',
        TokenKind::At => '@',
        TokenKind::Pound => '#',
        TokenKind::Tilde => '~',
        TokenKind::Question => '?',
        TokenKind::Colon => ':',
        TokenKind::Dollar => '$',
        TokenKind::Eq => '=',
        TokenKind::Bang => '!',
        TokenKind::Lt => '<',
        TokenKind::Gt => '>',
        TokenKind::Minus => '-',
        TokenKind::And => '&',
        TokenKind::Or => '|',
        TokenKind::Plus => '+',
        TokenKind::Star => '*',
        TokenKind::Slash => '/',
        TokenKind::Caret => '^',
        TokenKind::Percent => '%',
        _ => return None,
    })
}

/// Whether a punctuation mark before a token of `kind` is joint to it.
fn joins(kind: TokenKind) -> bool {
    punct_char(kind).is_some()
        || matches!(
            kind,
            TokenKind::RawLifetime
                | TokenKind::GuardedStrPrefix
                | TokenKind::Lifetime { .. }
                | TokenKind::OpenParen
                | TokenKind::CloseParen
                | TokenKind::OpenBrace
                | TokenKind::CloseBrace
                | TokenKind::OpenBracket
                | TokenKind::CloseBracket
        )
}

fn ident(text: &str, span: Span, is_raw: IdentIsRaw) -> Tree {
    Tree::Leaf(Leaf::Ident(tt::Ident { sym: Symbol::intern(text), span, is_raw }))
}

fn punct(char: char, joint: bool, span: Span) -> Tree {
    let spacing = if joint { Spacing::Joint } else { Spacing::Alone };
    Tree::Leaf(Leaf::Punct(tt::Punct { char, spacing, span }))
}

/// A literal of the lexer's `kind`, `text` its whole text.
fn literal(text: &str, span: Span, kind: LiteralKind, suffix_start: u32) -> tt::Literal {
    let (kind, start_offset, end_offset) = match kind {
        LiteralKind::Int { .. } => (LitKind::Integer, 0, 0),
        LiteralKind::Float { .. } => (LitKind::Float, 0, 0),
        LiteralKind::Char { terminated } => (LitKind::Char, 1, terminated as usize),
        LiteralKind::Byte { terminated } => (LitKind::Byte, 2, terminated as usize),
        LiteralKind::Str { terminated } => (LitKind::Str, 1, terminated as usize),
        LiteralKind::ByteStr { terminated } => (LitKind::ByteStr, 2, terminated as usize),
        LiteralKind::CStr { terminated } => (LitKind::CStr, 2, terminated as usize),
        LiteralKind::RawStr { n_hashes } => {
            let n = n_hashes.unwrap_or_default();
            (LitKind::StrRaw(n), 2 + n as usize, 1 + n as usize)
        }
        LiteralKind::RawByteStr { n_hashes } => {
            let n = n_hashes.unwrap_or_default();
            (LitKind::ByteStrRaw(n), 3 + n as usize, 1 + n as usize)
        }
        LiteralKind::RawCStr { n_hashes } => {
            let n = n_hashes.unwrap_or_default();
            (LitKind::CStrRaw(n), 3 + n as usize, 1 + n as usize)
        }
    };
    let (lit, suffix) = text.split_at(suffix_start as usize);
    let lit = &lit[start_offset..lit.len() - end_offset];
    let suffix = if suffix == "_" { "" } else { suffix };
    tt::Literal::new(lit, span, kind, suffix)
}

/// A doc comment as the attribute it stands for.
fn doc(
    stream: &mut Vec<Tree>,
    text: &str,
    style: DocStyle,
    span: Span,
    range: std::ops::Range<usize>,
) {
    stream.push(punct('#', false, span));
    if style == DocStyle::Inner {
        stream.push(punct('!', false, span));
    }
    let value =
        tt::Literal::new(&text.escape_debug().to_string(), ranged(span, range), LitKind::Str, "");
    stream.push(Tree::Group {
        kind: DelimiterKind::Bracket,
        open: span,
        close: span,
        entire: span,
        trees: vec![
            ident("doc", span, IdentIsRaw::No),
            punct('=', false, span),
            Tree::Leaf(Leaf::Literal(value)),
        ],
    });
}

/// The tokens of `s`, or why it is none.
pub(crate) fn stream(s: &str, span: Span) -> Result<Vec<Tree>, String> {
    let mut groups: Vec<(DelimiterKind, std::ops::Range<usize>, Vec<Tree>)> =
        vec![(DelimiterKind::Invisible, 0..0, vec![])];
    let mut offset = 0;
    let mut tokens = lexer::tokenize(s, lexer::FrontmatterAllowed::No).peekable();
    while let Some(token) = tokens.next() {
        let range = offset..offset + token.len as usize;
        offset += token.len as usize;
        let joint = tokens.peek().is_some_and(|next| joins(next.kind));
        let opens = match token.kind {
            TokenKind::OpenParen => Some(DelimiterKind::Parenthesis),
            TokenKind::OpenBrace => Some(DelimiterKind::Brace),
            TokenKind::OpenBracket => Some(DelimiterKind::Bracket),
            _ => None,
        };
        if let Some(kind) = opens {
            groups.push((kind, range, vec![]));
            continue;
        }
        let closes = match token.kind {
            TokenKind::CloseParen => Some((DelimiterKind::Parenthesis, ')')),
            TokenKind::CloseBrace => Some((DelimiterKind::Brace, '}')),
            TokenKind::CloseBracket => Some((DelimiterKind::Bracket, ']')),
            _ => None,
        };
        if let Some((kind, char)) = closes {
            let open_kind = groups.last().map(|it| it.0);
            if open_kind != Some(kind) {
                return Err(match open_kind {
                    Some(DelimiterKind::Invisible) | None => format!("Unexpected '{char}'"),
                    Some(_) => format!("Expected '{char}'"),
                });
            }
            let (kind, open_range, trees) = groups.pop().unwrap();
            let group = Tree::Group {
                kind,
                open: ranged(span, open_range.clone()),
                close: ranged(span, range.clone()),
                entire: ranged(span, open_range.start..range.end),
                trees,
            };
            groups.last_mut().ok_or("Unbalanced delimiters")?.2.push(group);
            continue;
        }
        let stream = &mut groups.last_mut().ok_or("Unbalanced delimiters")?.2;
        let text = &s[range.clone()];
        match token.kind {
            TokenKind::LineComment { doc_style: None }
            | TokenKind::BlockComment { doc_style: None, .. }
            | TokenKind::Whitespace => {}
            TokenKind::LineComment { doc_style: Some(style) } => {
                doc(stream, &s[range.start + 3..range.end], style, span, range)
            }
            TokenKind::BlockComment { doc_style: Some(style), terminated } => {
                let end = if terminated { range.end - 2 } else { range.end };
                doc(stream, &s[range.start + 3..end], style, span, range)
            }
            TokenKind::Unknown => return Err(format!("Unknown token: `{text}`")),
            TokenKind::UnknownPrefix => return Err(format!("Unknown prefix: `{text}`")),
            TokenKind::UnknownPrefixLifetime => {
                return Err(format!("Unknown lifetime prefix: `{text}`"));
            }
            TokenKind::InvalidIdent => return Err(format!("Invalid identifier: `{text}`")),
            TokenKind::GuardedStrPrefix => {
                let bytes = s.as_bytes();
                let first = range.start..range.start + 1;
                stream.push(punct(bytes[range.start] as char, true, ranged(span, first)));
                let second = range.start + 1..range.end;
                stream.push(punct(bytes[range.start + 1] as char, joint, ranged(span, second)));
            }
            TokenKind::Ident => stream.push(ident(text, ranged(span, range), IdentIsRaw::No)),
            TokenKind::RawIdent => {
                let range = range.start + 2..range.end;
                stream.push(ident(&s[range.clone()], ranged(span, range), IdentIsRaw::Yes))
            }
            TokenKind::Literal { kind, suffix_start } => {
                let it = literal(text, ranged(span, range), kind, suffix_start);
                stream.push(Tree::Leaf(Leaf::Literal(it)))
            }
            TokenKind::RawLifetime | TokenKind::Lifetime { .. } => {
                if let TokenKind::Lifetime { starts_with_number: true } = token.kind {
                    return Err("Lifetime cannot start with a number".to_owned());
                }
                let skip = if token.kind == TokenKind::RawLifetime { 3 } else { 1 };
                let range = range.start + skip..range.end;
                let quote = range.start..range.start + 1;
                stream.push(punct('\'', true, ranged(span, quote)));
                let is_raw = if token.kind == TokenKind::RawLifetime {
                    IdentIsRaw::Yes
                } else {
                    IdentIsRaw::No
                };
                stream.push(ident(&s[range.clone()], ranged(span, range), is_raw))
            }
            TokenKind::Eof => break,
            kind => match punct_char(kind) {
                Some(char) => stream.push(punct(char, joint, ranged(span, range))),
                None => return Err(format!("Unknown token: `{text}`")),
            },
        }
    }
    match (groups.pop(), groups.is_empty()) {
        (Some((DelimiterKind::Invisible, _, trees)), true) => Ok(trees),
        _ => Err("Mismatched token groups".to_owned()),
    }
}

/// The literal `s` is, with a leading `-` for a number.
pub(crate) fn literal_from_str(s: &str, span: Span) -> Option<tt::Literal> {
    let mut tokens = lexer::tokenize(s, lexer::FrontmatterAllowed::No);
    let first = tokens.next()?;
    let (minus, lit) =
        if first.kind == TokenKind::Minus { (1, tokens.next()?) } else { (0, first) };
    if tokens.next().is_some() {
        return None;
    }
    let TokenKind::Literal { kind, suffix_start } = lit.kind else { return None };
    if minus == 1 && !matches!(kind, LiteralKind::Int { .. } | LiteralKind::Float { .. }) {
        return None;
    }
    let it = literal(&s[minus..], span, kind, suffix_start);
    if minus == 0 {
        return Some(it);
    }
    let (text, suffix) = it.text_and_suffix();
    Some(tt::Literal::new(&format!("-{text}"), span, it.kind, suffix))
}
