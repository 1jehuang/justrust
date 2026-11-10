//! Exact impl-head lifetime applications, independent of provided method bodies.
use super::*;

pub(super) fn application(
    plan: &PlanCtx,
    declaration: ir::DeclId,
    expected_self: &ir::Ty,
) -> Result<Option<Arc<tr::OwnerRegionApplication>>, String> {
    let info = plan.registry.declaration(declaration)?;
    if info.kind != ir::DeclKind::TraitImpl
        || plan.registry.impl_ids.get(info.source_index) != Some(&declaration)
    {
        return Err("owner lifetime application requires its original impl declaration".into());
    }
    let block = plan
        .program
        .impls
        .get(info.source_index)
        .ok_or("missing original lifetime application impl")?;
    let path = block
        .trait_path
        .as_ref()
        .ok_or("missing owner trait application")?;
    let trait_decl = plan.registry.resolve_trait_path(path)?;
    let count = if plan.catalog.declaration(trait_decl).is_some() {
        0
    } else {
        plan.registry.source_trait(trait_decl)?.regions.params.len()
    };
    let written = match path {
        TraitPath::Source(path) => {
            if path
                .segments
                .iter()
                .take(path.segments.len().saturating_sub(1))
                .any(|segment| !segment.region_args.is_empty())
            {
                return Err("unsupported: lifetime arguments on a nonfinal trait segment".into());
            }
            path.segments
                .last()
                .map(|segment| segment.region_args.as_slice())
                .unwrap_or(&[])
        }
        TraitPath::Builtin(_) => &[],
    };
    if count > ir::MAX_REGION_PARAMS || written.len() != count {
        return Err(
            "unsupported: trait lifetime application requires exact ordered lifetime arguments"
                .into(),
        );
    }
    let budget = &plan.budget;
    let mut actuals = Vec::new();
    for region in written {
        budget.charge_work(block.regions.params.len() + 1)?;
        budget.retain_bytes(std::mem::size_of::<ir::CarrierRegionTerm>())?;
        actuals.push(match region {
            RegionName::Static => ir::CarrierRegionTerm::Static,
            RegionName::Named(name) => {
                let key = plan
                    .registry
                    .names
                    .key(*name, LookupClass::LifetimeParameter)
                    .map_err(native_adapter::name_diagnostic)?;
                let mut found = None;
                for (index, binder) in block.regions.params.iter().enumerate() {
                    if plan
                        .registry
                        .names
                        .key(*binder, LookupClass::LifetimeParameter)
                        .map_err(native_adapter::name_diagnostic)?
                        == key
                    {
                        found = Some(index);
                        break;
                    }
                }
                ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                    declaration,
                    index: u16::try_from(found.ok_or("undeclared impl lifetime actual")?)
                        .map_err(|_| "impl lifetime ordinal overflow")?,
                })
            }
            RegionName::Infer => {
                return Err("unsupported: inferred owner lifetime application".into());
            }
        });
    }
    // Re-read the exact original self type. The caller's closed type is not an
    // authority for source-written lifetimes, even when its runtime shape fits.
    let self_ty = impl_storage::source_header(plan, info.source_index, budget)?;
    if &self_ty != expected_self {
        return Err("owner lifetime application changed original impl Self".into());
    }
    let params = [(BindingName::Wildcard, block.self_ty.clone())];
    let signature = ir::Signature {
        ref_contract: canonical_ref_contract(
            (plan.program.names.as_ref(), budget),
            &block.regions,
            &params,
            &Type::Unit,
            (ReceiverKind::None, tr::RegionContractMode::Positioned),
            std::slice::from_ref(&self_ty),
            &ir::Ty::Unit,
        )?,
        params: vec![self_ty],
        result: ir::Ty::Unit,
    };
    let regions = carrier_regions::attach(
        &plan.registry,
        &block.regions,
        &params,
        &Type::Unit,
        Some(block),
        &signature,
        ReceiverKind::None,
        original_source_regions(
            (plan.program.names.as_ref(), budget),
            declaration,
            &block.regions,
            &params,
            &Type::Unit,
            &signature.ref_contract,
            ReceiverKind::None,
        )?,
        budget,
    )?;
    budget.retain_bytes(std::mem::size_of::<tr::OwnerRegionApplication>())?;
    Ok(Some(Arc::new(tr::OwnerRegionApplication {
        implementation: declaration,
        trait_decl,
        binder_count: u16::try_from(block.regions.params.len())
            .map_err(|_| "impl lifetime binder count overflow")?,
        actuals: actuals.into(),
        self_signature: signature,
        self_regions: regions,
    })))
}
