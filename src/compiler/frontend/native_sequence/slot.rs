//! Exact original native raw-slot recipe. This is neither arbitrary pointer
//! provenance nor a write envelope. Its consumer must independently bind a
//! receiver's issued buffer and initialized-element summary.
use super::*;

#[derive(Clone, Debug)]
pub struct NativeSequenceSlot {
    declaration: ir::DeclId,
    _source: crate::compiler::token_session::OriginalSpan,
    issuer: crate::compiler::source::EmbeddedOriginalFile,
    _stride: NativeStrideSource,
}

#[derive(Clone, Debug)]
struct NativeStrideSource {
    _declaration: ir::DeclId,
    _source: crate::compiler::token_session::OriginalSpan,
    _issuer: crate::compiler::source::EmbeddedOriginalFile,
}

impl NativeSequenceSlot {
    pub(in crate::compiler) fn matches_owner(
        &self,
        declaration: ir::DeclId,
        owner: &ir::StructTy,
    ) -> bool {
        self.declaration == declaration
            && owner.native_sequence_binding().is_some_and(|binding| {
                binding.matches_declaration(owner.def())
                    && binding._issuer.same_issuer(&self.issuer)
            })
    }
}

fn same_name(
    names: &NameStore,
    a: AstIdent,
    b: AstIdent,
    class: LookupClass,
) -> Result<bool, String> {
    Ok(names
        .key(a, class)
        .map_err(native_adapter::name_diagnostic)?
        == names
            .key(b, class)
            .map_err(native_adapter::name_diagnostic)?)
}
fn parameter_ty(names: &NameStore, ty: &Type, parameter: AstIdent) -> Result<bool, String> {
    match ty {
        Type::Param(actual) => same_name(names, *actual, parameter, LookupClass::TypeParameter),
        _ => Ok(false),
    }
}
fn raw_parameter_ty(
    names: &NameStore,
    ty: &Type,
    parameter: AstIdent,
    kind: ir::BorrowKind,
) -> Result<bool, String> {
    match ty {
        Type::RawPtr {
            kind: actual,
            pointee,
        } if *actual == kind => parameter_ty(names, pointee, parameter),
        _ => Ok(false),
    }
}
fn variable(names: &NameStore, value: &Expr, expected: AstIdent) -> Result<bool, String> {
    match value {
        Expr::Var(path) if path.segments().len() == 1 => {
            same_name(names, path.segments()[0], expected, LookupClass::Local)
        }
        _ => Ok(false),
    }
}
fn usize_cast(value: &Expr) -> Option<&Expr> {
    match value {
        Expr::Cast {
            value,
            target: Type::Int(ir::IntTy::Usize),
        } => Some(value),
        _ => None,
    }
}
fn binary(value: &Expr, expected: BinaryOp) -> Option<(&Expr, &Expr)> {
    match value {
        Expr::Binary {
            op, left, right, ..
        } if *op == expected => Some((left, right)),
        _ => None,
    }
}
fn usize_literal(value: &Expr, expected: u128) -> bool {
    matches!(value, Expr::Integer(value)
        if value.magnitude == expected && value.suffix == Some(ir::IntTy::Usize))
}

fn original_function(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<
    Option<(
        crate::compiler::token_session::OriginalSpan,
        crate::compiler::source::EmbeddedOriginalFile,
    )>,
    String,
> {
    budget.charge_work(1)?;
    let Some(function) = registry.source_callable(declaration) else {
        return Ok(None);
    };
    let info = registry.declaration(declaration)?;
    let names = registry.program.names.as_ref();
    budget.charge_work(function.name.segments().len().saturating_add(1))?;
    let key = name_scope::path_key(names, &function.name, Namespace::Value)?;
    if registry.functions.get(&key) != Some(&declaration)
        || info.path.module != function.module
        || info.visibility != function.visibility
    {
        return Ok(None);
    }
    let Some(member) = function.name.segments().last() else {
        return Ok(None);
    };
    let span = names
        .span(*member)
        .map_err(native_adapter::name_diagnostic)?;
    let crate::compiler::expansion_span::SpanOrigin::Original(source) = names
        .session()
        .span(span)
        .map_err(|error| error.to_string())?
        .origin
    else {
        return Ok(None);
    };
    for issuer in registry.program.embedded_issuers.iter() {
        budget.charge_work(1)?;
        if issuer.authorizes(names.session().sources(), source) {
            return Ok(Some((source, issuer.clone())));
        }
    }
    Ok(None)
}

/// Check the entire fixed source calculation of the public element stride:
/// adjacent `[T;2]` addresses at 4096, without dereferencing their contents.
fn stride_source(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Option<NativeStrideSource>, String> {
    budget.charge_work(96)?;
    let Some((source, issuer)) = original_function(registry, declaration, budget)? else {
        return Ok(None);
    };
    let function = registry
        .source_callable(declaration)
        .ok_or("native stride source disappeared")?;
    let names = registry.program.names.as_ref();
    let [parameter] = function.type_params.as_slice() else {
        return Ok(None);
    };
    if !function.params.is_empty()
        || !function.param_mut.is_empty()
        || function.regions.receiver.is_some()
        || function.unsafety
        || function.constness != Constness::Runtime
        || function.static_mut
        || function.impl_params != 0
        || !function.bounds.is_empty()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
        || !matches!(function.result, Type::Int(ir::IntTy::Usize))
        || registry.declaration(declaration)?.param_env.arity() != 1
    {
        return Ok(None);
    }
    let [
        Stmt::Let {
            name: BindingName::Named(base),
            mutable: false,
            ty: Type::Infer,
            value: base_value,
        },
        Stmt::Let {
            name: BindingName::Named(second),
            mutable: false,
            ty: Type::Infer,
            value: second_value,
        },
    ] = function.body.statements.as_slice()
    else {
        return Ok(None);
    };
    let Expr::Cast {
        value,
        target:
            Type::RawPtr {
                kind: ir::BorrowKind::Shared,
                pointee,
            },
    } = base_value
    else {
        return Ok(None);
    };
    let Type::Array(element, 2) = pointee.as_ref() else {
        return Ok(None);
    };
    if !usize_literal(value, 4096) || !parameter_ty(names, element, *parameter)? {
        return Ok(None);
    }
    let Expr::Cast { value, target } = second_value else {
        return Ok(None);
    };
    if !raw_parameter_ty(names, target, *parameter, ir::BorrowKind::Shared)? {
        return Ok(None);
    }
    let Expr::Unsafe(block) = value.as_ref() else {
        return Ok(None);
    };
    if !block.statements.is_empty() {
        return Ok(None);
    }
    let Some(Expr::BorrowValue {
        kind: ir::BorrowKind::Shared,
        value,
    }) = block.tail.as_deref()
    else {
        return Ok(None);
    };
    let Expr::Index {
        base: indexed,
        index: 1,
        ..
    } = value.as_ref()
    else {
        return Ok(None);
    };
    let Expr::Deref { value } = indexed.as_ref() else {
        return Ok(None);
    };
    if !variable(names, value, *base)? {
        return Ok(None);
    }
    let Some(tail) = function.body.tail.as_deref() else {
        return Ok(None);
    };
    let Some((left, right)) = binary(tail, BinaryOp::Sub) else {
        return Ok(None);
    };
    let Some(left) = usize_cast(left) else {
        return Ok(None);
    };
    if !variable(names, left, *second)? || !usize_literal(right, 4096) {
        return Ok(None);
    }
    Ok(Some(NativeStrideSource {
        _declaration: declaration,
        _source: source,
        _issuer: issuer,
    }))
}

/// Source schema issuance, never inferred from a printed callee name. The exact
/// original declaration must calculate `base + index * size_of::<T>()` and its
/// stride declaration must independently match the complete adjacent-element
/// recipe above. Raw fixtures and alternate schema adapters cannot mint this.
pub(in crate::compiler::frontend) fn issue_slot(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Option<Arc<NativeSequenceSlot>>, String> {
    budget.charge_work(96)?;
    let Some((source, issuer)) = original_function(registry, declaration, budget)? else {
        return Ok(None);
    };
    let function = registry
        .source_callable(declaration)
        .ok_or("native slot source disappeared")?;
    let names = registry.program.names.as_ref();
    let [parameter] = function.type_params.as_slice() else {
        return Ok(None);
    };
    let [
        (BindingName::Named(base), base_ty),
        (BindingName::Named(index), Type::Int(ir::IntTy::Usize)),
    ] = function.params.as_slice()
    else {
        return Ok(None);
    };
    if function.unsafety
        || function.constness != Constness::Runtime
        || function.static_mut
        || function.impl_params != 0
        || !function.bounds.is_empty()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
        || function.param_mut.as_slice() != [false, false]
        || function.regions.receiver.is_some()
        || !function.body.statements.is_empty()
        || registry.declaration(declaration)?.param_env.arity() != 1
        || !raw_parameter_ty(names, base_ty, *parameter, ir::BorrowKind::Mut)?
        || !raw_parameter_ty(names, &function.result, *parameter, ir::BorrowKind::Mut)?
    {
        return Ok(None);
    }
    let Some(Expr::Cast { value, target }) = function.body.tail.as_deref() else {
        return Ok(None);
    };
    if !raw_parameter_ty(names, target, *parameter, ir::BorrowKind::Mut)? {
        return Ok(None);
    }
    let Some((left, right)) = binary(value, BinaryOp::Add) else {
        return Ok(None);
    };
    let Some(left) = usize_cast(left) else {
        return Ok(None);
    };
    if !variable(names, left, *base)? {
        return Ok(None);
    }
    let Some((left, right)) = binary(right, BinaryOp::Mul) else {
        return Ok(None);
    };
    if !variable(names, left, *index)? {
        return Ok(None);
    }
    let Expr::Apply { path, value } = right else {
        return Ok(None);
    };
    let Expr::Call { name, args } = value.as_ref() else {
        return Ok(None);
    };
    if !args.is_empty() {
        return Ok(None);
    }
    budget.charge_work(path.segments.len().saturating_add(1))?;
    let (_, arguments) = path.application()?;
    let [argument] = arguments else {
        return Ok(None);
    };
    if !parameter_ty(names, argument, *parameter)? {
        return Ok(None);
    }
    for segment in &path.segments {
        budget.charge_work(1)?;
        if !segment.region_args.is_empty() || !segment.associated_types.is_empty() {
            return Ok(None);
        }
    }
    let path = path.path(names)?;
    let key = name_scope::path_key(names, &path, Namespace::Value)?;
    if name_scope::path_key(names, name, Namespace::Value)? != key {
        return Ok(None);
    }
    let Some(stride) = registry.functions.get(&key).copied() else {
        return Ok(None);
    };
    let Some(stride) = stride_source(registry, stride, budget)? else {
        return Ok(None);
    };
    budget.charge_nodes(1)?;
    budget.retain_bytes(
        std::mem::size_of::<NativeSequenceSlot>() + 2 * std::mem::size_of::<usize>(),
    )?;
    Ok(Some(Arc::new(NativeSequenceSlot {
        declaration,
        _source: source,
        issuer,
        _stride: stride,
    })))
}
