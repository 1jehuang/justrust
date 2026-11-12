//! Graph-read inputs for the exact source signature of an associated target.
//! Source metadata never substitutes for the final executable call proof.
use super::*;

impl Checker<'_> {
    pub(super) fn associated_signature_nominal_inputs(
        &self,
        target: &ir::CallTarget,
    ) -> Result<tr::GraphInvocationInputs, String> {
        if self.nominal_inputs.is_none() {
            return Ok(tr::GraphInvocationInputs::default());
        }
        let facts = self
            .facts()
            .validated()
            .ok_or("associated signature inputs require original caller facts")?;
        let mut scratch = tr::TraitScratch::default();
        let signature = facts
            .source_direct_call_signature_with_scratch(target, &mut scratch)
            .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?;
        let mut sources = Vec::new();
        let mut missing = Vec::new();
        for root in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.collect_associated_signature_nominals(
                root,
                &mut sources,
                &mut missing,
                &mut scratch,
                false,
                0,
            )?;
        }
        // Every independent root has been inspected before suspending. Never
        // perform receipt replay while any required graph read is unavailable.
        if !missing.is_empty() {
            *self.nominal_blocked.borrow_mut() = Some(NominalPreparation::Blocked {
                nominals: missing.into(),
                declarations: Arc::from([]),
                needs_source_schemas: false,
                unresolved_constraints: false,
            });
            return Err("missing graph input: associated signature nominal realizations".into());
        }
        let mut realized = Vec::new();
        for source in sources {
            let ty = self.realize_constructor_source(&source)?;
            self.infer_storage(1)?;
            realized.push(ty);
        }
        tr::GraphInvocationInputs::new(
            realized,
            self.nominal_inputs.and_then(|inputs| inputs.engine),
            facts,
            &mut scratch,
        )
        .map_err(|error| error.into_legacy_diagnostic(facts.budget()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn collect_associated_signature_nominals(
        &self,
        ty: &ir::Ty,
        sources: &mut Vec<ir::Ty>,
        missing: &mut Vec<NominalDemand>,
        scratch: &mut tr::TraitScratch,
        needed_as_actual: bool,
        depth: usize,
    ) -> Result<bool, String> {
        self.infer_work(depth)?;
        if let Some((declaration, arguments)) = nominal_parts(ty) {
            // Occurrence is only a demand filter, never authority to rewrite
            // source fields. Ordered nominal actuals also need typed reads for
            // the existing complete constructor-substitution receipt replay.
            let needed = needed_as_actual
                || ir::contains_associated_types(ty, &self.signatures.plan()?.budget)?;
            let closed = arguments.iter().all(ir::Ty::is_closed_shallow);
            if needed && closed {
                let facts = self
                    .facts()
                    .validated()
                    .ok_or("associated nominal inputs require original caller facts")?;
                if !facts
                    .world()
                    .prove_original_declaration_predicates(declaration, arguments, facts, scratch)
                    .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?
                {
                    return Ok(false);
                }
            }
            let mut children_ready = true;
            for argument in arguments {
                children_ready &= self.collect_associated_signature_nominals(
                    argument,
                    sources,
                    missing,
                    scratch,
                    needed && closed,
                    depth + 1,
                )?;
            }
            // Symbolic arguments remain a pending frontier, not a recipe for
            // an executable realization or a fabricated normal form.
            if !closed || !children_ready {
                return Ok(false);
            }
            if !needed {
                return Ok(true);
            }
            let demand = self.nominal_input_demand(declaration, arguments)?;
            let inputs = self
                .nominal_inputs
                .ok_or("associated signature lost graph input adapter")?;
            if (inputs.nominal_type)(&demand)?.is_none() {
                let mut duplicate = false;
                for previous in missing.iter() {
                    self.infer_work(depth)?;
                    if previous.declaration == demand.declaration
                        && previous.arguments.len() == demand.arguments.len()
                    {
                        let mut same = true;
                        for (left, right) in previous.arguments.iter().zip(demand.arguments.iter())
                        {
                            same &=
                                left.same_with(right, depth + 1, &mut |d| self.infer_work(d))?;
                        }
                        duplicate |= same;
                    }
                }
                if !duplicate {
                    self.infer_storage(1)?;
                    missing.push(demand);
                }
            }
            self.infer_storage(1)?;
            sources.push(ty.clone());
            return Ok(true);
        }
        let mut children_ready = true;
        macro_rules! child {
            ($value:expr) => {
                children_ready &= self.collect_associated_signature_nominals(
                    $value,
                    sources,
                    missing,
                    scratch,
                    needed_as_actual,
                    depth + 1,
                )?
            };
        }
        match ty {
            ir::Ty::Tuple(parts) => {
                for part in parts.iter() {
                    child!(part);
                }
            }
            ir::Ty::Ref {
                referent: child, ..
            }
            | ir::Ty::RawPtr { pointee: child, .. }
            | ir::Ty::Array { element: child, .. }
            | ir::Ty::Slice { element: child } => {
                child!(child);
            }
            ir::Ty::Cell(wrapper) => {
                child!(wrapper.argument());
            }
            ir::Ty::ManuallyDrop(wrapper) => {
                child!(wrapper.argument());
            }
            ir::Ty::PhantomData(wrapper) => {
                child!(wrapper.argument());
            }
            ir::Ty::FnPointer(pointer) => {
                for part in pointer
                    .inputs
                    .iter()
                    .chain(std::iter::once(&pointer.output))
                {
                    child!(part);
                }
            }
            ir::Ty::FnItem(item) => {
                for part in item.type_args.iter() {
                    child!(part);
                }
            }
            ir::Ty::Dyn(object) => {
                for part in object.args.iter() {
                    child!(part);
                }
                for (_, part) in object.associated_types.iter() {
                    child!(part);
                }
            }
            // Unresolved projections do not authorize speculative graph reads.
            ir::Ty::AssociatedType(_)
            | ir::Ty::Param(_)
            | ir::Ty::DependentArray { .. }
            | ir::Ty::Opaque(_) => return Ok(false),
            _ => {}
        }
        Ok(children_ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn associated_signature_inputs_preserve_missing_graph_and_symbolic_frontiers() {
        let program = parse("trait Family{type Value;}struct Key;impl Family for Key{type Value=i32;}struct Packet<T:Family>{value:T::Value}struct Other<T:Family>{value:T::Value}trait Enabled{}struct Needs<T:Enabled>{value:T}impl<T:Family+Enabled> Family for Needs<T>{type Value=<T as Family>::Value;}trait Candidate{}impl<T:Family+Enabled> Candidate for Packet<Needs<T>>{}fn concrete(value:Packet<Key>,other:Other<Key>){}fn symbolic<T:Family>(value:Packet<T>,other:Other<T>){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration_schema = |_| Ok(None);
        let source_type = |_: &Type| Ok(None);
        let nominal_type = |_: &NominalDemand| Ok(None);
        let inputs =
            NominalPreparationInputs::new(&declaration_schema, &source_type, &nominal_type);
        for (index, closed) in [(0, true), (1, false)] {
            let declaration = plan.function_decl(index).unwrap();
            let signatures = plan.environment(Some(declaration)).unwrap();
            let function = plan.callable_body(&plan.program, declaration).unwrap();
            let signature = plan.function_schema(declaration).unwrap().signature;
            let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
            checker.declaration = Some(declaration);
            checker.nominal_inputs = Some(&inputs);
            let target = ir::CallTarget {
                declaration,
                type_args: checker
                    .facts()
                    .validated()
                    .unwrap()
                    .env()
                    .ordered_params()
                    .map(ir::Ty::Param)
                    .collect::<Vec<_>>()
                    .into(),
            };
            let result = checker.associated_signature_nominal_inputs(&target);
            if closed {
                assert_eq!(
                    result.err().unwrap(),
                    "missing graph input: associated signature nominal realizations"
                );
                let blocked = checker.nominal_blocked.borrow();
                let Some(NominalPreparation::Blocked { nominals, .. }) = blocked.as_ref() else {
                    panic!("missing exact nominal frontier")
                };
                // Both independent roots and their one shared ordered actual
                // are demanded together, with no duplicate Key demand.
                assert_eq!(nominals.len(), 3);
                for parameter in &signature.params {
                    let (owner, arguments) = nominal_parts(parameter).unwrap();
                    assert!(nominals.iter().any(|demand| demand.declaration == owner
                        && demand.arguments.len() == arguments.len()));
                }
            } else {
                assert!(result.unwrap().is_empty());
                assert!(checker.nominal_blocked.borrow().is_none());
            }
        }
        let main = plan.function_decl(2).unwrap();
        let signatures = plan.environment(Some(main)).unwrap();
        let function = plan.callable_body(&plan.program, main).unwrap();
        let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
        checker.declaration = Some(main);
        checker.nominal_inputs = Some(&inputs);
        let world = plan.trait_world().unwrap();
        let concrete = plan
            .function_schema(plan.function_decl(0).unwrap())
            .unwrap()
            .signature;
        let (_, actuals) = nominal_parts(&concrete.params[0]).unwrap();
        let packet = nominal_parts(&concrete.params[0]).unwrap().0;
        let candidate = world
            .all_headers()
            .impls
            .iter()
            .find(|header| {
                nominal_parts(&header.trait_ref.self_ty).is_some_and(|(owner, _)| owner == packet)
            })
            .unwrap();
        let facts = checker.facts().validated().unwrap();
        let mut scratch = tr::TraitScratch::default();
        let unproved = world
            .original_implementation_self_for_discovery(
                candidate.declaration,
                actuals,
                facts,
                &mut scratch,
            )
            .unwrap();
        assert!(ir::contains_associated_types(&unproved, &budget).unwrap());
        let mut collected = Vec::new();
        let mut missing = Vec::new();
        assert!(
            !checker
                .collect_associated_signature_nominals(
                    &unproved,
                    &mut collected,
                    &mut missing,
                    &mut scratch,
                    false,
                    0,
                )
                .unwrap()
        );
        assert!(collected.is_empty());
        assert!(missing.is_empty());
        assert!(checker.nominal_blocked.borrow().is_none());
    }
}
