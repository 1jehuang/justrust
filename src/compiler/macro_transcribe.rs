//! Committed, iterative transcription with physical occurrence accounting.
use std::ops::Range;

use super::expansion_budget::{InvocationBudget, Storage};
use super::expansion_span::{ExpansionId, SpanId};
use super::hygiene::Transparency;
use super::macro_cursor::TokenSlice;
use super::macro_matcher::{CaptureValue, Captures, PathComponent};
use super::macro_rules::{
    Atom, DefinitionSite, Element, MacroError, RepeatOp, Syntax, syntax, work,
};
use super::token::{Delimiter, InputToken, Token, TokenKind};
use super::token_session::{BudgetVec, SymbolId, TokenError, TokenSession};
use super::token_tree::{GroupPolicy, Node, TokenStream, group};

fn binder(schema: &Syntax, name: SymbolId, session: &TokenSession) -> Result<usize, MacroError> {
    for (index, binder) in schema.binders.as_slice().iter().enumerate() {
        work(session)?;
        if binder.name == name {
            return Ok(index);
        }
    }
    Err(syntax("unknown transcriber metavariable"))
}
fn path_matches(
    actual: &[PathComponent],
    schema: &[usize],
    iterations: &[usize],
    session: &TokenSession,
) -> Result<bool, MacroError> {
    if actual.len() != schema.len() || iterations.len() < schema.len() {
        return Ok(false);
    }
    for (index, component) in actual.iter().enumerate() {
        work(session)?;
        if component.repetition != schema[index] || component.iteration != iterations[index] {
            return Ok(false);
        }
    }
    Ok(true)
}
fn value<'a>(
    captures: &'a Captures,
    schema: &Syntax,
    binder_id: usize,
    iterations: &[usize],
    session: &TokenSession,
) -> Result<&'a CaptureValue, MacroError> {
    let binding = &schema.binders.as_slice()[binder_id];
    if binding.depth > iterations.len() {
        return Err(syntax("metavariable is still repeating at this depth"));
    }
    let schema_path = &schema.binder_paths.as_slice()[binding.path.clone()];
    for capture in captures.values.as_slice() {
        work(session)?;
        if capture.binder == binder_id
            && path_matches(
                &captures.paths.as_slice()[capture.path.clone()],
                schema_path,
                iterations,
                session,
            )?
        {
            return Ok(&capture.value);
        }
    }
    Err(syntax("missing capture at transcriber repetition depth"))
}
fn repetition_count(
    body: Range<usize>,
    template: &Syntax,
    schema: &Syntax,
    captures: &Captures,
    iterations: &[usize],
    session: &TokenSession,
) -> Result<usize, MacroError> {
    let mut pending = BudgetVec::new(session.budget(), Storage::Temporary)?;
    pending.push(body)?;
    let mut count = None;
    while let Some(range) = pending.pop() {
        for node in template.sequence(range) {
            work(session)?;
            match &template.nodes.as_slice()[*node] {
                Element::Variable { name, .. } => {
                    if session.symbol(*name)? == "crate" {
                        continue;
                    }
                    let binder_id = binder(schema, *name, session)?;
                    let binding = &schema.binders.as_slice()[binder_id];
                    if binding.depth <= iterations.len() {
                        continue;
                    }
                    let binding_path = &schema.binder_paths.as_slice()[binding.path.clone()];
                    let id = binding_path[iterations.len()];
                    let mut found = None;
                    for repetition in captures.repetitions.as_slice() {
                        work(session)?;
                        if repetition.id == id
                            && path_matches(
                                &captures.paths.as_slice()[repetition.parent.clone()],
                                &binding_path[..iterations.len()],
                                iterations,
                                session,
                            )?
                        {
                            found = Some(repetition.count);
                            break;
                        }
                    }
                    let found = found.ok_or(TokenError::InvalidHandle)?;
                    if let Some(previous) = count {
                        if previous != found {
                            return Err(syntax("inconsistent repetition cardinalities"));
                        }
                    } else {
                        count = Some(found);
                    }
                }
                Element::Group { body, .. } | Element::Repeat { body, .. } => {
                    pending.push(body.clone())?
                }
                _ => {}
            }
        }
    }
    count.ok_or_else(|| syntax("repetition has no metavariable at this depth"))
}
fn occurrence(
    span: SpanId,
    definition: bool,
    expansion: ExpansionId,
    session: &mut TokenSession,
) -> Result<SpanId, MacroError> {
    work(session)?;
    let mut context = session.span(span)?.context;
    if definition {
        context = session.apply_mark(context, expansion, Transparency::SemiTransparent)?;
    }
    Ok(session.reissue_span(span, context, expansion)?)
}
fn emit_leaf(
    mut token: Token,
    definition: bool,
    expansion: ExpansionId,
    invocation: &InvocationBudget,
    output: &mut BudgetVec<InputToken>,
    session: &mut TokenSession,
) -> Result<(), MacroError> {
    work(session)?;
    let weight = match token.kind {
        TokenKind::Opaque(id) => session.fragment(id)?.physical_weight(),
        _ => 1,
    };
    invocation.emit(weight)?;
    token.span = occurrence(token.span, definition, expansion, session)?;
    if let TokenKind::Ident(ref mut ident) = token.kind {
        ident.context = session.span(token.span)?.context;
    }
    output.push(InputToken::Leaf(token))?;
    Ok(())
}
enum SliceTask {
    Slice { input: TokenSlice, at: usize },
    Close { kind: Delimiter, span: SpanId },
}
fn emit_slice(
    input: TokenSlice,
    definition: bool,
    expansion: ExpansionId,
    invocation: &InvocationBudget,
    output: &mut BudgetVec<InputToken>,
    session: &mut TokenSession,
) -> Result<(), MacroError> {
    let mut pending = BudgetVec::new(session.budget(), Storage::Temporary)?;
    pending.push(SliceTask::Slice { input, at: 0 })?;
    while let Some(task) = pending.pop() {
        work(session)?;
        match task {
            SliceTask::Close { kind, span } => {
                invocation.emit(1)?;
                let span = occurrence(span, definition, expansion, session)?;
                output.push(InputToken::Close { kind, span })?;
            }
            SliceTask::Slice { input, at } => {
                if at == input.len(session)? {
                    continue;
                }
                let node = input.node(session, at)?.clone();
                pending.push(SliceTask::Slice {
                    input: input.clone(),
                    at: at + 1,
                })?;
                match node {
                    Node::Leaf(token) => {
                        emit_leaf(token, definition, expansion, invocation, output, session)?
                    }
                    Node::Delimited {
                        kind, open, close, ..
                    } => {
                        let children = input
                            .children(session, at)?
                            .ok_or(TokenError::InvalidRange)?;
                        invocation.emit(1)?;
                        let span = occurrence(open, definition, expansion, session)?;
                        output.push(InputToken::Open { kind, span })?;
                        pending.push(SliceTask::Close { kind, span: close })?;
                        pending.push(SliceTask::Slice {
                            input: children,
                            at: 0,
                        })?;
                    }
                }
            }
        }
    }
    Ok(())
}
enum Task {
    Sequence {
        range: Range<usize>,
        at: usize,
    },
    Close {
        kind: Delimiter,
        span: SpanId,
    },
    Repeat {
        body: Range<usize>,
        separator: Option<Atom>,
        count: usize,
        next: usize,
        depth: usize,
    },
}
#[allow(clippy::too_many_arguments)]
pub(super) fn transcribe(
    template: &Syntax,
    schema: &Syntax,
    captures: &Captures,
    site: DefinitionSite,
    expansion: ExpansionId,
    invocation: &InvocationBudget,
    session: &mut TokenSession,
) -> Result<TokenStream, MacroError> {
    let mut output = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut tasks = BudgetVec::new(session.budget(), Storage::Temporary)?;
    let mut iterations = BudgetVec::new(session.budget(), Storage::Temporary)?;
    tasks.push(Task::Sequence {
        range: template.root.clone(),
        at: 0,
    })?;
    while let Some(task) = tasks.pop() {
        work(session)?;
        match task {
            Task::Close { kind, span } => {
                invocation.emit(1)?;
                let span = occurrence(span, true, expansion, session)?;
                output.push(InputToken::Close { kind, span })?;
            }
            Task::Repeat {
                body,
                separator,
                count,
                next,
                depth,
            } => {
                iterations.truncate(depth);
                if next == count {
                    continue;
                }
                iterations.push(next)?;
                if next != 0
                    && let Some(separator) = &separator
                {
                    emit_slice(
                        separator.tokens.clone(),
                        true,
                        expansion,
                        invocation,
                        &mut output,
                        session,
                    )?;
                }
                tasks.push(Task::Repeat {
                    body: body.clone(),
                    separator,
                    count,
                    next: next + 1,
                    depth,
                })?;
                tasks.push(Task::Sequence { range: body, at: 0 })?;
            }
            Task::Sequence { range, at } => {
                let sequence = template.sequence(range.clone());
                if at == sequence.len() {
                    continue;
                }
                let node = &template.nodes.as_slice()[sequence[at]];
                tasks.push(Task::Sequence { range, at: at + 1 })?;
                match node {
                    Element::Atom(atom) => emit_slice(
                        atom.tokens.clone(),
                        true,
                        expansion,
                        invocation,
                        &mut output,
                        session,
                    )?,
                    Element::Variable { name, span } => {
                        if session.symbol(*name)? == "crate" {
                            emit_leaf(
                                Token {
                                    kind: TokenKind::DollarCrate(site.defining_crate),
                                    span: *span,
                                },
                                true,
                                expansion,
                                invocation,
                                &mut output,
                                session,
                            )?;
                        } else {
                            let binder_id = binder(schema, *name, session)?;
                            match value(
                                captures,
                                schema,
                                binder_id,
                                iterations.as_slice(),
                                session,
                            )? {
                                CaptureValue::Tokens(tokens) => emit_slice(
                                    tokens.clone(),
                                    false,
                                    expansion,
                                    invocation,
                                    &mut output,
                                    session,
                                )?,
                                CaptureValue::Literal { id, span } => emit_leaf(
                                    Token {
                                        kind: TokenKind::Opaque(*id),
                                        span: *span,
                                    },
                                    false,
                                    expansion,
                                    invocation,
                                    &mut output,
                                    session,
                                )?,
                            }
                        }
                    }
                    Element::Group {
                        kind,
                        open,
                        close,
                        body,
                    } => {
                        invocation.emit(1)?;
                        let span = occurrence(*open, true, expansion, session)?;
                        output.push(InputToken::Open { kind: *kind, span })?;
                        tasks.push(Task::Close {
                            kind: *kind,
                            span: *close,
                        })?;
                        tasks.push(Task::Sequence {
                            range: body.clone(),
                            at: 0,
                        })?;
                    }
                    Element::Repeat {
                        body,
                        separator,
                        op,
                        ..
                    } => {
                        let count = repetition_count(
                            body.clone(),
                            template,
                            schema,
                            captures,
                            iterations.as_slice(),
                            session,
                        )?;
                        if (*op == RepeatOp::Plus && count == 0)
                            || (*op == RepeatOp::Optional && count > 1)
                        {
                            return Err(syntax(
                                "transcriber repetition cardinality violates its operator",
                            ));
                        }
                        tasks.push(Task::Repeat {
                            body: body.clone(),
                            separator: separator.clone(),
                            count,
                            next: 0,
                            depth: iterations.len(),
                        })?;
                    }
                    Element::Bind { .. } => return Err(TokenError::InvalidHandle.into()),
                }
            }
        }
    }
    Ok(group(session, output.as_slice(), GroupPolicy::Generated)?)
}
