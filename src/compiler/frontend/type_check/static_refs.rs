//! `&S` of an immutable static item: one interned `&'static T` per declaration.
//! Only closed scalar and array initializers are admitted; any other static
//! borrow gets an explicit unsupported diagnostic.
use super::*;

impl Checker<'_> {
    pub(in crate::compiler::frontend) fn static_ref(
        &mut self,
        path: &AstPath,
    ) -> Result<ir::ExprId, String> {
        let data = self.static_data(path)?;
        self.charge_node(std::mem::size_of::<ir::TypedExpr>() + 32)?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: data.reference_ty(),
            completion: ir::Completion::Returns,
            kind: ir::ExprKind::StaticRef(data),
        });
        Ok(id)
    }
    pub(in crate::compiler::frontend) fn static_data(
        &self,
        path: &AstPath,
    ) -> Result<ir::StaticData, String> {
        let plan = self.signatures.plan()?;
        let budget = plan.budget.clone();
        // Scalar statics keep a constant declaration. Aggregate statics are
        // inlined per value use and keep only a checked zero-parameter
        // `const fn` body under the same canonical path. The resolver admits
        // `Expr::StaticRef` only for source `static` items.
        let key = self.path_key(path, Namespace::Value)?;
        let (declaration, ty, initializer) =
            if let Some(declaration) = plan.registry.constants.get(&key).copied() {
                let source = plan.constant_source(declaration)?;
                if source.storage != ConstantStorage::Static {
                    return Err("contract: static borrow names a constant item".into());
                }
                (declaration, &source.ty, &source.initializer)
            } else if let Some(declaration) = plan.registry.functions.get(&key).copied() {
                let body = plan.callable_body(&plan.program, declaration)?;
                if body.constness != Constness::Const
                    || !body.params.is_empty()
                    || !body.type_params.is_empty()
                    || !body.body.statements.is_empty()
                {
                    return Err("contract: static borrow names a non-static body".into());
                }
                let initializer = body
                    .body
                    .tail
                    .as_deref()
                    .ok_or("contract: static initializer body is empty")?;
                (declaration, &body.result, initializer)
            } else {
                return Err("contract: static item absent from registry".into());
            };
        let ty = static_type(ty, &budget, 0)?;
        let mut values = Vec::new();
        static_values(initializer, &ty, &mut values, &budget, 0)?;
        ir::StaticData::new(declaration, ty, values, &budget)
    }
}

const UNSUPPORTED: &str =
    "unsupported: borrowed statics require a closed scalar or array type and literal initializer";

fn static_type(ty: &Type, budget: &ir::PlanBudget, depth: usize) -> Result<ir::Ty, String> {
    budget.charge_work(1)?;
    if depth > 128 {
        return Err(UNSUPPORTED.into());
    }
    Ok(match ty {
        Type::Int(ty) => ir::Ty::Int(*ty),
        Type::Bool => ir::Ty::Bool,
        Type::Char => ir::Ty::Char,
        Type::Array(element, len) if *len <= ir::MAX_ARRAY_LEN => ir::Ty::Array {
            element: Arc::new(static_type(element, budget, depth + 1)?),
            len: *len,
        },
        _ => return Err(UNSUPPORTED.into()),
    })
}

fn static_values(
    expr: &Expr,
    ty: &ir::Ty,
    values: &mut Vec<ir::PromotedScalar>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.charge_work(1)?;
    if depth > 128 || values.len() > ir::MAX_ARRAY_LEN {
        return Err(UNSUPPORTED.into());
    }
    let scalar = match (expr, ty) {
        (Expr::Integer(literal), ir::Ty::Int(_)) => {
            ir::PromotedScalar::Int(resolve_integer(literal, false, Some(ty))?)
        }
        (
            Expr::Unary {
                op: UnaryOp::Neg,
                value,
                ..
            },
            ir::Ty::Int(_),
        ) => match value.as_ref() {
            Expr::Integer(literal) => {
                ir::PromotedScalar::Int(resolve_integer(literal, true, Some(ty))?)
            }
            _ => return Err(UNSUPPORTED.into()),
        },
        (Expr::Bool(value), ir::Ty::Bool) => ir::PromotedScalar::Bool(*value),
        (Expr::Char(value), ir::Ty::Char) => ir::PromotedScalar::Char(ir::CharValue {
            value: u32::from(*value),
        }),
        (Expr::Array(elements), ir::Ty::Array { element, len }) if elements.len() == *len => {
            for value in elements {
                static_values(value, element, values, budget, depth + 1)?;
            }
            return Ok(());
        }
        (Expr::Repeat { value, len: count }, ir::Ty::Array { element, len })
            if count == len && *len > 0 =>
        {
            let start = values.len();
            static_values(value, element, values, budget, depth + 1)?;
            let width = values.len() - start;
            for _ in 1..*len {
                for index in start..start + width {
                    if values.len() >= ir::MAX_ARRAY_LEN {
                        return Err(UNSUPPORTED.into());
                    }
                    budget.charge_work(1)?;
                    values.push(values[index]);
                }
            }
            return Ok(());
        }
        _ => return Err(UNSUPPORTED.into()),
    };
    scalar.validate()?;
    values.push(scalar);
    Ok(())
}
