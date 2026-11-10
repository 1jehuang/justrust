//! Name-to-slot resolution for const parameter values and repeat extents.
//! Names are used only at the source boundary. Published operands carry the
//! resolved declaration owner and source-ordered slot, never display text.
use super::type_check::Signatures;
use super::*;

pub(super) fn array_type(
    element: ir::Ty,
    argument: &ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, String> {
    budget.charge_work(1)?;
    budget.retain_bytes(std::mem::size_of::<ir::Ty>())?;
    let ty = match argument {
        ir::Ty::Const(value) => {
            value.closed_key()?;
            let len = value
                .as_usize()
                .ok_or("E0308: array length must be usize")?;
            if len > ir::MAX_ARRAY_TYPE_LEN {
                return Err("array length limit exceeded".into());
            }
            ir::Ty::Array {
                element: Arc::new(element),
                len,
            }
        }
        ir::Ty::Param(parameter) => {
            let operand = ir::ConstOperand::new(
                ir::ConstParamId {
                    owner: parameter.owner,
                    index: parameter.index,
                },
                caller,
                budget,
            )?;
            if operand.scalar_type() != ir::ConstType::Int(ir::IntTy::Usize) {
                return Err("E0308: array length const parameter must have type usize".into());
            }
            ir::Ty::DependentArray {
                element: Arc::new(element),
                length: operand.parameter(),
            }
        }
        _ => return Err("E0747: array length requires a const parameter".into()),
    };
    caller.validate_ty_with_budget(&ty, budget)?;
    Ok(ty)
}

pub(super) fn parameter_value(
    signatures: &Signatures,
    name: AstIdent,
    budget: &ir::PlanBudget,
) -> Result<Option<ir::ConstOperand>, String> {
    let plan = signatures.plan()?;
    budget.charge_work(1)?;
    let key = plan.registry.parameter_key(name)?;
    let Some(ir::Ty::Param(parameter)) = signatures.params.get(&key) else {
        return Ok(None);
    };
    let facts = if let Some(facts) = signatures.facts.as_deref() {
        ir::FactsCtx::Validated(facts)
    } else if let Some(env) = signatures.param_env.as_deref() {
        ir::FactsCtx::Universal(env)
    } else {
        return Err("const parameter name lacks its authoritative declaration environment".into());
    };
    let kind = match facts {
        ir::FactsCtx::Validated(facts) => facts.env().parameter_kind(*parameter),
        ir::FactsCtx::Universal(env) => env.parameter_kind(*parameter),
        ir::FactsCtx::Concrete => None,
    };
    if !matches!(kind, Some(ir::GenericParamKind::Const(_))) {
        return Ok(None);
    }
    ir::ConstOperand::new(
        ir::ConstParamId {
            owner: parameter.owner,
            index: parameter.index,
        },
        facts,
        budget,
    )
    .map(Some)
}

pub(super) fn parameter_path_value(
    signatures: &Signatures,
    path: &AstPath,
    budget: &ir::PlanBudget,
) -> Result<Option<ir::ConstOperand>, String> {
    path.check(signatures.plan()?.program.names.as_ref())
        .map_err(native_adapter::name_diagnostic)?;
    let (PathAnchor::Relative, [name]) = (path.anchor(), path.segments()) else {
        return Ok(None);
    };
    parameter_value(signatures, *name, budget)
}
