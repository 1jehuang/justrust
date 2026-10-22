//! Pure, bounded token-only declarative rules. This does not admit source macros,
//! load sources, resolve names, or certify any compiler/IR capability.
use std::ops::Range;

use super::expansion_budget::{BudgetFailure, Counter, Depth, Storage};
use super::expansion_span::{CrateId, ExpansionId, ExpansionRecord, SpanId};
use super::macro_cursor::{LogicalToken, MacroCursor, TokenSlice};
use super::macro_fragments::FragmentKind;
use super::token::{Delimiter, Edition, Token, TokenKind};
use super::token_session::{BudgetVec, SessionId, SymbolId, TokenError, TokenSession};
use super::token_tree::{Node, TokenStream};

#[derive(Clone, Copy, Debug)]
pub struct DefinitionSite {
    pub span: SpanId,
    pub edition: Edition,
    pub defining_crate: CrateId,
}
#[derive(Clone, Copy, Debug)]
pub struct CallSite {
    pub span: SpanId,
    pub parent: Option<ExpansionId>,
}

#[derive(Debug)]
pub enum MacroError {
    NoMatch,
    Syntax {
        span: Option<SpanId>,
        message: &'static str,
    },
    Unsupported {
        span: SpanId,
        fragment: SymbolId,
    },
    Ambiguous {
        span: Option<SpanId>,
    },
    Resource(BudgetFailure),
    Contract(TokenError),
}
impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMatch => f.write_str("no macro rule matched"),
            Self::Syntax { message, .. } => f.write_str(message),
            Self::Unsupported { .. } => f.write_str("unsupported macro fragment kind"),
            Self::Ambiguous { .. } => f.write_str("local ambiguity in macro matcher"),
            Self::Resource(error) => error.fmt(f),
            Self::Contract(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for MacroError {}
impl From<TokenError> for MacroError {
    fn from(error: TokenError) -> Self {
        match error {
            TokenError::Budget(error) => Self::Resource(error),
            other => Self::Contract(other),
        }
    }
}
impl From<BudgetFailure> for MacroError {
    fn from(error: BudgetFailure) -> Self {
        Self::Resource(error)
    }
}
pub(super) fn syntax(message: &'static str) -> MacroError {
    MacroError::Syntax {
        span: None,
        message,
    }
}
pub(super) fn work(session: &TokenSession) -> Result<(), MacroError> {
    session.budget().charge(Counter::Work, 1)?;
    Ok(())
}

/// Immutable, session-qualified compiled rules. Fields cannot be forged or edited.
pub struct RulesDef {
    owner: SessionId,
    site: DefinitionSite,
    arms: BudgetVec<Arm>,
}
impl std::fmt::Debug for RulesDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RulesDef")
            .field("owner", &self.owner)
            .field("site", &self.site)
            .field("arms", &self.arms.len())
            .finish_non_exhaustive()
    }
}
pub(super) struct Arm {
    pub matcher: Syntax,
    pub transcriber: Syntax,
    pub program: super::macro_matcher::Program,
}
/// Immutable output and its issuing expansion provenance. No semantic authority.
pub struct ExpandedTokens {
    owner: SessionId,
    tokens: TokenSlice,
    expansion: ExpansionId,
}
impl std::fmt::Debug for ExpandedTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpandedTokens")
            .field("owner", &self.owner)
            .field("expansion", &self.expansion)
            .finish_non_exhaustive()
    }
}
impl ExpandedTokens {
    pub fn tokens(&self, session: &TokenSession) -> Result<TokenSlice, MacroError> {
        session.budget().check()?;
        if self.owner != session.id() {
            return Err(TokenError::ForeignHandle.into());
        }
        self.tokens.len(session)?;
        Ok(self.tokens.clone())
    }
    pub fn expansion(&self, session: &TokenSession) -> Result<ExpansionId, MacroError> {
        self.tokens(session)?;
        session.expansion(self.expansion)?;
        Ok(self.expansion)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepeatOp {
    Star,
    Plus,
    Optional,
}
#[derive(Clone)]
pub(super) struct Atom {
    pub logical: LogicalToken,
    pub tokens: TokenSlice,
}
#[derive(Clone)]
pub(super) enum Element {
    Atom(Atom),
    Group {
        kind: Delimiter,
        open: SpanId,
        close: SpanId,
        body: Range<usize>,
    },
    Bind {
        binder: usize,
        kind: FragmentKind,
    },
    Variable {
        name: SymbolId,
        span: SpanId,
    },
    Repeat {
        body: Range<usize>,
        separator: Option<Atom>,
        op: RepeatOp,
        id: usize,
    },
}
#[derive(Clone)]
pub(super) struct Binder {
    pub name: SymbolId,
    pub depth: usize,
    pub path: Range<usize>,
}
pub(super) struct Syntax {
    pub nodes: BudgetVec<Element>,
    pub edges: BudgetVec<usize>,
    pub root: Range<usize>,
    pub binders: BudgetVec<Binder>,
    pub binder_paths: BudgetVec<usize>,
}
impl Syntax {
    fn new(session: &TokenSession) -> Result<Self, MacroError> {
        Ok(Self {
            nodes: BudgetVec::new(session.budget(), Storage::Retained)?,
            edges: BudgetVec::new(session.budget(), Storage::Retained)?,
            root: 0..0,
            binders: BudgetVec::new(session.budget(), Storage::Retained)?,
            binder_paths: BudgetVec::new(session.budget(), Storage::Retained)?,
        })
    }
    pub(super) fn sequence(&self, range: Range<usize>) -> &[usize] {
        &self.edges.as_slice()[range]
    }
}
#[derive(Clone)]
enum Wrapper {
    Root,
    Group {
        kind: Delimiter,
        open: SpanId,
        close: SpanId,
    },
    Repeat {
        separator: Option<Atom>,
        op: RepeatOp,
        id: usize,
    },
}
struct ParseFrame {
    input: TokenSlice,
    at: usize,
    start: usize,
    repetition_depth: usize,
    wrapper: Wrapper,
}
fn leaf(
    slice: &TokenSlice,
    session: &TokenSession,
    at: usize,
) -> Result<Option<Token>, MacroError> {
    if at >= slice.len(session)? {
        return Ok(None);
    }
    Ok(match slice.node(session, at)? {
        Node::Leaf(token) => Some(*token),
        _ => None,
    })
}
pub(super) fn punct(token: Option<Token>, ch: char) -> bool {
    matches!(token, Some(Token { kind: TokenKind::Punct { ch: got, .. }, .. }) if got == ch)
}
fn ident(token: Option<Token>) -> Option<SymbolId> {
    match token {
        Some(Token {
            kind: TokenKind::Ident(ident),
            ..
        }) => Some(ident.symbol),
        _ => None,
    }
}
fn read_atom(
    slice: &TokenSlice,
    session: &TokenSession,
    at: usize,
) -> Result<Option<Atom>, MacroError> {
    if at >= slice.len(session)? {
        return Ok(None);
    }
    let mut cursor = MacroCursor::new(session, slice.subslice(session, at..slice.len(session)?)?)?;
    let mark = cursor.mark(session)?;
    let Some(logical) = cursor.bump(session)? else {
        return Ok(None);
    };
    Ok(Some(Atom {
        logical,
        tokens: cursor.capture_since(session, mark)?,
    }))
}
fn repetition_tail(
    slice: &TokenSlice,
    session: &TokenSession,
    at: usize,
) -> Result<(Option<Atom>, RepeatOp, usize), MacroError> {
    let first = leaf(slice, session, at)?;
    let operator = |token: Option<Token>| match token.map(|token| token.kind) {
        Some(TokenKind::Punct { ch: '*', .. }) => Some(RepeatOp::Star),
        Some(TokenKind::Punct { ch: '+', .. }) => Some(RepeatOp::Plus),
        Some(TokenKind::Punct { ch: '?', .. }) => Some(RepeatOp::Optional),
        _ => None,
    };
    if let Some(op) = operator(first) {
        return Ok((None, op, at + 1));
    }
    let separator =
        read_atom(slice, session, at)?.ok_or_else(|| syntax("expected repetition operator"))?;
    if matches!(separator.logical, LogicalToken::Group { .. }) {
        return Err(syntax("invalid repetition separator"));
    }
    let end = at + separator.tokens.len(session)?;
    let op = operator(leaf(slice, session, end)?)
        .ok_or_else(|| syntax("expected repetition operator after separator"))?;
    if op == RepeatOp::Optional {
        return Err(syntax("optional repetition cannot have a separator"));
    }
    Ok((Some(separator), op, end + 1))
}
fn parse(input: TokenSlice, matcher: bool, session: &TokenSession) -> Result<Syntax, MacroError> {
    let mut syntax_arena = Syntax::new(session)?;
    let mut frames = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut pending = BudgetVec::new(session.budget(), Storage::Temporary)?;
    frames.push(ParseFrame {
        input,
        at: 0,
        start: 0,
        repetition_depth: 0,
        wrapper: Wrapper::Root,
    })?;
    let mut repeat_id = 0usize;
    while let Some(mut frame) = frames.pop() {
        work(session)?;
        session.budget().check_depth(
            Depth::GeneratedGroups,
            session.budget().checked_u32(frames.len() as u64)?,
        )?;
        if frame.at == frame.input.len(session)? {
            let start = syntax_arena.edges.len();
            for child in &pending.as_slice()[frame.start..] {
                work(session)?;
                syntax_arena.edges.push(*child)?;
            }
            let body = start..syntax_arena.edges.len();
            pending.truncate(frame.start);
            let node = match frame.wrapper {
                Wrapper::Root => {
                    syntax_arena.root = body;
                    break;
                }
                Wrapper::Group { kind, open, close } => Element::Group {
                    kind,
                    open,
                    close,
                    body,
                },
                Wrapper::Repeat { separator, op, id } => {
                    if matcher {
                        let mut nullable = true;
                        for index in syntax_arena.sequence(body.clone()) {
                            work(session)?;
                            nullable &= matches!(
                                syntax_arena.nodes.as_slice()[*index],
                                Element::Repeat {
                                    op: RepeatOp::Star | RepeatOp::Optional,
                                    ..
                                }
                            );
                        }
                        if nullable {
                            return Err(syntax(
                                "repetition matcher must consume at least one token",
                            ));
                        }
                    }
                    Element::Repeat {
                        body,
                        separator,
                        op,
                        id,
                    }
                }
            };
            let index = syntax_arena.nodes.len();
            syntax_arena.nodes.push(node)?;
            pending.push(index)?;
            continue;
        }
        let at = frame.at;
        let node = frame.input.node(session, at)?.clone();
        frame.at += 1;
        let mut child_frame = None;
        let element = match node {
            Node::Delimited {
                kind, open, close, ..
            } => {
                let children = frame
                    .input
                    .children(session, at)?
                    .ok_or(TokenError::InvalidRange)?;
                child_frame = Some(ParseFrame {
                    input: children,
                    at: 0,
                    start: pending.len(),
                    repetition_depth: frame.repetition_depth,
                    wrapper: Wrapper::Group { kind, open, close },
                });
                None
            }
            Node::Leaf(token) if punct(Some(token), '$') => {
                if frame.at >= frame.input.len(session)? {
                    return Err(syntax("expected metavariable after dollar"));
                }
                let next = frame.input.node(session, frame.at)?.clone();
                match next {
                    Node::Delimited {
                        kind: Delimiter::Parenthesis,
                        ..
                    } => {
                        let body = frame
                            .input
                            .children(session, frame.at)?
                            .ok_or(TokenError::InvalidRange)?;
                        let (separator, op, next) =
                            repetition_tail(&frame.input, session, frame.at + 1)?;
                        frame.at = next;
                        let id = repeat_id;
                        repeat_id = repeat_id.checked_add(1).ok_or(TokenError::IdExhausted)?;
                        child_frame = Some(ParseFrame {
                            input: body,
                            at: 0,
                            start: pending.len(),
                            repetition_depth: frame.repetition_depth + 1,
                            wrapper: Wrapper::Repeat { separator, op, id },
                        });
                        None
                    }
                    Node::Leaf(next) => {
                        let name = ident(Some(next))
                            .ok_or_else(|| syntax("expected metavariable name"))?;
                        frame.at += 1;
                        if matcher {
                            if !punct(leaf(&frame.input, session, frame.at)?, ':') {
                                return Err(syntax("expected fragment specifier"));
                            }
                            let fragment = leaf(&frame.input, session, frame.at + 1)?
                                .ok_or_else(|| syntax("expected fragment kind"))?;
                            let fragment_name = ident(Some(fragment))
                                .ok_or_else(|| syntax("expected fragment kind"))?;
                            let kind = match session.symbol(fragment_name)? {
                                "tt" => FragmentKind::Tt,
                                "ident" => FragmentKind::Ident,
                                "literal" => FragmentKind::Literal,
                                "expr" | "expr_2021" => FragmentKind::Expr,
                                "ty" => FragmentKind::Ty,
                                "pat" => FragmentKind::Pat,
                                "pat_param" => FragmentKind::PatParam,
                                "path" => FragmentKind::Path,
                                "stmt" => FragmentKind::Stmt,
                                "block" => FragmentKind::Block,
                                "item" => FragmentKind::Item,
                                "vis" => FragmentKind::Vis,
                                "lifetime" => FragmentKind::Lifetime,
                                "meta" => FragmentKind::Meta,
                                _ => {
                                    return Err(MacroError::Unsupported {
                                        span: fragment.span,
                                        fragment: fragment_name,
                                    });
                                }
                            };
                            frame.at += 2;
                            for binder in syntax_arena.binders.as_slice() {
                                work(session)?;
                                if binder.name == name {
                                    return Err(syntax("duplicate matcher binding"));
                                }
                            }
                            let binder = syntax_arena.binders.len();
                            let path_start = syntax_arena.binder_paths.len();
                            for ancestor in frames.as_slice().iter().chain(std::iter::once(&frame))
                            {
                                work(session)?;
                                if let Wrapper::Repeat { id, .. } = ancestor.wrapper {
                                    syntax_arena.binder_paths.push(id)?;
                                }
                            }
                            let path = path_start..syntax_arena.binder_paths.len();
                            syntax_arena.binders.push(Binder {
                                name,
                                depth: frame.repetition_depth,
                                path,
                            })?;
                            Some(Element::Bind { binder, kind })
                        } else {
                            Some(Element::Variable {
                                name,
                                span: token.span,
                            })
                        }
                    }
                    _ => return Err(syntax("invalid dollar construct")),
                }
            }
            Node::Leaf(_) => {
                let atom = read_atom(&frame.input, session, at)?.ok_or(TokenError::InvalidRange)?;
                frame.at = at + atom.tokens.len(session)?;
                Some(Element::Atom(atom))
            }
        };
        frames.push(frame)?;
        if let Some(child) = child_frame {
            frames.push(child)?;
        }
        if let Some(element) = element {
            let index = syntax_arena.nodes.len();
            syntax_arena.nodes.push(element)?;
            pending.push(index)?;
        }
    }
    Ok(syntax_arena)
}

/// Compile the *interior* of a macro_rules definition. All matcher schemas are
/// checked even when unused. Transcriber variable lookup is intentionally lazy.
pub fn compile_rules(
    body: TokenSlice,
    site: DefinitionSite,
    session: &mut TokenSession,
) -> Result<RulesDef, MacroError> {
    session.span(site.span)?;
    session.crate_root(site.defining_crate)?;
    let mut arms = BudgetVec::new(session.budget(), Storage::Retained)?;
    let mut at = 0;
    let length = body.len(session)?;
    while at < length {
        work(session)?;
        let matcher_body = body
            .children(session, at)?
            .ok_or_else(|| syntax("expected delimited matcher"))?;
        at += 1;
        let arrow =
            read_atom(&body, session, at)?.ok_or_else(|| syntax("expected => after matcher"))?;
        if !matches!(
            arrow.logical,
            LogicalToken::Operator {
                chars: ['=', '>', '\0'],
                len: 2,
                ..
            }
        ) {
            return Err(syntax("expected => after matcher"));
        }
        at += arrow.tokens.len(session)?;
        if at == length {
            return Err(syntax("expected delimited transcriber"));
        }
        let transcriber_body = body
            .children(session, at)?
            .ok_or_else(|| syntax("expected delimited transcriber"))?;
        at += 1;
        let matcher = parse(matcher_body, true, session)?;
        let transcriber = parse(transcriber_body, false, session)?;
        let program = super::macro_matcher::compile(&matcher, session)?;
        arms.push(Arm {
            matcher,
            transcriber,
            program,
        })?;
        if at < length {
            if !punct(leaf(&body, session, at)?, ';') {
                return Err(syntax("expected semicolon between macro rules"));
            }
            at += 1;
        }
    }
    Ok(RulesDef {
        owner: session.id(),
        site,
        arms,
    })
}

/// Arguments are the OUTERGROUP INTERIOR, not the invocation delimiter itself.
/// Only genuine NoMatch advances to a later arm. First full success commits.
pub fn expand_rules(
    definition: &RulesDef,
    args: TokenSlice,
    call: CallSite,
    session: &mut TokenSession,
) -> Result<ExpandedTokens, MacroError> {
    session.budget().check()?;
    if definition.owner != session.id() {
        return Err(TokenError::ForeignHandle.into());
    }
    args.len(session)?;
    session.span(call.span)?;
    if let Some(parent) = call.parent {
        session.expansion(parent)?;
    }
    let invocation = session.budget().begin_invocation()?;
    for arm in definition.arms.as_slice() {
        work(session)?;
        let captures = match super::macro_matcher::run(
            &arm.program,
            &arm.matcher,
            args.clone(),
            &invocation,
            session,
        ) {
            Ok(captures) => captures,
            Err(MacroError::NoMatch) => continue,
            Err(error) => return Err(error),
        };
        let expansion = session.add_expansion(ExpansionRecord {
            definition: definition.site.span,
            call: call.span,
            parent: call.parent,
            defining_crate: definition.site.defining_crate,
            edition: definition.site.edition,
        })?;
        let stream: TokenStream = super::macro_transcribe::transcribe(
            &arm.transcriber,
            &arm.matcher,
            &captures,
            definition.site,
            expansion,
            &invocation,
            session,
        )?;
        let tokens = TokenSlice::roots(session, stream)?;
        return Ok(ExpandedTokens {
            owner: session.id(),
            tokens,
            expansion,
        });
    }
    Err(MacroError::NoMatch)
}

#[cfg(test)]
mod tests {
    use super::super::expansion_budget::{ExpansionBudget, ExpansionLimits};
    use super::super::lexer::lex_prepared;
    use super::super::source::PreparedLexInput;
    use super::super::token_tree::{GroupPolicy, group};
    use super::*;
    use std::path::Path;

    struct Fixture {
        session: TokenSession,
        roots: TokenSlice,
        site: DefinitionSite,
    }
    impl Fixture {
        fn new(text: &str) -> Self {
            Self::limited(text, ExpansionLimits::default())
        }
        fn limited(text: &str, limits: ExpansionLimits) -> Self {
            let budget = ExpansionBudget::new(limits);
            let input = PreparedLexInput::identity(Path::new("rules.rs"), text, &budget).unwrap();
            let mut session = TokenSession::new(input.snapshot().clone(), budget).unwrap();
            let tokens = lex_prepared(&input, Edition::E2024, &mut session).unwrap();
            let mut physical = BudgetVec::new(session.budget(), Storage::Temporary).unwrap();
            for event in tokens.events() {
                physical.push(event.token).unwrap();
            }
            let stream = group(
                &mut session,
                physical.as_slice(),
                GroupPolicy::OriginalSource,
            )
            .unwrap();
            let roots = TokenSlice::roots(&session, stream).unwrap();
            let defining_crate = session
                .register_crate_root(input.snapshot().file(0).unwrap())
                .unwrap();
            let site = DefinitionSite {
                span: tokens.eof().span,
                edition: Edition::E2024,
                defining_crate,
            };
            Self {
                session,
                roots,
                site,
            }
        }
        fn part(&self, index: usize) -> TokenSlice {
            self.roots.children(&self.session, index).unwrap().unwrap()
        }
        fn definition(&mut self, index: usize) -> Result<RulesDef, MacroError> {
            compile_rules(self.part(index), self.site, &mut self.session)
        }
        fn expand(
            &mut self,
            definition: &RulesDef,
            args: TokenSlice,
        ) -> Result<ExpandedTokens, MacroError> {
            expand_rules(
                definition,
                args,
                CallSite {
                    span: self.site.span,
                    parent: None,
                },
                &mut self.session,
            )
        }
        fn first_symbol(&self, output: &ExpandedTokens) -> String {
            let tokens = output.tokens(&self.session).unwrap();
            let Node::Leaf(Token {
                kind: TokenKind::Ident(ident),
                ..
            }) = tokens.node(&self.session, 0).unwrap()
            else {
                panic!("expected identifier output")
            };
            self.session.symbol(ident.symbol).unwrap().to_owned()
        }
    }

    #[test]
    fn ordered_fallback_and_committed_transcription() {
        let mut f =
            Fixture::new("{ (fixed) => { $missing }; ($x:tt) => { fallback } } {other} {fixed}");
        let definition = f.definition(0).unwrap();
        let output = f.expand(&definition, f.part(1)).unwrap();
        assert_eq!(f.first_symbol(&output), "fallback");
        assert!(matches!(
            f.expand(&definition, f.part(2)),
            Err(MacroError::Syntax {
                message: "unknown transcriber metavariable",
                ..
            })
        ));
    }

    #[test]
    fn literal_minus_commits_but_word_and_plus_can_fallback() {
        let mut f = Fixture::new(
            "{ ($x:literal) => { literal }; ($($x:tt)*) => { fallback } } {-word} {word} {+1} {-true} {-\"x\"} {->} {-=} {--1} {- >} {- =}",
        );
        let definition = f.definition(0).unwrap();
        for index in [1, 8, 9, 10] {
            assert!(matches!(
                f.expand(&definition, f.part(index)),
                Err(MacroError::Syntax { .. })
            ));
        }
        for index in [2, 3, 6, 7] {
            let output = f.expand(&definition, f.part(index)).unwrap();
            assert_eq!(f.first_symbol(&output), "fallback");
        }
        for index in [4, 5] {
            let output = f.expand(&definition, f.part(index)).unwrap();
            assert_eq!(f.first_symbol(&output), "literal");
        }
    }

    #[test]
    fn literal_opaque_and_tt_transparent_forwarding() {
        let mut f = Fixture::new(
            "{ ($x:literal) => {$x} } { ($x:tt) => {$x} } { (42) => { concrete }; ($x:literal) => { opaque } } {42}",
        );
        let literal = f.definition(0).unwrap();
        let tt = f.definition(1).unwrap();
        let sink = f.definition(2).unwrap();
        let output = f.expand(&literal, f.part(3)).unwrap();
        let opaque = output.tokens(&f.session).unwrap();
        let Node::Leaf(Token {
            kind: TokenKind::Opaque(original),
            ..
        }) = opaque.node(&f.session, 0).unwrap()
        else {
            panic!("expected opaque literal")
        };
        let original = *original;
        let forwarded = f.expand(&tt, opaque).unwrap().tokens(&f.session).unwrap();
        assert!(
            matches!(forwarded.node(&f.session, 0).unwrap(), Node::Leaf(Token { kind: TokenKind::Opaque(id), .. }) if *id == original)
        );
        let output = f.expand(&sink, forwarded).unwrap();
        assert_eq!(f.first_symbol(&output), "opaque");
        let output = f.expand(&tt, f.part(3)).unwrap();
        let output = f.expand(&sink, output.tokens(&f.session).unwrap()).unwrap();
        assert_eq!(f.first_symbol(&output), "concrete");
    }

    #[test]
    fn unused_matcher_validation_and_lazy_transcriber_names() {
        for source in [
            "{ ($x:tt $x:ident) => {} }",
            "{ ($($($x:tt)*)*) => {} }",
            "{ ($($x:tt),?) => {} }",
        ] {
            let mut f = Fixture::new(source);
            assert!(matches!(f.definition(0), Err(MacroError::Syntax { .. })));
        }
        let mut f = Fixture::new("{ ($x:unknown_fragment) => {}; () => {} }");
        assert!(matches!(
            f.definition(0),
            Err(MacroError::Unsupported { .. })
        ));
        let mut f = Fixture::new("{ ($x:expr) => {$x}; () => {empty} } {1+2} {}");
        let definition = f.definition(0).unwrap();
        let output = f.expand(&definition, f.part(1)).unwrap();
        // Expression matchers are supported now. Verify arm selection here;
        // forwarding opacity is a separate fragment-representation contract.
        assert!(
            !output
                .tokens(&f.session)
                .unwrap()
                .is_empty(&f.session)
                .unwrap()
        );
        let output = f.expand(&definition, f.part(2)).unwrap();
        assert_eq!(f.first_symbol(&output), "empty");
        let mut f = Fixture::new("{ () => {$missing} } {}");
        let definition = f.definition(0).unwrap();
        assert!(matches!(
            f.expand(&definition, f.part(1)),
            Err(MacroError::Syntax {
                message: "unknown transcriber metavariable",
                ..
            })
        ));
    }

    #[test]
    fn nested_zero_repetitions_and_separator_cardinality() {
        let mut f =
            Fixture::new("{ ($( [$($x:ident),*] );*) => { $( ( $($x),* ) );* } } {[];[a,b];[]}");
        let definition = f.definition(0).unwrap();
        let output = f
            .expand(&definition, f.part(1))
            .unwrap()
            .tokens(&f.session)
            .unwrap();
        assert_eq!(output.len(&f.session).unwrap(), 5);
        assert_eq!(
            output
                .children(&f.session, 0)
                .unwrap()
                .unwrap()
                .len(&f.session)
                .unwrap(),
            0
        );
        assert_eq!(
            output
                .children(&f.session, 2)
                .unwrap()
                .unwrap()
                .len(&f.session)
                .unwrap(),
            3
        );
        assert_eq!(
            output
                .children(&f.session, 4)
                .unwrap()
                .unwrap()
                .len(&f.session)
                .unwrap(),
            0
        );
        let mut f = Fixture::new(
            "{ ($($x:ident),*; $($y:ident),*) => { $( $x $y )* }; ($($t:tt)*) => { fallback } } {a,b;c}",
        );
        let definition = f.definition(0).unwrap();
        assert!(matches!(
            f.expand(&definition, f.part(1)),
            Err(MacroError::Syntax {
                message: "inconsistent repetition cardinalities",
                ..
            })
        ));
    }

    #[test]
    fn logical_tt_and_raw_identifier_controls() {
        let mut f =
            Fixture::new("{ ($x:tt) => {one}; ($($x:tt)*) => {many} } {>>=} {> > =} {'a} {++}");
        let definition = f.definition(0).unwrap();
        for (index, expected) in [(1, "one"), (2, "many"), (3, "one"), (4, "many")] {
            let output = f.expand(&definition, f.part(index)).unwrap();
            assert_eq!(f.first_symbol(&output), expected);
        }
        let mut f = Fixture::new(
            "{ (foo) => {fixed}; ($x:ident) => {ident}; ($x:tt) => {tt} } {r#foo} {_}",
        );
        let definition = f.definition(0).unwrap();
        let output = f.expand(&definition, f.part(1)).unwrap();
        assert_eq!(f.first_symbol(&output), "ident");
        let output = f.expand(&definition, f.part(2)).unwrap();
        assert_eq!(f.first_symbol(&output), "tt");
    }

    #[test]
    fn local_ambiguity_cannot_fallback() {
        let mut f =
            Fixture::new("{ ($($x:ident)* $y:ident) => {}; ($($x:tt)*) => {fallback} } {a}");
        let definition = f.definition(0).unwrap();
        assert!(matches!(
            f.expand(&definition, f.part(1)),
            Err(MacroError::Ambiguous { .. })
        ));
    }

    #[test]
    fn missing_transcriber_and_spaced_arrow_are_syntax_not_contract() {
        for text in [
            "{ () => }",
            "{ () = > {} }",
            "{ ($x:tt) => }",
            "{ ($) => {} }",
        ] {
            let mut f = Fixture::new(text);
            assert!(matches!(f.definition(0), Err(MacroError::Syntax { .. })));
        }
    }

    #[test]
    fn resource_poison_cannot_become_an_ordered_arm_mismatch() {
        let limits = ExpansionLimits {
            matcher_states: 0,
            ..ExpansionLimits::default()
        };
        let mut f = Fixture::limited("{ (no) => {}; ($($x:tt)*) => {fallback} } {yes}", limits);
        let definition = f.definition(0).unwrap();
        assert!(matches!(
            f.expand(&definition, f.part(1)),
            Err(MacroError::Resource(_))
        ));
        assert!(f.session.budget().snapshot().failure.is_some());
    }

    #[test]
    fn dollar_separator_and_optional_plus_cardinality() {
        let mut f = Fixture::new("{ ($($x:ident)$*) => { $($x)$* } } {a$b$c}");
        let definition = f.definition(0).unwrap();
        let output = f.expand(&definition, f.part(1)).unwrap();
        assert_eq!(
            output.tokens(&f.session).unwrap().len(&f.session).unwrap(),
            5
        );
        for text in [
            "{ ($($x:ident)*) => { $($x)+ } } {}",
            "{ ($($x:ident)*) => { $($x)? } } {a b}",
            "{ ($($x:ident)*) => { $x } } {a}",
            "{ () => { $(fixed)* } } {}",
        ] {
            let mut f = Fixture::new(text);
            let definition = f.definition(0).unwrap();
            assert!(matches!(
                f.expand(&definition, f.part(1)),
                Err(MacroError::Syntax { .. })
            ));
        }
    }

    #[test]
    fn opaque_payload_occurrences_exhaust_shared_invocation_output() {
        let limits = ExpansionLimits {
            invocation_output: 3,
            ..ExpansionLimits::default()
        };
        let mut f = Fixture::limited(
            "{ ($x:literal) => {$x $x}; ($($x:tt)*) => {fallback} } {-1}",
            limits,
        );
        let definition = f.definition(0).unwrap();
        assert!(matches!(
            f.expand(&definition, f.part(1)),
            Err(MacroError::Resource(_))
        ));
        assert!(f.session.budget().snapshot().failure.is_some());
    }

    #[test]
    fn owner_and_occurrence_hygiene_are_checked() {
        let mut f = Fixture::new("{ ($x:ident) => { introduced $x $x } } {captured}");
        let definition = f.definition(0).unwrap();
        let argument = f.part(1);
        let Node::Leaf(original) = argument.node(&f.session, 0).unwrap() else {
            panic!()
        };
        let original = *original;
        let output = f.expand(&definition, argument).unwrap();
        let tokens = output.tokens(&f.session).unwrap();
        let mut seen = Vec::new();
        for index in 0..3 {
            let Node::Leaf(token) = tokens.node(&f.session, index).unwrap() else {
                panic!()
            };
            assert_eq!(
                f.session.span(token.span).unwrap().expansion,
                Some(output.expansion(&f.session).unwrap())
            );
            seen.push(*token);
        }
        assert_ne!(seen[1].span, seen[2].span);
        assert_ne!(seen[1].span, original.span);
        let TokenKind::Ident(captured) = original.kind else {
            panic!()
        };
        assert!(
            matches!(seen[1].kind, TokenKind::Ident(ident) if ident.context == captured.context)
        );
        assert!(
            matches!(seen[0].kind, TokenKind::Ident(ident) if ident.context != captured.context)
        );
        let mut other = Fixture::new("{} {}");
        assert!(matches!(
            other.expand(&definition, other.part(1)),
            Err(MacroError::Contract(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            output.tokens(&other.session),
            Err(MacroError::Contract(TokenError::ForeignHandle))
        ));
    }
}
