//! Arithmetic sites are attached to the exact CFG operation emitted for a
//! checked expression. Borrow/diagnostic origins are not runtime-site authority:
//! synthetic compound updates can share an origin with their evaluated RHS.
use super::*;
use std::sync::Arc;

pub(super) type Row = (usize, usize, Arc<ir::RuntimeLocation>);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CfgRuntimeSites(Arc<[Row]>);

impl CfgRuntimeSites {
    pub(super) fn from_lowering(mut rows: Vec<Row>) -> Result<Self, String> {
        rows.sort_unstable_by_key(|row| (row.0, row.1));
        if rows
            .windows(2)
            .any(|pair| (pair[0].0, pair[0].1) == (pair[1].0, pair[1].1))
        {
            return Err("duplicate CFG arithmetic runtime site".into());
        }
        Ok(Self(rows.into()))
    }

    pub fn get(&self, block: BlockId, statement: usize) -> Option<&Arc<ir::RuntimeLocation>> {
        let index = self
            .0
            .binary_search_by_key(&(block.0, statement), |row| (row.0, row.1))
            .ok()?;
        Some(&self.0[index].2)
    }
}

impl Lowerer<'_> {
    pub(super) fn assign_source_runtime(
        &mut self,
        destination: Place,
        value: Rvalue,
        origin: Option<ExprId>,
    ) {
        let block = self.current;
        let statement = self.blocks[block.0].statements.len();
        self.assign(destination, value, origin);
        self.capture_runtime_statement(block, statement, origin);
    }

    pub(super) fn source_statement(&mut self, value: Statement, origin: Option<ExprId>) {
        let block = self.current;
        let statement = self.blocks[block.0].statements.len();
        self.statement(value, origin);
        self.capture_runtime_statement(block, statement, origin);
    }

    fn capture_runtime_statement(
        &mut self,
        block: BlockId,
        statement: usize,
        origin: Option<ExprId>,
    ) {
        if self.blocks[block.0].statements.len() != statement + 1 {
            return;
        }
        let Some(location) = origin
            .and_then(|id| self.source.runtime_sites.get(id))
            .cloned()
        else {
            return;
        };
        if let Err(error) = admit_cfg_push(&mut self.runtime_sites, self.budget) {
            self.divergence_resource(&error);
            return;
        }
        self.runtime_sites.push((block.0, statement, location));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_origins_cannot_authorize_synthetic_compound_traps() {
        use crate::compiler::frontend::{self, PlanCtx, PublishedCallableBody};
        for text in [
            "fn f(a:i32,b:i32)->i32{let mut x=1;x+=a+b;x}fn main(){}",
            "fn f(a:&mut [i32;1],i:usize){a[i+0]+=1;}fn main(){}",
        ] {
            let program = frontend::parse(text).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let PublishedCallableBody::Source(publication) = plan
                .publish_callable_body(plan.function_decl(0).unwrap(), &budget)
                .unwrap()
            else {
                panic!("expected source publication");
            };
            let function = lower_source_closed(&publication, &budget).unwrap();
            let mut authorized = 0;
            let mut synthetic = 0;
            for (block, data) in function.blocks.iter().enumerate() {
                for (index, statement) in data.statements.iter().enumerate() {
                    if matches!(
                        statement.value,
                        Statement::Assign {
                            value: Rvalue::Binary { .. },
                            ..
                        } | Statement::IndexAssignOp { .. }
                    ) {
                        assert!(statement.origin.is_some());
                        if function.runtime_sites.get(BlockId(block), index).is_some() {
                            authorized += 1;
                        } else {
                            synthetic += 1;
                        }
                    }
                }
            }
            assert_eq!(authorized, 1, "{text}");
            assert_eq!(synthetic, 1, "{text}");
        }
    }
}
