//! Pointwise lifetime transport through an original projected item family.
//!
//! This is deliberately not a return-body summary. The only narrowed case is
//! one independent result family, one by-value input family, and one selected
//! trait-head correspondence between that result and a projected input item.
//! Every step uses an original parameter identity. Concrete type equality is
//! never used to discover an input occurrence.
use super::*;
#[path = "callable_result_sources.rs"]
mod callable_results;

struct ItemRegionContext<'a> {
    env: &'a Arc<GenericEnv>,
    arguments: &'a [Ty],
    predicates: &'a [Predicate],
    self_ty: &'a Ty,
}

impl ValidatedTraitWorld {
    #[cfg(test)]
    pub(super) fn rejects_partial_item_regions_for_tests(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<bool> {
        let scratch = &mut TraitScratch::default();
        let parameter = view.original_env.ordered_params().next().unwrap();
        let actual = &view.arguments[0];
        let locations = charged(&self.budget, ir::carrier_locations(actual, &self.budget))?;
        let wanted = &locations[0];
        let raw = self.normalized_declaration_schema(view.declaration)?;
        let projection = raw
            .predicates
            .iter()
            .find_map(|row| match row {
                Predicate::ProjectionEq { projection, .. } => Some(projection),
                _ => None,
            })
            .unwrap();
        for other in [
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(Ty::Param(parameter)),
            },
            Ty::AssociatedType(Arc::new(projection.clone())),
        ] {
            let self_ty = Ty::Tuple(Arc::from([Ty::Param(parameter), other]));
            let context = ItemRegionContext {
                env: &view.original_env,
                arguments: &view.arguments,
                predicates: &[],
                self_ty: &self_ty,
            };
            if self
                .item_parameter_regions(
                    parameter,
                    wanted,
                    &context,
                    &ir::CarrierPath::default(),
                    facts,
                    0,
                    scratch,
                )?
                .is_some()
            {
                return Ok(false);
            }
        }
        let self_ty = Ty::Param(parameter);
        let context = ItemRegionContext {
            env: &view.original_env,
            arguments: &view.arguments,
            predicates: &raw.predicates,
            self_ty: &self_ty,
        };
        if self
            .item_parameter_regions(
                parameter,
                wanted,
                &context,
                &ir::CarrierPath::default(),
                facts,
                0,
                scratch,
            )?
            .is_some()
        {
            return Ok(false);
        }
        let (bound, input) = raw
            .predicates
            .iter()
            .find_map(|row| match row {
                Predicate::Implements(goal) if goal.self_ty == raw.signature.result => {
                    match goal.args.as_ref() {
                        [Ty::AssociatedType(input)] => Some((goal, input.as_ref())),
                        _ => None,
                    }
                }
                _ => None,
            })
            .unwrap();
        let context = ItemRegionContext {
            env: &raw.env,
            arguments: &view.arguments,
            predicates: &raw.predicates,
            self_ty: &raw.signature.params[0],
        };
        if !self.closed_item_root_context(&raw, bound, input, &context, facts, scratch)? {
            return Ok(false);
        }
        let actual_bridge = ir::AssociatedTypeProjection {
            item: projection.item,
            trait_ref: TraitRef {
                declaration: projection.trait_ref.declaration,
                self_ty: self.item_region_substitution(
                    &projection.trait_ref.self_ty,
                    &context,
                    facts,
                    scratch,
                )?,
                args: Arc::from([]),
            },
        };
        let membership = self.require_in(
            &Predicate::Implements(actual_bridge.trait_ref.clone()),
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        let replay = self.replay_implementation_assignment_with(
            &actual_bridge,
            &membership,
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        if !matches!(replay, Ty::AssociatedType(_)) {
            return Ok(false);
        }
        match self.implementation_equality_rhs_with(
            &actual_bridge,
            &membership,
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        ) {
            Err(error) if error.kind() == FactsErrorKind::Unsupported => {}
            _ => return Ok(false),
        }
        for mutation in 0..5 {
            let mut forged = membership.clone();
            let record = Arc::make_mut(&mut forged.0);
            match mutation {
                0 => record.phase = FactsPhase::Universal,
                1 => record.goal = Predicate::Implements(bound.clone()),
                2 => record.origin = ProofOrigin::Assumption,
                _ => {
                    let ProofOrigin::Implementation {
                        declaration,
                        arguments,
                    } = &mut record.origin
                    else {
                        return Ok(false);
                    };
                    if mutation == 3 {
                        *declaration = view.declaration;
                    } else {
                        *arguments = Arc::from([Ty::Bool]);
                    }
                }
            }
            match self.replay_implementation_assignment_with(
                &actual_bridge,
                &forged,
                ProofContext::from_facts(facts),
                scratch.for_world(self.id),
            ) {
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata => {}
                _ => return Ok(false),
            }
        }
        let sized = TraitRef {
            declaration: self.catalog.id(CatalogItem::Sized),
            self_ty: raw.signature.result.clone(),
            args: Arc::from([]),
        };
        if !self
            .item_region_predicates(
                &[Predicate::Implements(sized.clone())],
                &context,
                facts,
                scratch,
            )?
            .is_empty()
        {
            return Ok(false);
        }
        let mut lookalike = sized.clone();
        lookalike.declaration = bound.declaration;
        if self
            .item_region_predicates(
                &[Predicate::Implements(lookalike)],
                &context,
                facts,
                scratch,
            )?
            .len()
            != 1
        {
            return Ok(false);
        }
        let mut malformed = sized;
        malformed.args = Arc::from([raw.signature.params[0].clone()]);
        if self
            .item_region_predicates(
                &[Predicate::Implements(malformed)],
                &context,
                facts,
                scratch,
            )?
            .len()
            != 1
        {
            return Ok(false);
        }
        let required = self
            .headers
            .items
            .iter()
            .find(|item| item.trait_decl == bound.declaration && item.default.is_none())
            .unwrap();
        if self
            .item_region_declared_context(required.declaration)?
            .is_some()
        {
            return Ok(false);
        }
        let actual_output =
            self.item_region_substitution(&raw.signature.result, &context, facts, scratch)?;
        let actual_element = self.item_region_substitution(
            &Ty::AssociatedType(Arc::new(input.clone())),
            &context,
            facts,
            scratch,
        )?;
        // The original bridge projection supplies the genuine constructor
        // argument, not a fabricated type or an independently guessed wrapper.
        let actual_wrapper =
            self.item_region_substitution(&projection.trait_ref.self_ty, &context, facts, scratch)?;
        let required_call = ir::TraitCall {
            trait_ref: TraitRef {
                declaration: bound.declaration,
                self_ty: actual_output,
                args: Arc::from([actual_element]),
            },
            item: required.declaration,
            method_args: Arc::from([actual_wrapper]),
        };
        let callee = Callee::TraitItem(required_call);
        let required_view = self.carrier_contract(SignatureSubject::Call(&callee), facts)?;
        required_view.validate_for_with_scratch(facts, scratch)?;
        if required_view.contract.results.len() != 1
            || required_view.contract.results[0].sources.len() != 1
        {
            return Ok(false);
        }
        let expected_source = self.item_region_prefix(
            &ir::CarrierPath(Arc::from([ir::CarrierStep::Field(0)])),
            &view.contract.results[0].sources[0].location,
        )?;
        if required_view.contract.results[0].sources[0].param != 0
            || !self.same_location(
                &required_view.contract.results[0].sources[0].location,
                &expected_source,
            )?
        {
            return Ok(false);
        }
        let required_raw = self.normalized_declaration_schema(required.declaration)?;
        let (required_bound, required_projection) = required_raw
            .predicates
            .iter()
            .find_map(|row| {
                let Predicate::Implements(goal) = row else {
                    return None;
                };
                (goal.self_ty == required_raw.signature.result && goal.args.len() == 1).then(|| {
                    let projection = required_raw
                        .predicates
                        .iter()
                        .find_map(|row| match row {
                            Predicate::ProjectionEq { projection, .. } => Some(projection),
                            _ => None,
                        })
                        .unwrap();
                    (goal, projection)
                })
            })
            .unwrap();
        let required_context = ItemRegionContext {
            env: &required_raw.env,
            arguments: &required_view.arguments,
            predicates: &required_raw.predicates,
            self_ty: &required_raw.signature.params[0],
        };
        if !self.closed_item_required_root(
            required.declaration,
            &required_raw,
            required_bound,
            required_projection,
            &required_context,
            facts,
            scratch,
        )? {
            return Ok(false);
        }
        for mutation in 0..3 {
            let mut forged = required_raw.clone();
            let mut rows = forged.predicates.to_vec();
            let mut owner = required.declaration;
            match mutation {
                0 => rows.push(Predicate::Implements(required_projection.trait_ref.clone())),
                1 => {
                    let row = rows
                        .iter_mut()
                        .find(|row| matches!(row, Predicate::ProjectionEq { .. }))
                        .unwrap();
                    let Predicate::ProjectionEq { ty, .. } = row else {
                        unreachable!();
                    };
                    *ty = required_raw.signature.result.clone();
                }
                _ => owner = view.declaration,
            }
            forged.predicates = rows.into();
            match self.closed_item_required_root(
                owner,
                &forged,
                required_bound,
                required_projection,
                &required_context,
                facts,
                scratch,
            ) {
                Ok(false) => {}
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata => {}
                _ => return Ok(false),
            }
        }
        for mutation in 0..6 {
            let mut forged = required_view.as_ref().clone();
            match mutation {
                0 => {
                    Arc::make_mut(&mut Arc::make_mut(&mut forged.contract.results)[0].sources)[0]
                        .location
                        .path = ir::CarrierPath(Arc::from([
                        ir::CarrierStep::Field(0),
                        ir::CarrierStep::Field(0),
                        ir::CarrierStep::Field(0),
                    ]))
                }
                1 => forged.declaration = view.declaration,
                2 => forged.arguments = Arc::from([]),
                3 => Arc::make_mut(&mut forged.source_regions).declaration = view.declaration,
                4 => {
                    Arc::make_mut(&mut forged.original_contract.results)[0].sources =
                        required_view.contract.results[0].sources.clone()
                }
                _ => {
                    let parameter_equality::EqualityResultSubject::Required(call) =
                        &mut forged.result_subject
                    else {
                        return Ok(false);
                    };
                    call.trait_ref.args = Arc::from([]);
                }
            }
            match forged.validate_for(facts) {
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata => {}
                _ => return Ok(false),
            }
        }
        let mut forged = raw.clone();
        let mut rows = forged.predicates.to_vec();
        let mut foreign_bound = bound.clone();
        foreign_bound.self_ty = raw.signature.params[0].clone();
        rows.push(Predicate::Implements(foreign_bound));
        forged.predicates = rows.into();
        Ok(!self.closed_item_root_context(&forged, bound, input, &context, facts, scratch)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn projected_item_result_sources(
        &self,
        declaration: DeclId,
        result: &ir::CarrierReturn,
        output: &ExpandedCarrier,
        raw: &GenericSchema,
        arguments: &[Ty],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Arc<[ir::InputCarrier]>>> {
        charged(&self.budget, self.budget.charge_work(raw.predicates.len()))?;
        if matches!(raw.signature.result, Ty::Enum(_)) {
            return self.direct_projected_envelope_sources(
                declaration,
                result,
                output,
                raw,
                arguments,
                facts,
                scratch,
            );
        }
        if matches!(raw.signature.result, Ty::Struct(_)) {
            return self.direct_item_result_sources(
                declaration,
                result,
                output,
                raw,
                arguments,
                facts,
                scratch,
            );
        }
        if facts.phase() != FactsPhase::Closed
            || raw.signature.params.len() != 1
            || !result.location.path.0.is_empty()
            // Existing equality-result sealing must be mandatory for this
            // declaration, including when a forged view removes its receipt.
            || !raw.predicates.iter().any(|row| matches!(row, Predicate::ProjectionEq { .. }))
        {
            return Ok(None);
        }
        let required = self.info(declaration)?.kind == DeclKind::TraitItem;
        let Ty::Param(result_parameter) = raw.signature.result else {
            return Ok(None);
        };
        let Ty::Param(input_parameter) = raw.signature.params[0] else {
            return Ok(None);
        };
        if result_parameter == input_parameter
            || result.location.channel
                != (ir::CarrierChannel::ParamTail {
                    parameter: result_parameter,
                    prefix: 0,
                })
        {
            return Ok(None);
        }
        let Some(component) = output.component.as_ref() else {
            return Ok(None);
        };
        let context = ItemRegionContext {
            env: &raw.env,
            arguments,
            predicates: &raw.predicates,
            self_ty: &raw.signature.params[0],
        };
        let rows = self.item_region_predicates(&raw.predicates, &context, facts, scratch)?;
        let mut bound = None;
        for predicate in rows {
            charged(&self.budget, self.budget.charge_work(1))?;
            let goal = predicate.trait_ref();
            let mentions = self.item_region_mentions(&goal.self_ty, result_parameter, 0)?;
            let mut argument_mentions = false;
            for argument in goal.args.iter() {
                argument_mentions |= self.item_region_mentions(argument, result_parameter, 0)?;
            }
            let rhs_mentions = match predicate {
                Predicate::ProjectionEq { ty, .. } => {
                    self.item_region_mentions(ty, result_parameter, 0)?
                }
                _ => false,
            };
            if !mentions && !argument_mentions && !rhs_mentions {
                continue;
            }
            let Predicate::Implements(goal) = predicate else {
                return Ok(None);
            };
            if goal.self_ty != Ty::Param(result_parameter)
                || goal.args.len() != 1
                || argument_mentions
                || bound.is_some()
            {
                return Ok(None);
            }
            let projection = match &goal.args[0] {
                Ty::AssociatedType(projection) => projection.as_ref(),
                Ty::Param(parameter) if required => {
                    let mut linked = None;
                    for row in raw.predicates.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if let Predicate::ProjectionEq {
                            projection,
                            ty: Ty::Param(p),
                        } = row
                            && p == parameter
                            && projection.trait_ref.self_ty == Ty::Param(input_parameter)
                            && linked.replace(projection).is_some()
                        {
                            return Ok(None);
                        }
                    }
                    let Some(projection) = linked else {
                        return Ok(None);
                    };
                    projection
                }
                _ => return Ok(None),
            };
            if projection.trait_ref.self_ty != Ty::Param(input_parameter) {
                return Ok(None);
            }
            bound = Some((goal, projection));
        }
        let Some((goal, projection)) = bound else {
            return Ok(None);
        };
        let context = ItemRegionContext {
            env: &raw.env,
            arguments,
            predicates: &raw.predicates,
            self_ty: &raw.signature.params[0],
        };
        if required {
            if !self.closed_item_required_root(
                declaration,
                raw,
                goal,
                projection,
                &context,
                facts,
                scratch,
            )? {
                return Ok(None);
            }
        } else {
            if !self.closed_item_root_context(raw, goal, projection, &context, facts, scratch)? {
                return Ok(None);
            }
            // Only executable declarations have retained body obligations. A
            // required item uses its separate original trait-header protocol.
            let Some(original) = self.item_region_declared_context(declaration)? else {
                return Ok(None);
            };
            if original.origins.is_empty() {
                return Ok(None);
            }
        }
        let actual_self = self.item_region_substitution(&goal.self_ty, &context, facts, scratch)?;
        let actual_argument =
            self.item_region_substitution(&goal.args[0], &context, facts, scratch)?;
        retain::<Ty>(&self.budget, 1)?;
        let actual_goal = TraitRef {
            declaration: goal.declaration,
            self_ty: actual_self,
            args: Arc::from([actual_argument]),
        };
        let membership = self.require_in(
            &Predicate::Implements(actual_goal),
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &membership.0.origin
        else {
            return Ok(None);
        };
        let position = self
            .impl_header_index
            .get(&self.budget, *declaration)?
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "result item region selection lost original impl",
                )
            })?;
        let header = &self.headers.impls[position];
        if header.trait_ref.args.len() != 1 {
            return Ok(None);
        }
        let selected = ItemRegionContext {
            env: &header.schema_env,
            arguments,
            predicates: &header.predicates,
            self_ty: &header.trait_ref.self_ty,
        };
        let inverse = self.item_region_locations(
            &header.trait_ref.self_ty,
            &selected,
            component,
            facts,
            scratch,
        )?;
        if inverse.is_empty() {
            return Ok(None);
        }
        let mut sources = Vec::new();
        for (parameter, relative) in inverse {
            let mut witnessed = false;
            for location in charged(
                &self.budget,
                ir::carrier_locations(&header.trait_ref.args[0], &self.budget),
            )?
            .iter()
            {
                if !matches!(location.channel, ir::CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
                {
                    continue;
                }
                charged(&self.budget, self.budget.charge_work(location.path.0.len()))?;
                if location
                    .path
                    .0
                    .iter()
                    .any(|step| matches!(step, ir::CarrierStep::VariantField { .. }))
                {
                    return Ok(None);
                }
                for expanded in self.expand_carrier_with_scratch(
                    location,
                    selected.env,
                    selected.arguments,
                    &[],
                    facts,
                    scratch,
                )? {
                    let Some(component) = expanded.component.as_ref() else {
                        continue;
                    };
                    if !self.same_location(component, &relative)? {
                        continue;
                    }
                    let Some(found) = self.item_projection_regions(
                        projection,
                        &expanded.location,
                        &context,
                        &ir::CarrierPath::default(),
                        facts,
                        0,
                        scratch,
                    )?
                    else {
                        return Ok(None);
                    };
                    witnessed = true;
                    for source in found {
                        if sources.len() >= ir::MAX_LOAN_SITES {
                            return Err(FactsError::resource(
                                &self.budget,
                                "projected item source limit exceeded",
                            ));
                        }
                        push(
                            &self.budget,
                            &mut sources,
                            ir::InputCarrier {
                                param: 0,
                                location: source,
                            },
                        )?;
                    }
                }
            }
            if !witnessed {
                return Ok(None);
            }
        }
        if sources.is_empty() {
            return Ok(None);
        }
        retain::<ir::InputCarrier>(&self.budget, sources.len())?;
        Ok(Some(sources.into()))
    }

    /// Authenticate a direct implementation of an original required associated
    /// result envelope. The envelope grants no lifetime relationship: only the
    /// original selected assignment and its callable equality protocol do.
    #[allow(clippy::too_many_arguments)]
    fn direct_projected_envelope_sources(
        &self,
        declaration: DeclId,
        result: &ir::CarrierReturn,
        output: &ExpandedCarrier,
        raw: &GenericSchema,
        arguments: &[Ty],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Arc<[ir::InputCarrier]>>> {
        if facts.phase() != FactsPhase::Closed
            || raw.signature.params.len() != 1
            || !result.sources.is_empty()
        {
            return Ok(None);
        }
        let Ty::Enum(envelope) = &raw.signature.result else {
            return Ok(None);
        };
        let [Ty::Param(parameter)] = envelope.def().type_args.as_ref() else {
            return Ok(None);
        };
        let Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent,
        } = &raw.signature.params[0]
        else {
            return Ok(None);
        };
        let Ty::Struct(_) = referent.as_ref() else {
            return Ok(None);
        };
        let info = self.info(declaration)?;
        let Some(owner) = info.owner else {
            return Ok(None);
        };
        let Some(position) = self.impl_header_index.get(&self.budget, owner)? else {
            return Ok(None);
        };
        let header = &self.headers.impls[position];
        if header.declaration != owner
            || parameter.owner.declaration != owner
            || info.own_env.arity() != 0
            || !matches!(header.schema_env.total_arity(), 2 | 4)
            || arguments.len() != header.schema_env.total_arity()
            || raw.env.total_arity() != header.schema_env.total_arity()
            || !self.method_environment_matches(&raw.env, &header.schema_env, &info.own_env)?
            || !header.trait_ref.args.is_empty()
            || !self.eq_ty(referent, &header.trait_ref.self_ty, 0)?
            || result.location.channel
                != (ir::CarrierChannel::ParamTail {
                    parameter: *parameter,
                    prefix: 0,
                })
        {
            return Ok(None);
        }
        let [ir::CarrierStep::VariantField { variant, field }] = result.location.path.0.as_ref()
        else {
            return Ok(None);
        };
        let original = self.schema.declaration_schema(declaration)?;
        if !self.same_env(&raw.env, &original.env)?
            || !self.eq_signature(&raw.signature, &original.signature)?
        {
            return Ok(None);
        }
        self.compare_predicate_rows(&raw.predicates, &original.predicates)?;
        let locations = charged(
            &self.budget,
            ir::carrier_locations(&raw.signature.result, &self.budget),
        )?;
        if locations.len() != 1 || !self.same_location(&locations[0], &result.location)? {
            return Ok(None);
        }
        if variant.owner != envelope.def().id {
            return Ok(None);
        }
        let Some(payload_variant) = envelope.def().variants.get(variant.index) else {
            return Ok(None);
        };
        let fields = nominal_fields(&payload_variant.fields, &self.budget)?;
        if fields
            .get(*field)
            .is_none_or(|field| field.ty != Ty::Param(*parameter))
        {
            return Ok(None);
        }
        let mut required_item = None;
        for (required, target) in header.items.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if *target == declaration && required_item.replace(*required).is_some() {
                return Ok(None);
            }
        }
        let Some(required_item) = required_item else {
            return Ok(None);
        };
        let required = self.item_header(required_item)?;
        if required.trait_decl != header.trait_ref.declaration
            || required.default.is_some()
            || self.info(required_item)?.own_env.arity() != 0
        {
            return Ok(None);
        }
        let required_raw = self.schema.declaration_schema(required_item)?;
        let Ty::Enum(required_envelope) = &required_raw.signature.result else {
            return Ok(None);
        };
        if required_envelope.def().source_decl != envelope.def().source_decl {
            return Ok(None);
        }
        let [Ty::AssociatedType(projection)] = required_envelope.def().type_args.as_ref() else {
            return Ok(None);
        };
        let trait_header = self.trait_header(header.trait_ref.declaration)?;
        if projection.trait_ref.declaration != header.trait_ref.declaration
            || projection.trait_ref.self_ty != Ty::Param(trait_header.self_param)
            || !projection.trait_ref.args.is_empty()
        {
            return Ok(None);
        }
        let [
            Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: required_self,
            },
        ] = required_raw.signature.params.as_slice()
        else {
            return Ok(None);
        };
        if required_self.as_ref() != &Ty::Param(trait_header.self_param) {
            return Ok(None);
        }
        let Some(required_variant) = required_envelope.def().variants.get(variant.index) else {
            return Ok(None);
        };
        let required_fields = nominal_fields(&required_variant.fields, &self.budget)?;
        let Some(required_field) = required_fields.get(*field) else {
            return Ok(None);
        };
        if !matches!(&required_field.ty, Ty::AssociatedType(p) if self.same_projection(p, projection)?)
        {
            return Ok(None);
        }
        let assignment = self.associated_value(owner, projection.item)?;
        self.checked_assignment_source(assignment)?;
        if assignment.rhs != Ty::Param(*parameter) {
            return Ok(None);
        }
        // Exact original rows: the implementation predicates plus its own
        // trait membership, never a concrete-type or enum-name correspondence.
        if raw.predicates.len() != header.predicates.len() + 1 {
            return Ok(None);
        }
        let own_row = Predicate::Implements(header.trait_ref.clone());
        for expected in header.predicates.iter().chain(std::iter::once(&own_row)) {
            let mut count = 0;
            for actual in raw.predicates.iter() {
                count += usize::from(self.eq_predicate(expected, actual)?);
            }
            if count != 1 {
                return Ok(None);
            }
        }
        let context = ItemRegionContext {
            env: &header.schema_env,
            arguments,
            predicates: &header.predicates,
            self_ty: &header.trait_ref.self_ty,
        };
        let mut output_projection = None;
        let mut borrowed_output = false;
        for row in header.predicates.iter() {
            if let Predicate::ProjectionEq { projection, ty } = row {
                let borrowed = matches!(ty,
                    Ty::Ref { kind: ir::BorrowKind::Shared, referent }
                        if referent.as_ref() == &Ty::Param(*parameter));
                if (ty == &Ty::Param(*parameter) || borrowed)
                    && output_projection.replace(projection).is_some()
                {
                    return Ok(None);
                }
                if borrowed {
                    borrowed_output = true;
                }
            }
        }
        let Some(output_projection) = output_projection else {
            return Ok(None);
        };
        if borrowed_output {
            if !self.closed_borrowed_item_context(
                *parameter,
                output_projection,
                projection,
                &context,
                facts,
                scratch,
            )? {
                return Ok(None);
            }
        } else if header.schema_env.total_arity() != 4 {
            return Ok(None);
        }
        // item_parameter_regions below follows this unique original equality
        // into item_projection_regions, which must authenticate the complete
        // callable context before returning any source. Do not prove those
        // same Sized and callable rows a second time here.
        if self
            .declared_callable_context(declaration)?
            .origins
            .is_empty()
        {
            return Ok(None);
        }
        let actual_self =
            self.item_region_substitution(&header.trait_ref.self_ty, &context, facts, scratch)?;
        let call = TraitCall {
            trait_ref: TraitRef {
                declaration: header.trait_ref.declaration,
                self_ty: actual_self.clone(),
                args: Arc::from([]),
            },
            item: required_item,
            method_args: Arc::from([]),
        };
        // The unique original required-item mapping above authenticates the
        // selected member. Replay its implementation obligation once below:
        // the exact selected owner and ordered actuals determine that same
        // member without rebuilding its complete signature a second time.
        let actual_projection = ir::AssociatedTypeProjection {
            item: projection.item,
            trait_ref: call.trait_ref.clone(),
        };
        let membership = self.require_in(
            &Predicate::Implements(call.trait_ref.clone()),
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        let ProofOrigin::Implementation {
            declaration: selected_owner,
            arguments: selected_arguments,
        } = &membership.0.origin
        else {
            return Ok(None);
        };
        if *selected_owner != owner || !self.eq_types(selected_arguments, arguments, 0)? {
            return Ok(None);
        }
        let replayed = self.replay_implementation_assignment_with(
            &actual_projection,
            &membership,
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        let expected =
            self.item_region_substitution(&Ty::Param(*parameter), &context, facts, scratch)?;
        if !self.eq_ty(&replayed, &expected, 0)? {
            return Ok(None);
        }
        let Some(component) = output.component.as_ref() else {
            return Ok(None);
        };
        retain::<ir::CarrierStep>(&self.budget, 1)?;
        let prefix = ir::CarrierPath(Arc::from([ir::CarrierStep::Referent]));
        let locations = if borrowed_output {
            let ir::CarrierChannel::Fixed(position) = component.channel else {
                return Ok(None);
            };
            let selected = if component.path.0.is_empty() {
                ir::CarrierLocation {
                    path: ir::CarrierPath::default(),
                    channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                        depth: position.depth.checked_add(1).ok_or_else(|| {
                            FactsError::resource(
                                &self.budget,
                                "borrowed item reference depth overflow",
                            )
                        })?,
                    }),
                }
            } else {
                self.item_region_prefix(&prefix, component)?
            };
            self.item_projection_regions(
                output_projection,
                &selected,
                &context,
                &prefix,
                facts,
                0,
                scratch,
            )?
        } else {
            self.item_parameter_regions(
                *parameter, component, &context, &prefix, facts, 0, scratch,
            )?
        };
        let Some(locations) = locations else {
            return Ok(None);
        };
        if locations.is_empty() {
            return Ok(None);
        }
        let mut sources = Vec::new();
        for location in locations {
            push(
                &self.budget,
                &mut sources,
                ir::InputCarrier { param: 0, location },
            )?;
        }
        retain::<ir::InputCarrier>(&self.budget, sources.len())?;
        Ok(Some(sources.into()))
    }

    #[cfg(test)]
    pub(super) fn rejects_forged_projected_envelope_for_tests(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<bool> {
        let scratch = &mut TraitScratch::default();
        view.validate_for_with_scratch(facts, scratch)?;
        let raw = self.normalized_declaration_schema(view.declaration)?;
        let [row] = view.original_contract.results.as_ref() else {
            return Ok(false);
        };
        let expansions = self.expand_carrier_with_scratch(
            &row.location,
            &raw.env,
            &view.arguments,
            &view.nominals,
            facts,
            scratch,
        )?;
        let [output] = expansions.as_slice() else {
            return Ok(false);
        };
        if self
            .direct_projected_envelope_sources(
                view.declaration,
                row,
                output,
                &raw,
                &view.arguments,
                facts,
                scratch,
            )?
            .as_deref()
            != Some(view.contract.results[0].sources.as_ref())
        {
            return Ok(false);
        }
        for mutation in 0..9 {
            let mut declaration = view.declaration;
            let mut forged = raw.clone();
            let mut row = row.clone();
            let mut args = view.arguments.clone();
            match mutation {
                0 => {
                    let Ty::Ref { kind, .. } = &mut forged.signature.params[0] else {
                        return Ok(false);
                    };
                    *kind = ir::BorrowKind::Shared;
                }
                1 => forged.signature.result = Ty::Bool,
                2 => {
                    let mut predicates = forged.predicates.to_vec();
                    let position = predicates
                        .iter()
                        .position(|row| matches!(row, Predicate::ProjectionEq { .. }))
                        .unwrap();
                    predicates.remove(position);
                    forged.predicates = predicates.into();
                }
                3 => {
                    let ir::CarrierStep::VariantField { variant, field } = &row.location.path.0[0]
                    else {
                        return Ok(false);
                    };
                    row.location.path =
                        ir::CarrierPath(Arc::from([ir::CarrierStep::VariantField {
                            variant: ir::VariantId {
                                owner: variant.owner,
                                index: 0,
                            },
                            field: *field,
                        }]));
                }
                4 => {
                    row.location.channel =
                        ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
                }
                5 => args = Arc::from([]),
                6 => forged.signature.params[0] = Ty::Bool,
                7 => {
                    let mut reordered = args.to_vec();
                    reordered.swap(0, 1);
                    args = reordered.into();
                }
                8 => {
                    declaration = self.info(view.declaration)?.owner.ok_or_else(|| {
                        FactsError::invalid(&self.budget, "projected envelope lost original owner")
                    })?;
                }
                _ => unreachable!(),
            }
            match self.direct_projected_envelope_sources(
                declaration,
                &row,
                output,
                &forged,
                &args,
                facts,
                scratch,
            ) {
                Ok(None) => {}
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// A selected implementation can be a direct call whose original result is
    /// nominal, rather than the required item's Self parameter. Transport only
    /// the exact original impl/method correspondence back through that required
    /// item's checked protocol. Concrete type coincidence grants no authority.
    #[allow(clippy::too_many_arguments)]
    fn direct_item_result_sources(
        &self,
        declaration: DeclId,
        result: &ir::CarrierReturn,
        output: &ExpandedCarrier,
        raw: &GenericSchema,
        arguments: &[Ty],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Arc<[ir::InputCarrier]>>> {
        if facts.phase() != FactsPhase::Closed
            || raw.signature.params.len() != 1
            || !result.sources.is_empty()
        {
            return Ok(None);
        }
        let Ty::Struct(nominal) = &raw.signature.result else {
            return Ok(None);
        };
        let [Ty::Param(element)] = nominal.def().type_args.as_ref() else {
            return Ok(None);
        };
        let Ty::Param(input) = raw.signature.params[0] else {
            return Ok(None);
        };
        let info = self.info(declaration)?;
        let Some(owner) = info.owner else {
            return Ok(None);
        };
        let Some(position) = self.impl_header_index.get(&self.budget, owner)? else {
            return Ok(None);
        };
        let header = &self.headers.impls[position];
        if header.declaration != owner
            || header.schema_env.total_arity() != 1
            || header.schema_env.ordered_params().next() != Some(*element)
            || element.owner.declaration != owner
            || input.owner.declaration != declaration
            || input.index != 0
            || input == *element
            || info.own_env.arity() != 1
            || raw.env.total_arity() != 2
            || arguments.len() != 2
            || !self.method_environment_matches(&raw.env, &header.schema_env, &info.own_env)?
            || header.trait_ref.args.as_ref() != [Ty::Param(*element)]
            || !self.eq_ty(&header.trait_ref.self_ty, &raw.signature.result, 0)?
            || result.location.channel
                != (ir::CarrierChannel::ParamTail {
                    parameter: *element,
                    prefix: 0,
                })
        {
            return Ok(None);
        }
        let original = self.schema.declaration_schema(declaration)?;
        if !self.same_env(&raw.env, &original.env)?
            || !self.eq_signature(&raw.signature, &original.signature)?
        {
            return Ok(None);
        }
        self.compare_predicate_rows(&raw.predicates, &original.predicates)?;
        let mut original_location = false;
        for location in charged(
            &self.budget,
            ir::carrier_locations(&raw.signature.result, &self.budget),
        )?
        .iter()
        {
            original_location |= self.same_location(location, &result.location)?;
        }
        if !original_location {
            return Ok(None);
        }
        let retained = self.declared_callable_context(declaration)?;
        if retained.origins.is_empty() {
            return Ok(None);
        }
        let mut item = None;
        for (required, target) in header.items.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if *target == declaration && item.replace(*required).is_some() {
                return Ok(None);
            }
        }
        let Some(item) = item else {
            return Ok(None);
        };
        let required = self.item_header(item)?;
        if required.trait_decl != header.trait_ref.declaration
            || required.default.is_some()
            || self.info(item)?.own_env.arity() != 1
        {
            return Ok(None);
        }
        let context = ItemRegionContext {
            env: &raw.env,
            arguments,
            predicates: &raw.predicates,
            self_ty: &raw.signature.params[0],
        };
        if !self
            .item_region_predicates(&header.predicates, &context, facts, scratch)?
            .is_empty()
        {
            return Ok(None);
        }
        let rows = self.item_region_predicates(&raw.predicates, &context, facts, scratch)?;
        let mut projection = None;
        for row in rows.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Predicate::ProjectionEq {
                projection: candidate,
                ty: Ty::Param(parameter),
            } = row
                && parameter == element
                && candidate.trait_ref.self_ty == Ty::Param(input)
                && candidate.trait_ref.args.is_empty()
                && projection.replace(candidate).is_some()
            {
                return Ok(None);
            }
        }
        let Some(projection) = projection else {
            return Ok(None);
        };
        let expected = [
            Predicate::Implements(header.trait_ref.clone()),
            Predicate::Implements(projection.trait_ref.clone()),
            Predicate::ProjectionEq {
                projection: projection.clone(),
                ty: Ty::Param(*element),
            },
        ];
        if rows.len() != expected.len() {
            return Ok(None);
        }
        for expected in expected.iter() {
            let mut count = 0;
            for row in rows.iter() {
                count += usize::from(self.eq_predicate(row, expected)?);
            }
            if count != 1 {
                return Ok(None);
            }
        }
        // The exact mapped required declaration is the authority for the input
        // equality as well as its output family. Re-selection must return this
        // same direct implementation and every ordered original actual.
        retain::<Ty>(&self.budget, 2)?;
        let call = ir::TraitCall {
            trait_ref: TraitRef {
                declaration: header.trait_ref.declaration,
                self_ty: self.item_region_substitution(
                    &header.trait_ref.self_ty,
                    &context,
                    facts,
                    scratch,
                )?,
                args: Arc::from([self.item_region_substitution(
                    &Ty::Param(*element),
                    &context,
                    facts,
                    scratch,
                )?]),
            },
            item,
            method_args: Arc::from([self.item_region_substitution(
                &Ty::Param(input),
                &context,
                facts,
                scratch,
            )?]),
        };
        let selected = self.resolve_trait_call_with(&call, facts, scratch)?;
        let Callee::Direct(target) = selected.callee() else {
            return Ok(None);
        };
        if target.declaration != declaration || !self.eq_types(&target.type_args, arguments, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "direct item region transport changed selected implementation",
            ));
        }
        // Reuse the required item's original source protocol, not a published
        // carrier view whose full equality replay repeatedly reconstructs source
        // transport. The Param result guard makes this dependency
        // strictly direct-nominal -> required-parameter -> projected input.
        let required_raw = self.normalized_declaration_schema(item)?;
        if !matches!(required_raw.signature.result, Ty::Param(_)) {
            return Ok(None);
        }
        let required_retained = self.declaration_carriers(item)?;
        let mut required_arguments = self.trait_arguments(&call.trait_ref)?;
        for argument in call.method_args.iter() {
            push(&self.budget, &mut required_arguments, argument.clone())?;
        }
        let mut sources = None;
        for row in required_retained.contract.results.iter() {
            for required_output in self.expand_carrier_with_scratch(
                &row.location,
                &required_raw.env,
                &required_arguments,
                &[],
                facts,
                scratch,
            )? {
                if !self.same_location(&required_output.location, &output.location)? {
                    continue;
                }
                let Some(expected) = self.projected_item_result_sources(
                    item,
                    row,
                    &required_output,
                    &required_raw,
                    &required_arguments,
                    facts,
                    scratch,
                )?
                else {
                    return Ok(None);
                };
                if sources.replace(expected).is_some() {
                    return Ok(None);
                }
            }
        }
        Ok(sources)
    }

    #[cfg(test)]
    pub(super) fn rejects_forged_direct_item_regions_for_tests(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<bool> {
        let scratch = &mut TraitScratch::default();
        view.validate_for_with_scratch(facts, scratch)?;
        let raw = self.normalized_declaration_schema(view.declaration)?;
        let [result] = view.original_contract.results.as_ref() else {
            return Ok(false);
        };
        let expansions = self.expand_carrier_with_scratch(
            &result.location,
            &raw.env,
            &view.arguments,
            &view.nominals,
            facts,
            scratch,
        )?;
        let [output] = expansions.as_slice() else {
            return Ok(false);
        };
        let genuine = self.direct_item_result_sources(
            view.declaration,
            result,
            output,
            &raw,
            &view.arguments,
            facts,
            scratch,
        )?;
        let Some(genuine) = genuine else {
            return Ok(false);
        };
        if genuine.as_ref() != view.contract.results[0].sources.as_ref() {
            return Ok(false);
        }
        for mutation in 0..6 {
            let mut forged = raw.clone();
            let mut result = result.clone();
            let mut arguments = view.arguments.as_ref();
            match mutation {
                0 => {
                    forged.predicates = forged
                        .predicates
                        .iter()
                        .filter(|row| !matches!(row, Predicate::ProjectionEq { .. }))
                        .cloned()
                        .collect();
                }
                1 => {
                    let mut rows = forged.predicates.to_vec();
                    let row = rows
                        .iter_mut()
                        .find(|row| matches!(row, Predicate::ProjectionEq { .. }))
                        .unwrap();
                    let Predicate::ProjectionEq { ty, .. } = row else {
                        unreachable!();
                    };
                    *ty = raw.signature.params[0].clone();
                    forged.predicates = rows.into();
                }
                2 => arguments = &[],
                3 => forged.signature.result = raw.signature.params[0].clone(),
                4 => {
                    let Ty::Struct(nominal) = &raw.signature.result else {
                        return Ok(false);
                    };
                    forged.signature.params[0] = nominal.def().type_args[0].clone();
                }
                _ => result.location.path = ir::CarrierPath(Arc::from([ir::CarrierStep::Field(7)])),
            }
            match self.direct_item_result_sources(
                view.declaration,
                &result,
                output,
                &forged,
                arguments,
                facts,
                scratch,
            ) {
                Ok(None) => {}
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    // A required trait item has no executable declaration body or retained
    // callable origins. Its selected implementation is a different authority.
    // Keep its transport conservative, without catching metadata/resource errors
    // for executable declarations which must possess their original context.
    fn item_region_declared_context(
        &self,
        declaration: DeclId,
    ) -> Result<Option<&equality::DeclaredCallableContext>> {
        if self.info(declaration)?.kind == DeclKind::TraitItem {
            return Ok(None);
        }
        self.declared_callable_context(declaration).map(Some)
    }

    /// The borrowed-item inverse is an original declaration protocol, not a
    /// monomorphic type match. Its two parameters are linked only by one exact
    /// associated equality `I::Item = &T`, with canonical Clone(T) (`Cloned`)
    /// or canonical Copy(T) (`Copied`). The wrapper physically owns I and no
    /// T, so the outer item-reference region cannot become a source for a
    /// stored carrier inside the cloned or copied T.
    fn closed_borrowed_item_context(
        &self,
        parameter: ParamId,
        input: &ir::AssociatedTypeProjection,
        required: &ir::AssociatedTypeProjection,
        context: &ItemRegionContext<'_>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<bool> {
        if context.env.total_arity() != 2
            || !input.trait_ref.args.is_empty()
            || input.item != required.item
            || input.trait_ref.declaration != required.trait_ref.declaration
        {
            return Ok(false);
        }
        let Ty::Param(iterator) = input.trait_ref.self_ty else {
            return Ok(false);
        };
        if iterator == parameter
            || iterator.owner != parameter.owner
            || !context.env.contains(iterator)
            || !context.env.contains(parameter)
        {
            return Ok(false);
        }
        let Ty::Struct(wrapper) = context.self_ty else {
            return Ok(false);
        };
        if wrapper.def().type_args.as_ref() != [Ty::Param(iterator)] {
            return Ok(false);
        }
        let fields = nominal_fields(&wrapper.def().fields, &self.budget)?;
        if fields.len() != 1 || fields[0].ty != Ty::Param(iterator) {
            return Ok(false);
        }
        let rows = self.item_region_predicates(context.predicates, context, facts, scratch)?;
        if rows.len() != 3 {
            return Ok(false);
        }
        retain::<Ty>(&self.budget, 1)?;
        // Exactly one duplication bound: Clone(T) or Copy(T), never both and
        // never a lookalike trait. Both produce an owned T from `&T`.
        let mut duplication = None;
        for candidate in [CatalogItem::Clone, CatalogItem::Copy] {
            let declaration = self.catalog.id(candidate);
            let row = Predicate::Implements(TraitRef {
                declaration,
                self_ty: Ty::Param(parameter),
                args: Arc::from([]),
            });
            let mut count = 0;
            for actual in &rows {
                charged(&self.budget, self.budget.charge_work(1))?;
                count += usize::from(self.eq_predicate(actual, &row)?);
            }
            if count == 1 && duplication.replace(declaration).is_some() {
                return Ok(false);
            }
        }
        let Some(duplication) = duplication else {
            return Ok(false);
        };
        let expected = [
            Predicate::Implements(TraitRef {
                declaration: duplication,
                self_ty: Ty::Param(parameter),
                args: Arc::from([]),
            }),
            Predicate::Implements(input.trait_ref.clone()),
            Predicate::ProjectionEq {
                projection: input.clone(),
                ty: Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(Ty::Param(parameter)),
                },
            },
        ];
        for expected in &expected {
            let mut count = 0;
            for row in &rows {
                charged(&self.budget, self.budget.charge_work(1))?;
                count += usize::from(self.eq_predicate(row, expected)?);
            }
            if count != 1 {
                return Ok(false);
            }
        }
        let actual =
            self.item_region_substitution(&Ty::Param(parameter), context, facts, scratch)?;
        self.require_in(
            &Predicate::Implements(TraitRef {
                declaration: duplication,
                self_ty: actual,
                args: Arc::from([]),
            }),
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        Ok(true)
    }

    // Only the canonical built-in Sized obligation is lifetime-neutral. Replay
    // its actual closed proof before excluding it from the structural linkage.
    // Every other original predicate remains present, including lookalike traits.
    fn item_region_predicates<'a>(
        &self,
        predicates: &'a [Predicate],
        context: &ItemRegionContext<'_>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Vec<&'a Predicate>> {
        let mut rows = Vec::new();
        for predicate in predicates {
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Predicate::Implements(goal) = predicate
                && goal.declaration == self.catalog.id(CatalogItem::Sized)
                && goal.args.is_empty()
                && matches!(goal.self_ty, Ty::Param(_))
            {
                let self_ty =
                    self.item_region_substitution(&goal.self_ty, context, facts, scratch)?;
                self.require_in(
                    &Predicate::Implements(TraitRef {
                        declaration: self.catalog.id(CatalogItem::Sized),
                        self_ty,
                        args: Arc::from([]),
                    }),
                    ProofContext::from_facts(facts),
                    scratch.for_world(self.id),
                )?;
            } else {
                push(&self.budget, &mut rows, predicate)?;
            }
        }
        Ok(rows)
    }

    fn item_region_mentions(&self, ty: &Ty, parameter: ParamId, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let all = |types: &[Ty]| -> Result<bool> {
            for ty in types {
                if self.item_region_mentions(ty, parameter, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        Ok(match ty {
            Ty::Param(p) => *p == parameter,
            Ty::AssociatedType(projection) => {
                self.item_region_mentions(&projection.trait_ref.self_ty, parameter, depth + 1)?
                    || all(&projection.trait_ref.args)?
            }
            Ty::Cell(cell) => self.item_region_mentions(cell.argument(), parameter, depth + 1)?,
            Ty::ManuallyDrop(wrapper) => {
                self.item_region_mentions(wrapper.argument(), parameter, depth + 1)?
            }
            Ty::Tuple(parts) => all(parts)?,
            Ty::Array { element, .. } | Ty::Slice { element } => {
                self.item_region_mentions(element, parameter, depth + 1)?
            }
            Ty::Ref { referent, .. }
            | Ty::RawPtr {
                pointee: referent, ..
            } => self.item_region_mentions(referent, parameter, depth + 1)?,
            Ty::Struct(n) => all(&n.def().type_args)?,
            Ty::Enum(n) => all(&n.def().type_args)?,
            Ty::Opaque(_)
            | Ty::Closure(_)
            | Ty::Dyn(_)
            | Ty::PhantomData(_)
            | Ty::FnPointer(_)
            | Ty::FnItem(_) => true,
            _ => false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn closed_item_required_root(
        &self,
        declaration: DeclId,
        raw: &GenericSchema,
        output: &TraitRef,
        input: &ir::AssociatedTypeProjection,
        context: &ItemRegionContext<'_>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<bool> {
        let item = self.item_header(declaration)?;
        let info = self.info(declaration)?;
        if item.default.is_some()
            || item.trait_decl != output.declaration
            || info.owner != Some(item.trait_decl)
            || !input.trait_ref.args.is_empty()
        {
            return Ok(false);
        }
        let (Ty::Param(result), Ty::Param(source), [Ty::Param(element)]) = (
            &raw.signature.result,
            &raw.signature.params[0],
            output.args.as_ref(),
        ) else {
            return Ok(false);
        };
        if result.owner.declaration != item.trait_decl
            || result.index != 0
            || element.owner.declaration != item.trait_decl
            || element.index != 1
            || source.owner.declaration != declaration
            || source.index != 0
            || raw.env.total_arity() != 3
            || output.self_ty != raw.signature.result
            || input.trait_ref.self_ty != raw.signature.params[0]
        {
            return Ok(false);
        }
        // Required items are intentionally absent from executable callable
        // contexts. Replay their own source schema instead of inventing body
        // origins or accepting an instantiated predicate as authority.
        let original = self.schema.declaration_schema(declaration)?;
        if !self.same_env(&raw.env, &original.env)?
            || !self.eq_signature(&raw.signature, &original.signature)?
        {
            return Ok(false);
        }
        self.compare_predicate_rows(&raw.predicates, &original.predicates)?;
        let rows = self.item_region_predicates(&raw.predicates, context, facts, scratch)?;
        if rows.len() != 3 {
            return Ok(false);
        }
        let expected = [
            Predicate::Implements(output.clone()),
            Predicate::Implements(input.trait_ref.clone()),
            Predicate::ProjectionEq {
                projection: input.clone(),
                ty: Ty::Param(*element),
            },
        ];
        for expected in expected.iter() {
            let mut count = 0;
            for row in rows.iter() {
                count += usize::from(self.eq_predicate(row, expected)?);
            }
            if count != 1 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn closed_item_projection_alias(
        &self,
        alias: &ir::AssociatedTypeProjection,
        context: &ItemRegionContext<'_>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<bool> {
        let Ty::Struct(wrapper) = context.self_ty else {
            return Ok(false);
        };
        let [Ty::Param(parameter)] = wrapper.def().type_args.as_ref() else {
            return Ok(false);
        };
        if alias.trait_ref.self_ty != Ty::Param(*parameter) || !alias.trait_ref.args.is_empty() {
            return Ok(false);
        }
        let fields = nominal_fields(&wrapper.def().fields, &self.budget)?;
        if fields.len() != 1 || fields[0].ty != Ty::Param(*parameter) {
            return Ok(false);
        }
        let rows = self.item_region_predicates(context.predicates, context, facts, scratch)?;
        Ok(rows.len() == 1
            && self.eq_predicate(rows[0], &Predicate::Implements(alias.trait_ref.clone()))?)
    }

    fn closed_item_root_context(
        &self,
        raw: &GenericSchema,
        output_bound: &TraitRef,
        input: &ir::AssociatedTypeProjection,
        context: &ItemRegionContext<'_>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<bool> {
        let rows = self.item_region_predicates(&raw.predicates, context, facts, scratch)?;
        if !(3..=4).contains(&rows.len()) || !input.trait_ref.args.is_empty() {
            return Ok(false);
        }
        let Ty::Param(input_parameter) = input.trait_ref.self_ty else {
            return Ok(false);
        };
        let mut bridge = None;
        for row in rows.iter().copied() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Predicate::ProjectionEq {
                projection,
                ty: Ty::AssociatedType(rhs),
            } = row
                && (!self.same_projection(rhs, input)? || bridge.replace(projection).is_some())
            {
                return Ok(false);
            }
        }
        let Some(bridge) = bridge else {
            return Ok(false);
        };
        let Ty::Struct(wrapper) = &bridge.trait_ref.self_ty else {
            return Ok(false);
        };
        if !bridge.trait_ref.args.is_empty()
            || wrapper.def().type_args.as_ref() != [Ty::Param(input_parameter)]
        {
            return Ok(false);
        }
        let fields = nominal_fields(&wrapper.def().fields, &self.budget)?;
        if fields.len() != 1 || fields[0].ty != Ty::Param(input_parameter) {
            return Ok(false);
        }
        let mut input_bound = false;
        let mut result_bound = false;
        let mut bridge_bound = false;
        for row in rows.iter().copied() {
            if let Predicate::Implements(_) = row {
                if self.eq_predicate(row, &Predicate::Implements(output_bound.clone()))? {
                    if result_bound {
                        return Ok(false);
                    }
                    result_bound = true;
                } else if self.eq_predicate(row, &Predicate::Implements(input.trait_ref.clone()))? {
                    if input_bound {
                        return Ok(false);
                    }
                    input_bound = true;
                } else if self
                    .eq_predicate(row, &Predicate::Implements(bridge.trait_ref.clone()))?
                {
                    if bridge_bound {
                        return Ok(false);
                    }
                    bridge_bound = true;
                } else {
                    return Ok(false);
                }
            } else if !matches!(row, Predicate::ProjectionEq { projection, .. } if self.same_projection(projection, bridge)?)
            {
                return Ok(false);
            }
        }
        if !result_bound || !bridge_bound || (rows.len() == 4 && !input_bound) {
            return Ok(false);
        }
        retain::<ir::AssociatedTypeProjection>(&self.budget, 2)?;
        let actual = self.item_region_substitution(
            &Ty::AssociatedType(Arc::new(bridge.clone())),
            context,
            facts,
            scratch,
        )?;
        let expected = self.item_region_substitution(
            &Ty::AssociatedType(Arc::new(input.clone())),
            context,
            facts,
            scratch,
        )?;
        if !self.eq_ty(&actual, &expected, 0)? {
            return Ok(false);
        }
        let self_ty =
            self.item_region_substitution(&bridge.trait_ref.self_ty, context, facts, scratch)?;
        let membership = self.require_in(
            &Predicate::Implements(TraitRef {
                declaration: bridge.trait_ref.declaration,
                self_ty,
                args: Arc::from([]),
            }),
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &membership.0.origin
        else {
            return Ok(false);
        };
        let Some(position) = self.impl_header_index.get(&self.budget, *declaration)? else {
            return Ok(false);
        };
        let header = &self.headers.impls[position];
        let Ty::Struct(wrapper) = &header.trait_ref.self_ty else {
            return Ok(false);
        };
        let [Ty::Param(parameter)] = wrapper.def().type_args.as_ref() else {
            return Ok(false);
        };
        let fields = nominal_fields(&wrapper.def().fields, &self.budget)?;
        let bridge_context = ItemRegionContext {
            env: &header.schema_env,
            arguments,
            predicates: &header.predicates,
            self_ty: &header.trait_ref.self_ty,
        };
        let bridge_rows =
            self.item_region_predicates(&header.predicates, &bridge_context, facts, scratch)?;
        if fields.len() != 1 || fields[0].ty != Ty::Param(*parameter) || bridge_rows.len() != 1 {
            return Ok(false);
        }
        let expected = TraitRef {
            declaration: input.trait_ref.declaration,
            self_ty: Ty::Param(*parameter),
            args: Arc::from([]),
        };
        if !self.eq_predicate(bridge_rows[0], &Predicate::Implements(expected.clone()))? {
            return Ok(false);
        }
        let value = self.associated_value(*declaration, bridge.item)?;
        self.checked_assignment_source(value)?;
        let Ty::AssociatedType(rhs) = &value.rhs else {
            return Ok(false);
        };
        self.same_projection(
            rhs,
            &ir::AssociatedTypeProjection {
                trait_ref: expected,
                item: input.item,
            },
        )
    }

    fn closed_item_callable_context(
        &self,
        output: &ir::AssociatedTypeProjection,
        context: &ItemRegionContext<'_>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<bool> {
        let rows = self.item_region_predicates(context.predicates, context, facts, scratch)?;
        if rows.len() != 4 || output.trait_ref.args.len() != 1 {
            return Ok(false);
        }
        let Ty::Param(callable) = output.trait_ref.self_ty else {
            return Ok(false);
        };
        let Ty::Tuple(arguments) = &output.trait_ref.args[0] else {
            return Ok(false);
        };
        let [Ty::Param(argument)] = arguments.as_ref() else {
            return Ok(false);
        };
        let mut input = None;
        let mut result = None;
        for predicate in rows.iter().copied() {
            if let Predicate::ProjectionEq {
                projection,
                ty: Ty::Param(parameter),
            } = predicate
            {
                if self.same_projection(projection, output)? {
                    if result.replace(*parameter).is_some() {
                        return Ok(false);
                    }
                } else if parameter == argument {
                    if input.replace(projection).is_some() {
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
                }
            }
        }
        let (Some(input), Some(result)) = (input, result) else {
            return Ok(false);
        };
        let Ty::Param(iterator) = input.trait_ref.self_ty else {
            return Ok(false);
        };
        if !input.trait_ref.args.is_empty()
            || iterator == callable
            || result == *argument
            || [iterator, callable].contains(&result)
            || [iterator, callable].contains(argument)
        {
            return Ok(false);
        }
        let mut input_bound = false;
        let mut output_bound = false;
        for predicate in rows.iter().copied() {
            if matches!(predicate, Predicate::Implements(_)) {
                if self.eq_predicate(predicate, &Predicate::Implements(input.trait_ref.clone()))? {
                    if input_bound {
                        return Ok(false);
                    }
                    input_bound = true;
                } else if self
                    .eq_predicate(predicate, &Predicate::Implements(output.trait_ref.clone()))?
                {
                    if output_bound {
                        return Ok(false);
                    }
                    output_bound = true;
                } else {
                    return Ok(false);
                }
            }
        }
        if !input_bound || !output_bound {
            return Ok(false);
        }
        let Ty::Struct(owner) = context.self_ty else {
            return Ok(false);
        };
        let fields = nominal_fields(&owner.def().fields, &self.budget)?;
        if fields.len() != 2
            || !fields.iter().any(|field| field.ty == Ty::Param(iterator))
            || !fields.iter().any(|field| field.ty == Ty::Param(callable))
        {
            return Ok(false);
        }
        let parameters = &owner.def().type_args;
        if parameters.len() != 2
            || !parameters.iter().any(|ty| *ty == Ty::Param(iterator))
            || !parameters.iter().any(|ty| *ty == Ty::Param(callable))
        {
            return Ok(false);
        }
        for location in charged(
            &self.budget,
            ir::carrier_locations(context.self_ty, &self.budget),
        )?
        .iter()
        {
            match location.channel {
                ir::CarrierChannel::ParamTail { parameter, .. }
                    if parameter == iterator || parameter == callable => {}
                ir::CarrierChannel::Fixed(_) => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn item_region_substitution(
        &self,
        ty: &Ty,
        context: &ItemRegionContext<'_>,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Ty> {
        retain::<ir::RefContract>(&self.budget, 1)?;
        let schema = GenericSchema {
            env: context.env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: ty.clone(),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let active = scratch.for_world(self.id);
        let instantiated =
            self.substitute_carrier_schema(&schema, context.arguments, facts, active)?;
        Ok(self
            .normalize_closed_projected_type_with(
                &instantiated.signature.result,
                ProofContext::from_facts(facts),
                false,
                active,
            )?
            .0)
    }

    fn item_region_locations(
        &self,
        ty: &Ty,
        context: &ItemRegionContext<'_>,
        wanted: &ir::CarrierLocation,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Vec<(ParamId, ir::CarrierLocation)>> {
        let mut matches = Vec::new();
        for location in charged(&self.budget, ir::carrier_locations(ty, &self.budget))?.iter() {
            let ir::CarrierChannel::ParamTail { parameter, .. } = location.channel else {
                continue;
            };
            charged(&self.budget, self.budget.charge_work(location.path.0.len()))?;
            if location
                .path
                .0
                .iter()
                .any(|step| matches!(step, ir::CarrierStep::VariantField { .. }))
            {
                continue;
            }
            for expanded in self.expand_carrier_with_scratch(
                location,
                context.env,
                context.arguments,
                &[],
                facts,
                scratch,
            )? {
                if self.same_location(&expanded.location, wanted)?
                    && let Some(component) = expanded.component
                {
                    push(&self.budget, &mut matches, (parameter, component))?;
                }
            }
        }
        Ok(matches)
    }

    fn item_region_prefix(
        &self,
        prefix: &ir::CarrierPath,
        location: &ir::CarrierLocation,
    ) -> Result<ir::CarrierLocation> {
        let mut path = Vec::new();
        for step in prefix.0.iter().chain(location.path.0.iter()) {
            push(&self.budget, &mut path, step.clone())?;
        }
        retain::<ir::CarrierStep>(&self.budget, path.len())?;
        Ok(ir::CarrierLocation {
            path: ir::CarrierPath(path.into()),
            channel: location.channel.clone(),
        })
    }

    // Resolve a parameter's lifetime component in the ORIGINAL impl self.
    // Missing physical occurrences may be reached only through an exact
    // original associated equality, never through equal monomorphic types.
    #[allow(clippy::too_many_arguments)]
    fn item_parameter_regions(
        &self,
        parameter: ParamId,
        wanted: &ir::CarrierLocation,
        context: &ItemRegionContext<'_>,
        prefix: &ir::CarrierPath,
        facts: &ValidatedFacts,
        depth: usize,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Vec<ir::CarrierLocation>>> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let mut physical = Vec::new();
        for location in charged(
            &self.budget,
            ir::carrier_locations(context.self_ty, &self.budget),
        )?
        .iter()
        {
            if matches!(
                location.channel,
                ir::CarrierChannel::ProjectionTail { .. } | ir::CarrierChannel::OpaqueTail { .. }
            ) {
                return Ok(None);
            }
            let ir::CarrierChannel::ParamTail {
                parameter: p,
                prefix: layers,
            } = location.channel
            else {
                continue;
            };
            if p != parameter {
                continue;
            }
            charged(&self.budget, self.budget.charge_work(location.path.0.len()))?;
            if layers != 0
                || location
                    .path
                    .0
                    .iter()
                    .any(|step| matches!(step, ir::CarrierStep::VariantField { .. }))
            {
                // Never publish a partial candidate set for one original
                // parameter merely because one occurrence is outside this slice.
                return Ok(None);
            }
            for expanded in self.expand_carrier_with_scratch(
                location,
                context.env,
                context.arguments,
                &[],
                facts,
                scratch,
            )? {
                if let Some(component) = expanded.component.as_ref()
                    && self.same_location(component, wanted)?
                {
                    let full = self.item_region_prefix(prefix, &expanded.location)?;
                    push(&self.budget, &mut physical, full)?;
                }
            }
        }
        let rows = self.item_region_predicates(context.predicates, context, facts, scratch)?;
        if !physical.is_empty() {
            if !rows.is_empty() {
                return Ok(None);
            }
            return Ok(Some(physical));
        }
        let mut equality = None;
        for predicate in rows.iter().copied() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Predicate::ProjectionEq {
                projection,
                ty: Ty::Param(p),
            } = predicate
                && *p == parameter
            {
                if equality.is_some() {
                    return Ok(None);
                }
                equality = Some(projection);
            }
        }
        let Some(projection) = equality else {
            return Ok(None);
        };
        self.item_projection_regions(
            projection,
            wanted,
            context,
            prefix,
            facts,
            depth + 1,
            scratch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn item_projection_regions(
        &self,
        projection: &ir::AssociatedTypeProjection,
        wanted: &ir::CarrierLocation,
        context: &ItemRegionContext<'_>,
        prefix: &ir::CarrierPath,
        facts: &ValidatedFacts,
        depth: usize,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Vec<ir::CarrierLocation>>> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let actual_self =
            self.item_region_substitution(&projection.trait_ref.self_ty, context, facts, scratch)?;
        let mut actual_arguments = Vec::new();
        for argument in projection.trait_ref.args.iter() {
            let actual = self.item_region_substitution(argument, context, facts, scratch)?;
            push(&self.budget, &mut actual_arguments, actual)?;
        }
        retain::<Ty>(&self.budget, actual_arguments.len())?;
        let actual = ir::AssociatedTypeProjection {
            item: projection.item,
            trait_ref: TraitRef {
                declaration: projection.trait_ref.declaration,
                self_ty: actual_self,
                args: actual_arguments.into(),
            },
        };
        let reduction = self.normalize_projection_for_facts_with_scratch(
            &actual,
            facts,
            scratch.for_world(self.id),
        )?;
        // A callable result can leave its receiver storage entirely. Its exact
        // parent-sealed receipt instead selects a tuple argument component.
        if let Ty::Closure(closure) = &actual.trait_ref.self_ty {
            if !self.closed_item_callable_context(projection, context, facts, scratch)?
                || !self.checked_closure_projection(&actual, facts.env())?
                || !wanted.path.0.is_empty()
                || wanted.channel != ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
            {
                return Ok(None);
            }
            let Some(receipt) = closure.result_receipt() else {
                return Ok(None);
            };
            let Some(origin) = receipt.argument_origin() else {
                return Ok(None);
            };
            let Some(mode) = self.closure_trait_kind(actual.trait_ref.declaration)? else {
                return Ok(None);
            };
            let target = Callee::Direct(self.closure_target(closure, mode)?);
            let view = self.carrier_contract(SignatureSubject::Call(&target), facts)?;
            view.validate_for_with_scratch(facts, scratch)?;
            if view.contract.results.len() != 1
                || view.contract.results[0].location != *wanted
                || !self.eq_ty(reduction.ty(), closure.result(), 0)?
            {
                return Ok(None);
            }
            retain::<ir::CarrierStep>(&self.budget, 1)?;
            let expected = ir::InputCarrier {
                param: 1,
                location: ir::CarrierLocation {
                    path: ir::CarrierPath(Arc::from([ir::CarrierStep::Field(origin.argument)])),
                    channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                        depth: origin.depth,
                    }),
                },
            };
            if view.contract.results[0].sources.as_ref() != [expected]
                || projection.trait_ref.args.len() != 1
            {
                return Ok(None);
            }
            let Ty::Tuple(arguments) = &projection.trait_ref.args[0] else {
                return Ok(None);
            };
            let Some(Ty::Param(parameter)) = arguments.get(origin.argument) else {
                return Ok(None);
            };
            let component = ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([])),
                channel: ir::CarrierChannel::Fixed(ir::ReferencePosition {
                    depth: origin.depth,
                }),
            };
            return self.item_parameter_regions(
                *parameter,
                &component,
                context,
                prefix,
                facts,
                depth + 1,
                scratch,
            );
        }
        let membership = self.require_in(
            &Predicate::Implements(actual.trait_ref.clone()),
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )?;
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &membership.0.origin
        else {
            return Ok(None);
        };
        let position = self
            .impl_header_index
            .get(&self.budget, *declaration)?
            .ok_or_else(|| {
                FactsError::invalid(&self.budget, "item region selection lost original impl")
            })?;
        let header = &self.headers.impls[position];
        let value = self.associated_value(*declaration, actual.item)?;
        self.checked_assignment_source(value)?;
        let next = ItemRegionContext {
            env: &header.schema_env,
            arguments,
            predicates: &header.predicates,
            self_ty: &header.trait_ref.self_ty,
        };
        let alias = match &value.rhs {
            Ty::AssociatedType(alias)
                if self.closed_item_projection_alias(alias, &next, facts, scratch)? =>
            {
                Some(alias.as_ref())
            }
            _ => None,
        };
        // Establish applicability from the authenticated original assignment
        // first. A projected family without the exact alias protocol has no
        // parameter inverse here, so it cannot authorize narrowed sources.
        let inverse = if alias.is_some() {
            Vec::new()
        } else {
            self.item_region_locations(&value.rhs, &next, wanted, facts, scratch)?
        };
        if inverse.is_empty() && alias.is_none() {
            return Ok(None);
        }
        // Authenticate the original assignment before closed substitution can
        // hide the single-field parameter-to-projection correspondence. Only
        // this exact alias protocol may normalize a replayed residual projection.
        let canonical = if alias.is_some() {
            let replay = self.replay_implementation_assignment_with(
                &actual,
                &membership,
                ProofContext::from_facts(facts),
                scratch.for_world(self.id),
            )?;
            self.normalize_closed_projected_type_with(
                &replay,
                ProofContext::from_facts(facts),
                false,
                scratch.for_world(self.id),
            )?
            .0
        } else {
            self.implementation_equality_rhs_with(
                &actual,
                &membership,
                ProofContext::from_facts(facts),
                scratch.for_world(self.id),
            )?
        };
        if !self.eq_ty(&canonical, reduction.ty(), 0)? {
            return Ok(None);
        }
        let Ty::Param(source_parameter) = projection.trait_ref.self_ty else {
            return Ok(None);
        };
        let mut anchors = Vec::new();
        for location in charged(
            &self.budget,
            ir::carrier_locations(context.self_ty, &self.budget),
        )?
        .iter()
        {
            let ir::CarrierChannel::ParamTail {
                parameter,
                prefix: layers,
            } = location.channel
            else {
                continue;
            };
            if parameter != source_parameter {
                continue;
            }
            charged(&self.budget, self.budget.charge_work(location.path.0.len()))?;
            if layers != 0
                || location
                    .path
                    .0
                    .iter()
                    .any(|step| matches!(step, ir::CarrierStep::VariantField { .. }))
            {
                return Ok(None);
            }
            let anchor = self.item_region_prefix(prefix, location)?.path;
            push(&self.budget, &mut anchors, anchor)?;
        }
        if anchors.is_empty() {
            return Ok(None);
        }
        let mut sources = Vec::new();
        for anchor in anchors {
            if let Some(alias) = alias {
                let Some(found) = self.item_projection_regions(
                    alias,
                    wanted,
                    &next,
                    &anchor,
                    facts,
                    depth + 1,
                    scratch,
                )?
                else {
                    return Ok(None);
                };
                for source in found {
                    push(&self.budget, &mut sources, source)?;
                }
                continue;
            }
            for (parameter, component) in &inverse {
                let Some(found) = self.item_parameter_regions(
                    *parameter,
                    component,
                    &next,
                    &anchor,
                    facts,
                    depth + 1,
                    scratch,
                )?
                else {
                    return Ok(None);
                };
                for source in found {
                    push(&self.budget, &mut sources, source)?;
                }
            }
        }
        Ok(Some(sources))
    }
}

#[cfg(test)]
mod cloned_item_source_tests {
    use super::*;
    use crate::compiler::{CompilerProvider, Input, Snapshot, TemplateBodyFact, keys};
    use crate::engine::Engine;

    #[test]
    fn cloned_struct_item_result_sources_use_original_stored_field() {
        let budget = PlanBudget::default();
        let source = "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let value=1i32;let escaped;{let values=vec![Item{value:&value}];let mut result=None;for item in values.iter().cloned(){result=Some(item);}escaped=result.unwrap();}println!(\"{}\",escaped.value);}";
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(source.into()),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: budget.clone(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let signatures = engine.request(keys::SignaturesDebtRoot.key());
        let main = engine.request(keys::item(crate::compiler::TEMPLATE_BODY, 0));
        engine.run(1).unwrap();
        main.completion().unwrap();
        let plan = signatures
            .fact::<crate::compiler::frontend::PlanCtx>(0)
            .unwrap();
        let template = main.fact::<TemplateBodyFact>(0).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let facts = plan
            .instance_facts(&CallTarget {
                declaration,
                type_args: Arc::from([]),
            })
            .unwrap();
        let calls = template
            .checked
            .exprs
            .iter()
            .filter_map(|expression| {
                let ir::ExprKind::Call { target, name, .. } = &expression.kind else {
                    return None;
                };
                name.ends_with("next").then_some(target)
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        let view = facts
            .carrier_contract(SignatureSubject::Call(calls[0]))
            .unwrap();
        let mut scratch = TraitScratch::default();
        for _ in 0..3 {
            view.validate_for_with_scratch(&facts, &mut scratch)
                .unwrap();
        }
        assert_eq!(view.contract.results.len(), 1);
        let expected = ir::InputCarrier {
            param: 0,
            location: ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([
                    ir::CarrierStep::Referent,
                    ir::CarrierStep::Field(0),
                    ir::CarrierStep::Field(0),
                    ir::CarrierStep::Referent,
                    ir::CarrierStep::SliceElement,
                    ir::CarrierStep::Field(0),
                ])),
                channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
            },
        };
        assert_eq!(
            view.contract.results[0].sources.as_ref(),
            &[expected],
            "original next result must retain only the original stored-field source",
        );
        for mutation in 0..3 {
            let mut forged = view.as_ref().clone();
            match mutation {
                0 => {
                    let source = &mut Arc::make_mut(
                        &mut Arc::make_mut(&mut forged.contract.results)[0].sources,
                    )[0];
                    source.location.path = ir::CarrierPath(Arc::from([
                        ir::CarrierStep::Referent,
                        ir::CarrierStep::Field(0),
                        ir::CarrierStep::Field(0),
                    ]));
                }
                1 => forged.declaration = declaration,
                _ => Arc::make_mut(&mut forged.contract.results)[0].sources = Arc::from([]),
            }
            assert!(
                matches!(forged.validate_for_with_scratch(&facts, &mut scratch), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata),
                "forged original stored-field source mutation={mutation}"
            );
        }
        assert!(!budget.has_resource_failure());
    }
}
