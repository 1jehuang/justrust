//! Immutable prerequisites and independently computed source callable schemas.
//!
//! Construction scratch never escapes a producer. Canonical nominal deltas
//! accompany each schema so the debt constructor can retain every referenced
//! application without relying on another callable's interning order.
use super::*;

#[derive(Debug)]
pub(crate) struct CallableSignatureInputs {
    registry: Arc<DeclRegistry>,
    contexts: Arc<HashMap<ir::DeclId, CallableContext>>,
    nominal_schemas: Arc<HashMap<ir::DeclId, Arc<NominalSchema>>>,
    nominals: Arc<CanonicalNominals>,
    authority: ir::PlanAuthority,
    modules: Vec<CallableSignatureModule>,
    assembly_storage: ir::RetainedStorage,
}
impl crate::engine::Fact for CallableSignatureInputs {}

#[derive(Clone, Debug)]
struct CallableSignatureModule {
    module: ModuleId,
    callables: Vec<usize>,
    input_size: usize,
}

/// A keyed module's complete callable schemas, not a whole-program join.
#[derive(Debug)]
pub(crate) struct ModuleSignatureOutput {
    inputs: Arc<CallableSignatureInputs>,
    ordinal: usize,
    callables: Vec<(usize, Arc<CallableSignatureOutput>)>,
    error: Option<(usize, String)>,
}
impl crate::engine::Fact for ModuleSignatureOutput {}

#[derive(Debug)]
pub(crate) struct CallableSignatureOutput {
    inputs: Arc<CallableSignatureInputs>,
    declaration: ir::DeclId,
    schema: StoredSchema,
    source_regions: Arc<ir::SourceRegionEnv>,
    region_mode: tr::RegionContractMode,
    nominals: CanonicalNominals,
}
impl crate::engine::Fact for CallableSignatureOutput {}

impl CallableSignatureInputs {
    pub(super) fn new(
        registry: Arc<DeclRegistry>,
        mut contexts: HashMap<ir::DeclId, CallableContext>,
        state: &mut PlanState,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<Self>, String> {
        // Published baseline payloads must not rely on refundable construction
        // leases held by the suspended debt constructor.
        for reservation in std::mem::take(&mut state.reservations) {
            reservation.retain()?;
        }
        for id in registry
            .function_ids
            .iter()
            .take(registry.callable_sources.len())
        {
            budget.charge_work(1)?;
            if !contexts.contains_key(id) {
                budget.retain_bytes(512)?;
                let info = registry.declaration(*id)?;
                contexts.insert(
                    *id,
                    CallableContext {
                        module: info.path.module,
                        self_ty: None,
                        associated_owner: None,
                        receiver: ReceiverKind::None,
                    },
                );
            }
        }
        let count = registry.callable_sources.len();
        let mut module_slots = HashMap::new();
        reserve_registry_map(&mut module_slots, count, budget)?;
        budget.retain_bytes(
            count
                .checked_mul(std::mem::size_of::<CallableSignatureModule>())
                .ok_or("signature module inventory overflow")?,
        )?;
        let mut modules: Vec<CallableSignatureModule> = Vec::new();
        modules
            .try_reserve_exact(count)
            .map_err(|_| "signature module inventory allocation failed")?;
        for (index, declaration) in registry.function_ids.iter().take(count).enumerate() {
            budget.charge_work(1)?;
            let module = contexts
                .get(declaration)
                .ok_or("missing sealed callable context")?
                .module;
            let ordinal = *module_slots.entry(module).or_insert_with(|| {
                modules.push(CallableSignatureModule {
                    module,
                    callables: Vec::new(),
                    input_size: 0,
                });
                modules.len() - 1
            });
            let group = &mut modules[ordinal];
            // Geometric Vec growth retains at most four slots per inserted
            // index, including the minimum allocation for singleton modules.
            budget.retain_bytes(4 * std::mem::size_of::<usize>())?;
            group
                .callables
                .try_reserve(1)
                .map_err(|_| "signature module callable allocation failed")?;
            group.callables.push(index);
            group.input_size = group
                .input_size
                .checked_add(registry.source_callable_input_size(*declaration))
                .ok_or("signature module work size overflow")?;
        }
        let assembly_storage = budget
            .reserve_bytes(
                count
                    .checked_mul(
                        std::mem::size_of::<Option<Arc<CallableSignatureOutput>>>()
                            + std::mem::size_of::<Arc<CallableSignatureOutput>>(),
                    )
                    .ok_or("signature output assembly overflow")?,
            )?
            .retain()?;
        let mut baseline = NominalInterner::default();
        merge_nominals(
            &mut baseline,
            &state.nominals.applications,
            &state.nominals.by_slot,
            budget,
        )?;
        budget
            .retain_bytes(std::mem::size_of::<Self>() + std::mem::size_of::<CanonicalNominals>())?;
        Ok(Arc::new(Self {
            registry,
            contexts: Arc::new(contexts),
            nominal_schemas: state.schemas.clone(),
            nominals: Arc::new(CanonicalNominals {
                applications: baseline.applications,
                by_slot: baseline.by_slot,
            }),
            authority: budget.authority().clone(),
            modules,
            assembly_storage,
        }))
    }

    pub(super) fn contexts(&self) -> Arc<HashMap<ir::DeclId, CallableContext>> {
        self.contexts.clone()
    }

    pub(crate) fn callable_count(&self) -> usize {
        self.registry.callable_sources.len()
    }

    pub(crate) fn module_count(&self) -> usize {
        self.modules.len()
    }

    pub(crate) fn module_input_size(&self, ordinal: usize) -> Result<usize, String> {
        Ok(self
            .modules
            .get(ordinal)
            .ok_or("missing signature module")?
            .input_size)
    }

    pub(crate) fn declaration(&self, index: usize) -> Result<ir::DeclId, String> {
        if index >= self.callable_count() {
            return Err("missing source callable signature".into());
        }
        self.registry
            .function_ids
            .get(index)
            .copied()
            .ok_or_else(|| "missing source callable declaration".into())
    }
}

impl CallableSignatureOutput {
    pub(super) fn install(
        &self,
        inputs: &Arc<CallableSignatureInputs>,
        index: usize,
        source_schemas: &mut SourceSchemasBuilder,
        state: &mut PlanState,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        if !Arc::ptr_eq(&self.inputs, inputs)
            || self.declaration != inputs.declaration(index)?
            || !inputs.authority.same_authority(budget.authority())
            || !self
                .schema
                .params_storage
                .authority()
                .same_authority(budget.authority())
        {
            return Err(
                "callable signature output belongs to different inputs or authority".into(),
            );
        }
        let slot = source_schemas::decl_slot(self.declaration, &inputs.registry, budget)?;
        source_schemas.functions.insert(
            slot,
            StoredSchema::copy(&self.schema.value, budget)?,
            budget,
        )?;
        source_schemas.source_regions.insert(
            source_schemas::decl_slot(self.declaration, &inputs.registry, budget)?,
            self.source_regions.clone(),
            budget,
        )?;
        source_schemas.set_region_mode(
            self.declaration,
            self.region_mode,
            &inputs.registry,
            budget,
        )?;
        merge_nominals(
            &mut state.nominals,
            &self.nominals.applications,
            &self.nominals.by_slot,
            budget,
        )
    }
}

fn merge_nominals(
    target: &mut NominalInterner,
    applications: &HashMap<(ir::DeclId, Vec<ir::Ty>), ir::Ty>,
    slots: &HashMap<usize, ir::Ty>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    reserve_merge_map(&mut target.applications, applications.len(), budget)?;
    reserve_merge_map(&mut target.by_slot, slots.len(), budget)?;
    for (slot, ty) in slots {
        budget.charge_work(1)?;
        if let Some(previous) = target.by_slot.get(slot) {
            let same = match (previous, ty) {
                (ir::Ty::Struct(a), ir::Ty::Struct(b)) => {
                    a.def().source_decl == b.def().source_decl
                        && a.def().type_args == b.def().type_args
                }
                (ir::Ty::Enum(a), ir::Ty::Enum(b)) => {
                    a.def().source_decl == b.def().source_decl
                        && a.def().type_args == b.def().type_args
                }
                _ => false,
            };
            if !same {
                return Err("unsupported: nominal application identity collision".into());
            }
        } else {
            target.by_slot.insert(*slot, ty.clone());
        }
    }
    for (key, ty) in applications {
        let (declaration, arguments) = key;
        budget.charge_work(arguments.len().saturating_add(1))?;
        if target.applications.contains_key(key) {
            continue;
        }
        budget.retain_bytes(
            arguments
                .len()
                .checked_mul(std::mem::size_of::<ir::Ty>())
                .ok_or("signature nominal key overflow")?,
        )?;
        let mut copied = Vec::new();
        copied
            .try_reserve_exact(arguments.len())
            .map_err(|_| "signature nominal key allocation failed")?;
        copied.extend_from_slice(arguments);
        target
            .applications
            .insert((*declaration, copied), ty.clone());
    }
    Ok(())
}

fn reserve_merge_map<K: Eq + std::hash::Hash, V>(
    table: &mut HashMap<K, V>,
    additional: usize,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let needed = table
        .len()
        .checked_add(additional)
        .ok_or("signature nominal map overflow")?;
    if needed <= table.capacity() {
        return Ok(());
    }
    let buckets = needed
        .checked_mul(8)
        .and_then(|n| n.checked_add(6))
        .map(|n| n / 7)
        .and_then(|n| n.max(4).checked_next_power_of_two())
        .ok_or("signature nominal map overflow")?;
    let bytes = buckets
        .checked_mul(std::mem::size_of::<(K, V)>().saturating_add(1))
        .and_then(|n| n.checked_add(16))
        .ok_or("signature nominal map overflow")?;
    budget.retain_bytes(bytes)?;
    table
        .try_reserve(additional)
        .map_err(|_| "signature nominal map allocation failed")?;
    Ok(())
}

pub(crate) fn compute_module_signatures(
    inputs: Arc<CallableSignatureInputs>,
    ordinal: usize,
    budget: &ir::PlanBudget,
) -> Result<ModuleSignatureOutput, String> {
    if !inputs.authority.same_authority(budget.authority()) {
        return Err("signature module planning authority changed".into());
    }
    let group = inputs
        .modules
        .get(ordinal)
        .ok_or("missing signature module")?;
    budget.retain_bytes(
        group
            .callables
            .len()
            .checked_mul(std::mem::size_of::<(usize, Arc<CallableSignatureOutput>)>())
            .and_then(|n| {
                n.checked_add(
                    std::mem::size_of::<ModuleSignatureOutput>() + 2 * std::mem::size_of::<usize>(),
                )
            })
            .ok_or("signature module output allocation overflow")?,
    )?;
    let mut callables = Vec::new();
    callables
        .try_reserve_exact(group.callables.len())
        .map_err(|_| "signature module output allocation failed")?;
    let mut error = None;
    for &index in &group.callables {
        budget.charge_work(1)?;
        let declaration = inputs.declaration(index)?;
        if inputs
            .contexts
            .get(&declaration)
            .map(|context| context.module)
            != Some(group.module)
        {
            return Err("signature callable belongs to another module".into());
        }
        match compute_callable_signature(inputs.clone(), index, budget) {
            Ok(signature) => callables.push((index, Arc::new(signature))),
            Err(diagnostic) => {
                // Resource/infrastructure faults are not source diagnostics.
                if budget.first_resource_failure().is_some()
                    || diagnostic.starts_with("infrastructure:")
                {
                    return Err(diagnostic);
                }
                // Keep the module diagnostic and pre-admit the selected
                // diagnostic's owned copy at the debt assembly boundary.
                budget.retain_bytes(
                    diagnostic
                        .len()
                        .checked_mul(2)
                        .ok_or("signature diagnostic allocation overflow")?,
                )?;
                error = Some((index, diagnostic));
                break;
            }
        }
    }
    Ok(ModuleSignatureOutput {
        inputs,
        ordinal,
        callables,
        error,
    })
}

/// The only cross-module assembly remains at the existing signatures debt
/// boundary. Storage for both the indexed scratch and final ordered vector was
/// admitted before publishing the inputs, without retaining a budget handle.
pub(crate) fn finish_module_signatures(
    inputs: &Arc<CallableSignatureInputs>,
    modules: Vec<Arc<ModuleSignatureOutput>>,
) -> Result<Vec<Arc<CallableSignatureOutput>>, String> {
    if modules.len() != inputs.module_count() {
        return Err("signature module output count changed".into());
    }
    if !inputs
        .assembly_storage
        .authority()
        .same_authority(&inputs.authority)
    {
        return Err("signature module assembly authority changed".into());
    }
    let mut first_error: Option<&(usize, String)> = None;
    for (ordinal, output) in modules.iter().enumerate() {
        let group = &inputs.modules[ordinal];
        if !Arc::ptr_eq(inputs, &output.inputs) || output.ordinal != ordinal {
            return Err("signature module output belongs to different inputs or module".into());
        }
        if output.callables.len() > group.callables.len() {
            return Err("signature module callable output count changed".into());
        }
        // Source diagnostics must not mask malformed successful prefixes,
        // including prefixes from another module whose diagnostic loses the
        // original callable-order comparison below.
        for ((index, signature), expected) in output.callables.iter().zip(&group.callables) {
            if index != expected
                || !Arc::ptr_eq(inputs, &signature.inputs)
                || signature.declaration != inputs.declaration(*index)?
            {
                return Err("signature module callable identity changed".into());
            }
        }
        if let Some(error) = &output.error {
            if group.callables.get(output.callables.len()) != Some(&error.0) {
                return Err("signature module diagnostic identity changed".into());
            }
            if first_error.is_none_or(|previous| error.0 < previous.0) {
                first_error = Some(error);
            }
        } else if output.callables.len() != group.callables.len() {
            return Err("signature module callable output count changed".into());
        }
    }
    if let Some((_, diagnostic)) = first_error {
        return Err(diagnostic.clone());
    }
    let count = inputs.callable_count();
    let mut ordered = Vec::new();
    ordered
        .try_reserve_exact(count)
        .map_err(|_| "signature module assembly allocation failed")?;
    ordered.resize_with(count, || None);
    for output in &modules {
        for (index, signature) in &output.callables {
            let slot = ordered
                .get_mut(*index)
                .ok_or("signature module callable index changed")?;
            if slot.replace(signature.clone()).is_some() {
                return Err("duplicate signature module callable output".into());
            }
        }
    }
    let mut outputs = Vec::new();
    outputs
        .try_reserve_exact(count)
        .map_err(|_| "signature output assembly allocation failed")?;
    let bytes = ordered
        .capacity()
        .checked_mul(std::mem::size_of::<Option<Arc<CallableSignatureOutput>>>())
        .and_then(|n| {
            outputs
                .capacity()
                .checked_mul(std::mem::size_of::<Arc<CallableSignatureOutput>>())
                .and_then(|m| n.checked_add(m))
        })
        .ok_or("signature output assembly capacity overflow")?;
    if bytes > inputs.assembly_storage.bytes() {
        return Err("infrastructure: signature output assembly exceeds prepaid storage".into());
    }
    for output in ordered {
        outputs.push(output.ok_or("missing signature module callable output")?);
    }
    Ok(outputs)
}

pub(super) fn compute_standalone(
    inputs: Arc<CallableSignatureInputs>,
    budget: &ir::PlanBudget,
) -> Result<Vec<Arc<CallableSignatureOutput>>, String> {
    let count = inputs.module_count();
    budget.retain_bytes(
        count
            .checked_mul(std::mem::size_of::<Arc<ModuleSignatureOutput>>())
            .ok_or("signature outputs allocation overflow")?,
    )?;
    let mut outputs = Vec::new();
    outputs
        .try_reserve_exact(count)
        .map_err(|_| "signature outputs allocation failed")?;
    for index in 0..count {
        outputs.push(Arc::new(compute_module_signatures(
            inputs.clone(),
            index,
            budget,
        )?));
    }
    finish_module_signatures(&inputs, outputs)
}

/// No executor is needed for the standalone API: its callback is immediately
/// ready and the constructor has no other suspension points. Graph callers use
/// the async constructor directly and suspend on engine facts instead.
pub(super) fn immediate<T>(
    future: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let mut future = std::pin::pin!(future);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::future::Future::poll(future.as_mut(), &mut cx) {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => {
            Err("infrastructure: standalone signature constructor unexpectedly suspended".into())
        }
    }
}

pub(crate) fn compute_callable_signature(
    inputs: Arc<CallableSignatureInputs>,
    index: usize,
    budget: &ir::PlanBudget,
) -> Result<CallableSignatureOutput, String> {
    if !inputs.authority.same_authority(budget.authority()) {
        return Err("callable signature planning authority changed".into());
    }
    let id = inputs.declaration(index)?;
    let registry = &inputs.registry;
    let program = registry.program.as_ref();
    let mut state = PlanState::scratch(&inputs.nominal_schemas, &inputs.nominals);
    let f = match registry.callable_sources[index] {
        CallableSource::Free(i) => &program.functions[i],
        CallableSource::Associated {
            implementation,
            item,
        } => &program.impls[implementation].items[item].function,
        CallableSource::TraitDefault { trait_index, item } => program.traits[trait_index].items
            [item]
            .default
            .as_ref()
            .ok_or("missing source default body")?,
    };
    let info = registry.declaration(id)?;
    state.source_context = Some(registry.source_type_context(id)?);
    let mut bindings = generic_bindings(registry, &info.env, budget)?;
    let context = inputs
        .contexts
        .get(&id)
        .ok_or("missing sealed callable context")?
        .clone();
    if let Some(ty) = &context.self_ty {
        bindings.insert(GenericBindingKey::SelfType, ty.clone());
    }
    let caller = if !info.env.is_generic() {
        ir::FactsCtx::Concrete
    } else {
        ir::FactsCtx::Universal(&info.env)
    };
    let mut names = std::collections::HashSet::new();
    for (name, _) in &f.params {
        if let Some(key) = name.checked_key(program.names.as_ref())?
            && !names.insert(key)
        {
            return Err("duplicate parameter".into());
        }
    }
    // Additional exact conservative admission, not a refund or carveout
    // from the existing broad declaration reservation.
    let capacity = f
        .params
        .len()
        .checked_add(usize::from(context.receiver != ReceiverKind::None))
        .ok_or("signature params overflow")?;
    let (mut params, params_storage) = StoredSchema::params(capacity, budget)?;
    budget.charge_work(f.params.len())?;
    for (_, ty) in &f.params {
        params.push(state.source_ty(registry, ty, &bindings, caller, budget, 0, true)?);
    }
    if context.receiver != ReceiverKind::None {
        budget.charge_work(params.len() + 1)?;
        params.insert(
            0,
            receiver_ty(
                context.receiver,
                context
                    .self_ty
                    .clone()
                    .ok_or("receiver without Self type")?,
            ),
        );
    }
    let result = state.source_ty(registry, &f.result, &bindings, caller, budget, 0, true)?;
    reject_slice_result(&result)?;
    if info.env.is_generic() {
        for ty in params.iter().chain(std::iter::once(&result)) {
            if !raw_type_has_projection(ty, budget, 0)? {
                validate_root_carrier(ty, caller, budget)?;
            }
        }
    }
    let region_mode = source_region_mode(
        &f.regions,
        &f.params,
        &f.result,
        matches!(registry.callable_sources[index], CallableSource::Free(_)),
        budget,
    )?;
    state.retain(budget, 128)?;
    let signature = ir::Signature {
        ref_contract: canonical_ref_contract(
            (program.names.as_ref(), budget),
            &f.regions,
            &f.params,
            &f.result,
            (context.receiver, region_mode),
            &params,
            &result,
        )?,
        params,
        result,
    };
    let source_regions = carrier_regions::attach(
        registry,
        &f.regions,
        &f.params,
        &f.result,
        match registry.callable_sources[index] {
            CallableSource::Associated { implementation, .. } => {
                Some(&program.impls[implementation])
            }
            _ => None,
        },
        &signature,
        context.receiver,
        original_source_regions(
            (program.names.as_ref(), budget),
            id,
            &f.regions,
            &f.params,
            &f.result,
            &signature.ref_contract,
            context.receiver,
        )?,
        budget,
    )?;
    let mut rows = Vec::new();
    append_implicit_sized(registry, &info.env, &mut rows, budget)?;
    if let CallableSource::Associated { implementation, .. } = registry.callable_sources[index] {
        append_source_bounds(
            &mut state,
            registry,
            &bindings,
            &program.impls[implementation].bounds,
            &mut rows,
            caller,
            budget,
        )?;
        if let Some(path) = &program.impls[implementation].trait_path {
            let trait_ref = state
                .source_trait_application(
                    registry,
                    path,
                    context
                        .self_ty
                        .as_ref()
                        .ok_or("trait impl item without Self")?,
                    &bindings,
                    caller,
                    budget,
                )?
                .reject_equalities()?;
            if rows.len() >= tr::MAX_PREDICATES {
                return Err("predicate environment limit exceeded".into());
            }
            budget.retain_bytes(std::mem::size_of::<ir::Predicate>())?;
            let old_capacity = rows.capacity();
            rows.try_reserve_exact(1).map_err(|_| {
                super::super::source::SourceError::from_token(
                    super::super::token_session::TokenError::Budget(
                        registry.names.budget().poison_allocation(),
                    ),
                )
                .to_string()
            })?;
            budget.retain_bytes(
                rows.capacity()
                    .saturating_sub(old_capacity)
                    .saturating_sub(1)
                    .checked_mul(std::mem::size_of::<ir::Predicate>())
                    .ok_or("trait impl item predicate capacity overflow")?,
            )?;
            rows.push(ir::Predicate::Implements(trait_ref));
        }
    }
    append_source_bounds(
        &mut state, registry, &bindings, &f.bounds, &mut rows, caller, budget,
    )?;
    if let CallableSource::TraitDefault { trait_index, .. } = registry.callable_sources[index] {
        let declaration = registry.trait_ids[trait_index];
        let owner = registry.declaration(declaration)?;
        append_source_bounds(
            &mut state,
            registry,
            &bindings,
            &program.traits[trait_index].bounds,
            &mut rows,
            caller,
            budget,
        )?;
        let mut args = symbolic_args(&owner.param_env, budget)?;
        args.remove(0);
        budget.retain_bytes(256 + args.capacity() * std::mem::size_of::<ir::Ty>())?;
        rows.push(ir::Predicate::Implements(ir::TraitRef {
            declaration,
            self_ty: context.self_ty.clone().ok_or("default without Self")?,
            args: args.into(),
        }));
    }
    let schema = StoredSchema {
        value: tr::GenericSchema {
            env: info.env.clone(),
            signature,
            predicates: rows.into(),
        },
        params_storage,
    };
    for reservation in std::mem::take(&mut state.reservations) {
        reservation.retain()?;
    }
    let nominals = CanonicalNominals {
        applications: state.nominals.applications,
        by_slot: state.nominals.by_slot,
    };
    budget.retain_bytes(
        std::mem::size_of::<CallableSignatureOutput>() + 2 * std::mem::size_of::<usize>(),
    )?;
    Ok(CallableSignatureOutput {
        inputs,
        declaration: id,
        schema,
        source_regions,
        region_mode,
        nominals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "struct Wrap<T>{value:T} fn first(x:Wrap<i32>)->Wrap<i32>{x} fn second(x:Wrap<i32>)->Wrap<i32>{x} fn main(){} mod nested{pub fn inside(x:i32)->i32{x}}";

    fn construct(
        budget: &ir::PlanBudget,
        compute: impl FnOnce(
            Arc<CallableSignatureInputs>,
        ) -> Result<Vec<Arc<CallableSignatureOutput>>, String>,
    ) -> Result<PlanCtx, String> {
        use crate::compiler::{expansion_budget, owner_path};
        let owner = |domain| {
            owner_path::standalone_issued(&expansion_budget::ExpansionBudget::default(), domain)
                .unwrap()
        };
        immediate(PlanCtx::from_shared_program_with_signatures(
            Arc::new(parse(SOURCE).unwrap()),
            budget,
            true,
            owner(owner_path::OwnerDomain::DeclarationSnapshot),
            owner(owner_path::OwnerDomain::TraitWorld),
            |inputs| std::future::ready(compute(inputs)),
        ))
    }

    #[test]
    fn callable_signature_producers_preserve_nominals_in_reverse_completion_order() {
        let budget = ir::PlanBudget::default();
        let plan = construct(&budget, |inputs| {
            let mut outputs = Vec::new();
            for index in (0..inputs.callable_count()).rev() {
                outputs.push(Arc::new(compute_callable_signature(
                    inputs.clone(),
                    index,
                    &budget,
                )?));
            }
            outputs.reverse();
            let user = outputs
                .iter()
                .filter(|output| {
                    let info = inputs.registry.declaration(output.declaration).unwrap();
                    match info.path.item {
                        DeclItemName::Source(name) => matches!(
                            inputs.registry.names.spelling(name).unwrap(),
                            "first" | "second"
                        ),
                        _ => false,
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(user.len(), 2);
            assert!(
                !user[0].nominals.applications.is_empty(),
                "concrete application must be produced by the callable, not its symbolic baseline"
            );
            assert_eq!(
                user[0].schema.value.signature.params,
                user[1].schema.value.signature.params
            );
            assert_eq!(
                user[0].schema.value.signature.result,
                user[1].schema.value.signature.result
            );
            Ok(outputs)
        })
        .unwrap();
        let concrete = plan
            .nominals
            .applications
            .iter()
            .filter(|((_, args), _)| args.as_slice() == [ir::Ty::Int(ir::IntTy::I32)])
            .collect::<Vec<_>>();
        assert_eq!(
            concrete.len(),
            1,
            "both independent producers must retain one canonical application"
        );
        for id in plan.registry.function_ids.iter().take(2) {
            let signature = plan.signature(*id).unwrap();
            assert_eq!(signature.signature.params[0], signature.signature.result);
        }
    }

    #[test]
    fn callable_signature_producers_reject_foreign_inputs_payer_and_slots() {
        let budget = ir::PlanBudget::default();
        let foreign = ir::PlanBudget::default();
        construct(&budget, |inputs| {
            assert!(
                compute_callable_signature(inputs.clone(), 0, &foreign)
                    .unwrap_err()
                    .contains("authority")
            );
            assert!(
                compute_callable_signature(inputs.clone(), inputs.callable_count(), &budget)
                    .is_err()
            );
            let mut output = compute_callable_signature(inputs.clone(), 0, &budget)?;
            let copied_inputs = Arc::new(CallableSignatureInputs {
                registry: inputs.registry.clone(),
                contexts: inputs.contexts.clone(),
                nominal_schemas: inputs.nominal_schemas.clone(),
                nominals: inputs.nominals.clone(),
                authority: inputs.authority.clone(),
                modules: inputs.modules.clone(),
                assembly_storage: inputs.assembly_storage.clone(),
            });
            let mut builder = SourceSchemasBuilder::default();
            let mut state = PlanState::scratch(&inputs.nominal_schemas, &inputs.nominals);
            assert!(
                output
                    .install(&copied_inputs, 0, &mut builder, &mut state, &budget)
                    .unwrap_err()
                    .contains("different inputs")
            );
            assert!(
                output
                    .install(
                        &inputs,
                        inputs.callable_count(),
                        &mut builder,
                        &mut state,
                        &budget
                    )
                    .is_err()
            );
            output.schema = StoredSchema::copy(&output.schema.value, &foreign)?;
            assert!(
                output
                    .install(&inputs, 0, &mut builder, &mut state, &budget)
                    .unwrap_err()
                    .contains("authority")
            );
            compute_standalone(inputs, &budget)
        })
        .unwrap();
    }

    #[test]
    fn callable_signature_constructor_rejects_missing_or_reordered_outputs() {
        let budget = ir::PlanBudget::default();
        assert!(
            construct(&budget, |_| Ok(Vec::new()))
                .unwrap_err()
                .contains("count changed")
        );
        assert!(
            construct(&budget, |inputs| {
                let mut outputs = compute_standalone(inputs, &budget)?;
                outputs.swap(0, 1);
                Ok(outputs)
            })
            .unwrap_err()
            .contains("different inputs")
        );
    }

    #[test]
    fn callable_signature_modules_preserve_slots_and_reject_wrong_module_outputs() {
        let budget = ir::PlanBudget::default();
        let foreign = ir::PlanBudget::default();
        construct(&budget, |inputs| {
            assert!(inputs.module_count() >= 2);
            assert!(inputs.module_input_size(inputs.module_count()).is_err());
            assert!(
                compute_module_signatures(inputs.clone(), 0, &foreign)
                    .unwrap_err()
                    .contains("authority")
            );
            assert!(
                compute_module_signatures(inputs.clone(), inputs.module_count(), &budget).is_err()
            );
            assert!(
                finish_module_signatures(&inputs, Vec::new())
                    .unwrap_err()
                    .contains("count changed")
            );
            let mut modules = (0..inputs.module_count())
                .map(|ordinal| {
                    compute_module_signatures(inputs.clone(), ordinal, &budget).map(Arc::new)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let outputs = finish_module_signatures(&inputs, modules.clone())?;
            assert_eq!(outputs.len(), inputs.callable_count());
            for (index, output) in outputs.iter().enumerate() {
                assert_eq!(output.declaration, inputs.declaration(index)?);
            }
            // A source diagnostic cannot hide a forged successful prefix.
            // Keep its diagnostic index valid so rejection must authenticate
            // the prefix rather than merely validate the error's position.
            let first = &modules[0];
            assert!(first.callables.len() >= 2);
            for forged_prefix in [
                (first.callables[1].0, first.callables[0].1.clone()),
                (first.callables[0].0, first.callables[1].1.clone()),
            ] {
                let mut forged = modules.clone();
                forged[0] = Arc::new(ModuleSignatureOutput {
                    inputs: inputs.clone(),
                    ordinal: 0,
                    callables: vec![forged_prefix],
                    error: Some((first.callables[1].0, "forged source diagnostic".into())),
                });
                assert_eq!(
                    finish_module_signatures(&inputs, forged).unwrap_err(),
                    "signature module callable identity changed"
                );
            }
            modules.swap(0, 1);
            assert!(
                finish_module_signatures(&inputs, modules)
                    .unwrap_err()
                    .contains("different inputs or module")
            );
            Ok(outputs)
        })
        .unwrap();
    }
}
