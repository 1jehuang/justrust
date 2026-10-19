//! Source-authenticated nonmoving index probes. These are explicit checked
//! operations, not reads of a possibly non-Copy element or diagnostic origins.
use super::*;

impl type_check::Checker<'_> {
    pub(super) fn assignment_bounds_probe(
        &mut self,
        statement: ir::Stmt,
        site: super::super::expansion_span::SpanId,
        statements: &mut Vec<ir::Stmt>,
    ) -> Result<ir::Stmt, String> {
        let (array, index, value, op) = match statement {
            ir::Stmt::AssignIndex {
                array,
                index,
                value,
            } => (array, index, value, None),
            ir::Stmt::AssignIndexOp {
                array,
                index,
                value,
                op,
            } => (array, index, value, Some(op)),
            other => return Ok(other),
        };
        let element =
            match ir::indexed_array_ty(&self.locals[array.local.0].ty, &array.projection, false)? {
                ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                    element.as_ref().clone()
                }
                _ => return Err("indexed assignment probe requires array or slice".into()),
            };
        let saved_value =
            self.temporary(&format!("$bv{}", self.locals.len()), element.clone(), false)?;
        let saved_index = self.temporary(
            &format!("$bx{}", self.locals.len()),
            ir::Ty::Int(ir::IntTy::Usize),
            false,
        )?;
        self.charge_node(2 * std::mem::size_of::<ir::Stmt>())?;
        statements.push(ir::Stmt::Let {
            local: saved_value,
            value,
        });
        statements.push(ir::Stmt::Let {
            local: saved_index,
            value: index,
        });
        let index = self.for_expr(
            ir::Ty::Int(ir::IntTy::Usize),
            ir::ExprKind::Local(saved_index),
        )?;
        let value = self.for_expr(element, ir::ExprKind::Local(saved_value))?;
        self.charge_node(array.projection.len() * std::mem::size_of::<ir::Projection>())?;
        let probe_index = self.for_expr(
            ir::Ty::Int(ir::IntTy::Usize),
            ir::ExprKind::Local(saved_index),
        )?;
        let probe = self.for_expr(
            ir::Ty::Unit,
            ir::ExprKind::BoundsCheck {
                array: array.clone(),
                index: probe_index,
            },
        )?;
        self.record_runtime_span(site, probe)?;
        self.charge_node(std::mem::size_of::<ir::Stmt>())?;
        statements.push(ir::Stmt::Expr(probe));
        Ok(match op {
            Some(op) => ir::Stmt::AssignIndexOp {
                array,
                index,
                value,
                op,
            },
            None => ir::Stmt::AssignIndex {
                array,
                index,
                value,
            },
        })
    }

    pub(super) fn index_bounds_probe(
        &mut self,
        array: &ir::Place,
        index: ir::ExprId,
        original: &Expr,
        statements: &mut Vec<ir::Stmt>,
    ) -> Result<(), String> {
        let id = self.for_expr(
            ir::Ty::Unit,
            ir::ExprKind::BoundsCheck {
                array: array.clone(),
                index,
            },
        )?;
        self.exprs[id.0].completion = self.exprs[index.0].completion;
        if self.exprs[id.0].completion == ir::Completion::Never {
            self.exprs[id.0].ty = ir::Ty::Never;
        }
        self.record_runtime_site(original, id)?;
        self.charge_node(std::mem::size_of::<ir::Stmt>())?;
        statements.push(ir::Stmt::Expr(id));
        Ok(())
    }
}
