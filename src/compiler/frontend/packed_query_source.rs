//! Operation-specific packed-byte query authority. No allocator effect follows.
use super::*;
#[cfg(test)]
#[path = "packed_query_source_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub struct PackedReserveAuthority {
    source: Arc<BoundFunctionSource>,
    callee_source: Arc<BoundFunctionSource>,
    declaration: ir::DeclId,
    raw_realloc: ir::DeclId,
    query_event: usize,
    query_token: OriginalSpan,
}
impl PartialEq for PackedReserveAuthority {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
            && Arc::ptr_eq(&self.callee_source, &other.callee_source)
            && self.declaration == other.declaration
            && self.raw_realloc == other.raw_realloc
            && self.query_event == other.query_event
            && self.query_token == other.query_token
    }
}
impl Eq for PackedReserveAuthority {}

/// The caller has already checked every consumed token's original provenance.
/// This binds one exact lexer-token invocation, never a substring of source text.
pub(super) fn original_query_token(
    names: &impl NameAccess,
    tokens: &[Token],
) -> Result<Option<OriginalSpan>, String> {
    let mut found = None;
    for window in tokens.windows(5) {
        names
            .budget()
            .charge(crate::compiler::expansion_budget::Counter::Work, 1)
            .map_err(|error| error.to_string())?;
        if window.iter().map(|token| token.text.as_str()).ne([
            "__justrust_storage_stride",
            "!",
            "(",
            "u8",
            ")",
        ]) {
            continue;
        }
        if found.is_some() || window.iter().any(|token| token.string) {
            return Ok(None);
        }
        let snapshot = names.session().sources();
        let mut spans = [None; 5];
        for (index, token) in window.iter().enumerate() {
            if token.whole.provenance() != TokenProvenance::Prepared {
                return Ok(None);
            }
            let origin = names
                .session()
                .span(token.whole.span)
                .map_err(|e| e.to_string())?;
            let crate::compiler::expansion_span::SpanOrigin::Original(span) = origin.origin else {
                return Ok(None);
            };
            spans[index] = Some(snapshot.render_span(span).map_err(|e| e.to_string())?);
        }
        let first = spans[0].as_ref().ok_or("packed query first token absent")?;
        let last = spans[4]
            .as_ref()
            .ok_or("packed query closing token absent")?;
        for pair in spans.windows(2) {
            let a = pair[0].as_ref().ok_or("packed query token absent")?;
            let b = pair[1].as_ref().ok_or("packed query token absent")?;
            if a.file != b.file || a.hi > b.lo {
                return Ok(None);
            }
        }
        found = Some(
            snapshot
                .span(
                    snapshot
                        .file(first.file.0 as usize)
                        .map_err(|e| e.to_string())?,
                    first.lo,
                    last.hi,
                )
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(found)
}

fn parameter(function: &Function, index: usize) -> Result<AstIdent, String> {
    let Some((BindingName::Named(name), Type::Int(ir::IntTy::Usize))) = function.params.get(index)
    else {
        return Err("packed reserve requires exact ordered usize parameters".into());
    };
    if function.param_mut.get(index).copied().unwrap_or(false) {
        return Err("packed reserve parameter mutability changed".into());
    }
    Ok(*name)
}
fn variable(names: &NameStore, value: &Expr, expected: AstIdent) -> Result<bool, String> {
    let Expr::Var(path) = value else {
        return Ok(false);
    };
    if path.segments().len() != 1 {
        return Ok(false);
    }
    let actual = names
        .key(path.segments()[0], LookupClass::Local)
        .map_err(native_adapter::name_diagnostic)?;
    let expected = names
        .key(expected, LookupClass::Local)
        .map_err(native_adapter::name_diagnostic)?;
    Ok(actual == expected)
}
fn integer(value: &Expr, expected: u128) -> bool {
    matches!(value, Expr::Integer(value) if value.magnitude == expected && value.suffix == Some(ir::IntTy::Usize))
}
fn binary(value: &Expr, expected: BinaryOp) -> Option<(&Expr, &Expr)> {
    match value {
        Expr::Binary {
            op, left, right, ..
        } if *op == expected => Some((left, right)),
        _ => None,
    }
}
fn return_zero(value: &Block) -> bool {
    value.tail.is_none()
        && matches!(value.statements.as_slice(), [Stmt::Return(Some(value))] if integer(value, 0))
}
fn empty(value: &Block) -> bool {
    value.tail.is_none() && value.statements.is_empty()
}
fn if_expression(value: &Stmt) -> Option<(&Expr, &Block, &Block)> {
    match value {
        Stmt::Expr(Expr::If {
            condition,
            then_block,
            else_block,
        })
        | Stmt::ImplicitExpr(Expr::If {
            condition,
            then_block,
            else_block,
        }) => Some((condition, then_block, else_block)),
        _ => None,
    }
}

/// Exact semantic syntax contract over an already authenticated complete body.
/// Identifier spellings do not grant authority: binders are original occurrences.
fn contract<'a>(function: &'a Function, names: &NameStore) -> Result<&'a AstPath, String> {
    if function.params.len() != 3
        || !matches!(function.result, Type::Int(ir::IntTy::Usize))
        || function.unsafety
        || function.constness != Constness::Runtime
        || function.body.statements.len() != 3
    {
        return Err("packed reserve canonical signature/body changed".into());
    }
    let address = parameter(function, 0)?;
    let old = parameter(function, 1)?;
    let new = parameter(function, 2)?;
    let Stmt::Let {
        name: BindingName::Named(unit),
        mutable: false,
        ty: Type::Infer,
        value: Expr::TypeAlignment(Type::Int(ir::IntTy::U8)),
    } = &function.body.statements[0]
    else {
        return Err("packed reserve exact u8 query occurrence changed".into());
    };
    let mut local_keys = Vec::new();
    // Four fixed channels: admit bounded work/storage before allocation.
    names
        .budget()
        .charge(crate::compiler::expansion_budget::Counter::Work, 4)
        .map_err(|error| error.to_string())?;
    let _channel_bytes = names
        .budget()
        .reserve(
            crate::compiler::expansion_budget::Storage::Retained,
            (4 * std::mem::size_of::<NameKey>()) as u64,
        )
        .map_err(|error| error.to_string())?;
    local_keys
        .try_reserve_exact(4)
        .map_err(|_| "packed reserve channel allocation failed")?;
    for channel in [address, old, new, *unit] {
        let key = names
            .key(channel, LookupClass::Local)
            .map_err(native_adapter::name_diagnostic)?;
        if local_keys.contains(&key) {
            return Err("packed reserve aliases distinct source binder channels".into());
        }
        local_keys.push(key);
    }
    let (zero, then_zero, else_zero) =
        if_expression(&function.body.statements[1]).ok_or("packed reserve zero guard changed")?;
    let (left, right) =
        binary(zero, BinaryOp::Eq).ok_or("packed reserve zero comparison changed")?;
    if !variable(names, left, new)?
        || !integer(right, 0)
        || !return_zero(then_zero)
        || !empty(else_zero)
    {
        return Err("packed reserve zero guard operands changed".into());
    }
    let (overflow, then_overflow, else_overflow) = if_expression(&function.body.statements[2])
        .ok_or("packed reserve overflow guard changed")?;
    let (old_guard, new_guard) =
        binary(overflow, BinaryOp::Or).ok_or("packed reserve overflow order changed")?;
    for (guard, count) in [(old_guard, old), (new_guard, new)] {
        let (lhs, rhs) =
            binary(guard, BinaryOp::Gt).ok_or("packed reserve overflow comparison changed")?;
        let (maximum, divisor) =
            binary(rhs, BinaryOp::Div).ok_or("packed reserve overflow divisor changed")?;
        if !variable(names, lhs, count)?
            || !integer(maximum, u64::MAX as u128)
            || !variable(names, divisor, *unit)?
        {
            return Err("packed reserve overflow operands changed".into());
        }
    }
    if !return_zero(then_overflow) || !empty(else_overflow) {
        return Err("packed reserve overflow branches changed".into());
    }
    let Some(Expr::Call { name, args }) = function.body.tail.as_deref() else {
        return Err("packed reserve final callee changed".into());
    };
    if args.len() != 3 || !variable(names, &args[0], address)? {
        return Err("packed reserve ordered call channels changed".into());
    }
    for (argument, count) in [(&args[1], old), (&args[2], new)] {
        let (lhs, rhs) =
            binary(argument, BinaryOp::Mul).ok_or("packed reserve extent operation changed")?;
        if !variable(names, lhs, count)? || !variable(names, rhs, *unit)? {
            return Err("packed reserve extent channels changed".into());
        }
    }
    Ok(name)
}

fn original(bound: &BoundFunctionSource) -> Result<&SourceFunction, String> {
    let FunctionPhase::Resolved { original, .. } = &bound.source.phase else {
        return Err("packed reserve lost original correspondence".into());
    };
    Ok(original)
}

impl PackedReserveAuthority {
    fn issue(
        program: &Program,
        registry: &DeclRegistry,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let info = registry.declaration(declaration)?;
        let source = info
            .embedded_function
            .as_ref()
            .ok_or("packed reserve lacks original function authority")?;
        source.replay_admission(program, registry, declaration, budget)?;
        // Names select the operation only AFTER genuine source replay. They are
        // never an alternate path for issuing authority to copied user source.
        let root = program
            .prelude_module
            .ok_or("packed reserve has no embedded module root")?;
        let DeclItemName::Source(item) = info.path.item else {
            return Err("packed reserve declaration is not original source".into());
        };
        let module = program
            .modules
            .name(info.path.module)
            .map_err(module_diagnostic)?
            .ok_or("packed reserve module name absent")?;
        if program
            .modules
            .parent(info.path.module)
            .map_err(module_diagnostic)?
            != Some(root)
            || program
                .names
                .spelling(module)
                .map_err(native_adapter::name_diagnostic)?
                != "mem"
            || program
                .names
                .spelling(item)
                .map_err(native_adapter::name_diagnostic)?
                != "packed_reserve"
        {
            return Err("packed reserve operation is not its exact original declaration".into());
        }
        let function = program
            .functions
            .get(source.function_index)
            .ok_or("packed reserve source missing")?;
        let call = contract(function, &program.names)?;
        let key = name_scope::path_key(program.names.as_ref(), call, Namespace::Value)?;
        let raw_realloc = *registry
            .functions
            .get(&key)
            .ok_or("packed reserve callee not registered")?;
        let callee_info = registry.declaration(raw_realloc)?;
        let callee_source = callee_info
            .embedded_function
            .as_ref()
            .ok_or("packed reserve callee lacks original function authority")?;
        callee_source.replay_admission(program, registry, raw_realloc, budget)?;
        let callee = program
            .functions
            .get(callee_source.function_index)
            .ok_or("packed reserve callee missing")?;
        let item = callee
            .name
            .segments()
            .last()
            .ok_or("packed reserve callee name absent")?;
        if program
            .names
            .spelling(*item)
            .map_err(native_adapter::name_diagnostic)?
            != "raw_realloc"
            || callee.module != function.module
            || callee.params.len() != 3
            || !matches!(callee.result, Type::Int(ir::IntTy::Usize))
        {
            return Err(
                "packed reserve resolved target differs from original allocator declaration".into(),
            );
        }
        for index in 0..3 {
            parameter(callee, index)?;
        }
        let original_source = original(source)?;
        if !original_source
            .issuer
            .same_issuer(&original(callee_source)?.issuer)
        {
            return Err("packed reserve callee has a foreign embedded issuer".into());
        }
        let (query_event, query_token) = original_source
            .packed_query
            .ok_or("packed reserve lacks one original u8 query token/event correspondence")?;
        if !original_source
            .issuer
            .authorizes(program.names.session().sources(), query_token)
        {
            return Err("packed reserve original query token has foreign source".into());
        }
        budget.charge_work(1)?;
        Ok(Self {
            source: source.clone(),
            callee_source: callee_source.clone(),
            declaration,
            raw_realloc,
            query_event,
            query_token,
        })
    }

    pub(in crate::compiler) fn replay(
        &self,
        program: &Program,
        registry: &DeclRegistry,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let actual = Self::issue(program, registry, declaration, budget)?;
        if self != &actual {
            return Err(
                "packed reserve authority differs from current registered source/query occurrence"
                    .into(),
            );
        }
        Ok(())
    }
}
impl PlanCtx {
    pub(in crate::compiler) fn packed_reserve_authority(
        &self,
        declaration: ir::DeclId,
    ) -> Result<PackedReserveAuthority, String> {
        PackedReserveAuthority::issue(&self.program, &self.registry, declaration, &self.budget)
    }
}
