//! Canonical normal-path cleanup introduced by authenticated physical mapping.
use super::*;

/// The caller independently replays physical normalization before sealing. This
/// helper derives only concrete cleanup descriptors, never hidden types or calls.
pub(in crate::compiler) fn materialize(
    original: &Function,
    mapped: &mut Function,
    source: &Arc<crate::compiler::source_replay::CanonicalSourcePublication>,
    closed: &ValidatedFacts,
    budget: &ir::PlanBudget,
) -> Result<Vec<BlockId>, String> {
    original
        .validate_source_publication(source, budget)
        .map_err(|error| error.to_string())?;
    if closed.phase() != crate::compiler::traits::FactsPhase::Closed
        || closed.declaration() != source.facts().declaration()
        || !Arc::ptr_eq(closed.world(), source.facts().world())
        || !closed.budget().same_authority(budget)
        || mapped.locals.len() != original.locals.len()
    {
        return Err("physical cleanup changed source, world, phase or local inventory".into());
    }
    let mut placements = Vec::new();
    for index in 0..mapped.blocks.len() {
        admit_cfg_push(&mut placements, Some(budget))?;
        placements.push(BlockId(index));
    }
    let mut introduced = Vec::new();
    for (index, local) in mapped.locals.iter().enumerate() {
        admit_cfg_push(&mut introduced, Some(budget))?;
        introduced.push(
            index != mapped.return_local.0
                && descriptor(
                    &original.locals[index].ty,
                    ir::FactsCtx::from_validated(source.facts()),
                )?
                .is_none()
                && descriptor(&local.ty, ir::FactsCtx::from_validated(closed))?.is_some(),
        );
    }
    if !introduced.iter().any(|value| *value) {
        return Ok(placements);
    }
    let admission = AdmittedSource::from_publication(source, budget)?;
    if !admit(&admission)? {
        return Err(
            "unsupported: physical cleanup requires original canonical owning temporary scopes"
                .into(),
        );
    }
    let old_sites = mapped.runtime_sites.clone();
    let original_blocks = mapped.blocks.len();
    let mut rows = Vec::new();
    for index in 0..original_blocks {
        budget.charge_work(1)?;
        let old = std::mem::replace(&mut mapped.blocks[index], empty_block());
        let mut emitter = Emitter {
            function: mapped,
            block: BlockId(index),
            statements: Vec::new(),
            rows: &mut rows,
            closed,
            budget,
        };
        for (position, statement) in old.statements.into_iter().enumerate() {
            budget.charge_work(1)?;
            let origin = statement.origin;
            let runtime = old_sites.get(BlockId(index), position).cloned();
            match statement.value {
                Statement::StorageDead(local) if introduced.get(local.0) == Some(&true) => {
                    let owner = place(local);
                    emitter.drop(owner, origin)?;
                    emitter.emit(Statement::StorageDead(local), origin, runtime)?;
                }
                Statement::Assign { destination, value }
                    if introduced.get(destination.local.0) == Some(&true) =>
                {
                    if emitter.cleanup(&destination)?.is_some() {
                        let temporary = emitter.temporary(&destination, origin)?;
                        emitter.emit(
                            Statement::Assign {
                                destination: place(temporary),
                                value,
                            },
                            origin,
                            runtime,
                        )?;
                        emitter.drop(destination.clone(), origin)?;
                        emitter.transfer(temporary, destination, origin)?;
                    } else {
                        emitter.emit(Statement::Assign { destination, value }, origin, runtime)?;
                    }
                }
                value => emitter.emit(value, origin, runtime)?,
            }
        }
        // Two-phase activation belongs to the original call, not the old
        // block number after its preceding ownership events have been split.
        for loan in &mut emitter.function.loan_sites {
            budget.charge_work(1)?;
            if let LoanTiming::TwoPhase { activation, .. } = &mut loan.timing
                && *activation == BlockId(index)
            {
                *activation = emitter.block;
            }
        }
        placements[index] = emitter.block;
        let mut terminator = old.terminator;
        if let Terminator::Call {
            destination, next, ..
        } = &mut terminator.value
            && introduced.get(destination.local.0) == Some(&true)
            && emitter.cleanup(destination)?.is_some()
        {
            let owner = destination.clone();
            let temporary = emitter.temporary(&owner, terminator.origin)?;
            *destination = place(temporary);
            let old_next = *next;
            *next = emitter.new_block()?;
            let result = *next;
            emitter.finish(terminator);
            emitter.block = result;
            emitter.drop(owner.clone(), None)?;
            emitter.transfer(temporary, owner, None)?;
            emitter.finish(Located {
                value: Terminator::Goto(old_next),
                origin: None,
            });
        } else {
            emitter.finish(terminator);
        }
    }
    mapped.runtime_sites = CfgRuntimeSites::from_lowering(rows)?;
    Ok(placements)
}

fn empty_block() -> BasicBlock {
    BasicBlock {
        statements: Vec::new(),
        terminator: Located {
            value: Terminator::Unreachable,
            origin: None,
        },
    }
}

struct Emitter<'a> {
    function: &'a mut Function,
    block: BlockId,
    statements: Vec<Located<Statement>>,
    rows: &'a mut Vec<runtime_sites::Row>,
    closed: &'a ValidatedFacts,
    budget: &'a ir::PlanBudget,
}
impl Emitter<'_> {
    fn emit(
        &mut self,
        value: Statement,
        origin: Option<ExprId>,
        runtime: Option<Arc<ir::RuntimeLocation>>,
    ) -> Result<(), String> {
        if let Some(runtime) = runtime {
            admit_cfg_push(self.rows, Some(self.budget))?;
            self.rows
                .push((self.block.0, self.statements.len(), runtime));
        }
        admit_cfg_push(&mut self.statements, Some(self.budget))?;
        self.statements.push(Located { value, origin });
        Ok(())
    }
    fn cleanup(&self, owner: &Place) -> Result<Option<Arc<ValidatedDestructor>>, String> {
        let ty = self
            .closed
            .projected_ty(&self.function.locals[owner.local.0].ty, &owner.projection)
            .map_err(|error| error.into_legacy_diagnostic(self.budget))?;
        let result = descriptor(&ty, ir::FactsCtx::from_validated(self.closed))?;
        if result.is_some()
            && owner.projection.iter().any(|projection| {
                matches!(
                    projection,
                    Projection::ConstantIndex(_) | Projection::DynamicIndex(_)
                )
            })
        {
            return Err("unsupported: indexed normal Drop replacement".into());
        }
        Ok(result)
    }
    fn new_block(&mut self) -> Result<BlockId, String> {
        self.budget.charge_nodes(1)?;
        admit_cfg_push(&mut self.function.blocks, Some(self.budget))?;
        let next = BlockId(self.function.blocks.len());
        self.function.blocks.push(empty_block());
        Ok(next)
    }
    fn finish(&mut self, terminator: Located<Terminator>) {
        self.function.blocks[self.block.0] = BasicBlock {
            statements: std::mem::take(&mut self.statements),
            terminator,
        };
    }
    fn drop(&mut self, owner: Place, origin: Option<ExprId>) -> Result<(), String> {
        let Some(destructor) = self.cleanup(&owner)? else {
            return Ok(());
        };
        self.budget.charge_nodes(1)?;
        self.budget
            .retain_bytes(std::mem::size_of::<ValidatedCleanupCall>())?;
        let cleanup = Arc::new(ValidatedCleanupCall { owner, destructor });
        cleanup.validate(self.function, self.closed, &mut TraitScratch::default())?;
        let next = self.new_block()?;
        self.finish(Located {
            value: Terminator::Drop { cleanup, next },
            origin,
        });
        self.block = next;
        Ok(())
    }
    fn temporary(&mut self, owner: &Place, origin: Option<ExprId>) -> Result<LocalId, String> {
        let ty = self
            .closed
            .projected_ty(&self.function.locals[owner.local.0].ty, &owner.projection)
            .map_err(|error| error.into_legacy_diagnostic(self.budget))?;
        self.budget.charge_nodes(1)?;
        admit_cfg_push(&mut self.function.locals, Some(self.budget))?;
        let local = LocalId(self.function.locals.len());
        self.function.locals.push(ir::LocalDecl {
            name: "$physical_cleanup_value".into(),
            ty,
            mutable: true,
        });
        self.emit(Statement::StorageLive(local), origin, None)?;
        Ok(local)
    }
    fn transfer(
        &mut self,
        temporary: LocalId,
        owner: Place,
        origin: Option<ExprId>,
    ) -> Result<(), String> {
        self.emit(
            Statement::Assign {
                destination: owner,
                value: Rvalue::Use(Operand::Move(place(temporary))),
            },
            origin,
            None,
        )?;
        self.emit(Statement::StorageDead(temporary), origin, None)
    }
}
