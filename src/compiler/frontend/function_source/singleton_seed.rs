//! Finite authored-occurrence capsule for the native singleton seed shape.
//! This records syntax, not a storage role. Admission still replays the full
//! canonical owner/constructor/append recipes in native_sequence::element.
use super::*;

fn authored_path(
    names: &impl NameAccess,
    value: &AstPath,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<(), String> {
    let authored = value
        .original_use(names)
        .map_err(native_adapter::name_diagnostic)?;
    emit(FunctionSyntaxEvent::OriginalPath(
        authored.anchor(),
        authored.segments().len(),
    ))?;
    for name in authored.segments() {
        emit(FunctionSyntaxEvent::Name(*name))?;
    }
    Ok(())
}
fn parameter_type(
    names: &impl NameAccess,
    value: &Type,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    let name = match value {
        Type::Param(name) => *name,
        Type::Named(path) => {
            let authored = path
                .original_use(names)
                .map_err(native_adapter::name_diagnostic)?;
            let (Some(actual), Some(original)) = (path.single(), authored.single()) else {
                return Ok(false);
            };
            // Only the exact unqualified authored parameter use may normalize
            // to Type::Param. A qualified or retargeted path is different syntax.
            if crate::compiler::semantic_name::occurrence_index(names, actual)
                .map_err(native_adapter::name_diagnostic)?
                != crate::compiler::semantic_name::occurrence_index(names, original)
                    .map_err(native_adapter::name_diagnostic)?
            {
                return Ok(false);
            }
            original
        }
        _ => return Ok(false),
    };
    emit(FunctionSyntaxEvent::SeedParameterType)?;
    emit(FunctionSyntaxEvent::Name(name))?;
    Ok(true)
}
fn applied_type(
    names: &impl NameAccess,
    value: &Type,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    let Type::Apply(path) = value else {
        return Ok(false);
    };
    let Some(last) = path.segments.last() else {
        return Ok(false);
    };
    for (index, segment) in path.segments.iter().enumerate() {
        if !segment.region_args.is_empty()
            || !segment.associated_types.is_empty()
            || (index + 1 != path.segments.len() && !segment.args.is_empty())
        {
            return Ok(false);
        }
    }
    let [argument] = last.args.as_slice() else {
        return Ok(false);
    };
    emit(FunctionSyntaxEvent::SeedAppliedType)?;
    authored_path(names, &path.path(names)?, emit)?;
    parameter_type(names, argument, emit)
}
fn variable(
    names: &impl NameAccess,
    value: &Expr,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    let Expr::Var(path) = value else {
        return Ok(false);
    };
    emit(FunctionSyntaxEvent::Variable)?;
    authored_path(names, path, emit)?;
    Ok(true)
}
fn constructor(
    names: &impl NameAccess,
    value: &Expr,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    emit(FunctionSyntaxEvent::SeedConstructor)?;
    let arguments = match value {
        Expr::Call { name, args } => {
            let authored = name
                .original_use(names)
                .map_err(native_adapter::name_diagnostic)?;
            let Some((member, owner)) = authored.segments().split_last() else {
                return Ok(false);
            };
            if owner.is_empty() {
                return Ok(false);
            }
            // The resolver certifies the inferred owner with the complete
            // authored call path, including its member. Replay that identical
            // occurrence on both sides of the parser/resolver boundary.
            authored_path(names, name, emit)?;
            emit(FunctionSyntaxEvent::Name(*member))?;
            args
        }
        Expr::AssociatedCall {
            owner: Type::Apply(owner),
            name,
            type_args,
            args,
        } => {
            // This is precisely the resolver's marker for an unwritten impl
            // argument, not permission to erase an authored concrete argument.
            let (_, actuals) = owner.application()?;
            if !matches!(actuals, [Type::Infer]) || !type_args.is_empty() {
                return Ok(false);
            }
            for segment in &owner.segments {
                if !segment.region_args.is_empty() || !segment.associated_types.is_empty() {
                    return Ok(false);
                }
            }
            authored_path(names, &owner.path(names)?, emit)?;
            emit(FunctionSyntaxEvent::Name(*name))?;
            args
        }
        _ => return Ok(false),
    };
    let [argument] = arguments.as_slice() else {
        return Ok(false);
    };
    variable(names, argument, emit)
}

pub(super) fn recipe(
    names: &impl NameAccess,
    function: &Function,
    emit: &mut impl FnMut(FunctionSyntaxEvent) -> Result<(), String>,
) -> Result<bool, String> {
    let [parameter] = function.type_params.as_slice() else {
        return Ok(false);
    };
    if function.static_mut
        || !function.bounds.is_empty()
        || function.impl_params != 0
        || function.regions.receiver.is_some()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
        || (!function.param_mut.is_empty() && function.param_mut.len() != function.params.len())
    {
        return Ok(false);
    }
    let [(first, element), (capacity, Type::Int(ir::IntTy::Usize))] = function.params.as_slice()
    else {
        return Ok(false);
    };
    let [
        Stmt::Let {
            name,
            mutable: true,
            ty: annotation,
            value,
        },
        Stmt::Expr(Expr::MethodCall {
            receiver,
            name: append,
            type_args,
            args,
        }),
    ] = function.body.statements.as_slice()
    else {
        return Ok(false);
    };
    let [appended] = args.as_slice() else {
        return Ok(false);
    };
    if !type_args.is_empty() {
        return Ok(false);
    }
    let Some(tail) = function.body.tail.as_deref() else {
        return Ok(false);
    };
    emit(FunctionSyntaxEvent::SingletonSeed)?;
    authored_path(names, &function.name, emit)?;
    emit(FunctionSyntaxEvent::Name(*parameter))?;
    emit(FunctionSyntaxEvent::Parameter(
        binding(*first),
        function.param_mut.first().copied().unwrap_or(false),
    ))?;
    if !parameter_type(names, element, emit)? {
        return Ok(false);
    }
    emit(FunctionSyntaxEvent::Parameter(
        binding(*capacity),
        function.param_mut.get(1).copied().unwrap_or(false),
    ))?;
    emit(FunctionSyntaxEvent::IntType(ir::IntTy::Usize))?;
    if !applied_type(names, &function.result, emit)? {
        return Ok(false);
    }
    emit(FunctionSyntaxEvent::Block(2, true))?;
    emit(FunctionSyntaxEvent::Let(binding(*name), true))?;
    if !applied_type(names, annotation, emit)? || !constructor(names, value, emit)? {
        return Ok(false);
    }
    emit(FunctionSyntaxEvent::SeedAppend)?;
    emit(FunctionSyntaxEvent::Name(*append))?;
    Ok(variable(names, receiver, emit)?
        && variable(names, appended, emit)?
        && variable(names, tail, emit)?)
}
