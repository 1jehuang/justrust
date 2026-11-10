//! Exact safe function item reification, shared by casts and coercion sites.
use super::*;

impl Checker<'_> {
    /// Inference equations at authored argument coercion sites only. Cached
    /// source evidence remains unchanged and ordinary checking emits PtrCast.
    pub(super) fn function_pointer_inference_equation(
        &self,
        formal: &ir::Ty,
        evidence: &InferTerm,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        match (formal, evidence.as_ref()) {
            (ir::Ty::FnPointer(_), InferKind::Rigid(ir::Ty::FnItem(item))) => {
                let facts = self
                    .facts()
                    .validated()
                    .ok_or("pointer inference requires canonical caller facts")?;
                let ty = facts
                    .function_pointer_item_type(item)
                    .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
                self.infer_type_term(&ty, None, depth + 1)
            }
            (ir::Ty::Tuple(parameters), InferKind::Tuple(inputs))
                if parameters.len() == inputs.len() =>
            {
                self.infer_storage(inputs.len())?;
                let fields = parameters
                    .iter()
                    .zip(inputs.iter())
                    .map(|(formal, input)| {
                        self.function_pointer_inference_equation(formal, input, depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.infer_node(InferKind::Tuple(fields.into()))
            }
            (
                ir::Ty::Array { element, .. } | ir::Ty::DependentArray { element, .. },
                InferKind::Array(actual, len),
            ) => {
                let actual =
                    self.function_pointer_inference_equation(element, actual, depth + 1)?;
                self.infer_node(InferKind::Array(actual, *len))
            }
            _ => Ok(evidence.clone()),
        }
    }

    pub(super) fn coerce_function_pointer_value(
        &mut self,
        value: ir::ExprId,
        target: &ir::Ty,
    ) -> Result<ir::ExprId, String> {
        let source = &self.exprs[value.0];
        if source.ty == *target || source.ty == ir::Ty::Never {
            return Ok(value);
        }
        if matches!(source.ty, ir::Ty::Closure(_)) {
            return Err("unsupported: closure to function pointer coercion requires a canonical native body adapter".into());
        }
        ir::validate_pointer_cast(&source.ty, target, self.facts())?;
        let completion = source.completion;
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: target.clone(),
            completion,
            kind: ir::ExprKind::PtrCast { value },
        });
        Ok(id)
    }
}
