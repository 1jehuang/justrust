//! Finite original function syntax, issued only by the parser's embedded-source arm.
//! This receipt deliberately does not retain an AST, checked body, world, or budget.
use super::*;
use crate::compiler::token_session::OriginalSpan;
#[cfg(test)]
#[path = "function_source_tests.rs"]
mod tests;

#[path = "packed_query_source.rs"]
pub(in crate::compiler) mod packed_query;

#[derive(Clone, Debug)]
enum FunctionSyntaxEvent {
    Path(PathAnchor, usize),
    OriginalPath(PathAnchor, usize),
    Name(AstIdent),
    IntType(ir::IntTy),
    InferType,
    UnitType,
    Parameter(Option<AstIdent>, bool),
    Block(usize, bool),
    Let(Option<AstIdent>, bool),
    Expression(bool),
    Return(bool),
    Integer(u128, Option<ir::IntTy>),
    Variable,
    Call(usize),
    TypeAlignment,
    Unsafe,
    NativeSyscall(usize),
    Binary(BinaryOp, Option<super::super::expansion_span::SpanId>),
    If,
}

fn path(
    names: &impl NameAccess,
    path: &AstPath,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<(), String> {
    path.check(names).map_err(native_adapter::name_diagnostic)?;
    emit(FunctionSyntaxEvent::Path(
        path.anchor(),
        path.segments().len(),
    ))?;
    for name in path.segments() {
        emit(FunctionSyntaxEvent::Name(*name))?;
    }
    let original = path
        .original_use(names)
        .map_err(native_adapter::name_diagnostic)?;
    emit(FunctionSyntaxEvent::OriginalPath(
        original.anchor(),
        original.segments().len(),
    ))?;
    for name in original.segments() {
        emit(FunctionSyntaxEvent::Name(*name))?;
    }
    Ok(())
}

fn optional_name(
    names: &NameStore,
    a: Option<AstIdent>,
    b: Option<AstIdent>,
) -> Result<bool, String> {
    match (a, b) {
        (Some(a), Some(b)) => exact_source_ident(names, a, b),
        (None, None) => Ok(true),
        _ => Ok(false),
    }
}
fn event_equal(
    names: &NameStore,
    a: Option<&FunctionSyntaxEvent>,
    b: &FunctionSyntaxEvent,
) -> Result<bool, String> {
    let Some(a) = a else {
        return Ok(false);
    };
    Ok(match (a, b) {
        (FunctionSyntaxEvent::Path(a, n), FunctionSyntaxEvent::Path(b, m))
        | (FunctionSyntaxEvent::OriginalPath(a, n), FunctionSyntaxEvent::OriginalPath(b, m)) => {
            a == b && n == m
        }
        (FunctionSyntaxEvent::Name(a), FunctionSyntaxEvent::Name(b)) => {
            exact_source_ident(names, *a, *b)?
        }
        (FunctionSyntaxEvent::IntType(a), FunctionSyntaxEvent::IntType(b)) => a == b,
        (FunctionSyntaxEvent::InferType, FunctionSyntaxEvent::InferType)
        | (FunctionSyntaxEvent::UnitType, FunctionSyntaxEvent::UnitType)
        | (FunctionSyntaxEvent::Variable, FunctionSyntaxEvent::Variable)
        | (FunctionSyntaxEvent::TypeAlignment, FunctionSyntaxEvent::TypeAlignment)
        | (FunctionSyntaxEvent::If, FunctionSyntaxEvent::If) => true,
        (FunctionSyntaxEvent::Parameter(a, x), FunctionSyntaxEvent::Parameter(b, y))
        | (FunctionSyntaxEvent::Let(a, x), FunctionSyntaxEvent::Let(b, y)) => {
            x == y && optional_name(names, *a, *b)?
        }
        (FunctionSyntaxEvent::Block(n, a), FunctionSyntaxEvent::Block(m, b)) => n == m && a == b,
        (FunctionSyntaxEvent::Expression(a), FunctionSyntaxEvent::Expression(b))
        | (FunctionSyntaxEvent::Return(a), FunctionSyntaxEvent::Return(b)) => a == b,
        (FunctionSyntaxEvent::Integer(a, x), FunctionSyntaxEvent::Integer(b, y)) => {
            a == b && x == y
        }
        (FunctionSyntaxEvent::Call(a), FunctionSyntaxEvent::Call(b))
        | (FunctionSyntaxEvent::NativeSyscall(a), FunctionSyntaxEvent::NativeSyscall(b)) => a == b,
        (FunctionSyntaxEvent::Unsafe, FunctionSyntaxEvent::Unsafe) => true,
        (FunctionSyntaxEvent::Binary(a, x), FunctionSyntaxEvent::Binary(b, y)) => a == b && x == y,
        _ => false,
    })
}
fn binding(name: BindingName) -> Option<AstIdent> {
    match name {
        BindingName::Named(name) => Some(name),
        BindingName::Wildcard => None,
    }
}
fn ty(
    _names: &impl NameAccess,
    value: &Type,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    emit(match value {
        Type::Int(value) => FunctionSyntaxEvent::IntType(*value),
        Type::Infer => FunctionSyntaxEvent::InferType,
        Type::Unit => FunctionSyntaxEvent::UnitType,
        _ => return Ok(false),
    })?;
    Ok(true)
}
fn expression(
    names: &impl NameAccess,
    value: &Expr,
    depth: usize,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    if depth > 64 {
        return Ok(false);
    }
    match value {
        Expr::Integer(value) => emit(FunctionSyntaxEvent::Integer(value.magnitude, value.suffix))?,
        Expr::Var(value) => {
            emit(FunctionSyntaxEvent::Variable)?;
            path(names, value, emit)?;
        }
        Expr::Call { name, args } => {
            emit(FunctionSyntaxEvent::Call(args.len()))?;
            path(names, name, emit)?;
            for argument in args {
                if !expression(names, argument, depth + 1, emit)? {
                    return Ok(false);
                }
            }
        }
        Expr::Unsafe(value) => {
            emit(FunctionSyntaxEvent::Unsafe)?;
            if !block(names, value, depth + 1, emit)? {
                return Ok(false);
            }
        }
        Expr::NativeSyscall(arguments) => {
            if arguments.len() != 7 {
                return Ok(false);
            }
            emit(FunctionSyntaxEvent::NativeSyscall(arguments.len()))?;
            for argument in arguments {
                if !expression(names, argument, depth + 1, emit)? {
                    return Ok(false);
                }
            }
        }
        Expr::TypeAlignment(value) => {
            emit(FunctionSyntaxEvent::TypeAlignment)?;
            if !ty(names, value, emit)? {
                return Ok(false);
            }
        }
        Expr::Binary {
            op,
            left,
            right,
            site,
        } => {
            emit(FunctionSyntaxEvent::Binary(*op, *site))?;
            if !expression(names, left, depth + 1, emit)?
                || !expression(names, right, depth + 1, emit)?
            {
                return Ok(false);
            }
        }
        Expr::If {
            condition,
            then_block,
            else_block,
        } => {
            emit(FunctionSyntaxEvent::If)?;
            if !expression(names, condition, depth + 1, emit)?
                || !block(names, then_block, depth + 1, emit)?
                || !block(names, else_block, depth + 1, emit)?
            {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
fn block(
    names: &impl NameAccess,
    value: &Block,
    depth: usize,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    if depth > 64 {
        return Ok(false);
    }
    emit(FunctionSyntaxEvent::Block(
        value.statements.len(),
        value.tail.is_some(),
    ))?;
    for statement in &value.statements {
        match statement {
            Stmt::Let {
                name,
                mutable,
                ty: annotation,
                value,
            } => {
                emit(FunctionSyntaxEvent::Let(binding(*name), *mutable))?;
                if !ty(names, annotation, emit)? || !expression(names, value, depth + 1, emit)? {
                    return Ok(false);
                }
            }
            Stmt::Expr(value) | Stmt::ImplicitExpr(value) => {
                emit(FunctionSyntaxEvent::Expression(matches!(
                    statement,
                    Stmt::ImplicitExpr(_)
                )))?;
                if !expression(names, value, depth + 1, emit)? {
                    return Ok(false);
                }
            }
            Stmt::Return(value) => {
                emit(FunctionSyntaxEvent::Return(value.is_some()))?;
                if let Some(value) = value
                    && !expression(names, value, depth + 1, emit)?
                {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    if let Some(value) = &value.tail
        && !expression(names, value, depth + 1, emit)?
    {
        return Ok(false);
    }
    Ok(true)
}
fn recipe(
    names: &impl NameAccess,
    function: &Function,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    if function.static_mut
        || !function.type_params.is_empty()
        || !function.bounds.is_empty()
        || function.impl_params != 0
        || function.regions.receiver.is_some()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
        || (!function.param_mut.is_empty() && function.param_mut.len() != function.params.len())
    {
        return Ok(false);
    }
    path(names, &function.name, emit)?;
    for (index, (name, annotation)) in function.params.iter().enumerate() {
        emit(FunctionSyntaxEvent::Parameter(
            binding(*name),
            function.param_mut.get(index).copied().unwrap_or(false),
        ))?;
        if !ty(names, annotation, emit)? {
            return Ok(false);
        }
    }
    if !ty(names, &function.result, emit)? {
        return Ok(false);
    }
    block(names, &function.body, 0, emit)
}

#[derive(Clone, Debug)]
pub(super) struct SourceFunction {
    source: OriginalSpan,
    function_index: usize,
    issuer: crate::compiler::source::EmbeddedOriginalFile,
    module: ModuleId,
    scope: Option<crate::compiler::semantic_scope::ScopeId>,
    visibility: ir::Visibility,
    constness: Constness,
    unsafety: bool,
    parameters: usize,
    events: Arc<[FunctionSyntaxEvent]>,
    // Exact original builtin invocation and its full-recipe event ordinal.
    packed_query: Option<(usize, OriginalSpan)>,
    _admitted: crate::compiler::expansion_budget::RetainedBytes,
}

/// Unsupported syntax receives no privileged receipt. It remains ordinary Rust
/// input and is not rejected merely because this finite authority grammar ends.
pub(super) fn issue(
    function: &Function,
    source: OriginalSpan,
    function_index: usize,
    issuer: &crate::compiler::source::EmbeddedOriginalFile,
    query_token: Option<OriginalSpan>,
    names: &impl NameAccess,
) -> Result<Option<Arc<SourceFunction>>, String> {
    if !issuer.authorizes(names.session().sources(), source) {
        return Ok(None);
    }
    let mut count = 0usize;
    if !recipe(names, function, &mut |_| {
        names
            .budget()
            .charge(crate::compiler::expansion_budget::Counter::Work, 1)
            .map_err(|error| error.to_string())?;
        count = count
            .checked_add(1)
            .ok_or("function source recipe sizing overflow")?;
        Ok(())
    })? {
        return Ok(None);
    }
    use crate::compiler::expansion_budget::Storage;
    let budget = names.budget();
    let bytes = count
        .checked_mul(std::mem::size_of::<FunctionSyntaxEvent>())
        .and_then(|n| n.checked_add(std::mem::size_of::<SourceFunction>()))
        .ok_or("function source recipe allocation overflow")?;
    let admitted = budget
        .reserve(Storage::Retained, bytes as u64)
        .and_then(|lease| lease.retain())
        .map_err(|error| error.to_string())?;
    budget
        .charge(
            crate::compiler::expansion_budget::Counter::Work,
            count as u64,
        )
        .map_err(|error| error.to_string())?;
    let mut events = Vec::new();
    events
        .try_reserve_exact(count)
        .map_err(|_| "function source recipe allocation failed")?;
    if !recipe(names, function, &mut |event| {
        events.push(event);
        Ok(())
    })? || events.len() != count
    {
        return Err("function source recipe changed during issuance".into());
    }
    let mut query_event = None;
    let mut query_count = 0usize;
    for (index, event) in events.iter().enumerate() {
        budget
            .charge(crate::compiler::expansion_budget::Counter::Work, 1)
            .map_err(|error| error.to_string())?;
        if matches!(event, FunctionSyntaxEvent::TypeAlignment) {
            query_count = query_count.checked_add(1).ok_or("query count overflow")?;
            if matches!(
                events.get(index + 1),
                Some(FunctionSyntaxEvent::IntType(ir::IntTy::U8))
            ) {
                query_event = Some(index);
            }
        }
    }
    let packed_query = if query_count == 1 {
        query_event.zip(query_token)
    } else {
        None
    };
    Ok(Some(Arc::new(SourceFunction {
        source,
        function_index,
        issuer: issuer.clone(),
        module: function.module,
        scope: function.lexical_scope,
        visibility: function.visibility.clone(),
        constness: function.constness,
        unsafety: function.unsafety,
        parameters: function.params.len(),
        events: events.into(),
        packed_query,
        _admitted: admitted,
    })))
}

impl SourceFunction {
    pub(super) fn replay_source(
        &self,
        function: &Function,
        names: &NameStore,
        issuers: &[crate::compiler::source::EmbeddedOriginalFile],
    ) -> Result<(), String> {
        if !self
            .issuer
            .authorizes(names.session().sources(), self.source)
            || !issuers
                .iter()
                .any(|issuer| issuer.same_issuer(&self.issuer))
            || self.module != function.module
            || self.scope != function.lexical_scope
            || self.visibility != function.visibility
            || self.constness != function.constness
            || self.unsafety != function.unsafety
            || self.parameters != function.params.len()
        {
            return Err("embedded function changed original header or issuer".into());
        }
        let mut index = 0usize;
        let complete = recipe(names, function, &mut |event| {
            names
                .budget()
                .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                .map_err(|error| error.to_string())?;
            if !event_equal(names, self.events.get(index), &event)? {
                return Err("embedded function changed original syntax recipe".into());
            }
            index += 1;
            Ok(())
        })?;
        if !complete || index != self.events.len() {
            return Err("embedded function changed complete original syntax recipe".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(in crate::compiler) struct FunctionSource {
    phase: FunctionPhase,
}
#[derive(Clone, Debug)]
enum FunctionPhase {
    Original(Arc<SourceFunction>),
    Resolved {
        original: Arc<SourceFunction>,
        expected: Arc<[FunctionSyntaxEvent]>,
        _admitted: crate::compiler::expansion_budget::RetainedBytes,
    },
}

/// Reject header information not retained by Function, rather than recovering
/// a default ABI from a lossy parsed representation. This never issues authority.
fn supported_header<'a>(token: impl Fn(usize) -> Option<&'a str>) -> bool {
    let mut header = 0usize;
    if token(header) == Some("pub") {
        header += 1;
        if token(header) == Some("(") {
            header += 1;
            if !matches!(token(header), Some("crate" | "super" | "self")) {
                return false;
            }
            header += 1;
            if token(header) != Some(")") {
                return false;
            }
            header += 1;
        }
    }
    if token(header) == Some("const") {
        header += 1;
    }
    if token(header) == Some("unsafe") {
        header += 1;
    }
    token(header) == Some("fn")
}

pub(super) fn issue_parsed(
    parser: &Parser<'_, '_>,
    start: Option<usize>,
    function_index: usize,
    function: &Function,
) -> Result<Option<Arc<FunctionSource>>, String> {
    if parser.embedded_issuers.is_empty() {
        return Ok(None);
    }
    let (Some(start), Some(end)) = (start, parser.cursor.index()) else {
        return Ok(None);
    };
    let Some(tokens) = parser.input.get(start..end) else {
        return Ok(None);
    };
    if !supported_header(|index| tokens.get(index).map(|token| token.text.as_str())) {
        return Ok(None);
    }
    let Some(source) = original_token_range(parser.names, tokens)? else {
        return Ok(None);
    };
    let snapshot = parser.names.session().sources();
    let Some(issuer) = parser
        .embedded_issuers
        .iter()
        .find(|issuer| issuer.authorizes(snapshot, source))
    else {
        return Ok(None);
    };
    let query_token = packed_query::original_query_token(parser.names, tokens)?;
    Ok(issue(
        function,
        source,
        function_index,
        issuer,
        query_token,
        parser.names,
    )?
    .map(|original| {
        Arc::new(FunctionSource {
            phase: FunctionPhase::Original(original),
        })
    }))
}

// This is only the parser's original-range predicate, never an authority mint.
// The caller still checks the original header, loader issuer and full recipe.
fn original_token_range<'a>(
    names: &impl NameAccess,
    tokens: impl IntoIterator<Item = &'a Token>,
) -> Result<Option<OriginalSpan>, String> {
    let snapshot = names.session().sources();
    let mut first = None;
    let mut last = None;
    let mut closes = false;
    let mut body_depth = 0usize;
    let mut body_started = false;
    for token in tokens {
        names
            .budget()
            .charge(crate::compiler::expansion_budget::Counter::Work, 1)
            .map_err(|error| error.to_string())?;
        if token.whole.provenance() != TokenProvenance::Prepared {
            return Ok(None);
        }
        // The consumed range must end at this function's outer closing brace,
        // not an inner block close or any following item token.
        if body_started && body_depth == 0 {
            return Ok(None);
        }
        if !token.string {
            match token.text.as_str() {
                "{" => {
                    body_started = true;
                    body_depth = body_depth
                        .checked_add(1)
                        .ok_or("embedded function token brace depth overflow")?;
                }
                "}" => {
                    let Some(depth) = body_depth.checked_sub(1) else {
                        return Ok(None);
                    };
                    body_depth = depth;
                }
                _ => {}
            }
        }
        let Ok(origin) = names.session().span(token.whole.span) else {
            return Ok(None);
        };
        let crate::compiler::expansion_span::SpanOrigin::Original(span) = origin.origin else {
            return Ok(None);
        };
        let rendered = snapshot
            .render_span(span)
            .map_err(|error| error.to_string())?;
        if let Some(previous) = last {
            let previous = snapshot
                .render_span(previous)
                .map_err(|error| error.to_string())?;
            if previous.file != rendered.file || previous.hi > rendered.lo {
                return Ok(None);
            }
        }
        first.get_or_insert(span);
        last = Some(span);
        closes = token.text == "}";
    }
    if !closes || !body_started || body_depth != 0 {
        return Ok(None);
    }
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(None);
    };
    let handle = first.file();
    let first = snapshot
        .render_span(first)
        .map_err(|error| error.to_string())?;
    let last = snapshot
        .render_span(last)
        .map_err(|error| error.to_string())?;
    let source = snapshot
        .span(handle, first.lo, last.hi)
        .map_err(|error| error.to_string())?;
    Ok(Some(source))
}

impl FunctionSource {
    pub(super) fn original(&self) -> Result<&Arc<SourceFunction>, String> {
        match &self.phase {
            FunctionPhase::Original(original) => Ok(original),
            FunctionPhase::Resolved { .. } => {
                Err("embedded function resolution receipt was reused as original".into())
            }
        }
    }

    /// Only call paths are transformed, using the resolver's existing canonical
    /// function lookup. Every other event comes from the original receipt.
    pub(super) fn resolve_calls(
        &self,
        names: &NameStore,
        mut resolve: impl FnMut(&AstPath) -> Result<Option<AstPath>, String>,
    ) -> Result<Option<Arc<Self>>, String> {
        let original = self.original()?;
        let mut paths = Vec::new();
        let mut total = original.events.len();
        let mut index = 0usize;
        names
            .budget()
            .charge(
                crate::compiler::expansion_budget::Counter::Work,
                original.events.len() as u64,
            )
            .map_err(|error| error.to_string())?;
        while index < original.events.len() {
            if matches!(original.events[index], FunctionSyntaxEvent::Call(_)) {
                let path_start = index
                    .checked_add(1)
                    .ok_or("embedded call offset overflow")?;
                let names_start = path_start
                    .checked_add(1)
                    .ok_or("embedded call names offset overflow")?;
                let Some(FunctionSyntaxEvent::Path(anchor, count)) =
                    original.events.get(path_start)
                else {
                    return Err("embedded function call lost original path".into());
                };
                let path_end = index
                    .checked_add(2)
                    .and_then(|n| n.checked_add(*count))
                    .ok_or("embedded function original path sizing overflow")?;
                let row_bytes = count
                    .checked_mul(std::mem::size_of::<AstIdent>())
                    .ok_or("embedded function original path storage overflow")?;
                names
                    .budget()
                    .charge(
                        crate::compiler::expansion_budget::Counter::Work,
                        *count as u64,
                    )
                    .map_err(|error| error.to_string())?;
                let _scratch = names
                    .budget()
                    .reserve(
                        crate::compiler::expansion_budget::Storage::Retained,
                        row_bytes as u64,
                    )
                    .map_err(|error| error.to_string())?;
                let mut identifiers = Vec::new();
                identifiers
                    .try_reserve_exact(*count)
                    .map_err(|_| "embedded original path allocation failed")?;
                for event in original
                    .events
                    .get(names_start..path_end)
                    .ok_or("embedded original path missing")?
                {
                    let FunctionSyntaxEvent::Name(name) = event else {
                        return Err("embedded original path changed kind".into());
                    };
                    identifiers.push(*name);
                }
                let Some(FunctionSyntaxEvent::OriginalPath(original_anchor, original_count)) =
                    original.events.get(path_end)
                else {
                    return Err("embedded call lost authored path channel".into());
                };
                let end = path_end
                    .checked_add(1)
                    .and_then(|n| n.checked_add(*original_count))
                    .ok_or("embedded authored path sizing overflow")?;
                if original_anchor != anchor || original_count != count {
                    return Err("embedded parser call already changed authored path".into());
                }
                names
                    .budget()
                    .charge(
                        crate::compiler::expansion_budget::Counter::Work,
                        *original_count as u64,
                    )
                    .map_err(|error| error.to_string())?;
                let authored_start = path_end
                    .checked_add(1)
                    .ok_or("embedded authored offset overflow")?;
                let authored = original
                    .events
                    .get(authored_start..end)
                    .ok_or("embedded authored path missing")?;
                for (event, name) in authored.iter().zip(&identifiers) {
                    if !event_equal(names, Some(event), &FunctionSyntaxEvent::Name(*name))? {
                        return Err("embedded parser call changed authored occurrence".into());
                    }
                }
                let source_path = AstPath::new(names, *anchor, &identifiers)
                    .map_err(native_adapter::name_diagnostic)?;
                let Some(target) = resolve(&source_path)? else {
                    return Ok(None);
                };
                target
                    .check(names)
                    .map_err(native_adapter::name_diagnostic)?;
                let authored_target = target
                    .original_use(names)
                    .map_err(native_adapter::name_diagnostic)?;
                let old_names = count
                    .checked_add(*original_count)
                    .ok_or("embedded original path count overflow")?;
                let new_names = target
                    .segments()
                    .len()
                    .checked_add(authored_target.segments().len())
                    .ok_or("embedded resolved path count overflow")?;
                total = total
                    .checked_sub(old_names)
                    .and_then(|n| n.checked_add(new_names))
                    .ok_or("embedded resolved path storage overflow")?;
                // Each row is admitted before growing the correspondence vector.
                names
                    .budget()
                    .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                    .map_err(|error| error.to_string())?;
                let correspondence_bytes = target
                    .segments()
                    .len()
                    .checked_mul(std::mem::size_of::<AstIdent>())
                    .and_then(|bytes| {
                        bytes.checked_add(std::mem::size_of::<(
                            usize,
                            usize,
                            AstPath,
                            crate::compiler::expansion_budget::RetainedBytes,
                        )>())
                    })
                    .ok_or("embedded call correspondence storage overflow")?;
                let lease = names
                    .budget()
                    .reserve(
                        crate::compiler::expansion_budget::Storage::Retained,
                        correspondence_bytes as u64,
                    )
                    .and_then(|lease| lease.retain())
                    .map_err(|error| error.to_string())?;
                paths
                    .try_reserve_exact(1)
                    .map_err(|_| "embedded call correspondence allocation failed")?;
                paths.push((path_start, end, target, lease));
                index = end;
            } else {
                index = index
                    .checked_add(1)
                    .ok_or("embedded recipe scan overflow")?;
            }
        }
        let bytes = total
            .checked_mul(std::mem::size_of::<FunctionSyntaxEvent>())
            .ok_or("embedded resolved recipe storage overflow")?;
        let admitted = names
            .budget()
            .reserve(
                crate::compiler::expansion_budget::Storage::Retained,
                bytes as u64,
            )
            .and_then(|lease| lease.retain())
            .map_err(|error| error.to_string())?;
        names
            .budget()
            .charge(
                crate::compiler::expansion_budget::Counter::Work,
                total as u64,
            )
            .map_err(|error| error.to_string())?;
        let mut expected = Vec::new();
        expected
            .try_reserve_exact(total)
            .map_err(|_| "embedded resolved recipe allocation failed")?;
        let mut cursor = 0usize;
        for (start, end, target, _) in &paths {
            expected.extend_from_slice(&original.events[cursor..*start]);
            path(names, target, &mut |event| {
                expected.push(event);
                Ok(())
            })?;
            cursor = *end;
        }
        expected.extend_from_slice(&original.events[cursor..]);
        if expected.len() != total {
            return Err("embedded resolved recipe cardinality changed".into());
        }
        Ok(Some(Arc::new(Self {
            phase: FunctionPhase::Resolved {
                original: original.clone(),
                expected: expected.into(),
                _admitted: admitted,
            },
        })))
    }

    pub(super) fn matches_resolved(
        &self,
        function: &Function,
        names: &NameStore,
    ) -> Result<bool, String> {
        let FunctionPhase::Resolved {
            original, expected, ..
        } = &self.phase
        else {
            return Err("embedded function lacks authenticated resolution correspondence".into());
        };
        if function.module != original.module
            || function.lexical_scope != original.scope
            || function.visibility != original.visibility
            || function.constness != original.constness
            || function.unsafety != original.unsafety
            || function.params.len() != original.parameters
        {
            return Ok(false);
        }
        let mut index = 0usize;
        let mut same = true;
        let complete = recipe(names, function, &mut |event| {
            names
                .budget()
                .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                .map_err(|error| error.to_string())?;
            same &= event_equal(names, expected.get(index), &event)?;
            index += 1;
            Ok(())
        })?;
        Ok(complete && same && index == expected.len())
    }
}

/// Canonical registration adds immutable declaration/binder identities only.
/// The Program, registry, names, world, and planning budget are borrowed again
/// for every admission replay, never retained through this value.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct BoundFunctionSource {
    source: Arc<FunctionSource>,
    declaration: ir::DeclId,
    function_index: usize,
    binder: ir::BinderId,
}

fn canonical_header(info: &DeclInfo, index: usize, declaration: ir::DeclId) -> Result<(), String> {
    if info.id != declaration
        || info.kind != ir::DeclKind::Function
        || info.path.kind != ir::DeclKind::Function
        || info.path.associated.is_some()
        || info.source_index != index
        || info.owner.is_some()
        || info.receiver != ReceiverKind::None
        || !info.type_params.is_empty()
        || info.param_env.binder().declaration != declaration
        || info.param_env.arity() != 0
        || info.env.total_arity() != 0
        || info.env.scopes().len() != 1
        || info.env.scopes()[0].binder() != info.param_env.binder()
        || info.env.scopes()[0].arity() != 0
    {
        return Err("embedded function changed canonical declaration or environment".into());
    }
    Ok(())
}

fn canonical_item(info: &DeclInfo, function: &Function, names: &NameStore) -> Result<bool, String> {
    let (DeclItemName::Source(actual), Some(expected)) =
        (info.path.item, function.name.segments().last())
    else {
        return Ok(false);
    };
    exact_source_ident(names, actual, *expected)
}

pub(super) fn bind_registration(
    receipt: &Arc<FunctionSource>,
    program: &Program,
    info: &DeclInfo,
    function_index: usize,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Arc<BoundFunctionSource>, String> {
    canonical_header(info, function_index, declaration)?;
    let function = program
        .functions
        .get(function_index)
        .ok_or("embedded original function missing")?;
    let FunctionPhase::Resolved { original, .. } = &receipt.phase else {
        return Err("embedded function registration lacks resolved source receipt".into());
    };
    if original.function_index != function_index
        || !function
            .embedded_source
            .as_ref()
            .is_some_and(|actual| Arc::ptr_eq(actual, receipt))
        || !original
            .issuer
            .authorizes(program.names.session().sources(), original.source)
        || !program
            .embedded_issuers
            .iter()
            .any(|issuer| issuer.same_issuer(&original.issuer))
        || !receipt.matches_resolved(function, &program.names)?
        || info.path.module != function.module
        || !canonical_item(info, function, &program.names)?
        || info.visibility != function.visibility
    {
        return Err("embedded function registration changed original source or occurrence".into());
    }
    budget.charge_work(1)?;
    budget.retain_bytes(std::mem::size_of::<BoundFunctionSource>() + 32)?;
    Ok(Arc::new(BoundFunctionSource {
        source: receipt.clone(),
        declaration,
        function_index,
        binder: info.param_env.binder(),
    }))
}

impl BoundFunctionSource {
    pub(super) fn replay_admission(
        &self,
        program: &Program,
        registry: &DeclRegistry,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        let info = registry.declaration(declaration)?;
        canonical_header(info, self.function_index, declaration)?;
        let position = registry
            .function_position(declaration)
            .ok_or("embedded canonical function is not registered")?;
        if self.declaration != declaration
            || self.binder != info.param_env.binder()
            || !matches!(registry.callable_sources.get(position), Some(CallableSource::Free(index)) if *index == self.function_index)
        {
            return Err("embedded function changed canonical callable identity".into());
        }
        let function = program
            .functions
            .get(self.function_index)
            .ok_or("embedded canonical source missing")?;
        let FunctionPhase::Resolved { original, .. } = &self.source.phase else {
            return Err("embedded function lost original resolution receipt".into());
        };
        if !original
            .issuer
            .authorizes(program.names.session().sources(), original.source)
            || !program
                .embedded_issuers
                .iter()
                .any(|issuer| issuer.same_issuer(&original.issuer))
            || !function
                .embedded_source
                .as_ref()
                .is_some_and(|actual| Arc::ptr_eq(actual, &self.source))
            || !self.source.matches_resolved(function, &program.names)?
            || info.path.module != function.module
            || !canonical_item(info, function, &program.names)?
            || info.visibility != function.visibility
        {
            return Err(
                "embedded function admission changed issuer, source, or resolved syntax".into(),
            );
        }
        Ok(())
    }
}

impl PlanCtx {
    /// This prerequisite API does not itself admit any intrinsic or query kind.
    /// The operation-specific consumer must additionally check its exact kind,
    /// queried type, and occurrence in the authenticated function body.
    #[cfg(test)]
    pub(super) fn replay_embedded_function(&self, declaration: ir::DeclId) -> Result<(), String> {
        let info = self.registry.declaration(declaration)?;
        let receipt = info
            .embedded_function
            .as_ref()
            .ok_or("private operation lacks loader-issued original function authority")?;
        receipt.replay_admission(&self.program, &self.registry, declaration, &self.budget)
    }
}
