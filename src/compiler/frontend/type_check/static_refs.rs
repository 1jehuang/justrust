//! `&S` of an immutable static item: one interned `&'static T` per declaration.
//! Admitted types have only integer, `bool` and `char` leaves under arrays,
//! tuples and structs, with literal initializers. Any other static borrow
//! gets an explicit unsupported diagnostic.
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
        let ty = self.static_type(ty, &budget)?;
        let mut values = Vec::new();
        self.static_values(initializer, &ty, &mut values, &budget, 0)?;
        ir::StaticData::new(declaration, ty, values, &budget)
    }

    /// The declared type, admitted when every leaf is an integer, `bool` or
    /// `char` reached through arrays, tuples and structs.
    fn static_type(&self, source: &Type, budget: &ir::PlanBudget) -> Result<ir::Ty, String> {
        // Lowering is a pure type query; the closed-leaf walk below is what
        // admits the type (no references, enums, generics or drop glue).
        let ty = self
            .checked_type_actual(source)
            .map_err(|_| UNSUPPORTED.to_string())?;
        closed_static_shape(&ty, budget, 0)?;
        Ok(ty)
    }

    fn static_values(
        &self,
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
        match (expr, ty) {
            (Expr::Tuple(elements), ir::Ty::Tuple(fields)) if elements.len() == fields.len() => {
                for (value, field) in elements.iter().zip(fields.iter()) {
                    self.static_values(value, field, values, budget, depth + 1)?;
                }
                Ok(())
            }
            (Expr::TupleStruct { name, fields }, ir::Ty::Struct(structure))
                if self.static_constructor(name, structure)? =>
            {
                let definition = structure.def();
                let declared = definition.fields.read(budget)?;
                if fields.len() != declared.len() {
                    return Err("E0061: tuple struct constructor arity mismatch".into());
                }
                for (value, field) in fields.iter().zip(declared.iter()) {
                    self.static_values(value, &field.ty, values, budget, depth + 1)?;
                }
                Ok(())
            }
            (Expr::Struct { name, fields }, ir::Ty::Struct(structure))
                if self.static_constructor(name, structure)? =>
            {
                let declared = structure.def().fields.read(budget)?;
                let mut ordered: Vec<Option<&Expr>> = vec![None; declared.len()];
                for (field, value) in fields {
                    let (index, _) = self.field_type(ty, &FieldSelector::Named(*field))?;
                    let slot = ordered.get_mut(index).ok_or(UNSUPPORTED)?;
                    if slot.replace(value).is_some() {
                        return Err("E0062: field specified more than once".into());
                    }
                }
                for (value, field) in ordered.into_iter().zip(declared.iter()) {
                    let value = value.ok_or("E0063: missing field in static initializer")?;
                    self.static_values(value, &field.ty, values, budget, depth + 1)?;
                }
                Ok(())
            }
            _ => static_values(expr, ty, values, budget, depth, |value, element, values| {
                self.static_values(value, element, values, budget, depth + 1)
            }),
        }
    }

    /// The constructor path names exactly the static's declared struct.
    fn static_constructor(&self, name: &AstPath, structure: &ir::StructTy) -> Result<bool, String> {
        Ok(self.nominal_decl(name)? == structure.def().source_decl)
    }
}

/// Every leaf is a closed integer, `bool` or `char`: the payload is exactly
/// its flattened scalars, with no references, padding-visible enums or drops.
fn closed_static_shape(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<(), String> {
    budget.charge_work(1)?;
    if depth > 128 {
        return Err(UNSUPPORTED.into());
    }
    match ty {
        ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char => Ok(()),
        ir::Ty::Array { element, len } if *len <= ir::MAX_ARRAY_LEN => {
            closed_static_shape(element, budget, depth + 1)
        }
        ir::Ty::Tuple(fields) if !fields.is_empty() => {
            for field in fields.iter() {
                closed_static_shape(field, budget, depth + 1)?;
            }
            Ok(())
        }
        ir::Ty::Struct(structure) if !structure.def().fields.is_empty() => {
            for field in structure.def().fields.read(budget)?.iter() {
                closed_static_shape(&field.ty, budget, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(UNSUPPORTED.into()),
    }
}

const UNSUPPORTED: &str = "unsupported: borrowed statics require integer, bool or char leaves (through arrays, tuples and structs) and a literal initializer";

/// Scalar leaves, arrays and repeats. `nested` checks one element.
fn static_values(
    expr: &Expr,
    ty: &ir::Ty,
    values: &mut Vec<ir::PromotedScalar>,
    budget: &ir::PlanBudget,
    _depth: usize,
    mut nested: impl FnMut(&Expr, &ir::Ty, &mut Vec<ir::PromotedScalar>) -> Result<(), String>,
) -> Result<(), String> {
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
                nested(value, element, values)?;
            }
            return Ok(());
        }
        (Expr::Repeat { value, len: count }, ir::Ty::Array { element, len })
            if count == len && *len > 0 =>
        {
            let start = values.len();
            nested(value, element, values)?;
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
