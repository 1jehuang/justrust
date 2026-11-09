//! Exact caller-owned singleton slots. No abstract sequence summary becomes exact.
//! All scratch belongs to one borrow-plan operation and source roles default deny.
use super::super::frontend::{NativeElementRole, NativeElementSource};
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Other,
    Integer(usize),
    Owner,
    OwnerRef(LocalId, BorrowKind),
    SlotRef(LocalId, BorrowKind),
}
#[derive(Clone, Debug)]
struct SingletonCall {
    block: BlockId,
    expression: ir::ExprId,
    contract: Arc<super::super::traits::CarrierContractView>,
    evidence: Arc<ir::ValidatedCallEvidence>,
}
#[derive(Clone, Debug)]
pub(super) struct SingletonHeapSlot {
    seed: SingletonCall,
    owner: LocalId,
    cell: usize,
    activation: Option<(BlockId, usize)>,
    indices: Vec<SingletonCall>,
    _sources: Vec<Arc<NativeElementSource>>,
}
fn token(operand: &Operand, locals: &[Token]) -> Token {
    match operand {
        Operand::Constant(cfg::Constant::Int(value)) => {
            usize::try_from(value.bits).map_or(Token::Other, Token::Integer)
        }
        Operand::Copy(place) | Operand::Move(place) if place.projection.is_empty() => {
            locals.get(place.local.0).copied().unwrap_or(Token::Other)
        }
        _ => Token::Other,
    }
}
fn mutable_escape(token: Token) -> bool {
    matches!(
        token,
        Token::Owner | Token::OwnerRef(_, BorrowKind::Mut) | Token::SlotRef(_, BorrowKind::Mut)
    )
}
impl CarrierBuilder<'_, '_> {
    fn singleton_call(
        &mut self,
        block: BlockId,
        contract: &Arc<super::super::traits::CarrierContractView>,
        argument_count: usize,
    ) -> Result<Option<SingletonCall>, Exhausted> {
        let Some(function) = self.function.clone() else {
            return Ok(None);
        };
        let Some(expression) = self.f.blocks[block.0].terminator.origin else {
            return Ok(None);
        };
        self.budget.spend(function.call_evidence.len())?;
        let mut matching = function
            .call_evidence
            .iter()
            .filter(|capsule| capsule.call() == expression);
        let Some(evidence) = matching.next().cloned() else {
            return Ok(None);
        };
        if matching.next().is_some() {
            return Ok(None);
        }
        let Some(original) = evidence.source().exprs.get(expression.0) else {
            return Ok(None);
        };
        let ir::ExprKind::Call { args, .. } = &original.kind else {
            return Ok(None);
        };
        if args.len() != argument_count
            || evidence.trace().arguments.len() != argument_count
            || evidence
                .trace()
                .arguments
                .iter()
                .enumerate()
                .any(|(index, row)| row.argument != index || args.get(index) != Some(&row.value))
        {
            return Ok(None);
        }
        Ok(Some(SingletonCall {
            block,
            expression,
            contract: contract.clone(),
            evidence,
        }))
    }
    fn singleton_call_matches(&self, call: &SingletonCall) -> bool {
        self.f
            .blocks
            .get(call.block.0)
            .and_then(|block| block.terminator.origin)
            == Some(call.expression)
            && call.evidence.call() == call.expression
            && self
                .calls
                .get(call.block.0)
                .and_then(Option::as_ref)
                .is_some_and(|view| Arc::ptr_eq(view, &call.contract))
            && self.function.as_ref().is_some_and(|function| {
                function
                    .call_evidence
                    .iter()
                    .any(|capsule| Arc::ptr_eq(capsule, &call.evidence))
            })
    }
    pub(super) fn prepare_singleton_heap_slot(&mut self) -> Result<(), Exhausted> {
        let f = self.f;
        let (Some(function), Some(facts)) = (self.function.clone(), self.authority.clone()) else {
            return Ok(());
        };
        if facts.phase() != super::super::traits::FactsPhase::Closed || self.physical.is_some() {
            return Ok(());
        }
        // A structural IR lookalike or borrowed foreign facts cannot issue a slot.
        let provenance = self.ensure_provenance(&function, &facts);
        if self.metadata_value(provenance)?.is_none() {
            return Ok(());
        }
        self.budget
            .reserve_bytes(mul(f.locals.len(), size_of::<Token>())?)?;
        self.budget
            .reserve_bytes(mul(f.blocks.len(), size_of::<bool>())?)?;
        let mut locals = vec![Token::Other; f.locals.len()];
        let mut visited = vec![false; f.blocks.len()];
        let mut seed = None;
        let mut owner = None;
        let mut element = None;
        let mut activation = None;
        let mut allocation_live = false;
        let mut indices = Vec::new();
        let mut sources = Vec::new();
        let mut block = f.entry;
        loop {
            self.budget.spend(1)?;
            let Some(seen) = visited.get_mut(block.0) else {
                return Ok(());
            };
            if *seen {
                return Ok(());
            }
            *seen = true;
            for (statement_index, statement) in f.blocks[block.0].statements.iter().enumerate() {
                self.budget.spend(1)?;
                match &statement.value {
                    Statement::StorageLive(local) | Statement::StorageDead(local) => {
                        if locals[local.0] == Token::Owner {
                            allocation_live = false;
                        }
                        locals[local.0] = Token::Other;
                    }
                    Statement::Assign { destination, value } => {
                        if destination.projection.is_empty() {
                            let result = match value {
                                Rvalue::Use(operand) => {
                                    let result = token(operand, &locals);
                                    if mutable_escape(locals[destination.local.0]) {
                                        return Ok(());
                                    }
                                    if let Operand::Copy(place) | Operand::Move(place) = operand
                                        && !place.projection.is_empty()
                                        && matches!(
                                            locals[place.local.0],
                                            Token::Owner | Token::OwnerRef(..)
                                        )
                                    {
                                        return Ok(());
                                    }
                                    if result == Token::Owner {
                                        if !indices.is_empty()
                                            || !matches!(operand, Operand::Move(_))
                                        {
                                            return Ok(());
                                        }
                                        owner = Some(destination.local);
                                        activation = Some((block, statement_index));
                                    }
                                    if let Operand::Move(place) = operand
                                        && place.projection.is_empty()
                                    {
                                        locals[place.local.0] = Token::Other;
                                    }
                                    result
                                }
                                Rvalue::Borrow(loan) => {
                                    let loan = &f.loan_sites[loan.0];
                                    if loan.place.projection.is_empty()
                                        && locals[loan.place.local.0] == Token::Owner
                                    {
                                        Token::OwnerRef(loan.place.local, loan.kind)
                                    } else if loan.place.projection.is_empty() {
                                        match locals[loan.place.local.0] {
                                            Token::OwnerRef(..) | Token::SlotRef(..) => {
                                                return Ok(());
                                            }
                                            _ => Token::Other,
                                        }
                                    } else {
                                        match locals[loan.place.local.0] {
                                            Token::OwnerRef(root, _)
                                                if loan.place.projection.as_slice()
                                                    == [Projection::Deref] =>
                                            {
                                                Token::OwnerRef(root, loan.kind)
                                            }
                                            Token::SlotRef(root, _)
                                                if loan.place.projection.as_slice()
                                                    == [Projection::Deref] =>
                                            {
                                                Token::SlotRef(root, loan.kind)
                                            }
                                            Token::Owner | Token::OwnerRef(..) => return Ok(()),
                                            _ => Token::Other,
                                        }
                                    }
                                }
                                _ => {
                                    let mut escape = mutable_escape(locals[destination.local.0]);
                                    visit_rvalue(f, value, |place, _| {
                                        escape |= mutable_escape(locals[place.local.0]);
                                    });
                                    if escape {
                                        return Ok(());
                                    }
                                    Token::Other
                                }
                            };
                            locals[destination.local.0] = result;
                        } else {
                            match locals[destination.local.0] {
                                Token::Owner | Token::OwnerRef(..) => return Ok(()),
                                Token::SlotRef(_, BorrowKind::Mut)
                                    if destination.projection.as_slice() == [Projection::Deref] => {
                                }
                                Token::SlotRef(..) => return Ok(()),
                                _ => {}
                            }
                        }
                    }
                    _ => {
                        // Operations with an unmodelled write footprint cannot retain
                        // an initialized singleton allocation proof.
                        let mut touches = false;
                        if let Statement::IndexAssign { array, .. }
                        | Statement::IndexAssignOp { array, .. } = &statement.value
                        {
                            touches = mutable_escape(locals[array.local.0]);
                        }
                        if touches {
                            return Ok(());
                        }
                    }
                }
            }
            match &f.blocks[block.0].terminator.value {
                Terminator::Goto(next) => block = *next,
                Terminator::Drop { cleanup, next } => {
                    if locals[cleanup.owner().local.0] == Token::Owner {
                        allocation_live = false;
                    }
                    block = *next;
                }
                Terminator::Call {
                    signature,
                    args,
                    destination,
                    next,
                    ..
                } => {
                    let view = self.calls[block.0].clone();
                    let declaration = view.as_ref().map(|view| view.declaration());
                    let source = match declaration {
                        Some(declaration) => {
                            let role = facts
                                .native_sequence_element_with_scratch(
                                    declaration,
                                    &mut self.carrier_scratch,
                                )
                                .map_err(|error| error.into_legacy_diagnostic(&self.ledger));
                            let Some(role) = self.metadata_value(role)? else {
                                return Ok(());
                            };
                            role
                        }
                        None => None,
                    };
                    let mut handled = false;
                    if let (Some(source), Some(declaration)) = (source, declaration) {
                        let result_owner = match &signature.result {
                            Ty::Struct(owner) => Some(owner),
                            _ => None,
                        };
                        if let Some(result_owner) = result_owner
                            && source.role_for(declaration, result_owner)
                                == Some(NativeElementRole::Seed)
                        {
                            let [first, capacity] = args.as_slice() else {
                                return Ok(());
                            };
                            let Some(actual) = signature.result.owned_sequence_element() else {
                                return Ok(());
                            };
                            let Ty::Ref {
                                kind: BorrowKind::Mut,
                                referent,
                            } = actual
                            else {
                                return Ok(());
                            };
                            if seed.is_some()
                                || !destination.projection.is_empty()
                                || token(capacity, &locals) != Token::Integer(1)
                                || !self.copy_permission(referent)?
                                || !ir::carrier_locations(referent, &self.ledger)
                                    .map_err(|_| Exhausted)?
                                    .is_empty()
                                || signature.params.first() != Some(actual)
                                || !matches!(first, Operand::Move(_) | Operand::Copy(_))
                            {
                                return Ok(());
                            }
                            let Some(call) = self.singleton_call(
                                block,
                                view.as_ref().ok_or(Exhausted)?,
                                args.len(),
                            )?
                            else {
                                return Ok(());
                            };
                            seed = Some(call);
                            allocation_live = true;
                            owner = Some(destination.local);
                            element = Some(actual.clone());
                            locals[destination.local.0] = Token::Owner;
                            checked_vec_growth(&mut sources, self.budget)?;
                            sources.push(source.clone());
                            handled = true;
                        }
                        if let Some(Ty::Ref { kind, referent }) = signature.params.first()
                            && let Ty::Struct(input_owner) = referent.as_ref()
                            && let Some(
                                role @ (NativeElementRole::SharedIndex
                                | NativeElementRole::MutIndex),
                            ) = source.role_for(declaration, input_owner)
                        {
                            let [receiver, subscript] = args.as_slice() else {
                                return Ok(());
                            };
                            let Token::OwnerRef(root, borrowed) = token(receiver, &locals) else {
                                return Ok(());
                            };
                            let expected = if role == NativeElementRole::MutIndex {
                                BorrowKind::Mut
                            } else {
                                BorrowKind::Shared
                            };
                            if seed.is_none()
                                || !allocation_live
                                || referent.owned_sequence_element() != element.as_ref()
                                || owner != Some(root)
                                || *kind != expected
                                || borrowed != expected
                                || token(subscript, &locals) != Token::Integer(0)
                                || !destination.projection.is_empty()
                                || !matches!(&signature.result, Ty::Ref { kind, referent } if *kind == expected && Some(referent.as_ref()) == element.as_ref())
                            {
                                return Ok(());
                            }
                            locals[destination.local.0] = Token::SlotRef(root, expected);
                            checked_vec_growth(&mut indices, self.budget)?;
                            let Some(call) = self.singleton_call(
                                block,
                                view.as_ref().ok_or(Exhausted)?,
                                args.len(),
                            )?
                            else {
                                return Ok(());
                            };
                            indices.push(call);
                            checked_vec_growth(&mut sources, self.budget)?;
                            sources.push(source);
                            handled = true;
                        }
                    }
                    if !handled {
                        for argument in args {
                            self.budget.spend(1)?;
                            if mutable_escape(token(argument, &locals)) {
                                return Ok(());
                            }
                        }
                        if mutable_escape(locals[destination.local.0]) {
                            return Ok(());
                        }
                        locals[destination.local.0] = Token::Other;
                    }
                    for argument in args {
                        if let Operand::Move(place) = argument
                            && place.projection.is_empty()
                        {
                            locals[place.local.0] = Token::Other;
                        }
                    }
                    block = *next;
                }
                Terminator::Return | Terminator::Panic(_) | Terminator::Unreachable => break,
                // No merges or loops in this first exact allocation domain.
                Terminator::Switch { .. } | Terminator::SwitchVariant { .. } => return Ok(()),
            }
        }
        let (Some(seed), Some(owner), Some(element)) = (seed, owner, element) else {
            return Ok(());
        };
        if indices.is_empty() {
            return Ok(());
        }
        let cell = self.add_cell(
            CellKey {
                storage: CanonicalPlace {
                    root: CanonicalRoot::Owned(owner),
                    projection: vec![Projection::ConstantIndex(0)],
                },
                channel: CellChannel::ReferenceValue,
            },
            &element,
        )?;
        self.singleton_heap_slot = Some(SingletonHeapSlot {
            seed,
            owner,
            cell,
            activation,
            indices,
            _sources: sources,
        });
        Ok(())
    }

    pub(super) fn refine_singleton_target(
        &mut self,
        origin: ReferenceOriginId,
        target: &mut Target,
    ) -> Result<(), Exhausted> {
        let Some(slot) = &self.singleton_heap_slot else {
            return Ok(());
        };
        let capsules = self
            .function
            .as_ref()
            .map_or(0, |function| function.call_evidence.len());
        self.budget
            .spend(mul(slot.indices.len(), capsules.saturating_add(1))?)?;
        for call in &slot.indices {
            self.budget.spend(self.call_rows[call.block.0].len())?;
        }
        let issued = slot.indices.iter().any(|call| {
            self.singleton_call_matches(call)
                && self.call_rows[call.block.0].iter().any(|(location, row)| {
                    *row == origin && *location == fixed_location(ReferencePosition { depth: 0 })
                })
        });
        if !issued
            || target.place.root != CanonicalRoot::Owned(slot.owner)
            || !target.place.projection.is_empty()
        {
            return Ok(());
        }
        // The original indexed-call receipt selects a distinct initialized
        // element cell. It does not turn the owner summary cell into an exact
        // destination. Receiver loan authority remains unchanged.
        self.budget.reserve_bytes(size_of::<Projection>())?;
        target.place.projection.push(Projection::ConstantIndex(0));
        target.precision = TargetPrecision::Exact;
        Ok(())
    }
    pub(super) fn join_singleton_summary(
        &mut self,
        state: &mut FlowState,
        index: usize,
        origins: &CarrierOrigins,
    ) -> Result<(), Exhausted> {
        let Some(slot) = &self.singleton_heap_slot else {
            return Ok(());
        };
        if slot.cell != index {
            return Ok(());
        }
        let key = CellKey {
            storage: CanonicalPlace {
                root: CanonicalRoot::Owned(slot.owner),
                projection: vec![Projection::DynamicIndex(LocalId(usize::MAX))],
            },
            channel: CellChannel::ReferenceValue,
        };
        if let Some(summary) = self.cell(&key)? {
            // This is a conservative abstraction of an already authorized exact
            // slot store, never permission to write a summarized affine target.
            state.cells[summary].union(origins, self.budget)?;
        }
        Ok(())
    }
    pub(super) fn initialize_singleton_slot(
        &mut self,
        state: &mut FlowState,
        block: BlockId,
        statement: Option<usize>,
    ) -> Result<(), Exhausted> {
        let Some(slot) = &self.singleton_heap_slot else {
            return Ok(());
        };
        let at_activation = match slot.activation {
            Some((at_block, at_statement)) => block == at_block && statement == Some(at_statement),
            None => block == slot.seed.block && statement.is_none(),
        };
        self.budget.spend(
            self.function
                .as_ref()
                .map_or(0, |function| function.call_evidence.len()),
        )?;
        if !at_activation || !self.singleton_call_matches(&slot.seed) {
            return Ok(());
        }
        let cell = slot.cell;
        let owner = slot.owner;
        let key = CellKey {
            storage: CanonicalPlace {
                root: CanonicalRoot::Owned(owner),
                projection: vec![Projection::DynamicIndex(LocalId(usize::MAX))],
            },
            channel: CellChannel::ReferenceValue,
        };
        let Some(summary) = self.cell(&key)? else {
            return Err(Exhausted);
        };
        // Only the authenticated one-element seed and its exact owner move can
        // initialize this slot. A summary's cardinality is never inferred here.
        state.cells[cell] = state.cells[summary].clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton_heap_domain_requires_exact_original_seed_lineage() {
        for (text, expected) in [
            (
                r#"fn main(){let mut outer=1i32;let mut v=vec![&mut outer];{let mut short=2i32;v[0usize]=&mut short;}println!("{}",v[0usize]);}"#,
                true,
            ),
            (
                r#"fn main(){let mut outer=1i32;let mut replacement=2i32;let mut v=vec![&mut outer];v[0usize]=&mut replacement;println!("{}",v[0usize]);}"#,
                true,
            ),
            (
                r#"fn main(){let mut outer=1i32;let mut other=2i32;let mut v=vec![&mut outer];v.push(&mut other);let _=&mut v[0usize];}"#,
                false,
            ),
            (
                r#"fn main(){let mut outer=1i32;let mut v=vec![&mut outer];let _=&mut v[1usize];}"#,
                false,
            ),
            (
                r#"fn main(){let mut outer=1i32;let mut v=vec![&mut outer];v.reserve(4usize);let _=&mut v[0usize];}"#,
                false,
            ),
            (
                r#"fn main(){let mut outer=1i32;let mut v=vec![&mut outer];let choice=true;if choice{let _=&mut v[0usize];}}"#,
                false,
            ),
            (
                r#"fn main(){let mut outer=1i32;let mut v=vec![&mut outer];let _=&mut v[0usize];let mut moved=v;let _=&mut moved[0usize];}"#,
                false,
            ),
            (
                r#"fn main(){let mut outer=1i32;let mut other=2i32;let mut v=vec![&mut outer,&mut other];let _=&mut v[0usize];}"#,
                false,
            ),
        ] {
            let snapshot = Arc::new(crate::compiler::Snapshot {
                input: crate::compiler::Input::Text(text.into()),
                source_options: crate::compiler::source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let ledger = snapshot.budget.clone();
            let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
            let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
            let requested = engine.request(key.clone());
            engine.run(1).unwrap();
            requested.completion().unwrap();
            let body = engine
                .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
                .unwrap();
            let publication = body
                .publication
                .as_ref()
                .expect("original source publication");
            let function = Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap());
            let facts = publication.facts().clone();
            let mut budget = Budget::new(32_000_000);
            let mut builder = CarrierBuilder::new(
                &function,
                FactsCtx::Validated(&facts),
                ledger.clone(),
                &mut budget,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            builder.function = Some(function.clone());
            builder.authority = Some(facts.clone());
            builder.prepare_singleton_heap_slot().unwrap();
            assert_eq!(builder.singleton_heap_slot.is_some(), expected, "{text}");
            if let Some(slot) = &builder.singleton_heap_slot {
                assert_eq!(
                    builder.cells[slot.cell].key.storage.projection,
                    vec![Projection::ConstantIndex(0)]
                );
                assert!(
                    builder
                        .cell_index
                        .keys()
                        .any(|cell| cell.storage.root == CanonicalRoot::Owned(slot.owner)
                            && cell.storage.projection
                                == vec![Projection::DynamicIndex(LocalId(usize::MAX))]),
                    "abstract summary and exact element remain distinct"
                );
            }
            drop(builder);
            let mut changed = function.as_ref().clone();
            changed.entry = BlockId(changed.blocks.len() - 1);
            let changed = Arc::new(changed);
            let mut budget = Budget::new(32_000_000);
            let mut forged = CarrierBuilder::new(
                &changed,
                FactsCtx::Validated(&facts),
                ledger,
                &mut budget,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            forged.function = Some(changed.clone());
            forged.authority = Some(facts.clone());
            forged.prepare_singleton_heap_slot().unwrap();
            assert!(
                forged.singleton_heap_slot.is_none(),
                "a changed CFG cannot issue element storage"
            );
        }
    }
}
