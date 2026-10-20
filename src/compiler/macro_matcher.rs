//! Flat Thompson code and bounded lockstep matching. No backtracking or parser calls.
use std::cell::Cell;
use std::ops::Range;

use super::expansion_budget::{Counter, InvocationBudget, StateLease, Storage};
use super::expansion_span::SpanId;
use super::macro_cursor::{LogicalToken, MacroCursor, TokenSlice};
use super::macro_fragments::{FragmentId, FragmentKind, LiteralCapture};
use super::macro_rules::{Element, MacroError, RepeatOp, Syntax, syntax, work};
use super::token::{Delimiter, Token, TokenKind};
use super::token_session::{BudgetVec, TokenError, TokenSession};

const NONE: usize = usize::MAX;
#[derive(Clone)]
enum Op {
    Atom(LogicalToken),
    Open(Delimiter),
    Close(Delimiter),
    Capture { binder: usize, kind: FragmentKind },
    Enter(usize),
    Next(usize),
    Exit(usize),
    Split(usize),
    Accept,
}
struct PatchInstruction {
    op: Op,
    next: Cell<usize>,
}
struct Instruction {
    op: Op,
    next: usize,
}
pub(super) struct Program {
    code: BudgetVec<Instruction>,
    start: usize,
}
fn instruction(
    code: &mut BudgetVec<PatchInstruction>,
    op: Op,
    next: usize,
) -> Result<usize, MacroError> {
    let index = code.len();
    code.push(PatchInstruction {
        op,
        next: Cell::new(next),
    })?;
    Ok(index)
}
fn link_sequence(
    arena: &Syntax,
    range: Range<usize>,
    parts: &[(usize, usize)],
    code: &[PatchInstruction],
    session: &TokenSession,
) -> Result<Option<(usize, usize)>, MacroError> {
    let mut first = None;
    let mut last: Option<usize> = None;
    for node in arena.sequence(range) {
        work(session)?;
        let (head, tail) = parts[*node];
        if let Some(previous) = last {
            code[previous].next.set(head);
        }
        first.get_or_insert(head);
        last = Some(tail);
    }
    Ok(first.zip(last))
}
pub(super) fn compile(arena: &Syntax, session: &TokenSession) -> Result<Program, MacroError> {
    let mut code = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut parts = BudgetVec::new(session.budget(), Storage::Temporary)?;
    // Syntax nodes are postorder. Each child sequence belongs to exactly one parent.
    for element in arena.nodes.as_slice() {
        work(session)?;
        let pair = match element {
            Element::Atom(atom) => {
                let pc = instruction(&mut code, Op::Atom(atom.logical.clone()), NONE)?;
                (pc, pc)
            }
            Element::Bind { binder, kind } => {
                let pc = instruction(
                    &mut code,
                    Op::Capture {
                        binder: *binder,
                        kind: *kind,
                    },
                    NONE,
                )?;
                (pc, pc)
            }
            Element::Group { kind, body, .. } => {
                let close = instruction(&mut code, Op::Close(*kind), NONE)?;
                let body = link_sequence(
                    arena,
                    body.clone(),
                    parts.as_slice(),
                    code.as_slice(),
                    session,
                )?;
                let first = if let Some((head, tail)) = body {
                    code.as_slice()[tail].next.set(close);
                    head
                } else {
                    close
                };
                let open = instruction(&mut code, Op::Open(*kind), first)?;
                (open, close)
            }
            Element::Repeat {
                body,
                separator,
                op,
                id,
            } => {
                let (head, tail) = link_sequence(
                    arena,
                    body.clone(),
                    parts.as_slice(),
                    code.as_slice(),
                    session,
                )?
                .ok_or_else(|| syntax("empty matcher repetition"))?;
                let exit = instruction(&mut code, Op::Exit(*id), NONE)?;
                let next = instruction(&mut code, Op::Next(*id), exit)?;
                code.as_slice()[tail].next.set(next);
                if *op != RepeatOp::Optional {
                    let again = if let Some(separator) = separator {
                        instruction(&mut code, Op::Atom(separator.logical.clone()), head)?
                    } else {
                        head
                    };
                    let split = instruction(&mut code, Op::Split(exit), again)?;
                    code.as_slice()[next].next.set(split);
                }
                let initial = if *op == RepeatOp::Plus {
                    head
                } else {
                    instruction(&mut code, Op::Split(exit), head)?
                };
                let enter = instruction(&mut code, Op::Enter(*id), initial)?;
                (enter, exit)
            }
            Element::Variable { .. } => return Err(syntax("transcriber variable in matcher code")),
        };
        parts.push(pair)?;
    }
    let accept = instruction(&mut code, Op::Accept, NONE)?;
    let root = link_sequence(
        arena,
        arena.root.clone(),
        parts.as_slice(),
        code.as_slice(),
        session,
    )?;
    let start = if let Some((head, tail)) = root {
        code.as_slice()[tail].next.set(accept);
        head
    } else {
        accept
    };
    let mut frozen = BudgetVec::new(session.budget(), Storage::Retained)?;
    for instruction in code.as_slice() {
        work(session)?;
        frozen.push(Instruction {
            op: instruction.op.clone(),
            next: instruction.next.get(),
        })?;
    }
    Ok(Program {
        code: frozen,
        start,
    })
}

#[derive(Clone)]
enum EventKind {
    Atom(LogicalToken),
    Open(Delimiter),
    Close(Delimiter),
}
struct Event {
    kind: EventKind,
    tokens: TokenSlice,
    container: TokenSlice,
    physical: Range<usize>,
    end: Cell<usize>,
}
struct InputFrame {
    cursor: MacroCursor,
    input: TokenSlice,
    physical: usize,
}
enum InputTask {
    Frame(InputFrame),
    Close {
        open: usize,
        kind: Delimiter,
        tokens: TokenSlice,
        container: TokenSlice,
        physical: Range<usize>,
    },
}
fn flatten(input: TokenSlice, session: &TokenSession) -> Result<BudgetVec<Event>, MacroError> {
    let mut events: BudgetVec<Event> = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut tasks = BudgetVec::new(session.budget(), Storage::Temporary)?;
    tasks.push(InputTask::Frame(InputFrame {
        cursor: MacroCursor::new(session, input.clone())?,
        input,
        physical: 0,
    }))?;
    while let Some(task) = tasks.pop() {
        work(session)?;
        match task {
            InputTask::Close {
                open,
                kind,
                tokens,
                container,
                physical,
            } => {
                let end = events.len();
                events.as_slice()[open].end.set(end);
                events.push(Event {
                    kind: EventKind::Close(kind),
                    tokens,
                    container,
                    physical,
                    end: Cell::new(end),
                })?;
            }
            InputTask::Frame(mut frame) => {
                let mark = frame.cursor.mark(session)?;
                let Some(logical) = frame.cursor.bump(session)? else {
                    continue;
                };
                let tokens = frame.cursor.capture_since(session, mark)?;
                let start = frame.physical;
                frame.physical += tokens.len(session)?;
                let physical = start..frame.physical;
                let container = frame.input.clone();
                let index = events.len();
                match logical {
                    LogicalToken::Group { kind, children, .. } => {
                        events.push(Event {
                            kind: EventKind::Open(kind),
                            tokens: tokens.clone(),
                            container: container.clone(),
                            physical: physical.clone(),
                            end: Cell::new(NONE),
                        })?;
                        tasks.push(InputTask::Frame(frame))?;
                        tasks.push(InputTask::Close {
                            open: index,
                            kind,
                            tokens,
                            container,
                            physical,
                        })?;
                        tasks.push(InputTask::Frame(InputFrame {
                            cursor: MacroCursor::new(session, children.clone())?,
                            input: children,
                            physical: 0,
                        }))?;
                    }
                    other => {
                        events.push(Event {
                            kind: EventKind::Atom(other),
                            tokens,
                            container,
                            physical,
                            end: Cell::new(index),
                        })?;
                        tasks.push(InputTask::Frame(frame))?;
                    }
                }
            }
        }
    }
    Ok(events)
}
fn token_equal(left: TokenKind, right: TokenKind) -> bool {
    match (left, right) {
        (TokenKind::Ident(a), TokenKind::Ident(b)) => a.symbol == b.symbol && a.raw == b.raw,
        (
            TokenKind::Literal {
                kind: a,
                spelling: x,
            },
            TokenKind::Literal {
                kind: b,
                spelling: y,
            },
        ) => a == b && x == y,
        (TokenKind::Punct { ch: a, .. }, TokenKind::Punct { ch: b, .. }) => a == b,
        (TokenKind::DollarCrate(a), TokenKind::DollarCrate(b)) => a == b,
        // A literal-token matcher must NEVER inspect an opaque fragment payload.
        _ => false,
    }
}
fn atom_equal(
    left: &LogicalToken,
    right: &LogicalToken,
    session: &TokenSession,
) -> Result<bool, MacroError> {
    work(session)?;
    Ok(match (left, right) {
        (LogicalToken::Leaf(a), LogicalToken::Leaf(b)) => token_equal(a.kind, b.kind),
        (
            LogicalToken::Operator {
                chars: a, len: x, ..
            },
            LogicalToken::Operator {
                chars: b, len: y, ..
            },
        ) => a == b && x == y,
        (LogicalToken::Lifetime { ident: a, .. }, LogicalToken::Lifetime { ident: b, .. }) => {
            a.symbol == b.symbol && a.raw == b.raw
        }
        _ => false,
    })
}
#[derive(Clone)]
pub(super) enum CaptureValue {
    Tokens(TokenSlice),
    /// A multi-tree `expr` capture: transcribed inside invisible-group
    /// parentheses so operator precedence is preserved (`$e * $e`).
    Grouped(TokenSlice),
    Literal {
        id: FragmentId,
        span: SpanId,
    },
}
#[derive(Clone)]
enum HistoryEvent {
    Enter(usize),
    Next(usize),
    Exit(usize),
    Capture { binder: usize, value: CaptureValue },
}
struct History {
    parent: Option<usize>,
    event: HistoryEvent,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PathComponent {
    pub repetition: usize,
    pub iteration: usize,
}
pub(super) struct Capture {
    pub binder: usize,
    pub path: Range<usize>,
    pub value: CaptureValue,
}
pub(super) struct Repetition {
    pub id: usize,
    pub parent: Range<usize>,
    pub count: usize,
}
pub(super) struct Captures {
    pub values: BudgetVec<Capture>,
    pub paths: BudgetVec<PathComponent>,
    pub repetitions: BudgetVec<Repetition>,
}
fn record(
    history: &mut BudgetVec<History>,
    parent: Option<usize>,
    event: HistoryEvent,
    session: &TokenSession,
) -> Result<Option<usize>, MacroError> {
    work(session)?;
    session.budget().charge(Counter::CaptureElements, 1)?;
    let index = history.len();
    history.push(History { parent, event })?;
    Ok(Some(index))
}
fn copy_path(
    paths: &mut BudgetVec<PathComponent>,
    stack: &[PathComponent],
    session: &TokenSession,
) -> Result<Range<usize>, MacroError> {
    let start = paths.len();
    for component in stack {
        work(session)?;
        paths.push(*component)?;
    }
    Ok(start..paths.len())
}
fn materialize(
    history: &[History],
    mut tip: Option<usize>,
    session: &TokenSession,
) -> Result<Captures, MacroError> {
    let mut order = BudgetVec::new(session.budget(), Storage::Temporary)?;
    while let Some(index) = tip {
        work(session)?;
        order.push(index)?;
        tip = history[index].parent;
    }
    let mut stack: BudgetVec<PathComponent> = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut result = Captures {
        values: BudgetVec::new(session.budget(), Storage::Temporary)?,
        paths: BudgetVec::new(session.budget(), Storage::Temporary)?,
        repetitions: BudgetVec::new(session.budget(), Storage::Temporary)?,
    };
    while let Some(index) = order.pop() {
        work(session)?;
        match &history[index].event {
            HistoryEvent::Enter(id) => stack.push(PathComponent {
                repetition: *id,
                iteration: 0,
            })?,
            HistoryEvent::Next(id) => {
                let mut top = stack.pop().ok_or(TokenError::InvalidHandle)?;
                if top.repetition != *id {
                    return Err(TokenError::InvalidHandle.into());
                }
                top.iteration = top
                    .iteration
                    .checked_add(1)
                    .ok_or(TokenError::IdExhausted)?;
                stack.push(top)?;
            }
            HistoryEvent::Exit(id) => {
                let top = stack.pop().ok_or(TokenError::InvalidHandle)?;
                if top.repetition != *id {
                    return Err(TokenError::InvalidHandle.into());
                }
                let parent = copy_path(&mut result.paths, stack.as_slice(), session)?;
                result.repetitions.push(Repetition {
                    id: *id,
                    parent,
                    count: top.iteration,
                })?;
            }
            HistoryEvent::Capture { binder, value } => {
                let path = copy_path(&mut result.paths, stack.as_slice(), session)?;
                result.values.push(Capture {
                    binder: *binder,
                    path,
                    value: value.clone(),
                })?;
            }
        }
    }
    if stack.len() != 0 {
        return Err(TokenError::InvalidHandle.into());
    }
    Ok(result)
}
struct State {
    pc: usize,
    position: usize,
    history: Option<usize>,
    _lease: StateLease,
}
fn state(
    pc: usize,
    position: usize,
    history: Option<usize>,
    budget: &InvocationBudget,
) -> Result<State, MacroError> {
    Ok(State {
        pc,
        position,
        history,
        _lease: budget.reserve_states(1, 0)?,
    })
}
struct Seen {
    pc: usize,
    history: Option<usize>,
    _lease: StateLease,
}
fn probe(
    kind: FragmentKind,
    event: &Event,
    events: &[Event],
    position: usize,
    session: &mut TokenSession,
) -> Result<Option<(usize, CaptureValue)>, MacroError> {
    work(session)?;
    if matches!(event.kind, EventKind::Close(_)) {
        return Ok(None);
    }
    match kind {
        FragmentKind::Expr
        | FragmentKind::Ty
        | FragmentKind::Pat
        | FragmentKind::PatParam
        | FragmentKind::Path
        | FragmentKind::Stmt
        | FragmentKind::Block
        | FragmentKind::Item
        | FragmentKind::Vis
        | FragmentKind::Lifetime
        | FragmentKind::Meta => syntactic(kind, events, position, session),
        FragmentKind::Tt => Ok(Some((
            event.end.get() + 1,
            CaptureValue::Tokens(event.tokens.clone()),
        ))),
        FragmentKind::Ident => {
            let accepted = match &event.kind {
                EventKind::Atom(LogicalToken::Leaf(Token {
                    kind: TokenKind::Ident(ident),
                    ..
                })) => session.symbol(ident.symbol)? != "_",
                EventKind::Atom(LogicalToken::Leaf(Token {
                    kind: TokenKind::DollarCrate(_),
                    ..
                })) => true,
                _ => false,
            };
            Ok(accepted.then(|| (position + 1, CaptureValue::Tokens(event.tokens.clone()))))
        }
        FragmentKind::Literal => {
            if !matches!(event.kind, EventKind::Atom(LogicalToken::Leaf(_))) {
                return Ok(None);
            }
            let minus = matches!(
                event.kind,
                EventKind::Atom(LogicalToken::Leaf(Token {
                    kind: TokenKind::Punct { ch: '-', .. },
                    ..
                }))
            );
            let (tokens, end) = if minus {
                let end = if let Some(next) = events.get(position + 1) {
                    if matches!(next.kind, EventKind::Atom(_)) {
                        next.physical.end
                    } else {
                        event.physical.end
                    }
                } else {
                    event.physical.end
                };
                (
                    event
                        .container
                        .subslice(session, event.physical.start..end)?,
                    position + if end == event.physical.end { 1 } else { 2 },
                )
            } else {
                (event.tokens.clone(), position + 1)
            };
            match session.capture_literal(tokens)? {
                LiteralCapture::NoMatch => Ok(None),
                LiteralCapture::CommittedSyntax { primary } => Err(MacroError::Syntax {
                    span: Some(primary),
                    message: "expected literal after minus",
                }),
                LiteralCapture::Matched(id) => {
                    let span = match &event.kind {
                        EventKind::Atom(LogicalToken::Leaf(token)) => token.span,
                        _ => return Err(TokenError::InvalidHandle.into()),
                    };
                    Ok(Some((end, CaptureValue::Literal { id, span })))
                }
            }
        }
    }
}

fn punct_of(event: &Event) -> Option<([char; 3], u8)> {
    match &event.kind {
        EventKind::Atom(LogicalToken::Leaf(Token {
            kind: TokenKind::Punct { ch, .. },
            ..
        })) => Some(([*ch, '\0', '\0'], 1)),
        EventKind::Atom(LogicalToken::Operator { chars, len, .. }) => Some((*chars, *len)),
        _ => None,
    }
}
fn ident_of<'a>(event: &Event, session: &'a TokenSession) -> Result<Option<&'a str>, MacroError> {
    Ok(match &event.kind {
        EventKind::Atom(LogicalToken::Leaf(Token {
            kind: TokenKind::Ident(ident),
            ..
        })) if !ident.raw => Some(session.symbol(ident.symbol)?),
        _ => None,
    })
}
/// Token-tree level recognizer for the syntactic fragment kinds. It consumes
/// the longest run of top-level token trees that cannot belong to the
/// fragment's FOLLOW set (Rust Reference, "Follow-set ambiguity
/// restrictions"), so the remaining matcher sees exactly what rustc's parser
/// would leave. The captured tokens are parsed by the real grammar after
/// transcription; malformed fragments are rejected there.
fn syntactic(
    kind: FragmentKind,
    events: &[Event],
    position: usize,
    session: &mut TokenSession,
) -> Result<Option<(usize, CaptureValue)>, MacroError> {
    let Some(first) = events.get(position) else {
        return Ok(None);
    };
    if matches!(first.kind, EventKind::Close(_)) {
        return Ok(None);
    }
    let next_tree = |at: usize| -> usize {
        match events[at].kind {
            EventKind::Open(_) => events[at].end.get() + 1,
            _ => at + 1,
        }
    };
    let mut end = position;
    let mut trees = 0usize;
    match kind {
        FragmentKind::Lifetime => {
            if !matches!(first.kind, EventKind::Atom(LogicalToken::Lifetime { .. })) {
                return Ok(None);
            }
            end = position + 1;
            trees = 1;
        }
        FragmentKind::Block => {
            if !matches!(first.kind, EventKind::Open(Delimiter::Brace)) {
                return Ok(None);
            }
            end = next_tree(position);
            trees = 1;
        }
        FragmentKind::Vis => {
            if ident_of(first, session)? == Some("pub") {
                end = position + 1;
                trees = 1;
                if let Some(next) = events.get(end)
                    && matches!(next.kind, EventKind::Open(Delimiter::Parenthesis))
                {
                    end = next_tree(end);
                    trees = 2;
                }
            }
        }
        FragmentKind::Item => {
            while let Some(event) = events.get(end) {
                work(session)?;
                if matches!(event.kind, EventKind::Close(_)) {
                    break;
                }
                let brace = matches!(event.kind, EventKind::Open(Delimiter::Brace));
                let semi = punct_of(event) == Some([';', '\0', '\0']).map(|c| (c, 1));
                end = next_tree(end);
                trees += 1;
                if brace || semi {
                    break;
                }
            }
        }
        _ => {
            // expr, stmt, ty, path, pat, pat_param, meta
            let mut angle = 0i64;
            let typeish = matches!(kind, FragmentKind::Ty | FragmentKind::Path);
            while let Some(event) = events.get(end) {
                work(session)?;
                if matches!(event.kind, EventKind::Close(_)) {
                    break;
                }
                if let Some((chars, len)) = punct_of(event) {
                    let op = &chars[..len as usize];
                    let stop = match op {
                        [','] | [';'] | ['=', '>'] => angle == 0,
                        ['='] => {
                            matches!(
                                kind,
                                FragmentKind::Ty
                                    | FragmentKind::Path
                                    | FragmentKind::Pat
                                    | FragmentKind::PatParam
                            ) && angle == 0
                        }
                        ['|'] => {
                            matches!(
                                kind,
                                FragmentKind::PatParam | FragmentKind::Ty | FragmentKind::Path
                            ) && angle == 0
                        }
                        [':'] => typeish && angle == 0 && trees > 0,
                        ['>'] | ['>', '>'] | ['>', '='] | ['>', '>', '='] if typeish => {
                            let closes = op.iter().filter(|c| **c == '>').count() as i64;
                            if angle < closes {
                                true
                            } else {
                                angle -= closes;
                                false
                            }
                        }
                        _ => false,
                    };
                    if stop {
                        break;
                    }
                    if typeish && op.first() == Some(&'<') {
                        angle += op.iter().filter(|c| **c == '<').count() as i64;
                    }
                    if typeish && op == ['-', '>'] && trees == 0 {
                        break;
                    }
                }
                if let Some(word) = ident_of(event, session)? {
                    let stop = match kind {
                        FragmentKind::Ty | FragmentKind::Path => {
                            trees > 0 && angle == 0 && matches!(word, "as" | "where")
                        }
                        FragmentKind::Pat | FragmentKind::PatParam => {
                            trees > 0 && matches!(word, "if" | "in")
                        }
                        _ => false,
                    };
                    if stop {
                        break;
                    }
                }
                if typeish
                    && trees > 0
                    && angle == 0
                    && let EventKind::Open(Delimiter::Brace | Delimiter::Bracket) = event.kind
                {
                    break;
                }
                end = next_tree(end);
                trees += 1;
            }
        }
    }
    let last = if end > position {
        // The last top-level tree starts at or before `end - 1`; its physical
        // range is recorded on its closing/atom event.
        &events[end - 1]
    } else {
        first
    };
    if trees == 0 && kind != FragmentKind::Vis {
        return Ok(None);
    }
    let range = if trees == 0 {
        first.physical.start..first.physical.start
    } else {
        first.physical.start..last.physical.end
    };
    let tokens = first.container.subslice(session, range)?;
    let value = if trees > 1 && kind == FragmentKind::Expr {
        CaptureValue::Grouped(tokens)
    } else {
        CaptureValue::Tokens(tokens)
    };
    Ok(Some((end, value)))
}

pub(super) fn run(
    program: &Program,
    _schema: &Syntax,
    input: TokenSlice,
    invocation: &InvocationBudget,
    session: &mut TokenSession,
) -> Result<Captures, MacroError> {
    let events = flatten(input, session)?;
    let mut future = BudgetVec::new(session.budget(), Storage::Temporary)?;
    future.push(state(program.start, 0, None, invocation)?)?;
    let mut history = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut accepted: Option<Option<usize>> = None;
    for position in 0..=events.len() {
        work(session)?;
        let mut closure = BudgetVec::new(session.budget(), Storage::Temporary)?;
        let mut remaining = BudgetVec::new(session.budget(), Storage::Temporary)?;
        while let Some(thread) = future.pop() {
            work(session)?;
            if thread.position == position {
                closure.push(thread)?;
            } else {
                remaining.push(thread)?;
            }
        }
        future = remaining;
        let mut seen: BudgetVec<Seen> = BudgetVec::new(session.budget(), Storage::Temporary)?;
        let mut transitions = BudgetVec::new(session.budget(), Storage::Temporary)?;
        let mut fragments = 0usize;
        while let Some(thread) = closure.pop() {
            work(session)?;
            let mut duplicate = false;
            for old in seen.as_slice() {
                work(session)?;
                if old.pc == thread.pc {
                    if old.history != thread.history {
                        return Err(MacroError::Ambiguous { span: None });
                    }
                    duplicate = true;
                    break;
                }
            }
            if duplicate {
                continue;
            }
            seen.push(Seen {
                pc: thread.pc,
                history: thread.history,
                _lease: invocation.reserve_states(1, 0)?,
            })?;
            let instruction = program
                .code
                .as_slice()
                .get(thread.pc)
                .ok_or(TokenError::InvalidHandle)?;
            let next = instruction.next;
            match &instruction.op {
                Op::Split(alternate) => {
                    closure.push(state(*alternate, position, thread.history, invocation)?)?;
                    closure.push(state(next, position, thread.history, invocation)?)?;
                }
                Op::Enter(id) | Op::Next(id) | Op::Exit(id) => {
                    let event = match &instruction.op {
                        Op::Enter(_) => HistoryEvent::Enter(*id),
                        Op::Next(_) => HistoryEvent::Next(*id),
                        _ => HistoryEvent::Exit(*id),
                    };
                    let tip = record(&mut history, thread.history, event, session)?;
                    closure.push(state(next, position, tip, invocation)?)?;
                }
                Op::Accept => {
                    if position == events.len() {
                        if accepted.is_some() {
                            return Err(MacroError::Ambiguous { span: None });
                        }
                        accepted = Some(thread.history);
                    }
                }
                Op::Capture { binder, kind } => {
                    if let Some(event) = events.as_slice().get(position)
                        && let Some((end, value)) =
                            probe(*kind, event, events.as_slice(), position, session)?
                    {
                        fragments += 1;
                        let empty = end == position;
                        let tip = record(
                            &mut history,
                            thread.history,
                            HistoryEvent::Capture {
                                binder: *binder,
                                value,
                            },
                            session,
                        )?;
                        if empty {
                            closure.push(state(next, end, tip, invocation)?)?;
                        } else {
                            transitions.push(state(next, end, tip, invocation)?)?;
                        }
                    }
                }
                Op::Atom(pattern) => {
                    if let Some(Event {
                        kind: EventKind::Atom(actual),
                        ..
                    }) = events.as_slice().get(position)
                        && atom_equal(pattern, actual, session)?
                    {
                        transitions.push(state(next, position + 1, thread.history, invocation)?)?;
                    }
                }
                Op::Open(kind) | Op::Close(kind) => {
                    if let Some(event) = events.as_slice().get(position) {
                        let matches = match (&instruction.op, &event.kind) {
                            (Op::Open(_), EventKind::Open(actual))
                            | (Op::Close(_), EventKind::Close(actual)) => kind == actual,
                            _ => false,
                        };
                        if matches {
                            transitions.push(state(
                                next,
                                position + 1,
                                thread.history,
                                invocation,
                            )?)?;
                        }
                    }
                }
            }
        }
        // Rust's matcher does not look ahead to disambiguate fragment starts.
        if fragments > 1 || (fragments == 1 && transitions.len() > 1) {
            return Err(MacroError::Ambiguous { span: None });
        }
        while let Some(thread) = transitions.pop() {
            work(session)?;
            future.push(thread)?;
        }
    }
    match accepted {
        Some(tip) => materialize(history.as_slice(), tip, session),
        None => Err(MacroError::NoMatch),
    }
}
