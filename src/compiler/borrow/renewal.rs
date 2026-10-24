//! Conservative per-loan renewal proof over the existing solved carrier facts.
//!
//! An SCC edge is not itself permission to reuse a static loan identity. Every
//! path from that edge to a new issuance must first reach a point where the old
//! loan (including transitive reborrow dependencies) is dead. Joined carrier
//! facts overapproximate paths, so absence is a proof and presence is not.
use super::*;

impl Plan {
    pub(super) fn killed_before_rebirth(
        &self,
        f: &cfg::Function,
        start: BlockId,
        loan: usize,
        call_issuer: Option<BlockId>,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        // A returned carrier origin has no Borrow statement: its single
        // issuance point is the call terminator of `call_issuer`. Any other
        // origin without an explicit Borrow site keeps the existing boundary.
        if loan >= f.loan_sites.len() && call_issuer.is_none() {
            return Ok(false);
        }
        budget.reserve_bytes(mul(f.blocks.len(), size_of::<usize>() + size_of::<bool>())?)?;
        let mut discovered = vec![false; f.blocks.len()];
        let mut pending = Vec::with_capacity(f.blocks.len());
        discovered[start.0] = true;
        pending.push(start);
        // A pending path carries exactly one proposition: the previous dynamic
        // issuance has not yet been proved dead. Thus visiting a block once is
        // sufficient. A cycle with no reissuance cannot create a second loan.
        while let Some(block) = pending.pop() {
            budget.spend(1)?;
            let body = &f.blocks[block.0];
            let mut killed = false;
            for index in 0..=body.statements.len() {
                budget.spend(1)?;
                let Some(point) = &self.points[self.offsets[block.0] + index] else {
                    return Ok(false);
                };
                if !self.active(&point.origins, &point.live, budget)?.has(loan) {
                    killed = true;
                    break;
                }
                if let Some(statement) = body.statements.get(index)
                    && matches!(statement.value, Statement::Assign {
                        value: Rvalue::Borrow(LoanId(id)), ..
                    } if id == loan)
                {
                    return Ok(false);
                }
                // The terminator point of the issuing block reissues a
                // returned carrier origin while the old one is still live.
                if index == body.statements.len() && call_issuer == Some(block) {
                    return Ok(false);
                }
            }
            if !killed {
                for next in successors(&body.terminator.value).into_iter().flatten() {
                    if !discovered[next.0] {
                        discovered[next.0] = true;
                        pending.push(next);
                    }
                }
            }
        }
        Ok(true)
    }
}
