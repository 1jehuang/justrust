//! `&S` of an immutable static item: one interned `&'static T` per declaration.
//! Literal scalar/aggregate initializers and immutable promoted string/slice
//! leaves are stored once per declaration, with explicit native relocations.
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

    /// The declaration's closed scalar, aggregate and immutable reference
    /// storage type, read from its canonical graph type facts.
    fn static_type(&self, source: &Type, budget: &ir::PlanBudget) -> Result<ir::Ty, String> {
        // Lowering is a pure type query. The closed-leaf walk admits only
        // scalars and immutable promoted reference leaves.
        // This is the declaration's storage type, not a caller's explicit
        // generic actual. Its lifetimes were checked on the initializer body.
        // Do not reinterpret its 'static regions in the caller's environment.
        let ty = self.source_type_actual(source)?;
        self.facts().validate_ty_metadata_with_budget(&ty, budget)?;
        if ty.contains_slice(budget)? {
            ir::validate_slice_carrier_with_facts(&ty, self.facts())?;
        }
        closed_static_shape(&ty, budget, 0)?;
        Ok(ty)
    }

    fn static_values(
        &self,
        expr: &Expr,
        ty: &ir::Ty,
        values: &mut Vec<ir::StaticValue>,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        if depth > 128 || values.len() > ir::MAX_ARRAY_LEN {
            return Err(UNSUPPORTED.into());
        }
        match (expr, ty) {
            (Expr::Tuple(elements), ir::Ty::Unit) if elements.is_empty() => Ok(()),
            (Expr::Block(block), ir::Ty::Unit)
                if block.statements.is_empty() && block.tail.is_none() =>
            {
                Ok(())
            }
            (Expr::Str(text), ty) if ty.is_shared_str() => {
                budget.retain_bytes(text.len())?;
                values.push(ir::StaticValue::Text(Arc::from(text.as_str())));
                Ok(())
            }
            (
                Expr::BorrowValue {
                    kind: ir::BorrowKind::Shared,
                    value,
                },
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                },
            ) => {
                let ir::Ty::Slice { element } = referent.as_ref() else {
                    return Err(UNSUPPORTED.into());
                };
                let len = match value.as_ref() {
                    Expr::Array(elements) => elements.len(),
                    Expr::Repeat { len, .. } => *len,
                    _ => return Err(UNSUPPORTED.into()),
                };
                if len > ir::MAX_ARRAY_LEN {
                    return Err(UNSUPPORTED.into());
                }
                let array = ir::Ty::Array {
                    element: element.clone(),
                    len,
                };
                let mut contents = Vec::new();
                self.static_values(value, &array, &mut contents, budget, depth + 1)?;
                values.push(ir::StaticValue::Slice {
                    len,
                    values: contents.into(),
                });
                Ok(())
            }
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

/// The payload consists only of closed scalar or immutable promoted reference
/// leaves. Enums and mutable reference payloads stay unsupported.
fn closed_static_shape(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<(), String> {
    budget.charge_work(1)?;
    if depth > 128 {
        return Err(UNSUPPORTED.into());
    }
    match ty {
        ir::Ty::Unit | ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char => Ok(()),
        ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent,
        } => match referent.as_ref() {
            ir::Ty::Str => Ok(()),
            ir::Ty::Slice { element } => closed_static_shape(element, budget, depth + 1),
            _ => Err(UNSUPPORTED.into()),
        },
        ir::Ty::Array { element, len } if *len <= ir::MAX_ARRAY_LEN => {
            closed_static_shape(element, budget, depth + 1)
        }
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                closed_static_shape(field, budget, depth + 1)?;
            }
            Ok(())
        }
        ir::Ty::Struct(structure) => {
            for field in structure.def().fields.read(budget)?.iter() {
                closed_static_shape(&field.ty, budget, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(UNSUPPORTED.into()),
    }
}

const UNSUPPORTED: &str = "unsupported: borrowed static initializer requires closed scalar, string or shared-slice leaves through arrays, tuples and structs";

/// Scalar leaves, arrays and repeats. `nested` checks one element.
fn static_values(
    expr: &Expr,
    ty: &ir::Ty,
    values: &mut Vec<ir::StaticValue>,
    budget: &ir::PlanBudget,
    _depth: usize,
    mut nested: impl FnMut(&Expr, &ir::Ty, &mut Vec<ir::StaticValue>) -> Result<(), String>,
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
        (Expr::Repeat { value, len: count }, ir::Ty::Array { element, len }) if count == len => {
            if *len == 0 {
                return Ok(());
            }
            let start = values.len();
            nested(value, element, values)?;
            let width = values.len() - start;
            for _ in 1..*len {
                for index in start..start + width {
                    if values.len() >= ir::MAX_ARRAY_LEN {
                        return Err(UNSUPPORTED.into());
                    }
                    budget.charge_work(1)?;
                    values.push(values[index].clone());
                }
            }
            return Ok(());
        }
        _ => return Err(UNSUPPORTED.into()),
    };
    scalar.validate()?;
    values.push(ir::StaticValue::Scalar(scalar));
    Ok(())
}
