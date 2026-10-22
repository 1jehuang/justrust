//! Flat, immutable, owner-checked token grouping. Not a lexer or macro expander.
//! Grouping charges structural work/storage/depth only. Producers are responsible
//! for logical input/emitted token charges exactly once. Tests cannot establish
//! emitted macro acceptance from grouping alone.
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::Arc;

use super::expansion_budget::{ByteLease, Counter, Depth, Storage};
use super::expansion_span::SpanId;
use super::token::{Delimiter, InputToken, Token};
use super::token_session::{BudgetVec, SessionId, TokenError, TokenSession};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupPolicy {
    OriginalSource,
    Generated,
}
impl GroupPolicy {
    fn depth(self) -> Depth {
        match self {
            Self::OriginalSource => Depth::OriginalGroups,
            Self::Generated => Depth::GeneratedGroups,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StreamId(NonZeroU64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TreeId {
    session: SessionId,
    stream: StreamId,
    index: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TreeRange {
    session: SessionId,
    stream: StreamId,
    range: Range<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Leaf(Token),
    Delimited {
        kind: Delimiter,
        open: SpanId,
        children: TreeRange,
        close: SpanId,
    },
}

struct StreamData {
    session: SessionId,
    id: StreamId,
    nodes: BudgetVec<Node>,
    edges: BudgetVec<TreeId>,
    roots: TreeRange,
    _lease: ByteLease,
}

/// Shared immutable payload. Access requires the issuing live session.
#[derive(Clone)]
pub struct TokenStream {
    inner: Arc<StreamData>,
}
impl TokenStream {
    pub fn subrange(
        &self,
        session: &TokenSession,
        parent: &TreeRange,
        range: Range<usize>,
    ) -> Result<TreeRange, TokenError> {
        let len = self.children(session, parent)?.len();
        if range.start > range.end || range.end > len {
            return Err(TokenError::InvalidRange);
        }
        let start = parent
            .range
            .start
            .checked_add(checked_index(session, range.start)?)
            .ok_or(TokenError::InvalidRange)?;
        let end = parent
            .range
            .start
            .checked_add(checked_index(session, range.end)?)
            .ok_or(TokenError::InvalidRange)?;
        Ok(TreeRange {
            session: parent.session,
            stream: parent.stream,
            range: start..end,
        })
    }
    fn check_session(&self, session: &TokenSession) -> Result<(), TokenError> {
        session.budget().check()?;
        if !session.serves(self.inner.session) {
            return Err(TokenError::ForeignHandle);
        }
        Ok(())
    }
    pub fn roots(&self, session: &TokenSession) -> Result<TreeRange, TokenError> {
        self.check_session(session)?;
        Ok(self.inner.roots.clone())
    }
    pub fn node(&self, session: &TokenSession, id: TreeId) -> Result<&Node, TokenError> {
        self.check_session(session)?;
        if id.session != self.inner.session || id.stream != self.inner.id {
            return Err(TokenError::ForeignHandle);
        }
        self.inner
            .nodes
            .as_slice()
            .get(id.index as usize)
            .ok_or(TokenError::InvalidHandle)
    }
    pub fn children(
        &self,
        session: &TokenSession,
        range: &TreeRange,
    ) -> Result<&[TreeId], TokenError> {
        self.check_session(session)?;
        if range.session != self.inner.session || range.stream != self.inner.id {
            return Err(TokenError::ForeignHandle);
        }
        self.inner
            .edges
            .as_slice()
            .get(range.range.start as usize..range.range.end as usize)
            .ok_or(TokenError::InvalidRange)
    }
}

#[derive(Clone, Copy)]
struct Frame {
    kind: Delimiter,
    open: SpanId,
    pending_start: usize,
}

fn checked_index(session: &TokenSession, index: usize) -> Result<u32, TokenError> {
    Ok(session.budget().checked_u32(index as u64)?)
}

fn append_children(
    session: &TokenSession,
    stream: StreamId,
    pending: &[TreeId],
    edges: &mut BudgetVec<TreeId>,
) -> Result<TreeRange, TokenError> {
    let start = checked_index(session, edges.len())?;
    for id in pending {
        session.budget().charge(Counter::Work, 1)?;
        edges.push(*id)?;
    }
    Ok(TreeRange {
        session: session.id(),
        stream,
        range: start..checked_index(session, edges.len())?,
    })
}

/// Structural grouping only; no lexical classification, source loading or macros.
/// OriginalSource is selected by the source owner, not inferred from token text.
pub fn group(
    session: &mut TokenSession,
    input: &[InputToken],
    policy: GroupPolicy,
) -> Result<TokenStream, TokenError> {
    session.budget().check()?;
    session.budget().charge(Counter::Work, 1)?;
    let lease = session.budget().reserve(
        Storage::Retained,
        (std::mem::size_of::<StreamData>() + 2 * std::mem::size_of::<usize>()) as u64,
    )?;
    let stream = StreamId(session.issue_stream_id()?);
    let session = &*session;
    let mut nodes = BudgetVec::new(session.budget(), Storage::Retained)?;
    let mut edges = BudgetVec::new(session.budget(), Storage::Retained)?;
    let mut frames: BudgetVec<Frame> = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut pending = BudgetVec::new(session.budget(), Storage::Temporary)?;
    for token in input {
        session.budget().charge(Counter::Work, 1)?;
        session.span(token.span())?;
        match *token {
            InputToken::Leaf(token) => {
                token.validate(session)?;
                let id = TreeId {
                    session: session.id(),
                    stream,
                    index: checked_index(session, nodes.len())?,
                };
                nodes.push(Node::Leaf(token))?;
                pending.push(id)?;
            }
            InputToken::Open { kind, span } => {
                let depth = checked_index(session, frames.len())?
                    .checked_add(1)
                    .ok_or(TokenError::IdExhausted)?;
                session.budget().check_depth(policy.depth(), depth)?;
                frames.push(Frame {
                    kind,
                    open: span,
                    pending_start: pending.len(),
                })?;
            }
            InputToken::Close { kind, span } => {
                let frame = frames
                    .pop()
                    .ok_or(TokenError::UnbalancedDelimiter { span: Some(span) })?;
                if frame.kind != kind {
                    return Err(TokenError::UnbalancedDelimiter { span: Some(span) });
                }
                let children = append_children(
                    session,
                    stream,
                    pending
                        .as_slice()
                        .get(frame.pending_start..)
                        .ok_or(TokenError::InvalidRange)?,
                    &mut edges,
                )?;
                pending.truncate(frame.pending_start);
                let id = TreeId {
                    session: session.id(),
                    stream,
                    index: checked_index(session, nodes.len())?,
                };
                nodes.push(Node::Delimited {
                    kind,
                    open: frame.open,
                    children,
                    close: span,
                })?;
                pending.push(id)?;
            }
        }
    }
    if let Some(frame) = frames.last() {
        return Err(TokenError::UnbalancedDelimiter {
            span: Some(frame.open),
        });
    }
    let roots = append_children(session, stream, pending.as_slice(), &mut edges)?;
    Ok(TokenStream {
        inner: Arc::new(StreamData {
            session: session.id(),
            id: stream,
            nodes,
            edges,
            roots,
            _lease: lease,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::super::expansion_budget::{ExpansionBudget, ExpansionLimits};
    use super::super::source::source_map::SourceMap;
    use super::super::token::{Spacing, TokenKind};
    use super::super::token_session::SourceSnapshot;
    use super::*;

    fn session() -> (TokenSession, SpanId) {
        let mut map = SourceMap::default();
        map.add("tree.rs".into(), Arc::from("()[]{}+"));
        let sources = SourceSnapshot::new(
            map,
            crate::compiler::owner_path::standalone_issued(
                &crate::compiler::expansion_budget::ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let source = sources.span(sources.file(0).unwrap(), 0, 1).unwrap();
        let mut session = TokenSession::new(sources, ExpansionBudget::default()).unwrap();
        let span = session.original_span(source).unwrap();
        (session, span)
    }
    fn leaf(span: SpanId) -> InputToken {
        InputToken::Leaf(Token {
            kind: TokenKind::Punct {
                ch: '+',
                spacing: Spacing::Alone,
            },
            span,
        })
    }
    #[test]
    fn nested_order_flat_ownership_and_independent_delimiter_spans() {
        let (mut session, open) = session();
        let source = session
            .sources()
            .span(session.sources().file(0).unwrap(), 1, 2)
            .unwrap();
        let close = session.original_span(source).unwrap();
        let input = [
            leaf(open),
            InputToken::Open {
                kind: Delimiter::Brace,
                span: open,
            },
            InputToken::Open {
                kind: Delimiter::Bracket,
                span: open,
            },
            leaf(open),
            InputToken::Close {
                kind: Delimiter::Bracket,
                span: close,
            },
            leaf(close),
            InputToken::Close {
                kind: Delimiter::Brace,
                span: close,
            },
            leaf(close),
        ];
        let tree = group(&mut session, &input, GroupPolicy::OriginalSource).unwrap();
        let roots = tree
            .children(&session, &tree.roots(&session).unwrap())
            .unwrap();
        assert_eq!(roots.len(), 3);
        let Node::Delimited {
            kind,
            open: actual_open,
            children,
            close: actual_close,
        } = tree.node(&session, roots[1]).unwrap()
        else {
            panic!("group expected");
        };
        assert_eq!(
            (*kind, *actual_open, *actual_close),
            (Delimiter::Brace, open, close)
        );
        let inner = tree.children(&session, children).unwrap();
        assert_eq!(inner.len(), 2);
        assert!(matches!(
            tree.node(&session, inner[0]).unwrap(),
            Node::Delimited {
                kind: Delimiter::Bracket,
                ..
            }
        ));
        let cloned = tree.clone();
        assert!(Arc::ptr_eq(&tree.inner, &cloned.inner));
        assert_eq!(
            session.budget().snapshot().counters[Counter::InputTokens as usize],
            0
        );
        assert_eq!(
            session.budget().snapshot().counters[Counter::OutputTokens as usize],
            0
        );
    }
    #[test]
    fn foreign_session_stream_and_nested_handles_reject() {
        let (mut session, span) = session();
        let (other, other_span) = self::session();
        assert!(matches!(
            group(&mut session, &[leaf(other_span)], GroupPolicy::Generated),
            Err(TokenError::ForeignHandle)
        ));
        let a = group(&mut session, &[leaf(span)], GroupPolicy::Generated).unwrap();
        let b = group(&mut session, &[leaf(span)], GroupPolicy::Generated).unwrap();
        let roots = a.roots(&session).unwrap();
        let id = a.children(&session, &roots).unwrap()[0];
        assert!(matches!(a.node(&other, id), Err(TokenError::ForeignHandle)));
        assert!(matches!(
            b.node(&session, id),
            Err(TokenError::ForeignHandle)
        ));
        assert!(matches!(
            b.children(&session, &roots),
            Err(TokenError::ForeignHandle)
        ));
        assert!(a.roots(&other).is_err());
    }
    #[test]
    fn mismatch_unclosed_and_empty_root_are_explicit() {
        let (mut session, span) = session();
        let open = InputToken::Open {
            kind: Delimiter::Brace,
            span,
        };
        let close = InputToken::Close {
            kind: Delimiter::Bracket,
            span,
        };
        for input in [&[open][..], &[close][..], &[open, close][..]] {
            assert!(matches!(group(&mut session, input, GroupPolicy::Generated),
                Err(TokenError::UnbalancedDelimiter { span: Some(s) }) if s == span));
        }
        let empty = group(&mut session, &[], GroupPolicy::Generated).unwrap();
        assert!(
            empty
                .children(&session, &empty.roots(&session).unwrap())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn original_512_and_generated_256_policies_are_distinct() {
        for (policy, depth, pass) in [
            (GroupPolicy::OriginalSource, 512, true),
            (GroupPolicy::OriginalSource, 513, false),
            (GroupPolicy::Generated, 256, true),
            (GroupPolicy::Generated, 257, false),
        ] {
            let (mut session, span) = session();
            let input: Vec<_> = std::iter::repeat_n(
                InputToken::Open {
                    kind: Delimiter::Brace,
                    span,
                },
                depth,
            )
            .chain(std::iter::repeat_n(
                InputToken::Close {
                    kind: Delimiter::Brace,
                    span,
                },
                depth,
            ))
            .collect();
            assert_eq!(group(&mut session, &input, policy).is_ok(), pass);
        }
    }
    #[test]
    fn flat_payload_drops_on_ordinary_small_stack_and_returns_storage() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let (mut session, span) = session();
                let baseline = session.budget().snapshot().retained_bytes;
                let input: Vec<_> = std::iter::repeat_n(
                    InputToken::Open {
                        kind: Delimiter::Brace,
                        span,
                    },
                    512,
                )
                .chain(std::iter::repeat_n(
                    InputToken::Close {
                        kind: Delimiter::Brace,
                        span,
                    },
                    512,
                ))
                .collect();
                let tree = group(&mut session, &input, GroupPolicy::OriginalSource).unwrap();
                assert_eq!(session.budget().snapshot().temporary_bytes, 0);
                drop(tree);
                assert_eq!(session.budget().snapshot().retained_bytes, baseline);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn lowered_work_limit_is_budget_failure_not_empty_success() {
        use super::super::expansion_budget::{BudgetFailureKind, BudgetResource};
        // The real paid setup (owner issuance included), unchanged.
        let setup = |budget: ExpansionBudget| {
            let mut map = SourceMap::default();
            map.add("empty.rs".into(), Arc::from("+"));
            let sources = SourceSnapshot::new(
                map,
                crate::compiler::owner_path::standalone_issued(
                    &crate::compiler::expansion_budget::ExpansionBudget::default(),
                    crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
                )
                .unwrap(),
            )
            .unwrap();
            let source = sources.span(sources.file(0).unwrap(), 0, 1).unwrap();
            let mut session = TokenSession::new(sources, budget).unwrap();
            let span = session.original_span(source).unwrap();
            (session, span)
        };
        // Control: the exact Work the setup admits. Deterministic, since
        // owner ids and tags do not change Work.
        let control = ExpansionBudget::default();
        drop(setup(control.clone()));
        let setup_work = control.snapshot().counters[Counter::Work as usize];
        assert!(setup_work > 0, "paid setup admits Work");
        // Same policy, Work limit lowered to exactly the setup's cost: setup
        // succeeds at the limit, and group's first Work admission fails.
        let mut limits = ExpansionLimits::default();
        limits.counters[Counter::Work as usize] = setup_work;
        let budget = ExpansionBudget::new(limits);
        let (mut session, span) = setup(budget.clone());
        assert!(budget.snapshot().failure.is_none());
        assert_eq!(
            budget.snapshot().counters[Counter::Work as usize],
            setup_work
        );
        let failure = match group(&mut session, &[leaf(span)], GroupPolicy::OriginalSource) {
            Err(TokenError::Budget(failure)) => failure,
            Err(other) => panic!("lowered Work limit failed with {other:?}"),
            Ok(_) => panic!("lowered Work limit did not fail group"),
        };
        assert_eq!(failure.resource, BudgetResource::Counter(Counter::Work));
        assert_eq!(failure.kind, BudgetFailureKind::Limit);
        assert_eq!(failure.used, setup_work);
        assert_eq!(failure.limit, setup_work);
        // group's first admission is exactly one Work unit.
        assert_eq!(failure.requested, 1);
        assert_eq!(session.budget().snapshot().failure, Some(failure));
    }
}
