//! One source-issued native append operation, not general raw-pointer tracking.
//! The owner summary is weakly extended only. No write envelope or replacement
//! permission follows, and every unmatched raw store still suppresses effects.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct NativeStorageTransfer {
    pub(super) block: BlockId,
    pub(super) statement: usize,
    _slot_block: BlockId,
    cell: usize,
    source: usize,
    storage: InputCarrier,
    source_input: InputCarrier,
    _slot: Arc<super::super::frontend::NativeSequenceSlot>,
}

impl NativeStorageTransfer {
    pub(super) fn retained_paths(&self, budget: &mut Budget) -> Result<usize, Exhausted> {
        budget.spend(2)?;
        let steps = add(
            self.storage.location.path.0.len(),
            self.source_input.location.path.0.len(),
        )?;
        add(
            mul(steps, size_of::<CarrierStep>())?,
            4 * size_of::<usize>(),
        )
    }
}

fn operand_place(value: &Operand) -> Option<&Place> {
    match value {
        Operand::Copy(place) | Operand::Move(place) => Some(place),
        _ => None,
    }
}

/// Follow copies at their actual read time, never through a later definition.
/// Parameters have an implicit entry definition. Reassigning one, redefining a
/// traced temporary, or killing/restarting traced storage is conservatively
/// rejected even if the historical copy might otherwise remain usable.
fn prefix_source<'a>(
    statements: &'a [cfg::Located<Statement>],
    mut place: &'a Place,
    parameters: &[LocalId],
    ledger: &ir::PlanBudget,
    budget: &mut Budget,
) -> Result<Option<&'a Place>, Exhausted> {
    ledger.charge_work(1).map_err(|_| Exhausted)?;
    for statement in statements {
        budget.spend(1)?;
        match &statement.value {
            Statement::StorageLive(_) | Statement::StorageDead(_) => {}
            Statement::Assign {
                destination,
                value: Rvalue::Use(_),
            } if destination.projection.is_empty() => {}
            _ => return Ok(None),
        }
    }
    let mut before = statements.len();
    for depth in 0..=statements.len() {
        ledger.check_depth(depth).map_err(|_| Exhausted)?;
        budget.spend(parameters.len().saturating_add(1))?;
        let entry_definition = parameters.contains(&place.local);
        let mut definition = None;
        let mut live = None;
        // A later write/kill cannot be replayed as the value read by this hop.
        // Reject it rather than silently giving an earlier copy a new root.
        for statement in &statements[before..] {
            budget.spend(1)?;
            match &statement.value {
                Statement::StorageLive(local) | Statement::StorageDead(local)
                    if *local == place.local =>
                {
                    return Ok(None);
                }
                Statement::Assign { destination, .. } if destination.local == place.local => {
                    return Ok(None);
                }
                _ => {}
            }
        }
        // Only the strict preceding prefix may provide a reaching definition.
        for (index, statement) in statements[..before].iter().enumerate() {
            budget.spend(1)?;
            match &statement.value {
                Statement::StorageDead(local) if *local == place.local => return Ok(None),
                Statement::StorageLive(local) if *local == place.local => {
                    if entry_definition || live.replace(index).is_some() {
                        return Ok(None);
                    }
                }
                Statement::Assign {
                    destination,
                    value: Rvalue::Use(value),
                } if destination.local == place.local => {
                    if entry_definition || definition.is_some() {
                        return Ok(None);
                    }
                    let Some(source) = operand_place(value) else {
                        return Ok(None);
                    };
                    definition = Some((index, source));
                }
                _ => {}
            }
        }
        if let Some(live) = live
            && !definition.is_some_and(|(definition, _)| live < definition)
        {
            return Ok(None);
        }
        if !place.projection.is_empty() {
            // A field belongs to this exact unmodified incoming root. Following
            // a local reference reassignment would require a different proof.
            return Ok((definition.is_none() && live.is_none()).then_some(place));
        }
        match definition {
            Some((index, source)) => {
                // Every next lookup is strictly before the defining copy. This
                // also terminates cyclic/use-before-definition copy graphs.
                before = index;
                place = source;
            }
            None => return Ok(Some(place)),
        }
    }
    Ok(None)
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
    pub(super) fn prepare_native_storage(&mut self) -> Result<(), Exhausted> {
        let FactsCtx::Validated(facts) = self.facts else {
            return Ok(());
        };
        // This first bounded receipt is issued over the original generic body.
        // Closed bodies continue through the existing universal-proof ingress.
        if facts.phase() != super::super::traits::FactsPhase::Universal {
            return Ok(());
        }
        let Some(body) = self.body.clone() else {
            return Ok(());
        };
        if body.native_heap_invariants().is_empty() {
            return Ok(());
        }
        let f = self.f;
        if let Err(message) = f.validate_provenance(facts, &self.ledger) {
            let error = super::super::traits::FactsError::from_message(&self.ledger, &message);
            if error.is_resource() {
                return Err(Exhausted);
            }
            self.problem(f.entry, None, Problem::Invalid(message))?;
            return Ok(());
        }
        for (block_index, block) in f.blocks.iter().enumerate() {
            for (statement_index, statement) in block.statements.iter().enumerate() {
                self.budget.spend(1)?;
                let Statement::Assign {
                    destination,
                    value: Rvalue::Use(value),
                } = &statement.value
                else {
                    continue;
                };
                if destination.projection.as_slice() != [Projection::Deref] {
                    continue;
                }
                let Some(source_place) = operand_place(value) else {
                    continue;
                };
                let prefix = &block.statements[..statement_index];
                let pointer = Place {
                    local: destination.local,
                    projection: Vec::new(),
                };
                let Some(pointer) =
                    prefix_source(prefix, &pointer, &f.params, &self.ledger, self.budget)?
                else {
                    continue;
                };
                if !pointer.projection.is_empty() {
                    continue;
                }
                let Some(source) =
                    prefix_source(prefix, source_place, &f.params, &self.ledger, self.budget)?
                else {
                    continue;
                };
                if !source.projection.is_empty() {
                    continue;
                }
                if BlockId(block_index) == f.entry {
                    continue;
                }
                self.budget.spend(f.params.len().saturating_add(1))?;
                let Some(source_param) = f.params.iter().position(|local| *local == source.local)
                else {
                    continue;
                };
                let mut predecessor = None;
                let mut unique = true;
                for (index, candidate) in f.blocks.iter().enumerate() {
                    self.budget.spend(1)?;
                    if enters(&candidate.terminator.value, BlockId(block_index))
                        && predecessor.replace(index).is_some()
                    {
                        unique = false;
                    }
                }
                if !unique {
                    continue;
                }
                let Some(slot_block) = predecessor else {
                    continue;
                };
                let predecessor = &f.blocks[slot_block];
                let Terminator::Call {
                    target: ir::Callee::Direct(target),
                    args,
                    signature,
                    destination: result,
                    next,
                    ..
                } = &predecessor.terminator.value
                else {
                    continue;
                };
                if slot_block == block_index
                    || self
                        .calls
                        .get(slot_block)
                        .and_then(Option::as_ref)
                        .is_none()
                {
                    continue;
                }
                if *next != BlockId(block_index) || result != pointer || args.len() != 2 {
                    continue;
                }
                let Some(base) = operand_place(&args[0]) else {
                    continue;
                };
                let Some(index) = operand_place(&args[1]) else {
                    continue;
                };
                let Some(base) = prefix_source(
                    &predecessor.statements,
                    base,
                    &f.params,
                    &self.ledger,
                    self.budget,
                )?
                else {
                    continue;
                };
                let Some(index) = prefix_source(
                    &predecessor.statements,
                    index,
                    &f.params,
                    &self.ledger,
                    self.budget,
                )?
                else {
                    continue;
                };
                if base.local != index.local
                    || base.projection.as_slice() != [Projection::Deref, Projection::Field(0)]
                    || index.projection.as_slice() != [Projection::Deref, Projection::Field(1)]
                {
                    continue;
                }
                // The slot was read in the predecessor. Its native owner must
                // also survive unchanged through this block's store prefix.
                if prefix_source(prefix, base, &f.params, &self.ledger, self.budget)? != Some(base)
                    || prefix_source(prefix, index, &f.params, &self.ledger, self.budget)?
                        != Some(index)
                {
                    continue;
                }
                self.budget.spend(f.params.len().saturating_add(1))?;
                let Some(owner_param) = f.params.iter().position(|local| *local == base.local)
                else {
                    continue;
                };
                let Ty::Ref {
                    kind: BorrowKind::Mut,
                    referent: pointee,
                } = &f.locals[base.local.0].ty
                else {
                    continue;
                };
                let Ty::Struct(owner) = pointee.as_ref() else {
                    continue;
                };
                let Some(element) = pointee.owned_sequence_element() else {
                    continue;
                };
                let Ty::Param(parameter) = element else {
                    continue;
                };
                let [
                    Ty::RawPtr {
                        kind: BorrowKind::Mut,
                        pointee: input,
                    },
                    Ty::Int(ir::IntTy::Usize),
                ] = signature.params.as_slice()
                else {
                    continue;
                };
                if input.as_ref() != element
                    || !matches!(&signature.result,
                    Ty::RawPtr { kind: BorrowKind::Mut, pointee } if pointee.as_ref() == element)
                    || !matches!(&f.locals[destination.local.0].ty,
                        Ty::RawPtr { kind: BorrowKind::Mut, pointee } if pointee.as_ref() == element)
                {
                    continue;
                }
                if target.type_args.as_ref() != [Ty::Param(*parameter)]
                    || f.locals[source.local.0].ty != Ty::Param(*parameter)
                {
                    continue;
                }
                let slot = match facts.native_sequence_slot(target.declaration) {
                    Ok(slot) => slot,
                    Err(error) if error.is_resource() => return Err(Exhausted),
                    Err(error) => {
                        let message = error
                            .into_diagnostic(facts.budget())
                            .map_err(|_| Exhausted)?;
                        self.problem(BlockId(slot_block), None, Problem::Invalid(message))?;
                        continue;
                    }
                };
                let Some(slot) = slot else { continue };
                if !slot.matches_owner(target.declaration, owner) {
                    continue;
                }
                for invariant in body.native_heap_invariants() {
                    self.budget
                        .spend(invariant.sources.len() + self.cells.len() + 1)?;
                    if invariant.storage.param != owner_param
                        || invariant.sources.len() != 1
                        || invariant
                            .sources
                            .iter()
                            .any(|input| input.param != source_param)
                        || invariant.sources.is_empty()
                    {
                        continue;
                    }
                    let Some(cell) = self
                        .cells
                        .iter()
                        .position(|cell| cell.input.as_ref() == Some(&invariant.storage))
                    else {
                        continue;
                    };
                    self.retain(size_of::<NativeStorageTransfer>(), 1)?;
                    checked_vec_growth(&mut self.native_storage, self.budget)?;
                    self.native_storage.push(NativeStorageTransfer {
                        block: BlockId(block_index),
                        statement: statement_index,
                        _slot_block: BlockId(slot_block),
                        cell,
                        source: source_param,
                        storage: invariant.storage.clone(),
                        source_input: invariant.sources[0].clone(),
                        _slot: slot.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn write_native_storage(
        &mut self,
        state: &mut FlowState,
        value: &[(CarrierLocation, CarrierOrigins)],
        block: BlockId,
        statement: Option<usize>,
        diagnose: bool,
    ) -> Result<(), Exhausted> {
        self.budget.spend(self.native_storage.len() + 1)?;
        let Some(transfer_index) = self
            .native_storage
            .iter()
            .position(|transfer| transfer.block == block && Some(transfer.statement) == statement)
        else {
            return Ok(());
        };
        // Extract only Copy scalars. The source/destination rows and slot
        // capsule stay in their already-paid immutable receipt allocation.
        let (cell, source) = {
            let transfer = &self.native_storage[transfer_index];
            (transfer.cell, transfer.source)
        };
        let Some(body) = self.body.clone() else {
            return Err(Exhausted);
        };
        let storage = self.cells[cell].input.as_ref().ok_or(Exhausted)?;
        let invariant = body
            .native_heap_invariants()
            .iter()
            .find(|row| row.storage == *storage)
            .ok_or(Exhausted)?;
        let receipt_matches = {
            let transfer = &self.native_storage[transfer_index];
            self.budget.spend(
                storage
                    .location
                    .path
                    .0
                    .len()
                    .saturating_add(transfer.storage.location.path.0.len())
                    .saturating_add(transfer.source_input.location.path.0.len())
                    .saturating_add(
                        invariant
                            .sources
                            .first()
                            .map_or(0, |input| input.location.path.0.len()),
                    )
                    .saturating_add(1),
            )?;
            storage == &transfer.storage
                && invariant.sources.len() == 1
                && invariant.sources[0] == transfer.source_input
        };
        if !receipt_matches {
            return Err(Exhausted);
        }
        self.budget
            .spend(invariant.sources.len() + value.len() + 1)?;
        if value.len() != 1 || invariant.sources.iter().any(|input| input.param != source) {
            if diagnose {
                self.problem(
                    block,
                    statement,
                    Problem::Invalid(
                        "native initialized storage changed its exact source component".into(),
                    ),
                )?;
            }
            return Ok(());
        }
        for (location, origins) in value {
            if !location.path.0.is_empty()
                || location.channel != invariant.storage.location.channel
                || origins.count() == 0
                || !self.origin_allowed(origins, &invariant.sources)?
            {
                if diagnose {
                    self.problem(
                        block,
                        statement,
                        Problem::Invalid(
                            "native initialized storage lost its exact source carrier origins"
                                .into(),
                        ),
                    )?;
                }
                continue;
            }
            // Monotone initialized-summary union, never overwrite/refinement.
            state.cells[cell].union(origins, self.budget)?;
            self.membership(origins.count())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local(index: usize) -> Place {
        Place {
            local: LocalId(index),
            projection: Vec::new(),
        }
    }
    fn copy(to: Place, from: Place) -> cfg::Located<Statement> {
        cfg::Located {
            value: Statement::Assign {
                destination: to,
                value: Rvalue::Use(Operand::Copy(from)),
            },
            origin: None,
        }
    }
    fn moved(to: Place, from: Place) -> cfg::Located<Statement> {
        cfg::Located {
            value: Statement::Assign {
                destination: to,
                value: Rvalue::Use(Operand::Move(from)),
            },
            origin: None,
        }
    }
    fn storage(value: Statement) -> cfg::Located<Statement> {
        cfg::Located {
            value,
            origin: None,
        }
    }

    #[test]
    fn native_storage_lineage_temporal_parameter_and_future_definition_reject() {
        let ledger = ir::PlanBudget::default();
        let mut work = Budget::new(100_000);
        let parameters = [LocalId(0), LocalId(3)];
        let root = local(0);
        let output = local(2);
        // The reaching chain is 2 <- 1 <- incoming 0, with a legal StorageLive
        // before its definition. Unrelated local reassignments are not roots.
        let chronological = [
            storage(Statement::StorageLive(LocalId(1))),
            copy(local(1), root.clone()),
            copy(local(4), local(3)),
            copy(local(4), root.clone()),
            copy(output.clone(), local(1)),
        ];
        assert_eq!(
            prefix_source(&chronological, &output, &parameters, &ledger, &mut work).unwrap(),
            Some(&root)
        );
        // A copied entry value cannot acquire param3's identity from a later
        // reassignment of param0. Entry definitions count even without a stmt.
        let late_parameter = [
            copy(local(1), root.clone()),
            moved(root.clone(), local(3)),
            copy(output.clone(), local(1)),
        ];
        assert!(
            prefix_source(&late_parameter, &output, &parameters, &ledger, &mut work)
                .unwrap()
                .is_none()
        );
        let early_parameter = [
            moved(root.clone(), local(3)),
            copy(output.clone(), root.clone()),
        ];
        assert!(
            prefix_source(&early_parameter, &output, &parameters, &ledger, &mut work)
                .unwrap()
                .is_none()
        );
        // This copy graph would resolve to param0 only by reading forward from
        // the first statement. Such a future definition is never a proof.
        let future_definition = [
            copy(local(1), local(4)),
            copy(local(4), root.clone()),
            copy(output.clone(), local(1)),
        ];
        assert!(
            prefix_source(&future_definition, &output, &parameters, &ledger, &mut work)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn native_storage_lineage_late_base_index_mutation_and_storage_kills_reject() {
        let ledger = ir::PlanBudget::default();
        let mut work = Budget::new(100_000);
        let parameters = [LocalId(0), LocalId(3)];
        let output = local(2);
        for field in [0, 1] {
            let owner_field = Place {
                local: LocalId(0),
                projection: vec![Projection::Deref, Projection::Field(field)],
            };
            let chronological = [
                copy(local(1), owner_field.clone()),
                copy(output.clone(), local(1)),
            ];
            assert_eq!(
                prefix_source(&chronological, &output, &parameters, &ledger, &mut work).unwrap(),
                Some(&owner_field)
            );
            let late_owner = [
                copy(local(1), owner_field.clone()),
                moved(local(0), local(3)),
                copy(output.clone(), local(1)),
            ];
            assert!(
                prefix_source(&late_owner, &output, &parameters, &ledger, &mut work)
                    .unwrap()
                    .is_none()
            );
            let after_slot = [copy(output.clone(), local(20))];
            assert_eq!(
                prefix_source(&after_slot, &owner_field, &parameters, &ledger, &mut work).unwrap(),
                Some(&owner_field)
            );
            let changed_after_slot = [copy(output.clone(), local(20)), moved(local(0), local(3))];
            assert_eq!(
                prefix_source(
                    &changed_after_slot,
                    &output,
                    &parameters,
                    &ledger,
                    &mut work
                )
                .unwrap(),
                Some(&local(20))
            );
            assert!(
                prefix_source(
                    &changed_after_slot,
                    &owner_field,
                    &parameters,
                    &ledger,
                    &mut work
                )
                .unwrap()
                .is_none()
            );
            let late_field = [
                copy(local(1), owner_field.clone()),
                copy(owner_field, local(3)),
                copy(output.clone(), local(1)),
            ];
            assert!(
                prefix_source(&late_field, &output, &parameters, &ledger, &mut work)
                    .unwrap()
                    .is_none()
            );
        }
        for killed in [LocalId(0), LocalId(1)] {
            for kill in [
                Statement::StorageDead(killed),
                Statement::StorageLive(killed),
            ] {
                let changed_storage = [
                    copy(local(1), local(0)),
                    storage(kill),
                    copy(output.clone(), local(1)),
                ];
                assert!(
                    prefix_source(&changed_storage, &output, &parameters, &ledger, &mut work)
                        .unwrap()
                        .is_none()
                );
            }
        }
        let restarted = [
            copy(local(1), local(0)),
            storage(Statement::StorageDead(LocalId(1))),
            storage(Statement::StorageLive(LocalId(1))),
            copy(output.clone(), local(1)),
        ];
        assert!(
            prefix_source(&restarted, &output, &parameters, &ledger, &mut work)
                .unwrap()
                .is_none()
        );
        let live_uninitialized = [
            storage(Statement::StorageLive(LocalId(1))),
            copy(output.clone(), local(1)),
        ];
        assert!(
            prefix_source(
                &live_uninitialized,
                &output,
                &parameters,
                &ledger,
                &mut work
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn native_storage_lineage_exact_copies_mutation_cycles_and_poison() {
        let ledger = ir::PlanBudget::default();
        let mut work = Budget::new(100_000);
        let root = local(0);
        let output = local(2);
        let copies = [copy(local(1), root.clone()), copy(output.clone(), local(1))];
        assert_eq!(
            prefix_source(&copies, &output, &[LocalId(0)], &ledger, &mut work).unwrap(),
            Some(&root)
        );
        let duplicate = [
            copy(output.clone(), root.clone()),
            copy(output.clone(), local(1)),
        ];
        assert!(
            prefix_source(&duplicate, &output, &[LocalId(0)], &ledger, &mut work)
                .unwrap()
                .is_none()
        );
        let cyclic = [
            copy(output.clone(), local(1)),
            copy(local(1), output.clone()),
        ];
        assert!(
            prefix_source(&cyclic, &output, &[LocalId(0)], &ledger, &mut work)
                .unwrap()
                .is_none()
        );
        let field = Place {
            local: LocalId(0),
            projection: vec![Projection::Deref, Projection::Field(0)],
        };
        let mutation = [copy(field.clone(), root.clone())];
        assert!(
            prefix_source(&mutation, &field, &[LocalId(0)], &ledger, &mut work)
                .unwrap()
                .is_none()
        );
        ledger.poison_ledger_for_tests();
        assert!(prefix_source(&[], &root, &[LocalId(0)], &ledger, &mut work).is_err());
    }
}
