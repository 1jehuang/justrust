//! Lexical source blocks retain every initializer and effect, not just the tail.
use super::*;

pub(super) fn syntax(
    block: &Block,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    for statement in &block.statements {
        sr::work(budget, 1)?;
        let value = match statement {
            Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::ImplicitExpr(value) => value,
            // Assignments and control flow require their own source-state transport.
            _ => return Ok(false),
        };
        if !super::syntax(value, budget, depth + 1)? {
            return Ok(false);
        }
    }
    match block.tail.as_deref() {
        Some(tail) => super::syntax(tail, budget, depth + 1),
        None => Ok(true),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn supported(
    block: &Block,
    node: &sr::SourceNode,
    plan: &PlanCtx,
    function: &Function,
    inventory: &SourceInventory,
    original: Option<&OriginalSource>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    let block_node = node.child(19, 0, budget)?;
    for (index, statement) in block.statements.iter().enumerate() {
        sr::work(budget, 1)?;
        let here = block_node.child(2, index, budget)?;
        let (value, value_node) = match statement {
            Stmt::Let { value, .. } => (value, here.child(3, 0, budget)?),
            Stmt::Expr(value) | Stmt::ImplicitExpr(value) => (value, here.child(4, 0, budget)?),
            _ => return Ok(false),
        };
        if !super::supported(
            value,
            &value_node,
            plan,
            function,
            inventory,
            original,
            budget,
            depth + 1,
        )? {
            return Ok(false);
        }
    }
    match block.tail.as_deref() {
        Some(tail) => super::supported(
            tail,
            &block_node.child(6, 0, budget)?,
            plan,
            function,
            inventory,
            original,
            budget,
            depth + 1,
        ),
        None => Ok(true),
    }
}

impl Replay<'_> {
    pub(super) fn lexical_block(
        &self,
        source: &Block,
        node: &sr::SourceNode,
        actual: &ir::TypedBlock,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        validate_original_occurrence(self.original, node, self.budget)?;
        if source.statements.len() != actual.statements.len() {
            return Err(FactsError::invalid(
                self.budget,
                "input lexical block statement count changed",
            ));
        }
        let block_node = node.child(19, 0, self.budget)?;
        for (index, (source, actual)) in
            source.statements.iter().zip(&actual.statements).enumerate()
        {
            sr::work(self.budget, 1)?;
            let here = block_node.child(2, index, self.budget)?;
            match (source, actual) {
                (Stmt::Let { .. }, ir::TypedStmt::Let { local, value }) => {
                    let binding =
                        source_binding_index(&self.original.bindings, &here, self.budget)?
                            .ok_or_else(|| {
                                FactsError::invalid(
                                    self.budget,
                                    "input block source binding missing",
                                )
                            })?;
                    let (root, initializer) =
                        initializer(self.original, binding, self.checked, self.budget)?;
                    if root != *local || initializer != *value {
                        return Err(FactsError::invalid(
                            self.budget,
                            "input block lexical initializer changed",
                        ));
                    }
                    self.binding(binding, depth + 1)?;
                }
                (Stmt::Expr(source) | Stmt::ImplicitExpr(source), ir::TypedStmt::Expr(value)) => {
                    self.expression(
                        source,
                        &here.child(4, 0, self.budget)?,
                        *value,
                        None,
                        depth + 1,
                    )?;
                }
                _ => {
                    return Err(FactsError::unsupported(
                        self.budget,
                        "input block requires source-owned initialized lets and expression effects",
                    ));
                }
            }
        }
        match (source.tail.as_deref(), actual.tail) {
            (Some(source), Some(value)) => self.expression(
                source,
                &block_node.child(6, 0, self.budget)?,
                value,
                expected,
                depth + 1,
            ),
            (None, None) => Ok(ir::Ty::Unit),
            _ => Err(FactsError::invalid(
                self.budget,
                "input block source tail changed",
            )),
        }
    }
}
