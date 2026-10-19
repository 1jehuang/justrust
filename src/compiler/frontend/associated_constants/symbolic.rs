//! Symbolic member uses in the immutable universal-source publication.
use super::*;

#[derive(Clone)]
pub(in crate::compiler::frontend) struct SourceSymbolicConstantUse {
    pub(in crate::compiler::frontend) node: sr::SourceNode,
    pub(in crate::compiler::frontend) operand: Arc<ir::AssociatedConstOperand>,
}
impl CanonicalSourceCapability {
    pub(in crate::compiler::frontend) fn symbolic_constant(
        &self,
        expression: &Expr,
    ) -> Result<Option<Arc<ir::AssociatedConstOperand>>, String> {
        if self.symbolic_constants.is_empty() || !matches!(expression, Expr::AssociatedConst(_)) {
            return Ok(None);
        }
        let index = *self
            .inventory
            .associated_constant_pointers
            .get(&(expression as *const Expr as usize))
            .ok_or("symbolic constant lost original source occurrence")?;
        let row = self
            .symbolic_constants
            .get(index)
            .ok_or("symbolic constant lacks universal source publication")?;
        if row.node.owner != self.inventory.owner
            || row.operand.caller() != self.declaration
            || row.operand.path() != row.node.path.as_ref()
        {
            return Err("symbolic constant changed original source owner/path".into());
        }
        row.operand.validate(&self.facts, &self.plan.budget)?;
        Ok(Some(row.operand.clone()))
    }
    pub(in crate::compiler::frontend) fn constant_type(
        &self,
        expression: &Expr,
    ) -> Result<ir::Ty, String> {
        if let Some(operand) = self.symbolic_constant(expression)? {
            return Ok(operand.ty().clone());
        }
        Ok(const_inputs::scalar_ir_type(
            self.constant_value(expression)?.ty(),
        ))
    }
    pub(in crate::compiler::frontend) fn constant_kind(
        &self,
        expression: &Expr,
    ) -> Result<ir::ExprKind, String> {
        if let Some(operand) = self.symbolic_constant(expression)? {
            return Ok(ir::ExprKind::AssociatedConst(operand));
        }
        Ok(match self.constant_value(expression)? {
            crate::compiler::const_eval::ScalarValue::Int(value) => ir::ExprKind::Int(value),
            crate::compiler::const_eval::ScalarValue::Bool(value) => ir::ExprKind::Bool(value),
        })
    }
}

pub(in crate::compiler::frontend) fn validate_symbolic_constants(
    original: &OriginalSource,
    fresh: &SourceInventory,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    if original.symbolic_constants.is_empty() {
        if !original.symbolic_constant_emissions.is_empty() {
            return Err(FactsError::invalid(
                budget,
                "orphan symbolic constant emissions",
            ));
        }
        return Ok(());
    }
    if original.symbolic_constants.len() != fresh.associated_constants.len()
        || original.symbolic_constant_emissions.len() != original.symbolic_constants.len()
    {
        return Err(FactsError::invalid(
            budget,
            "symbolic constant original occurrence coverage changed",
        ));
    }
    for ((node, source), row) in fresh
        .associated_constants
        .iter()
        .zip(&original.symbolic_constants)
    {
        sr::work(budget, node.path.len() + 1)?;
        let replay = original
            .plan
            .associated_constant_operand(
                original.declaration,
                source,
                node.path.clone(),
                &original.facts,
                budget,
            )
            .map_err(|e| source_error(budget, &e))?;
        if node != &row.node || &replay != row.operand.as_ref() {
            return Err(FactsError::invalid(
                budget,
                "symbolic constant changed exact retained member application",
            ));
        }
    }
    for (position, (index, id)) in original.symbolic_constant_emissions.iter().enumerate() {
        sr::work(budget, position + 1)?;
        if original.symbolic_constant_emissions[..position]
            .iter()
            .any(|(old_index, old_id)| old_index == index || old_id == id)
        {
            return Err(FactsError::invalid(
                budget,
                "duplicate symbolic member expression/source mapping",
            ));
        }
        let row = original
            .symbolic_constants
            .get(*index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan symbolic member emission"))?;
        let expression = checked.exprs.get(id.0).ok_or_else(|| {
            FactsError::invalid(budget, "symbolic member checked expression disappeared")
        })?;
        if !matches!(&expression.kind, ir::ExprKind::AssociatedConst(operand) if operand.as_ref() == row.operand.as_ref())
            || &expression.ty != row.operand.ty()
            || expression.completion != ir::Completion::Returns
        {
            return Err(FactsError::invalid(
                budget,
                "symbolic member type/value authority changed during replay",
            ));
        }
    }
    Ok(())
}
