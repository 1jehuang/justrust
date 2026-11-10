//! Borrowed current-engine substitution over already-read nominal definitions.
//! No graph request, source-slot fallback, interner entry or retained world lives
//! here. A closed graph type can only keep its exact original typed read.
use crate::compiler::{frontend, ir};
use crate::engine::EngineAuthority;
mod calls;

#[cfg(test)]
pub(in crate::compiler) fn replay_closed_signature(
    engine: EngineAuthority,
    target: &ir::Callee,
    canonical: &ir::Signature,
    mapped: &ir::Signature,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    replay_closed_signature_with(
        engine,
        target,
        canonical,
        mapped,
        caller,
        budget,
        &mut crate::compiler::traits::TraitScratch::default(),
    )
}

pub(in crate::compiler) fn replay_closed_signature_with(
    engine: EngineAuthority,
    target: &ir::Callee,
    canonical: &ir::Signature,
    mapped: &ir::Signature,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    calls::realize(
        None,
        engine,
        target,
        (canonical, mapped),
        caller,
        budget,
        scratch,
    )
    .map(|_| ())
}

pub(in crate::compiler) fn replay_closed_type_mapping_with(
    context: Option<&frontend::PlanCtx>,
    engine: EngineAuthority,
    source: &ir::Ty,
    mapped: &ir::Ty,
    facts: &crate::compiler::traits::ValidatedFacts,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    calls::replay_type_mapping(context, engine, source, mapped, facts, budget, scratch)
}

pub(in crate::compiler) fn replay_closed_storage_with(
    engine: EngineAuthority,
    ty: &ir::Ty,
    facts: &crate::compiler::traits::ValidatedFacts,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    calls::replay_storage(engine, ty, facts, budget, scratch)
}

pub(in crate::compiler) fn replay_closed_body_signature_with(
    engine: EngineAuthority,
    signature: &ir::Signature,
    facts: &crate::compiler::traits::ValidatedFacts,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    calls::replay_body(engine, signature, facts, budget, scratch)
}

pub(in crate::compiler) async fn instantiate_checked_with_constants(
    template: &crate::compiler::ownership::VerifiedTemplateCfg,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    constants: &[std::sync::Arc<
        crate::compiler::associated_constant_graph::AssociatedConstantValueFact,
    >],
    cx: &crate::engine::Context,
) -> Result<ir::CheckedFunction, String> {
    let engine = cx.authority();
    let mut nominals = ExecutableNominals::default();
    loop {
        let mut adapter = ReadSubstitution {
            nominal_scratch: crate::compiler::traits::TraitScratch::default(),
            context,
            engine: Some(engine),
            executable: Some(&mut nominals),
            remint: Some(ParentLocalRemint {
                source: template.facts(),
                destination: closed_facts,
                environment: template.env(),
                parent_arguments: arguments,
                rows: Vec::new(),
                source_scratch: crate::compiler::traits::TraitScratch::default(),
                destination_scratch: crate::compiler::traits::TraitScratch::default(),
            }),
        };
        let attempt = ir::instantiate_checked_with_constants(
            template,
            arguments,
            closed_facts,
            &mut adapter,
            budget,
            constants,
        );
        drop(adapter);
        if !nominals.missing.is_empty() {
            // These are coordinates, not layout authority. Request every known
            // independent prerequisite before awaiting any successful typed read.
            budget.check_depth(0)?;
            let keys = std::mem::take(&mut nominals.missing);
            let reads = super::read_manifest(cx, &keys, budget)
                .await
                .map_err(|error| error.to_string())?;
            budget.retain_bytes(std::mem::size_of_val(reads.as_slice()))?;
            nominals
                .reads
                .try_reserve_exact(reads.len())
                .map_err(|_| "executable nominal read allocation failed")?;
            nominals.reads.extend(reads);
            continue;
        }
        let output = attempt?;
        output
            .body_substitution
            .as_ref()
            .ok_or("closed body omitted substitution proof")?
            .validate_current_engine(engine)?;
        return Ok(output);
    }
}

#[cfg(test)]
pub(in crate::compiler) fn substitute_ty(
    ty: &ir::Ty,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    caller: ir::FactsCtx<'_>,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
) -> Result<ir::Ty, String> {
    substitute_ty_with_scratch(
        ty,
        env,
        arguments,
        caller,
        context,
        budget,
        engine,
        &mut crate::compiler::traits::TraitScratch::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::compiler) fn substitute_ty_with_scratch(
    ty: &ir::Ty,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    caller: ir::FactsCtx<'_>,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<ir::Ty, String> {
    let mut adapter = ReadSubstitution {
        nominal_scratch: crate::compiler::traits::TraitScratch::default(),
        context,
        engine,
        executable: None,
        remint: None,
    };
    ir::substitute_ty_with_scratch(ty, env, arguments, caller, &mut adapter, budget, scratch)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::compiler) fn substitute_source_ty_with_scratch(
    ty: &ir::Ty,
    source_facts: &crate::compiler::traits::ValidatedFacts,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
    source_scratch: &mut crate::compiler::traits::TraitScratch,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<ir::Ty, String> {
    substitute_source_ty_for_use(
        ty,
        source_facts,
        env,
        arguments,
        closed_facts,
        context,
        budget,
        engine,
        source_scratch,
        scratch,
        true,
    )
}

/// Source operands used only to select proof/cleanup obligations, never physical layout.
#[allow(clippy::too_many_arguments)]
pub(in crate::compiler) fn substitute_source_schema_ty_with_scratch(
    ty: &ir::Ty,
    source_facts: &crate::compiler::traits::ValidatedFacts,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
    source_scratch: &mut crate::compiler::traits::TraitScratch,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<ir::Ty, String> {
    substitute_source_ty_for_use(
        ty,
        source_facts,
        env,
        arguments,
        closed_facts,
        context,
        budget,
        engine,
        source_scratch,
        scratch,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn substitute_source_ty_for_use(
    ty: &ir::Ty,
    source_facts: &crate::compiler::traits::ValidatedFacts,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
    source_scratch: &mut crate::compiler::traits::TraitScratch,
    scratch: &mut crate::compiler::traits::TraitScratch,
    executable_storage: bool,
) -> Result<ir::Ty, String> {
    SourceSubstitution::new(
        source_facts,
        env,
        arguments,
        closed_facts,
        context,
        budget,
        engine,
    )
    .ty_for_use(ty, source_scratch, scratch, executable_storage)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::compiler) fn substitute_callee_with_scratch(
    callee: &ir::Callee,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<ir::Callee, String> {
    let mut adapter = ReadSubstitution {
        nominal_scratch: crate::compiler::traits::TraitScratch::default(),
        context,
        engine,
        executable: None,
        remint: None,
    };
    ir::substitute_callee_with_scratch(
        callee,
        env,
        arguments,
        closed_facts,
        &mut adapter,
        budget,
        scratch,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::compiler) fn substitute_source_call_with_scratch(
    callee: &ir::Callee,
    signature: &ir::Signature,
    source_facts: &crate::compiler::traits::ValidatedFacts,
    env: &ir::GenericEnv,
    arguments: &[ir::Ty],
    closed_facts: &crate::compiler::traits::ValidatedFacts,
    context: &mut frontend::PlanCtx,
    budget: &ir::PlanBudget,
    engine: Option<EngineAuthority>,
    source_scratch: &mut crate::compiler::traits::TraitScratch,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(ir::Callee, Vec<ir::Ty>), String> {
    SourceSubstitution::new(
        source_facts,
        env,
        arguments,
        closed_facts,
        context,
        budget,
        engine,
    )
    .call(callee, signature, source_scratch, scratch)
}

/// One source-closing traversal. Proof scratch is local to the node and all
/// type roots share the same immutable source, destination and engine context.
/// Individual reads still authenticate their current capabilities and guards.
pub(in crate::compiler) struct SourceSubstitution<'a> {
    adapter: ReadSubstitution<'a>,
    budget: &'a ir::PlanBudget,
}

impl<'a> SourceSubstitution<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn new(
        source_facts: &'a crate::compiler::traits::ValidatedFacts,
        env: &'a ir::GenericEnv,
        arguments: &'a [ir::Ty],
        closed_facts: &'a crate::compiler::traits::ValidatedFacts,
        context: &'a mut frontend::PlanCtx,
        budget: &'a ir::PlanBudget,
        engine: Option<EngineAuthority>,
    ) -> Self {
        let adapter = ReadSubstitution {
            nominal_scratch: crate::compiler::traits::TraitScratch::default(),
            context,
            engine,
            executable: None,
            remint: Some(ParentLocalRemint {
                source: source_facts,
                destination: closed_facts,
                environment: env,
                parent_arguments: arguments,
                rows: Vec::new(),
                source_scratch: crate::compiler::traits::TraitScratch::default(),
                destination_scratch: crate::compiler::traits::TraitScratch::default(),
            }),
        };
        Self { adapter, budget }
    }

    pub(in crate::compiler) fn ty(
        &mut self,
        ty: &ir::Ty,
        source_scratch: &mut crate::compiler::traits::TraitScratch,
        scratch: &mut crate::compiler::traits::TraitScratch,
    ) -> Result<ir::Ty, String> {
        self.ty_for_use(ty, source_scratch, scratch, true)
    }

    pub(in crate::compiler) fn schema_ty(
        &mut self,
        ty: &ir::Ty,
        source_scratch: &mut crate::compiler::traits::TraitScratch,
        scratch: &mut crate::compiler::traits::TraitScratch,
    ) -> Result<ir::Ty, String> {
        self.ty_for_use(ty, source_scratch, scratch, false)
    }

    fn ty_for_use(
        &mut self,
        ty: &ir::Ty,
        source_scratch: &mut crate::compiler::traits::TraitScratch,
        scratch: &mut crate::compiler::traits::TraitScratch,
        executable_storage: bool,
    ) -> Result<ir::Ty, String> {
        let remint = self
            .adapter
            .remint
            .as_ref()
            .ok_or("source substitution lost its scoped context")?;
        let source_facts = remint.source;
        let closed_facts = remint.destination;
        let env = remint.environment;
        let arguments = remint.parent_arguments;
        let budget = self.budget;
        if !source_facts.budget().same_authority(budget) {
            return Err("source type closing substitution changed planning authority".into());
        }
        let already_current = closed_facts
            .validate_source_type_closing_context(source_facts, env, arguments)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        source_facts
            .validate_ty_metadata_scratch(ty, budget, source_scratch)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        // Even an exact closed source must walk its typed graph capabilities under
        // this engine. Metadata replay authenticates the world, not engine origin.
        // Keep this no-op source path logical: it does not mint physical layout.
        let mapped = if executable_storage && !already_current {
            ir::substitute_ty_with_scratch(
                ty,
                env,
                arguments,
                ir::FactsCtx::Validated(closed_facts),
                &mut self.adapter,
                budget,
                scratch,
            )?
        } else {
            ir::substitute_source_ty_with_scratch(
                ty,
                env,
                arguments,
                source_facts,
                closed_facts,
                &mut self.adapter,
                budget,
                source_scratch,
                scratch,
            )?
        };
        closed_facts
            .validate_ty_metadata_scratch(&mapped, budget, scratch)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        Ok(mapped)
    }

    pub(in crate::compiler) fn call(
        &mut self,
        callee: &ir::Callee,
        signature: &ir::Signature,
        source_scratch: &mut crate::compiler::traits::TraitScratch,
        scratch: &mut crate::compiler::traits::TraitScratch,
    ) -> Result<(ir::Callee, Vec<ir::Ty>), String> {
        let remint = self
            .adapter
            .remint
            .as_ref()
            .ok_or("source substitution lost its scoped context")?;
        let source_facts = remint.source;
        let closed_facts = remint.destination;
        let env = remint.environment;
        let arguments = remint.parent_arguments;
        let budget = self.budget;
        if !std::ptr::eq(source_facts.env().as_ref(), env)
            || !source_facts.budget().same_authority(budget)
            || !std::sync::Arc::ptr_eq(source_facts.world(), closed_facts.world())
        {
            return Err("call substitution changed original source environment".into());
        }
        source_facts
            .validate_source_flow_signature_with_scratch(
                ir::SignatureSubject::Call(callee),
                signature,
                source_scratch,
            )
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let inputs = source_facts
            .parent_local_call_inputs_with_scratch(callee, signature, source_scratch)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        budget.retain_bytes(std::mem::size_of_val(inputs.as_slice()))?;
        let mut nominals = Vec::new();
        nominals
            .try_reserve_exact(inputs.len())
            .map_err(|_| "call substitution nominal input allocation failed")?;
        for input in &inputs {
            nominals.push(ir::substitute_source_ty_with_scratch(
                input,
                env,
                arguments,
                source_facts,
                closed_facts,
                &mut self.adapter,
                budget,
                source_scratch,
                scratch,
            )?);
        }
        let target = ir::substitute_source_callee_with_nominals_scratch(
            callee,
            env,
            arguments,
            source_facts,
            closed_facts,
            &nominals,
            &mut self.adapter,
            budget,
            source_scratch,
            scratch,
        )?;
        Ok((target, nominals))
    }
}

struct ReadSubstitution<'a> {
    nominal_scratch: crate::compiler::traits::TraitScratch,
    context: &'a mut frontend::PlanCtx,
    engine: Option<EngineAuthority>,
    remint: Option<ParentLocalRemint<'a>>,
    executable: Option<&'a mut ExecutableNominals>,
}

#[derive(Default)]
struct ExecutableNominals {
    reads: Vec<super::ReadNominal>,
    missing: Vec<super::RealizationKey>,
}

struct ParentLocalRemint<'a> {
    source: &'a crate::compiler::traits::ValidatedFacts,
    destination: &'a crate::compiler::traits::ValidatedFacts,
    environment: &'a ir::GenericEnv,
    parent_arguments: &'a [ir::Ty],
    rows: Vec<(ir::Ty, std::sync::Arc<[ir::Ty]>, ir::Ty)>,
    source_scratch: crate::compiler::traits::TraitScratch,
    destination_scratch: crate::compiler::traits::TraitScratch,
}

impl ir::SubstitutionCx for ReadSubstitution<'_> {
    fn realizes_executable_nominals(&self) -> bool {
        self.executable.is_some()
    }
    fn realize_alignment_query(
        &mut self,
        query: &mut ir::TypeAlignmentQuery,
        queried: ir::Ty,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        if self.executable.is_none() {
            return ir::SubstitutionCx::realize_alignment_query(
                self.context,
                query,
                queried,
                caller,
                budget,
            );
        }
        let engine = self
            .engine
            .ok_or("alignment realization requires current engine")?;
        let facts = caller
            .validated()
            .ok_or("alignment realization requires validated facts")?;
        query.substitute_realized(
            queried,
            engine,
            self.context,
            facts,
            budget,
            &mut self.nominal_scratch,
        )
    }
    fn substitute_parent_local_nominal(
        &mut self,
        source: &ir::Ty,
        arguments: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let Some(remint) = &mut self.remint else {
            return ir::SubstitutionCx::substitute_parent_local_nominal(
                self.context,
                source,
                arguments,
                caller,
                budget,
            );
        };
        let current = caller
            .validated()
            .ok_or("source closing substitution requires validated destination facts")?;
        if !std::ptr::eq(current, remint.destination) {
            return Err("source closing substitution changed destination facts".into());
        }
        let receipt = match source {
            ir::Ty::Struct(value) => value.parent_local_receipt(),
            ir::Ty::Enum(value) => value.parent_local_receipt(),
            _ => return Err("source closing substitution requires nominal receipt".into()),
        }
        .ok_or("source closing substitution omitted canonical receipt")?;
        // Both authorities remain live and authenticated even on a memo hit.
        let owning_facts = if receipt.matches_owning_context(current)? {
            current
        } else {
            remint.source
        };
        let owning_scratch = if std::ptr::eq(owning_facts, current) {
            &mut remint.destination_scratch
        } else {
            &mut remint.source_scratch
        };
        owning_facts
            .validate_ty_metadata_scratch(source, budget, owning_scratch)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        for argument in arguments {
            current
                .validate_ty_metadata_scratch(argument, budget, &mut remint.destination_scratch)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
        for (original, retained_arguments, output) in &remint.rows {
            budget.charge_work(1)?;
            let same_source = match (original, source) {
                (ir::Ty::Struct(left), ir::Ty::Struct(right)) => {
                    std::ptr::eq(left.def(), right.def())
                        && matches!((left.parent_local_receipt(), right.parent_local_receipt()), (Some(a), Some(b)) if std::sync::Arc::ptr_eq(a,b))
                }
                (ir::Ty::Enum(left), ir::Ty::Enum(right)) => {
                    std::ptr::eq(left.def(), right.def())
                        && matches!((left.parent_local_receipt(), right.parent_local_receipt()), (Some(a), Some(b)) if std::sync::Arc::ptr_eq(a,b))
                }
                _ => false,
            };
            if !same_source || retained_arguments.len() != arguments.len() {
                continue;
            }
            let same = crate::compiler::traits::schema_instantiation::same_inputs(
                retained_arguments,
                arguments,
                budget,
                0,
            )
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
            if same {
                current
                    .validate_ty_metadata_scratch(output, budget, &mut remint.destination_scratch)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?;
                return Ok(output.clone());
            }
        }
        let output = receipt.remint_for_source_substitution(
            source,
            remint.source,
            current,
            remint.environment,
            remint.parent_arguments,
            arguments,
            budget,
        )?;
        budget.retain_bytes(
            std::mem::size_of::<(ir::Ty, std::sync::Arc<[ir::Ty]>, ir::Ty)>()
                + std::mem::size_of_val(arguments),
        )?;
        remint
            .rows
            .try_reserve_exact(1)
            .map_err(|_| "closed source receipt replay allocation failed")?;
        remint
            .rows
            .push((source.clone(), arguments.into(), output.clone()));
        Ok(output)
    }
    fn current_engine(&self) -> Option<EngineAuthority> {
        self.engine
    }
    fn realize_canonical_call_signature(
        &mut self,
        target: &ir::Callee,
        canonical: &ir::Signature,
        mapped: &ir::Signature,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<Option<ir::Signature>, String> {
        let engine = self
            .engine
            .ok_or("closed call realization has no current engine")?;
        calls::realize(
            Some(self.context),
            engine,
            target,
            (canonical, mapped),
            caller,
            budget,
            &mut self.nominal_scratch,
        )
        .map(Some)
    }
    fn nominal_application(&self, id: ir::NominalAppId) -> Result<ir::NominalAppInfo, String> {
        // The identity-only API remains source-only, even in this adapter.
        ir::SubstitutionCx::nominal_application(self.context, id)
    }

    fn retain_realized_nominal(
        &mut self,
        source: &ir::Ty,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let engine = self
            .engine
            .ok_or("graph nominal substitution has no current engine")?;
        let capability = match source {
            ir::Ty::Struct(value) => value.realization(),
            ir::Ty::Enum(value) => value.realization(),
            _ => return Err("typed nominal identity substitution requires a nominal".into()),
        }
        .ok_or("graph nominal substitution omitted its typed read")?;
        capability.retain_for_substitution_with_scratch(
            source,
            engine,
            caller,
            budget,
            &mut self.nominal_scratch,
        )
    }

    fn intern_nominal(
        &mut self,
        declaration: ir::DeclId,
        arguments: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        ir::SubstitutionCx::intern_nominal(self.context, declaration, arguments, caller, budget)
    }

    fn intern_substituted_nominal(
        &mut self,
        request: &ir::NominalSubstitutionRequest<'_>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        if let Some(inputs) = &mut self.executable {
            let facts = caller
                .validated()
                .ok_or("executable nominal substitution omitted checked caller")?;
            let remint = self
                .remint
                .as_mut()
                .ok_or("executable nominal substitution omitted original source")?;
            let mut coordinates = Vec::new();
            coordinates
                .try_reserve_exact(request.arguments().len())
                .map_err(|_| "executable nominal argument allocation failed")?;
            for argument in request.arguments() {
                // Normalize only argument coordinates here. Physical fields
                // come exclusively from the canonical realization producer,
                // never from rewriting a source definition in this adapter.
                let normalized = facts
                    .normalize_substituted_source_type(
                        argument,
                        remint.source,
                        remint.environment,
                        remint.parent_arguments,
                        budget,
                        &mut remint.destination_scratch,
                    )
                    .map_err(|error| error.into_legacy_diagnostic(budget))?;
                coordinates.push(
                    super::coordinates::Term::from_ty(&normalized, budget, 0)
                        .and_then(|term| term.encode(budget))
                        .map_err(|error| error.to_string())?,
                );
            }
            let words = coordinates
                .iter()
                .try_fold(6usize, |n, argument| {
                    n.checked_add(argument.len()).and_then(|n| n.checked_add(1))
                })
                .ok_or("executable nominal coordinate overflow")?;
            let key = super::RealizationKey::encode(
                facts.world().nominal_realization_world(),
                request.source_decl(),
                &coordinates,
                words,
            )
            .map_err(|error| error.to_string())?;
            if let Some(read) = inputs
                .reads
                .iter()
                .find(|read| read.application_key() == &key)
            {
                let definition = read.definition().map_err(|error| error.to_string())?;
                self.nominal_scratch
                    .replay_nominal(read, &definition, facts)?;
                return read.retain_for_substitution_with_scratch(
                    &definition,
                    self.engine.ok_or("executable nominal omitted engine")?,
                    caller,
                    budget,
                    &mut self.nominal_scratch,
                );
            }
            budget.retain_bytes(std::mem::size_of::<super::RealizationKey>())?;
            inputs
                .missing
                .try_reserve_exact(1)
                .map_err(|_| "executable nominal prerequisite allocation failed")?;
            inputs.missing.push(key);
            return Err("missing typed executable nominal realization".into());
        }
        ir::SubstitutionCx::intern_substituted_nominal(self.context, request, caller, budget)
    }

    fn instantiate_signature(
        &mut self,
        target: &ir::CallTarget,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Signature, String> {
        ir::SubstitutionCx::instantiate_signature(self.context, target, caller, budget)
    }
}
