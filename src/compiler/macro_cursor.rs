//! Checked immutable macro views. No source or semantic admission authority.
use super::expansion_budget::{Counter, Storage};
use super::expansion_span::SpanId;
use super::token::{Delimiter, Ident, Spacing, Token, TokenKind};
use super::token_session::{BudgetVec, SessionId, TokenError, TokenSession, allocate_identity};
use super::token_tree::{Node, TokenStream, TreeRange};
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::atomic::AtomicU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SliceId(NonZeroU64);
#[derive(Clone)]
pub struct TokenSlice {
    stream: TokenStream,
    range: TreeRange,
    id: SliceId,
}
impl TokenSlice {
    pub fn new(
        session: &TokenSession,
        stream: TokenStream,
        range: TreeRange,
    ) -> Result<Self, TokenError> {
        stream.children(session, &range)?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            stream,
            range,
            id: SliceId(allocate_identity(&NEXT)?),
        })
    }
    pub fn roots(session: &TokenSession, stream: TokenStream) -> Result<Self, TokenError> {
        let range = stream.roots(session)?;
        Self::new(session, stream, range)
    }
    pub fn len(&self, session: &TokenSession) -> Result<usize, TokenError> {
        Ok(self.stream.children(session, &self.range)?.len())
    }
    pub fn is_empty(&self, session: &TokenSession) -> Result<bool, TokenError> {
        Ok(self.len(session)? == 0)
    }
    pub fn node(&self, session: &TokenSession, index: usize) -> Result<&Node, TokenError> {
        let id = *self
            .stream
            .children(session, &self.range)?
            .get(index)
            .ok_or(TokenError::InvalidRange)?;
        self.stream.node(session, id)
    }
    pub fn subslice(
        &self,
        session: &TokenSession,
        range: Range<usize>,
    ) -> Result<Self, TokenError> {
        let range = self.stream.subrange(session, &self.range, range)?;
        Self::new(session, self.stream.clone(), range)
    }
    pub fn group_children(
        &self,
        session: &TokenSession,
        range: TreeRange,
    ) -> Result<Self, TokenError> {
        Self::new(session, self.stream.clone(), range)
    }
    pub fn children(
        &self,
        session: &TokenSession,
        index: usize,
    ) -> Result<Option<Self>, TokenError> {
        match self.node(session, index)? {
            Node::Delimited { children, .. } => {
                Ok(Some(self.group_children(session, children.clone())?))
            }
            Node::Leaf(_) => Ok(None),
        }
    }
    pub fn physical_weight(&self, session: &TokenSession) -> Result<u64, TokenError> {
        let mut pending = BudgetVec::new(session.budget(), Storage::Temporary)?;
        pending.push(self.range.clone())?;
        let mut total = 0u64;
        while let Some(range) = pending.pop() {
            for id in self.stream.children(session, &range)? {
                session.budget().charge(Counter::Work, 1)?;
                let weight = match self.stream.node(session, *id)? {
                    Node::Leaf(Token {
                        kind: TokenKind::Opaque(id),
                        ..
                    }) => session.fragment(*id)?.physical_weight(),
                    Node::Leaf(_) => 1,
                    Node::Delimited { children, .. } => {
                        pending.push(children.clone())?;
                        2
                    }
                };
                total = total.checked_add(weight).ok_or(TokenError::IdExhausted)?;
            }
        }
        Ok(total)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorMark {
    session: SessionId,
    slice: SliceId,
    position: usize,
}
#[derive(Clone)]
pub enum LogicalToken {
    Leaf(Token),
    Operator {
        chars: [char; 3],
        spans: [Option<SpanId>; 3],
        len: u8,
    },
    Lifetime {
        quote: SpanId,
        ident: Ident,
        span: SpanId,
    },
    Group {
        kind: Delimiter,
        open: SpanId,
        children: TokenSlice,
        close: SpanId,
    },
}
pub struct MacroCursor {
    input: TokenSlice,
    position: usize,
}
impl MacroCursor {
    pub fn new(session: &TokenSession, input: TokenSlice) -> Result<Self, TokenError> {
        input.len(session)?;
        Ok(Self { input, position: 0 })
    }
    pub fn mark(&self, session: &TokenSession) -> Result<CursorMark, TokenError> {
        self.input.len(session)?;
        Ok(CursorMark {
            session: session.id(),
            slice: self.input.id,
            position: self.position,
        })
    }
    fn check_mark(&self, session: &TokenSession, mark: CursorMark) -> Result<(), TokenError> {
        let len = self.input.len(session)?;
        if mark.session != session.id() || mark.slice != self.input.id {
            return Err(TokenError::ForeignHandle);
        }
        if mark.position > len {
            return Err(TokenError::InvalidRange);
        }
        Ok(())
    }
    pub fn restore(&mut self, session: &TokenSession, mark: CursorMark) -> Result<(), TokenError> {
        self.check_mark(session, mark)?;
        self.position = mark.position;
        Ok(())
    }
    pub fn capture_since(
        &self,
        session: &TokenSession,
        mark: CursorMark,
    ) -> Result<TokenSlice, TokenError> {
        self.check_mark(session, mark)?;
        self.input.subslice(session, mark.position..self.position)
    }
    pub fn peek(&self, session: &TokenSession) -> Result<Option<LogicalToken>, TokenError> {
        Ok(self.read(session)?.map(|(token, _)| token))
    }
    pub fn bump(&mut self, session: &TokenSession) -> Result<Option<LogicalToken>, TokenError> {
        let result = self.read(session)?;
        if let Some((token, count)) = result {
            self.position += count;
            Ok(Some(token))
        } else {
            Ok(None)
        }
    }
    fn read(&self, session: &TokenSession) -> Result<Option<(LogicalToken, usize)>, TokenError> {
        session.budget().charge(Counter::Work, 1)?;
        let len = self.input.len(session)?;
        if self.position == len {
            return Ok(None);
        }
        let token = match self.input.node(session, self.position)? {
            Node::Delimited {
                kind,
                open,
                children,
                close,
            } => {
                return Ok(Some((
                    LogicalToken::Group {
                        kind: *kind,
                        open: *open,
                        children: self.input.group_children(session, children.clone())?,
                        close: *close,
                    },
                    1,
                )));
            }
            Node::Leaf(token) => *token,
        };
        if let TokenKind::Punct {
            ch,
            spacing: Spacing::Joint,
        } = token.kind
        {
            if ch == '\''
                && self.position + 1 < len
                && let Node::Leaf(Token {
                    kind: TokenKind::Ident(ident),
                    span,
                }) = self.input.node(session, self.position + 1)?
            {
                return Ok(Some((
                    LogicalToken::Lifetime {
                        quote: token.span,
                        ident: *ident,
                        span: *span,
                    },
                    2,
                )));
            }
            let mut chars = ['\0'; 3];
            let mut spans = [None; 3];
            chars[0] = ch;
            spans[0] = Some(token.span);
            let mut count = 1;
            let mut joint = true;
            while joint && count < 3 && self.position + count < len {
                let Node::Leaf(Token {
                    kind: TokenKind::Punct { ch, spacing },
                    span,
                }) = self.input.node(session, self.position + count)?
                else {
                    break;
                };
                chars[count] = *ch;
                let candidate = &chars[..=count];
                if !matches!(
                    candidate,
                    [':', ':']
                        | ['-', '>']
                        | ['<', '-']
                        | ['=', '>']
                        | ['=', '=']
                        | ['!', '=']
                        | ['<', '=']
                        | ['>', '=']
                        | ['&', '&']
                        | ['|', '|']
                        | ['<', '<']
                        | ['>', '>']
                        | ['+', '=']
                        | ['-', '=']
                        | ['*', '=']
                        | ['/', '=']
                        | ['%', '=']
                        | ['^', '=']
                        | ['&', '=']
                        | ['|', '=']
                        | ['.', '.']
                        | ['.', '.', '.']
                        | ['.', '.', '=']
                        | ['<', '<', '=']
                        | ['>', '>', '=']
                ) {
                    chars[count] = '\0';
                    break;
                }
                spans[count] = Some(*span);
                joint = *spacing == Spacing::Joint;
                count += 1;
            }
            if count > 1 {
                return Ok(Some((
                    LogicalToken::Operator {
                        chars,
                        spans,
                        len: count as u8,
                    },
                    count,
                )));
            }
        }
        Ok(Some((LogicalToken::Leaf(token), 1)))
    }
}
