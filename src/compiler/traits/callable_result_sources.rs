//! Per-instance result sources for a family produced by a callable argument.
//!
//! `fn fold<B, F: FnMut(B, Self::Item) -> B>(self, init: B, f: F) -> B`
//! declares only `init` as the source of its `B` result. A concrete callable
//! can also return carriers it received through its argument tuple or holds
//! in its captures. Each such carrier is mapped back through the original
//! declared argument type to the exact caller input occurrence: a projected
//! item through its selected implementation, a family parameter through its
//! substituted inputs. The outer borrow of a collection is never a source
//! unless the callable result really selects it. An instance that cannot be
//! expressed fails closed instead of losing a loan.
use super::*;

impl ValidatedTraitWorld {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler::traits) fn callable_result_sources(
        &self,
        result: &ir::CarrierReturn,
        output: &ExpandedCarrier,
        declared_inputs: &[ir::InputCarrier],
        raw: &GenericSchema,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
        actuals: &nominal_carriers::CarrierFamilyActuals,
    ) -> Result<Vec<ir::InputCarrier>> {
        let mut sources = Vec::new();
        let ir::CarrierChannel::ParamTail {
            parameter: family,
            prefix: 0,
        } = result.location.channel
        else {
            return Ok(sources);
        };
        if facts.phase() != FactsPhase::Closed
            || !matches!(output.location.channel, ir::CarrierChannel::Fixed(_))
        {
            return Ok(sources);
        }
        let Some(component) = output.component.as_ref() else {
            return Ok(sources);
        };
        let unsupported = |message: &str| Err(FactsError::unsupported(&self.budget, message));
        for predicate in raw.predicates.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Predicate::ProjectionEq { projection, ty } = predicate else {
                continue;
            };
            let Ty::Param(callable) = projection.trait_ref.self_ty else {
                continue;
            };
            if self
                .closure_trait_kind(projection.trait_ref.declaration)?
                .is_none()
            {
                continue;
            }
            if *ty != Ty::Param(family) {
                if self.callable_type_mentions(ty, family, 0)? {
                    return unsupported(
                        "unsupported: callable result carries a reference family through a nested output type",
                    );
                }
                continue;
            }
            let [Ty::Tuple(declared)] = projection.trait_ref.args.as_ref() else {
                return unsupported("unsupported: callable bound without an argument tuple");
            };
            let Some(index) = raw.env.ordered_params().position(|p| p == callable) else {
                return unsupported("unsupported: callable bound names a foreign parameter");
            };
            let Some(actual) = args.get(index) else {
                return unsupported("unsupported: callable bound lost its instance argument");
            };
            // (tuple element, location within it) pairs selected by the actual
            // callable's own result contract, and whether captures flow out.
            let mut selected: Vec<(usize, ir::CarrierLocation)> = Vec::new();
            let mut captures = false;
            match actual {
                Ty::Closure(closure) => {
                    let Some(mode) = self.callable_bound_mode(callable, closure, raw)? else {
                        return unsupported(
                            "unsupported: callable bound mode does not match the closure",
                        );
                    };
                    let target = Callee::Direct(self.closure_target(closure, mode)?);
                    let view = self.carrier_contract(SignatureSubject::Call(&target), facts)?;
                    let Some(row) = self.callable_result_row(&view, component)? else {
                        return unsupported(
                            "unsupported: closure result lacks the instantiated result carrier",
                        );
                    };
                    for source in row.sources.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        match (source.param, source.location.path.0.split_first()) {
                            (0, _) => captures = true,
                            (1, Some((ir::CarrierStep::Field(k), rest))) => {
                                retain::<ir::CarrierStep>(&self.budget, rest.len())?;
                                push(
                                    &self.budget,
                                    &mut selected,
                                    (
                                        *k,
                                        ir::CarrierLocation {
                                            path: ir::CarrierPath(rest.to_vec().into()),
                                            channel: source.location.channel.clone(),
                                        },
                                    ),
                                )?;
                            }
                            _ => {
                                return unsupported(
                                    "unsupported: closure result source outside its argument tuple",
                                );
                            }
                        }
                    }
                }
                Ty::FnItem(item) => {
                    let target = Callee::Direct(item.as_ref().clone());
                    let view = self.carrier_contract(SignatureSubject::Call(&target), facts)?;
                    let Some(row) = self.callable_result_row(&view, component)? else {
                        return unsupported(
                            "unsupported: function item result lacks the instantiated result carrier",
                        );
                    };
                    for source in row.sources.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        push(
                            &self.budget,
                            &mut selected,
                            (source.param, source.location.clone()),
                        )?;
                    }
                }
                _ => {
                    return unsupported(
                        "unsupported: reference result through an opaque callable argument",
                    );
                }
            }
            if captures {
                self.callable_family_sources(
                    callable,
                    None,
                    declared_inputs,
                    raw,
                    args,
                    nominals,
                    facts,
                    scratch,
                    actuals,
                    &mut sources,
                )?;
            }
            for (element, relative) in selected {
                let Some(declared) = declared.get(element) else {
                    return unsupported("unsupported: callable result names a foreign argument");
                };
                self.callable_argument_sources(
                    declared,
                    &relative,
                    declared_inputs,
                    raw,
                    args,
                    nominals,
                    facts,
                    scratch,
                    actuals,
                    &mut sources,
                    0,
                )?;
            }
        }
        Ok(sources)
    }

    fn callable_result_row<'a>(
        &self,
        view: &'a CarrierContractView,
        component: &ir::CarrierLocation,
    ) -> Result<Option<&'a ir::CarrierReturn>> {
        for row in view.contract().results.iter() {
            if self.same_location(&row.location, component)? {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    // The bound selects the capability the callee uses, not the closure's
    // most permissive one. `FnOnce` alone comes from the `Output` projection.
    fn callable_bound_mode(
        &self,
        callable: ParamId,
        closure: &Arc<ir::ClosureTy>,
        raw: &GenericSchema,
    ) -> Result<Option<ir::ClosureKind>> {
        let mut chosen = None;
        for predicate in raw.predicates.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Predicate::Implements(goal) = predicate else {
                continue;
            };
            if goal.self_ty != Ty::Param(callable) {
                continue;
            }
            let Some(kind) = self.closure_trait_kind(goal.declaration)? else {
                continue;
            };
            chosen = Some(match (chosen, kind) {
                (Some(ir::ClosureKind::Fn), _) | (_, ir::ClosureKind::Fn) => ir::ClosureKind::Fn,
                (Some(ir::ClosureKind::FnMut), _) | (_, ir::ClosureKind::FnMut) => {
                    ir::ClosureKind::FnMut
                }
                _ => ir::ClosureKind::FnOnce,
            });
        }
        Ok(chosen.filter(|mode| closure.supports(*mode)))
    }

    fn callable_type_mentions(&self, ty: &Ty, parameter: ParamId, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        for location in charged(&self.budget, ir::carrier_locations(ty, &self.budget))?.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if matches!(location.channel, ir::CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // Every substituted occurrence of one declared family parameter whose
    // component is `relative` (or every component when `relative` is None).
    #[allow(clippy::too_many_arguments)]
    fn callable_family_sources(
        &self,
        parameter: ParamId,
        relative: Option<&ir::CarrierLocation>,
        declared_inputs: &[ir::InputCarrier],
        raw: &GenericSchema,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
        actuals: &nominal_carriers::CarrierFamilyActuals,
        sources: &mut Vec<ir::InputCarrier>,
    ) -> Result<bool> {
        let mut found = false;
        for input in declared_inputs {
            charged(&self.budget, self.budget.charge_work(1))?;
            if !matches!(input.location.channel, ir::CarrierChannel::ParamTail { parameter: p, prefix: 0 } if p == parameter)
            {
                continue;
            }
            for expanded in self.expand_carrier_in(
                &input.location,
                raw.env.as_ref(),
                args,
                nominals,
                facts,
                scratch,
                actuals,
            )? {
                let selected = match (relative, expanded.component.as_ref()) {
                    (None, _) => true,
                    (Some(wanted), Some(component)) => self.same_location(component, wanted)?,
                    (Some(_), None) => false,
                };
                if selected {
                    found = true;
                    self.push_callable_source(
                        sources,
                        ir::InputCarrier {
                            param: input.param,
                            location: expanded.location,
                        },
                    )?;
                }
            }
        }
        Ok(found)
    }

    #[allow(clippy::too_many_arguments)]
    fn callable_argument_sources(
        &self,
        declared: &Ty,
        relative: &ir::CarrierLocation,
        declared_inputs: &[ir::InputCarrier],
        raw: &GenericSchema,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
        actuals: &nominal_carriers::CarrierFamilyActuals,
        sources: &mut Vec<ir::InputCarrier>,
        level: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(level))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let unsupported = |message: &str| Err(FactsError::unsupported(&self.budget, message));
        match declared {
            Ty::Param(parameter) => {
                if !self.callable_family_sources(
                    *parameter,
                    Some(relative),
                    declared_inputs,
                    raw,
                    args,
                    nominals,
                    facts,
                    scratch,
                    actuals,
                    sources,
                )? {
                    return unsupported(
                        "unsupported: callable argument family has no caller input occurrence",
                    );
                }
                Ok(())
            }
            Ty::AssociatedType(projection) => {
                let Ty::Param(owner) = projection.trait_ref.self_ty else {
                    return unsupported(
                        "unsupported: callable argument projection of a non-parameter",
                    );
                };
                let mut position = None;
                for (index, ty) in raw.signature.params.iter().enumerate() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if *ty == Ty::Param(owner) && position.replace(index).is_some() {
                        return unsupported(
                            "unsupported: callable argument projection owner is not unique",
                        );
                    }
                }
                let Some(position) = position else {
                    return unsupported(
                        "unsupported: callable argument projection owner is not an input",
                    );
                };
                let context = ItemRegionContext {
                    env: &raw.env,
                    arguments: args,
                    predicates: &raw.predicates,
                    self_ty: &raw.signature.params[position],
                };
                let found = self.item_projection_regions(
                    projection,
                    relative,
                    &context,
                    &ir::CarrierPath::default(),
                    facts,
                    0,
                    scratch,
                )?;
                let found = match found {
                    Some(found) if !found.is_empty() => found,
                    // A root reference layer of the item (`Item = &'a T` at
                    // depth 0) names an impl lifetime, not a family. It can
                    // only be stored in the owner input, so every carrier of
                    // that input is a superset of its exact occurrence.
                    _ if relative.path.0.is_empty()
                        && matches!(relative.channel, ir::CarrierChannel::Fixed(_)) =>
                    {
                        if !self.callable_family_sources(
                            owner,
                            None,
                            declared_inputs,
                            raw,
                            args,
                            nominals,
                            facts,
                            scratch,
                            actuals,
                            sources,
                        )? {
                            return unsupported(
                                "unsupported: callable argument item carrier has no exact input occurrence",
                            );
                        }
                        return Ok(());
                    }
                    _ => {
                        return unsupported(
                            "unsupported: callable argument item carrier has no exact input occurrence",
                        );
                    }
                };
                for location in found {
                    let source = ir::InputCarrier {
                        param: position,
                        location,
                    };
                    let mut admitted = false;
                    for input in declared_inputs
                        .iter()
                        .filter(|input| input.param == position)
                    {
                        for expanded in self.expand_carrier_in(
                            &input.location,
                            raw.env.as_ref(),
                            args,
                            nominals,
                            facts,
                            scratch,
                            actuals,
                        )? {
                            admitted |= self.same_location(&source.location, &expanded.location)?;
                        }
                    }
                    if !admitted {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "callable argument source leaves original input family",
                        ));
                    }
                    self.push_callable_source(sources, source)?;
                }
                Ok(())
            }
            Ty::Ref { referent, .. } if relative.path.0.is_empty() => match relative.channel {
                // A borrow the callee creates for the call is local to it and
                // higher-ranked in the bound: it cannot flow into the result.
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }) => Ok(()),
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth }) => {
                    let inner = ir::CarrierLocation {
                        path: ir::CarrierPath::default(),
                        channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                            depth: depth - 1,
                        }),
                    };
                    self.callable_argument_sources(
                        referent,
                        &inner,
                        declared_inputs,
                        raw,
                        args,
                        nominals,
                        facts,
                        scratch,
                        actuals,
                        sources,
                        level + 1,
                    )
                }
                _ => unsupported("unsupported: callable reference argument carrier channel"),
            },
            _ => unsupported("unsupported: callable argument type for result source transport"),
        }
    }

    pub(in crate::compiler::traits) fn push_callable_source(
        &self,
        sources: &mut Vec<ir::InputCarrier>,
        source: ir::InputCarrier,
    ) -> Result<()> {
        charged(&self.budget, self.budget.charge_work(sources.len() + 1))?;
        if sources.contains(&source) {
            return Ok(());
        }
        if sources.len() >= ir::MAX_LOAN_SITES {
            return Err(FactsError::resource(
                &self.budget,
                "callable result source limit exceeded",
            ));
        }
        push(&self.budget, sources, source)
    }
}
