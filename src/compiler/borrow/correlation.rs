//! Path-correlated coexistence of two explicit loan issuances.
//! Four worlds are retained: neither, first, second, or both origins live.
//! Joins union worlds. They never turn alternative predecessors into aliases.
use super::*;

#[derive(Clone, Copy)]
struct Worlds(u8);
impl Worlds {
    fn map(self, mut transform: impl FnMut(u8) -> u8) -> Self {
        let mut output = 0;
        for world in 0..4 {
            if self.0 & (1 << world) != 0 {
                output |= 1 << transform(world);
            }
        }
        Self(output)
    }
    fn exclusive(self) -> bool {
        self.0 & 8 == 0 && self.0 & 10 != 0 && self.0 & 12 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joining_alternative_worlds_never_invents_coexistence() {
        let first_only = Worlds(1 << 1);
        let second_only = Worlds(1 << 2);
        let joined = Worlds(first_only.0 | second_only.0);
        assert!(joined.exclusive());
        assert!(!Worlds(joined.0 | (1 << 3)).exclusive());
        assert!(!Worlds(0).exclusive());
        assert!(!first_only.exclusive());
        // Issuing the other loan while the first remains live is simultaneous.
        assert!(!first_only.map(|world| world | 2).exclusive());
    }

    #[test]
    fn unknown_origins_and_exhaustion_cannot_issue_exclusion_proofs() {
        use crate::compiler::frontend;
        let program = frontend::parse(
            "fn probe<T>(){let a=1;let b=2;let x=&a;let y=&b;let _z=(*x,*y);} fn main(){}",
        )
        .unwrap();
        let source_budget = ir::PlanBudget::default();
        let source = frontend::PlanCtx::from_program(&program, &source_budget).unwrap();
        let declaration = source.function_decl(0).unwrap();
        let frontend::PublishedCallableBody::Source(publication) = source
            .publish_callable_body(declaration, &source_budget)
            .unwrap()
        else {
            panic!("source required")
        };
        let template = cfg::lower_source_template(&publication, &source_budget).unwrap();
        let function = template.function();
        assert!(function.loan_sites.len() >= 2);
        let plan = Plan::build_validated(
            function,
            template.facts(),
            None,
            &mut Budget::new(32_000_000),
            128 * 1024 * 1024,
        )
        .unwrap();
        assert!(
            !plan
                .exclusive_origins(
                    function,
                    0,
                    function.loan_sites.len(),
                    function.entry,
                    Some(0),
                    Phase::Read,
                    &mut Budget::new(0)
                )
                .unwrap()
        );
        assert!(
            !plan
                .exclusive_origins(
                    function,
                    usize::MAX,
                    0,
                    function.entry,
                    Some(0),
                    Phase::Read,
                    &mut Budget::new(0)
                )
                .unwrap()
        );
        assert!(
            plan.exclusive_origins(
                function,
                0,
                1,
                function.entry,
                Some(0),
                Phase::Read,
                &mut Budget::new(0)
            )
            .is_err()
        );
    }
}

impl Plan {
    fn pair_at_point(
        &self,
        worlds: Worlds,
        point: &Point,
        first: usize,
        second: usize,
        budget: &mut Budget,
    ) -> Result<Worlds, Exhausted> {
        budget.spend(4)?;
        let active = self.active(&point.origins, &point.live, budget)?;
        let mask = u8::from(active.has(first)) | (u8::from(active.has(second)) << 1);
        Ok(worlds.map(|world| world & mask))
    }

    fn pair_through_block(
        &self,
        f: &cfg::Function,
        block: BlockId,
        mut worlds: Worlds,
        first: usize,
        second: usize,
        stop: Option<(Option<usize>, Phase)>,
        budget: &mut Budget,
    ) -> Result<Option<Worlds>, Exhausted> {
        let body = &f.blocks[block.0];
        let stop_index = stop.map(|(statement, phase)| {
            let index = statement.unwrap_or(body.statements.len());
            index + usize::from(matches!(phase, Phase::Write) && index < body.statements.len())
        });
        for index in 0..=body.statements.len() {
            budget.spend(1)?;
            let Some(point) = self.points[self.offsets[block.0] + index].as_ref() else {
                return Ok(None);
            };
            worlds = self.pair_at_point(worlds, point, first, second, budget)?;
            let outgoing_query = stop.is_some_and(|(statement, phase)| {
                matches!(phase, Phase::Write)
                    && statement.unwrap_or(body.statements.len()) == body.statements.len()
            });
            if stop_index == Some(index) && !outgoing_query {
                return Ok(Some(worlds));
            }
            if let Some(statement) = body.statements.get(index)
                && let Statement::Assign {
                    value: Rvalue::Borrow(LoanId(id)),
                    ..
                } = statement.value
            {
                let issued = if id == first {
                    1
                } else if id == second {
                    2
                } else {
                    0
                };
                worlds = worlds.map(|world| world | issued);
            }
        }
        let Some(outgoing) = self.outgoing[block.0].as_ref() else {
            return Ok(None);
        };
        self.pair_at_point(worlds, outgoing, first, second, budget)
            .map(Some)
    }

    /// Absence of the simultaneous world is usable only for two actual explicit
    /// issuances. Parameters, returned carriers and incomplete evidence retain
    /// the ordinary conservative conflict check.
    pub(super) fn exclusive_origins(
        &self,
        f: &cfg::Function,
        first: usize,
        second: usize,
        block: BlockId,
        statement: Option<usize>,
        phase: Phase,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        if first == second || first >= f.loan_sites.len() || second >= f.loan_sites.len() {
            return Ok(false);
        }
        let mut births = [0usize; 2];
        for body in &f.blocks {
            budget.spend(body.statements.len() + 1)?;
            for statement in &body.statements {
                if let Statement::Assign {
                    value: Rvalue::Borrow(LoanId(id)),
                    ..
                } = statement.value
                {
                    births[0] += usize::from(id == first);
                    births[1] += usize::from(id == second);
                }
            }
        }
        if births != [1, 1] {
            return Ok(false);
        }
        budget.reserve_bytes(mul(
            f.blocks.len(),
            size_of::<u8>() + size_of::<bool>() + size_of::<usize>(),
        )?)?;
        let mut entries = vec![0u8; f.blocks.len()];
        let mut queued = vec![false; f.blocks.len()];
        let mut pending = VecDeque::with_capacity(f.blocks.len());
        // Explicit origins have not been issued on function entry.
        entries[f.entry.0] = 1;
        queued[f.entry.0] = true;
        pending.push_back(f.entry);
        while let Some(current) = pending.pop_front() {
            budget.spend(1)?;
            queued[current.0] = false;
            let Some(output) = self.pair_through_block(
                f,
                current,
                Worlds(entries[current.0]),
                first,
                second,
                None,
                budget,
            )?
            else {
                return Ok(false);
            };
            for next in successors(&f.blocks[current.0].terminator.value)
                .into_iter()
                .flatten()
            {
                budget.spend(1)?;
                let joined = entries[next.0] | output.0;
                if joined != entries[next.0] {
                    entries[next.0] = joined;
                    if !queued[next.0] {
                        pending.push_back(next);
                        queued[next.0] = true;
                    }
                }
            }
        }
        if entries[block.0] == 0 {
            return Ok(false);
        }
        Ok(self
            .pair_through_block(
                f,
                block,
                Worlds(entries[block.0]),
                first,
                second,
                Some((statement, phase)),
                budget,
            )?
            .is_some_and(Worlds::exclusive))
    }
}
