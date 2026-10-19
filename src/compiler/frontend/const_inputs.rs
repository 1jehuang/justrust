//! Transient per-callable direct constant input preparation and sealing.
//! No runtime constant lookup or evaluation occurs in synchronous checking.
use super::*;
use crate::compiler::{const_check, const_eval, const_source};

#[derive(Clone)]
pub(super) struct SourceConstantUse {
    pub(super) node: sr::SourceNode,
    pub(super) declaration: ir::DeclId,
    pub(super) value: Arc<const_check::ValueFact>,
}
pub(in crate::compiler) struct PreparedBodyConstants {
    declaration: ir::DeclId,
    inventory: Option<SourceInventory>,
    direct: Vec<ir::DeclId>,
}
impl PreparedBodyConstants {
    pub(in crate::compiler) fn direct(&self) -> &[ir::DeclId] {
        &self.direct
    }
}
pub(in crate::compiler) struct SealedBodyConstants {
    pub(super) declaration: ir::DeclId,
    pub(super) inventory: Option<SourceInventory>,
    pub(super) uses: Vec<SourceConstantUse>,
}
impl PlanCtx {
    pub(in crate::compiler) fn prepare_body_constants(
        &self,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<PreparedBodyConstants, FactsError> {
        self.check_budget(budget)
            .map_err(|e| source_error(budget, &e))?;
        let inventory = match self
            .callable_provider(declaration)
            .map_err(|e| source_error(budget, &e))?
        {
            CallableProvider::Source { .. } => {
                let function = self
                    .callable_body(&self.program, declaration)
                    .map_err(|e| source_error(budget, &e))?;
                Some(SourceInventory::build(
                    &self.program,
                    function,
                    sr::OriginalBodyId::new(budget)?,
                    budget,
                )?)
            }
            _ => None,
        };
        let mut direct = Vec::new();
        if let Some(inventory) = &inventory {
            for (_, key) in &inventory.constants {
                sr::work(budget, 1)?;
                let id = *self.registry.constants.get(key).ok_or_else(|| {
                    FactsError::invalid(
                        budget,
                        "source constant path absent from immutable registry",
                    )
                })?;
                sr::push(&mut direct, id, budget)?;
            }
        }
        sr::work(budget, direct.len().saturating_mul(usize::BITS as usize))?;
        direct.sort_unstable();
        direct.dedup();
        Ok(PreparedBodyConstants {
            declaration,
            inventory,
            direct,
        })
    }
    pub(in crate::compiler) fn seal_body_constants(
        &self,
        prepared: PreparedBodyConstants,
        values: Vec<Arc<const_check::ValueFact>>,
        budget: &ir::PlanBudget,
    ) -> Result<SealedBodyConstants, FactsError> {
        self.check_budget(budget)
            .map_err(|e| source_error(budget, &e))?;
        if values.len() != prepared.direct.len() {
            return Err(FactsError::invalid(
                budget,
                "constant input frontier coverage changed",
            ));
        }
        for (id, value) in prepared.direct.iter().zip(&values) {
            sr::work(budget, 1)?;
            let source = self
                .constant_source(*id)
                .map_err(|e| source_error(budget, &e))?;
            let ty = const_source::scalar_type(&source.ty).map_err(|e| source_error(budget, &e))?;
            if value.scalar().declaration() != *id
                || value.body().source().declaration != *id
                || value.scalar().value().ty() != ty
                || value.body().source().ty != ty
                || value.body().source().module != source.module
                || value.body().source().visibility != source.visibility
            {
                return Err(FactsError::invalid(
                    budget,
                    "constant graph input changed declaration/type/source identity",
                ));
            }
        }
        let mut uses = Vec::new();
        if let Some(inventory) = &prepared.inventory {
            for (node, key) in &inventory.constants {
                sr::work(budget, 1)?;
                let id = *self.registry.constants.get(key).ok_or_else(|| {
                    FactsError::invalid(budget, "constant occurrence lost declaration")
                })?;
                let index = prepared.direct.binary_search(&id).map_err(|_| {
                    FactsError::invalid(budget, "constant occurrence outside direct frontier")
                })?;
                sr::push(
                    &mut uses,
                    SourceConstantUse {
                        node: node.clone(),
                        declaration: id,
                        value: values[index].clone(),
                    },
                    budget,
                )?;
            }
        }
        Ok(SealedBodyConstants {
            declaration: prepared.declaration,
            inventory: prepared.inventory,
            uses,
        })
    }
}
impl CanonicalSourceCapability {
    pub(super) fn constant_value(&self, expr: &Expr) -> Result<const_eval::ScalarValue, String> {
        let index = *self
            .inventory
            .constant_pointers
            .get(&(expr as *const Expr as usize))
            .ok_or_else(|| {
                self.render(FactsError::invalid(
                    &self.plan.budget,
                    "constant lacks original source occurrence",
                ))
            })?;
        let row = self.constants.get(index).ok_or_else(|| {
            self.render(FactsError::invalid(
                &self.plan.budget,
                "constant occurrence lacks sealed graph value",
            ))
        })?;
        if row.node.owner != self.inventory.owner
            || row.declaration != row.value.scalar().declaration()
        {
            return Err(self.render(FactsError::invalid(
                &self.plan.budget,
                "constant input owner changed",
            )));
        }
        Ok(row.value.scalar().value())
    }
    pub(super) fn record_constant(&self, expr: &Expr, id: ir::ExprId) -> Result<(), String> {
        self.constant_value(expr)?;
        let index = *self
            .inventory
            .constant_pointers
            .get(&(expr as *const Expr as usize))
            .unwrap();
        sr::push(
            &mut self.constant_emissions.borrow_mut(),
            (index, id),
            &self.plan.budget,
        )
        .map_err(|e| self.render(e))
    }
}
pub(super) fn scalar_ir_type(ty: const_eval::ScalarTy) -> ir::Ty {
    match ty {
        const_eval::ScalarTy::Int(ty) => ir::Ty::Int(ty),
        const_eval::ScalarTy::Bool => ir::Ty::Bool,
    }
}
pub(super) fn validate_constants(
    original: &OriginalSource,
    fresh: &SourceInventory,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    if fresh.constants.len() != original.constants.len()
        || original.constant_emissions.len() != original.constants.len()
    {
        return Err(FactsError::invalid(
            budget,
            "constant original occurrence coverage changed",
        ));
    }
    for ((node, key), retained) in fresh.constants.iter().zip(&original.constants) {
        sr::work(budget, node.path.len() + 1)?;
        let id = original.plan.registry.constants.get(key).ok_or_else(|| {
            FactsError::invalid(budget, "constant source declaration disappeared")
        })?;
        if node != &retained.node
            || retained.declaration != *id
            || retained.value.scalar().declaration() != *id
            || retained.value.body().source().declaration != *id
        {
            return Err(FactsError::invalid(
                budget,
                "constant source occurrence/value publication changed",
            ));
        }
    }
    for (position, (occurrence, expression)) in original.constant_emissions.iter().enumerate() {
        sr::work(budget, position + 1)?;
        if original.constant_emissions[..position]
            .iter()
            .any(|(old_occurrence, old_expression)| {
                old_occurrence == occurrence || old_expression == expression
            })
        {
            return Err(FactsError::invalid(
                budget,
                "duplicate constant expression/source mapping",
            ));
        }
        let retained = original
            .constants
            .get(*occurrence)
            .ok_or_else(|| FactsError::invalid(budget, "orphan constant checked expression"))?;
        let expression = checked.exprs.get(expression.0).ok_or_else(|| {
            FactsError::invalid(budget, "constant checked expression disappeared")
        })?;
        let value = retained.value.scalar().value();
        let same = match (&expression.kind, value) {
            (ir::ExprKind::Int(actual), const_eval::ScalarValue::Int(expected)) => {
                *actual == expected
            }
            (ir::ExprKind::Bool(actual), const_eval::ScalarValue::Bool(expected)) => {
                *actual == expected
            }
            _ => false,
        };
        if !same || expression.ty != scalar_ir_type(value.ty()) {
            return Err(FactsError::invalid(
                budget,
                "constant graph value changed during source replay",
            ));
        }
    }
    Ok(())
}
