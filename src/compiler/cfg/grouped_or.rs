//! Grouped source alternatives converge before the one source guard/body.
use super::*;

impl Lowerer<'_> {
    pub(super) fn grouped_or_arm(
        &mut self,
        id: ExprId,
        source: &Place,
        rows: &[ir::MatchPattern],
        arm: &ir::MatchArm,
        destination: &Place,
        completes: bool,
        join: BlockId,
    ) -> Result<(), String> {
        self.divergence_work(rows.len())?;
        // Retry temporaries belong to this arm, not the surrounding match or
        // loop. Other arms never execute their StorageLive instructions.
        self.push_scope()?;
        let attempt = self.temporary(Ty::Int(IntTy::Usize), Some(id));
        let retry = self.temporary(Ty::Bool, Some(id));
        let guard_entry = self.block();
        let rejected = self.block();
        let mut entries = Vec::with_capacity(rows.len() + 1);
        entries.push(self.current);
        for _ in 1..rows.len() {
            entries.push(self.block());
        }
        entries.push(rejected);
        let mut bindings = None;
        for (index, row) in rows.iter().enumerate() {
            self.current = entries[index];
            let tested = self.pattern_test(id, source, row, completes)?;
            self.current = tested.selected;
            for local in tested.temporaries.iter().rev() {
                self.end_storage(*local, Some(id));
            }
            self.push_scope()?;
            self.initialize_pattern_bindings(id, source, row, completes)?;
            if bindings.is_none() {
                bindings = self.scopes.last().cloned();
            }
            self.assign(
                attempt.clone(),
                Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::Usize,
                    bits: index as u128,
                }))),
                Some(id),
            );
            self.terminate(Terminator::Goto(guard_entry), Some(id))?;
            self.scopes.pop();
            self.current = tested.rejected;
            for local in tested.temporaries.iter().rev() {
                self.end_storage(*local, Some(id));
            }
            self.terminate(Terminator::Goto(entries[index + 1]), Some(id))?;
        }
        self.current = guard_entry;
        self.push_scope()?;
        *self.scopes.last_mut().unwrap() = bindings.unwrap_or_default();
        if let Some(guard) = arm.guard {
            let condition = self.expression(guard)?;
            let pass = self.block();
            let fail = self.block();
            self.terminate(
                Terminator::Switch {
                    condition,
                    then_block: pass,
                    else_block: fail,
                },
                Some(id),
            )?;
            self.current = fail;
            self.cleanup(self.scopes.len() - 1, Some(id));
            // Only the matched row's continuation is eligible for retry.
            for index in 0..rows.len() - 1 {
                let next = self.block();
                let condition = retry.clone();
                self.assign(
                    condition.clone(),
                    Rvalue::Binary {
                        op: BinaryOp::Eq,
                        left: Operand::Copy(attempt.clone()),
                        right: Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::Usize,
                            bits: index as u128,
                        })),
                    },
                    Some(id),
                );
                self.terminate(
                    Terminator::Switch {
                        condition: Operand::Copy(condition),
                        then_block: entries[index + 1],
                        else_block: next,
                    },
                    Some(id),
                )?;
                self.current = next;
            }
            self.terminate(Terminator::Goto(rejected), Some(id))?;
            self.current = pass;
        }
        let value = self.expression(arm.body)?;
        if completes {
            self.assign(destination.clone(), Rvalue::Use(value), Some(id));
        }
        self.cleanup(self.scopes.len() - 1, Some(id));
        self.scopes.pop();
        self.cleanup(self.scopes.len() - 1, Some(id));
        self.terminate(Terminator::Goto(join), Some(id))?;
        self.current = rejected;
        self.cleanup(self.scopes.len() - 1, Some(id));
        self.scopes.pop();
        Ok(())
    }
}
