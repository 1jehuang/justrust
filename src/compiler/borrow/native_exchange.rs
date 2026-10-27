//! A paired exchange is not append authority. Both loads and both stores refer
//! to one initialized receiver summary and no imported origin can enter it.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct NativeStorageExchange {
    cell: usize,
    parameter: ir::ParamId,
    reads: [(BlockId, usize); 2],
    writes: [(BlockId, usize); 2],
    _source: Arc<super::super::frontend::NativeSequenceSwap>,
}
impl NativeStorageExchange {
    pub(super) fn writes(&self, block: BlockId, statement: usize) -> bool {
        self.writes.contains(&(block, statement))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Owner,
    IndexA,
    IndexB,
    Buffer,
    SlotA,
    SlotB,
    ElementA,
    ElementB,
    Unit,
}
#[derive(Clone, Copy, Default)]
struct Slot {
    token: Option<Token>,
    defined: bool,
    live: bool,
    entry: bool,
}
struct Trace {
    locals: Vec<Slot>,
    stage: usize,
    reads: [(BlockId, usize); 2],
    writes: [(BlockId, usize); 2],
}
impl Trace {
    fn new(
        count: usize,
        parameters: &[LocalId],
        ledger: &ir::PlanBudget,
        budget: &mut Budget,
    ) -> Result<Option<Self>, Exhausted> {
        ledger.charge_work(1).map_err(|_| Exhausted)?;
        let [owner, a, b] = parameters else {
            return Ok(None);
        };
        budget.reserve_bytes(mul(count, size_of::<Slot>())?)?;
        budget.nodes(1)?;
        budget.spend(count.saturating_add(1))?;
        let mut locals = vec![Slot::default(); count];
        for (local, token) in [
            (*owner, Token::Owner),
            (*a, Token::IndexA),
            (*b, Token::IndexB),
        ] {
            let Some(slot) = locals.get_mut(local.0) else {
                return Ok(None);
            };
            if slot.defined {
                return Ok(None);
            }
            *slot = Slot {
                token: Some(token),
                defined: true,
                live: true,
                entry: true,
            };
        }
        Ok(Some(Self {
            locals,
            stage: 0,
            reads: [(BlockId(usize::MAX), usize::MAX); 2],
            writes: [(BlockId(usize::MAX), usize::MAX); 2],
        }))
    }
    fn operand(&mut self, operand: &Operand) -> Option<Token> {
        let (place, moving) = match operand {
            Operand::Copy(place) => (place, false),
            Operand::Move(place) => (place, true),
            Operand::Constant(cfg::Constant::Unit) => return Some(Token::Unit),
            _ => return None,
        };
        let slot = self.locals.get_mut(place.local.0)?;
        if !slot.live {
            return None;
        }
        let token = match place.projection.as_slice() {
            [] => slot.token?,
            [Projection::Deref, Projection::Field(0)] if slot.token == Some(Token::Owner) => {
                Token::Buffer
            }
            _ => return None,
        };
        if moving && place.projection.is_empty() {
            slot.token = None
        }
        Some(token)
    }
    fn define(&mut self, local: LocalId, token: Token) -> bool {
        let Some(slot) = self.locals.get_mut(local.0) else {
            return false;
        };
        // Unit-only synthetic return plumbing carries no element or pointer.
        if slot.entry
            || (slot.defined && slot.token != Some(Token::Unit))
            || (token != Token::Unit && (!slot.live || slot.defined))
        {
            return false;
        }
        slot.token = Some(token);
        slot.defined = true;
        true
    }
    fn statement(
        &mut self,
        statement: &Statement,
        block: BlockId,
        index: usize,
        ledger: &ir::PlanBudget,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        ledger.charge_work(1).map_err(|_| Exhausted)?;
        budget.spend(1)?;
        match statement {
            Statement::StorageLive(local) => {
                let Some(slot) = self.locals.get_mut(local.0) else {
                    return Ok(false);
                };
                if slot.live || slot.defined {
                    return Ok(false);
                }
                slot.live = true;
                Ok(true)
            }
            Statement::StorageDead(local) => {
                let Some(slot) = self.locals.get_mut(local.0) else {
                    return Ok(false);
                };
                if slot.entry && self.stage != 4 {
                    return Ok(false);
                }
                slot.live = false;
                slot.token = None;
                Ok(true)
            }
            Statement::Assign {
                destination,
                value: Rvalue::Use(operand),
            } => {
                if destination.projection.as_slice() == [Projection::Deref] {
                    let target = self
                        .locals
                        .get(destination.local.0)
                        .filter(|slot| slot.live)
                        .and_then(|slot| slot.token);
                    let source = self.operand(operand);
                    let expected = match self.stage {
                        2 => (Token::SlotA, Token::ElementB),
                        3 => (Token::SlotB, Token::ElementA),
                        _ => return Ok(false),
                    };
                    if (target, source) != (Some(expected.0), Some(expected.1)) {
                        return Ok(false);
                    }
                    self.writes[self.stage - 2] = (block, index);
                    self.stage += 1;
                    return Ok(true);
                }
                if !destination.projection.is_empty() {
                    return Ok(false);
                }
                let raw = match operand {
                    Operand::Copy(place) | Operand::Move(place)
                        if place.projection.as_slice() == [Projection::Deref] =>
                    {
                        Some(place)
                    }
                    _ => None,
                };
                let token = if let Some(place) = raw {
                    let pointer = self
                        .locals
                        .get(place.local.0)
                        .filter(|slot| slot.live)
                        .and_then(|slot| slot.token);
                    let (expected, result) = match self.stage {
                        0 => (Token::SlotA, Token::ElementA),
                        1 => (Token::SlotB, Token::ElementB),
                        _ => return Ok(false),
                    };
                    if pointer != Some(expected) {
                        return Ok(false);
                    }
                    self.reads[self.stage] = (block, index);
                    self.stage += 1;
                    result
                } else {
                    let Some(token) = self.operand(operand) else {
                        return Ok(false);
                    };
                    token
                };
                Ok(self.define(destination.local, token))
            }
            _ => Ok(false),
        }
    }
}

fn enters(terminator: &Terminator, destination: BlockId) -> bool {
    match terminator {
        Terminator::Goto(next) | Terminator::Call { next, .. } | Terminator::Drop { next, .. } => {
            *next == destination
        }
        Terminator::Switch {
            then_block,
            else_block,
            ..
        }
        | Terminator::SwitchVariant {
            then_block,
            else_block,
            ..
        } => *then_block == destination || *else_block == destination,
        Terminator::Return | Terminator::Panic(_) | Terminator::Unreachable => false,
    }
}

impl CarrierBuilder<'_, '_> {
    pub(super) fn prepare_native_exchange(&mut self) -> Result<(), Exhausted> {
        let FactsCtx::Validated(facts) = self.facts else {
            return Ok(());
        };
        if facts.phase() != super::super::traits::FactsPhase::Universal {
            return Ok(());
        }
        let Some(body) = self.body.clone() else {
            return Ok(());
        };
        let source = match facts.native_sequence_swap(facts.declaration()) {
            Ok(Some(source)) => source,
            Ok(None) => return Ok(()),
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(error) => {
                let message = error
                    .into_diagnostic(facts.budget())
                    .map_err(|_| Exhausted)?;
                self.problem(self.f.entry, None, Problem::Invalid(message))?;
                return Ok(());
            }
        };
        let f = self.f;
        if let Err(message) = f.validate_provenance(facts, &self.ledger) {
            let error = super::super::traits::FactsError::from_message(&self.ledger, &message);
            if error.is_resource() {
                return Err(Exhausted);
            }
            self.problem(f.entry, None, Problem::Invalid(message))?;
            return Ok(());
        }
        let [receiver, _, _] = f.params.as_slice() else {
            return Ok(());
        };
        let Ty::Ref {
            kind: BorrowKind::Mut,
            referent,
        } = &f.locals[receiver.0].ty
        else {
            return Ok(());
        };
        let Ty::Struct(owner) = referent.as_ref() else {
            return Ok(());
        };
        let Some(Ty::Param(parameter)) = referent.owned_sequence_element() else {
            return Ok(());
        };
        let parameter = *parameter;
        // Canonical source has exactly two calls. No unrelated call is covered
        // by this receipt and neither a partial nor extra raw operation passes.
        let mut calls = [None, None];
        let mut count = 0;
        let mut raw_reads = 0usize;
        let mut raw_writes = 0usize;
        for (block_index, block) in f.blocks.iter().enumerate() {
            self.budget
                .spend(block.statements.len().saturating_add(1))?;
            if let Terminator::Call {
                target: ir::Callee::Direct(target),
                signature,
                args,
                destination,
                ..
            } = &block.terminator.value
            {
                if count == 2
                    || !source.matches_owner(facts.declaration(), owner, target.declaration)
                    || target.type_args.as_ref() != [Ty::Param(parameter)]
                    || args.len() != 2
                    || !destination.projection.is_empty()
                    || self
                        .calls
                        .get(block_index)
                        .and_then(Option::as_ref)
                        .is_none()
                    || !matches!(signature.params.as_slice(),[Ty::RawPtr {kind:BorrowKind::Mut,pointee},Ty::Int(ir::IntTy::Usize)] if pointee.as_ref()==&Ty::Param(parameter))
                    || !matches!(&signature.result,Ty::RawPtr {kind:BorrowKind::Mut,pointee} if pointee.as_ref()==&Ty::Param(parameter))
                {
                    return Ok(());
                }
                calls[count] = Some(BlockId(block_index));
                count += 1;
            } else if matches!(block.terminator.value, Terminator::Call { .. }) {
                return Ok(());
            }
            for statement in &block.statements {
                let mut failure = false;
                let budget = &mut self.budget;
                visit_rvalue_for_statement(f, &statement.value, |place, _| {
                    if budget
                        .spend(place.projection.len().saturating_add(1))
                        .is_err()
                    {
                        failure = true;
                        return;
                    }
                    match ir::raw_deref_index_optional(
                        &f.locals[place.local.0].ty,
                        &place.projection,
                        Some(&self.ledger),
                    ) {
                        Ok(Some(_)) => raw_reads = raw_reads.saturating_add(1),
                        Ok(None) => {}
                        Err(_) => failure = true,
                    }
                });
                if failure {
                    return Err(Exhausted);
                }
                if let Statement::Assign { destination, .. } = &statement.value
                    && ir::raw_deref_index_optional(
                        &f.locals[destination.local.0].ty,
                        &destination.projection,
                        Some(&self.ledger),
                    )
                    .map_err(|_| Exhausted)?
                    .is_some()
                {
                    raw_writes = raw_writes.saturating_add(1)
                }
            }
        }
        if count != 2 || raw_reads != 2 || raw_writes != 2 {
            return Ok(());
        }
        let [Some(first), Some(second)] = calls else {
            return Ok(());
        };
        let Some(mut trace) = Trace::new(f.locals.len(), &f.params, &self.ledger, self.budget)?
        else {
            return Ok(());
        };
        let mut current = first;
        for ordinal in 0..3 {
            let block = &f.blocks[current.0];
            for (index, statement) in block.statements.iter().enumerate() {
                if !trace.statement(&statement.value, current, index, &self.ledger, self.budget)? {
                    return Ok(());
                }
            }
            if ordinal == 2 {
                if !matches!(block.terminator.value, Terminator::Return) || trace.stage != 4 {
                    return Ok(());
                }
            } else {
                let Terminator::Call {
                    args,
                    destination,
                    next,
                    ..
                } = &block.terminator.value
                else {
                    return Ok(());
                };
                let expected = if ordinal == 0 {
                    Token::IndexA
                } else {
                    Token::IndexB
                };
                if trace.stage != 0
                    || trace.operand(&args[0]) != Some(Token::Buffer)
                    || trace.operand(&args[1]) != Some(expected)
                    || !trace.define(
                        destination.local,
                        if ordinal == 0 {
                            Token::SlotA
                        } else {
                            Token::SlotB
                        },
                    )
                    || (ordinal == 0 && *next != second)
                {
                    return Ok(());
                }
                // Neither successor may be entered through another path.
                let mut predecessors = 0usize;
                for candidate in &f.blocks {
                    self.budget.spend(1)?;
                    if enters(&candidate.terminator.value, *next) {
                        predecessors += 1
                    }
                }
                if predecessors != 1 {
                    return Ok(());
                }
                current = *next;
            }
        }
        let mut cell = None;
        for (index, candidate) in self.cells.iter().enumerate() {
            self.budget.spend(
                candidate
                    .input
                    .as_ref()
                    .map_or(1, |input| input.location.path.0.len().saturating_add(1)),
            )?;
            let Some(input) = &candidate.input else {
                continue;
            };
            if input.param == 0
                && input.location.path.0.as_ref()
                    == [CarrierStep::Referent, CarrierStep::SliceElement]
                && input.location.channel
                    == (CarrierChannel::ParamTail {
                        parameter,
                        prefix: 0,
                    })
                && cell.replace(index).is_some()
            {
                return Ok(());
            }
        }
        let Some(cell) = cell else { return Ok(()) };
        let input = self.cells[cell].input.as_ref().ok_or(Exhausted)?;
        self.budget.spend(
            body.original_contract()
                .inputs
                .len()
                .saturating_mul(3)
                .saturating_add(1),
        )?;
        if !body.original_contract().inputs.contains(input) {
            return Ok(());
        }
        self.retain(size_of::<NativeStorageExchange>(), 1)?;
        checked_vec_growth(&mut self.native_exchanges, self.budget)?;
        self.native_exchanges.push(NativeStorageExchange {
            cell,
            parameter,
            reads: trace.reads,
            writes: trace.writes,
            _source: source,
        });
        Ok(())
    }

    pub(super) fn read_native_exchange(
        &mut self,
        state: &FlowState,
        block: BlockId,
        statement: usize,
    ) -> Result<Option<Vec<(CarrierLocation, CarrierOrigins)>>, Exhausted> {
        self.ledger.charge_work(1).map_err(|_| Exhausted)?;
        self.budget
            .spend(self.native_exchanges.len().saturating_add(1))?;
        let Some(receipt) = self
            .native_exchanges
            .iter()
            .find(|receipt| receipt.reads.contains(&(block, statement)))
        else {
            return Ok(None);
        };
        let (cell, parameter) = (receipt.cell, receipt.parameter);
        let mut origins = self.empty_origins()?;
        origins.union(&state.cells[cell], self.budget)?;
        if origins.count() == 0 {
            self.problem(
                block,
                Some(statement),
                Problem::Invalid("native exchange read has no initialized owner origins".into()),
            )?;
            return Ok(Some(Vec::new()));
        }
        self.retain(
            4 * size_of::<(CarrierLocation, CarrierOrigins)>() + 2 * size_of::<usize>(),
            1,
        )?;
        Ok(Some(vec![(
            CarrierLocation {
                path: CarrierPath(Arc::from([])),
                channel: CarrierChannel::ParamTail {
                    parameter,
                    prefix: 0,
                },
            },
            origins,
        )]))
    }
    pub(super) fn write_native_exchange(
        &mut self,
        state: &mut FlowState,
        value: &[(CarrierLocation, CarrierOrigins)],
        block: BlockId,
        statement: Option<usize>,
        diagnose: bool,
    ) -> Result<(), Exhausted> {
        self.ledger.charge_work(1).map_err(|_| Exhausted)?;
        self.budget
            .spend(self.native_exchanges.len().saturating_add(1))?;
        let Some(statement) = statement else {
            return Ok(());
        };
        let Some(receipt) = self
            .native_exchanges
            .iter()
            .find(|receipt| receipt.writes(block, statement))
        else {
            return Ok(());
        };
        let (cell, parameter) = (receipt.cell, receipt.parameter);
        self.budget.spend(
            self.targets
                .len()
                .saturating_add(self.family_inputs.len())
                .saturating_add(value.len())
                .saturating_add(1),
        )?;
        let valid = matches!(value,[(location,origins)] if location.path.0.is_empty()
            && location.channel==(CarrierChannel::ParamTail {parameter,prefix:0})
            && origins.count()!=0 && *origins==state.cells[cell]);
        if !valid {
            if diagnose {
                self.problem(
                    block,
                    Some(statement),
                    Problem::Invalid(
                        "native exchange changed its initialized owner origins".into(),
                    ),
                )?
            }
            return Ok(());
        }
        // SAME-only monotone union. No overwrite, imported source or envelope.
        state.cells[cell].union(&value[0].1, self.budget)?;
        self.membership(value[0].1.count())?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn place(local: usize, raw: bool) -> Place {
        Place {
            local: LocalId(local),
            projection: if raw { vec![Projection::Deref] } else { vec![] },
        }
    }
    fn assignment(to: usize, to_raw: bool, from: usize, from_raw: bool) -> Statement {
        Statement::Assign {
            destination: place(to, to_raw),
            value: Rvalue::Use(Operand::Copy(place(from, from_raw))),
        }
    }
    fn exchange() -> [Statement; 4] {
        [
            assignment(5, false, 3, true),
            assignment(6, false, 4, true),
            assignment(3, true, 6, false),
            assignment(4, true, 5, false),
        ]
    }
    fn seeded(ledger: &ir::PlanBudget, budget: &mut Budget) -> Trace {
        let mut trace = Trace::new(10, &[LocalId(0), LocalId(1), LocalId(2)], ledger, budget)
            .unwrap()
            .unwrap();
        for local in 3..=6 {
            assert!(
                trace
                    .statement(
                        &Statement::StorageLive(LocalId(local)),
                        BlockId(0),
                        local,
                        ledger,
                        budget
                    )
                    .unwrap()
            );
        }
        assert!(trace.define(LocalId(3), Token::SlotA));
        assert!(trace.define(LocalId(4), Token::SlotB));
        trace
    }
    #[test]
    fn native_exchange_temporal_pair_and_reordered_extra_operations_reject() {
        let ledger = ir::PlanBudget::default();
        let mut budget = Budget::new(100_000);
        let statements = exchange();
        let mut genuine = seeded(&ledger, &mut budget);
        for (index, statement) in statements.iter().enumerate() {
            assert!(
                genuine
                    .statement(statement, BlockId(7), index, &ledger, &mut budget)
                    .unwrap()
            );
        }
        assert_eq!(genuine.stage, 4);
        assert_eq!(genuine.reads, [(BlockId(7), 0), (BlockId(7), 1)]);
        assert_eq!(genuine.writes, [(BlockId(7), 2), (BlockId(7), 3)]);
        assert!(
            !genuine
                .statement(&statements[3], BlockId(7), 4, &ledger, &mut budget)
                .unwrap()
        );
        assert!(
            !genuine
                .statement(&statements[0], BlockId(7), 4, &ledger, &mut budget)
                .unwrap()
        );
        let mut reordered = seeded(&ledger, &mut budget);
        assert!(
            !reordered
                .statement(&statements[1], BlockId(7), 0, &ledger, &mut budget)
                .unwrap()
        );
        let mut early_store = seeded(&ledger, &mut budget);
        assert!(
            !early_store
                .statement(&statements[2], BlockId(7), 0, &ledger, &mut budget)
                .unwrap()
        );
        let mut wrong_value = seeded(&ledger, &mut budget);
        for (index, statement) in statements[..2].iter().enumerate() {
            assert!(
                wrong_value
                    .statement(statement, BlockId(7), index, &ledger, &mut budget)
                    .unwrap()
            );
        }
        assert!(
            !wrong_value
                .statement(
                    &assignment(3, true, 5, false),
                    BlockId(7),
                    2,
                    &ledger,
                    &mut budget
                )
                .unwrap()
        );
    }
    #[test]
    fn native_exchange_temporal_wrong_owner_late_definition_and_kills_reject() {
        let ledger = ir::PlanBudget::default();
        let mut budget = Budget::new(100_000);
        let mut trace = seeded(&ledger, &mut budget);
        assert!(
            !trace
                .statement(
                    &assignment(0, false, 1, false),
                    BlockId(0),
                    0,
                    &ledger,
                    &mut budget
                )
                .unwrap()
        );
        assert!(
            !trace
                .statement(
                    &assignment(3, false, 4, false),
                    BlockId(0),
                    1,
                    &ledger,
                    &mut budget
                )
                .unwrap()
        );
        for kill in [
            Statement::StorageDead(LocalId(3)),
            Statement::StorageLive(LocalId(3)),
        ] {
            let mut trace = seeded(&ledger, &mut budget);
            if trace
                .statement(&kill, BlockId(0), 0, &ledger, &mut budget)
                .unwrap()
            {
                assert!(
                    !trace
                        .statement(&exchange()[0], BlockId(0), 1, &ledger, &mut budget)
                        .unwrap()
                );
                assert!(
                    !trace
                        .statement(
                            &Statement::StorageLive(LocalId(3)),
                            BlockId(0),
                            2,
                            &ledger,
                            &mut budget
                        )
                        .unwrap()
                );
            }
        }
        let mut future = seeded(&ledger, &mut budget);
        future.locals[3].token = None;
        assert!(
            !future
                .statement(&exchange()[0], BlockId(0), 0, &ledger, &mut budget)
                .unwrap()
        );
        let mut wrong_owner = seeded(&ledger, &mut budget);
        wrong_owner.locals[3].token = Some(Token::Buffer);
        assert!(
            !wrong_owner
                .statement(&exchange()[0], BlockId(0), 0, &ledger, &mut budget)
                .unwrap()
        );
        ledger.poison_ledger_for_tests();
        assert!(
            Trace::new(
                10,
                &[LocalId(0), LocalId(1), LocalId(2)],
                &ledger,
                &mut budget
            )
            .is_err()
        );
        assert!(
            trace
                .statement(&exchange()[0], BlockId(0), 0, &ledger, &mut budget)
                .is_err()
        );
    }
}
