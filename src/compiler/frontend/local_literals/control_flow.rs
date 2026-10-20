//! Body-local control-flow result constraints. These are inference hints only;
//! ordinary checking still validates target, reachability, moves and loans.
use super::*;

pub(super) struct ControlTarget {
    label: Option<NameKey>,
    result: Term,
    is_block: bool,
    return_depth: usize,
    has_break: bool,
}

impl Solver<'_, '_, '_> {
    pub(super) fn enter_control(
        &mut self,
        label: Option<AstIdent>,
        result: Term,
        is_block: bool,
    ) -> Result<(), String> {
        let label = self.checker.label_key(label)?;
        self.checker.infer_storage(1)?;
        self.control_targets.push(ControlTarget {
            label,
            result,
            is_block,
            return_depth: self.return_terms.len(),
            has_break: false,
        });
        Ok(())
    }

    pub(super) fn leave_control(&mut self) -> Result<bool, String> {
        self.control_targets
            .pop()
            .map(|target| target.has_break)
            .ok_or_else(|| "invalid metadata: missing inference control target".into())
    }

    fn control_target(
        &self,
        label: Option<AstIdent>,
        continuing: bool,
    ) -> Result<Option<usize>, String> {
        let label = self.checker.label_key(label)?;
        let index = self.control_targets.iter().rposition(|target| {
            target.return_depth == self.return_terms.len()
                && (label.is_none() || target.label == label)
        });
        let Some(index) = index else {
            // A nested hint query can start inside a loop owned by its caller.
            // Missing local context grants no target authority; ordinary checking
            // resolves that target and rejects genuinely out-of-scope transfers.
            return Ok(None);
        };
        if self.control_targets[index].is_block && (label.is_none() || continuing) {
            return Ok(None);
        }
        Ok(Some(index))
    }

    pub(super) fn infer_break(
        &mut self,
        label: Option<AstIdent>,
        value: Term,
    ) -> Result<Term, String> {
        if let Some(index) = self.control_target(label, false)? {
            let result = self.control_targets[index].result;
            self.coerce(value, result)?;
            self.control_targets[index].has_break = true;
        }
        self.fixed(ir::Ty::Never)
    }

    pub(super) fn infer_continue(&mut self, label: Option<AstIdent>) -> Result<Term, String> {
        self.control_target(label, true)?;
        self.fixed(ir::Ty::Never)
    }
}
