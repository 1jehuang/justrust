//! Dependency membership for already-established local structural terms.
use super::*;

impl Solver<'_, '_, '_> {
    pub(super) fn local_frontier_contains(
        &self,
        source: Term,
        needle: Term,
        depth: usize,
    ) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        if self.root(source) == self.root(needle) {
            return Ok(true);
        }
        // These edges already exist in the source solver. Following a tuple
        // item or reference does not infer its type, select a method, read a
        // nominal field, or authorize a coercion. Unknown nominal/opaque and
        // callable/projection producers retain their separate barriers.
        match self.domain(source) {
            Domain::Tuple(children) => {
                for child in children {
                    if self.local_frontier_contains(child, needle, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Domain::Ref(_, child)
            | Domain::RawPtr(_, child)
            | Domain::Array(child, _)
            | Domain::Slice(child) => self.local_frontier_contains(child, needle, depth + 1),
            Domain::DependentArray(element, length) => {
                Ok(self.local_frontier_contains(element, needle, depth + 1)?
                    || self.local_frontier_contains(length, needle, depth + 1)?)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn local_referents_ready(&self, targets: &[Term]) -> Result<bool, String> {
        for (actual, expected) in &self.pending_referents {
            self.checker.infer_work(0)?;
            let Domain::Ref(_, inner) = self.domain(*actual) else {
                return Ok(false);
            };
            if !matches!(self.domain(inner), Domain::Unknown)
                || !self.lazy_input_frontier(inner, targets, 0)?
                || !self.manifest_ready(&[*expected], 0)?
                || self.resolve(*expected, 0)?.is_none()
            {
                return Ok(false);
            }
        }
        // Do not equate the pending referent with its expected type. A local
        // input may reveal an extra reference layer, array, or Deref owner.
        // The ordinary coercion queue must replay that exact source after
        // independent lazy inputs become known.
        Ok(true)
    }

    /// Admit a pending arithmetic or unary result only when all of its
    /// operands are already frontier members or unsuffixed literals. Its
    /// Output is then a function of exactly those inputs, so later trait
    /// selection replays it; nothing is resolved or equated here. At most one
    /// new root per operator enters this bounded closure.
    pub(super) fn local_operator_results(&self, targets: &mut Vec<Term>) -> Result<(), String> {
        self.checker.infer_storage(self.operators.len())?;
        for _ in 0..self.operators.len() {
            let mut progress = false;
            for operator in &self.operators {
                self.checker.infer_work(0)?;
                let Some((left, right, result)) = operator.operand_result() else {
                    continue;
                };
                if !matches!(self.domain(result), Domain::Unknown) {
                    continue;
                }
                let mut member = false;
                for known in targets.iter() {
                    self.checker.infer_work(0)?;
                    member |= self.root(*known) == self.root(result);
                }
                if member {
                    continue;
                }
                let mut ready = true;
                for input in std::iter::once(left).chain(right) {
                    if !matches!(self.domain(input), Domain::Integer | Domain::Float)
                        && !self.lazy_input_frontier(input, targets, 0)?
                    {
                        ready = false;
                        break;
                    }
                }
                if ready {
                    targets.push(result);
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        Ok(())
    }
}
