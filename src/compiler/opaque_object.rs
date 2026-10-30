//! Encountered opaque source objects. All physical authority is borrowed within
//! this producer and only immutable machine-object bytes leave it.
use super::*;

pub(super) async fn produce(
    provider: &CompilerProvider,
    cx: &Context,
    key: &Key,
) -> Result<Option<backend::Object>, Error> {
    let facts = Facts { cx };
    let types = facts.types().await?;
    let (target, source_index) = facts.instance_of(key, &provider.0.budget).await?;
    let template = facts.template(source_index).await?;
    let source = provider.validated_body(&types, source_index, &template)?;
    let mut candidate = false;
    for ty in source
        .locals
        .iter()
        .map(|local| &local.ty)
        .chain(source.exprs.iter().map(|expression| &expression.ty))
        .chain(std::iter::once(&source.result))
        .chain(target.type_args.iter())
    {
        if opaque_instances::contains(ty, &provider.0.budget)? {
            candidate = true;
            break;
        }
    }
    if !candidate {
        return Ok(None);
    }
    let publication = template.publication.as_ref().ok_or_else(|| {
        Error::new(format!(
            "physical opaque object {} (source {source_index}) omitted original publication",
            source.name
        ))
    })?;
    let mut specialization = opaque_specialization::Specialization::new(
        publication,
        &target,
        &types,
        &provider.0.budget,
        cx.authority(),
    )?;
    let mut encountered = false;
    for ty in source
        .locals
        .iter()
        .map(|local| &local.ty)
        .chain(source.exprs.iter().map(|expression| &expression.ty))
        .chain(std::iter::once(&source.result))
    {
        if opaque_instances::contains(&specialization.ty(ty)?, &provider.0.budget)? {
            encountered = true;
            break;
        }
    }
    if !encountered {
        return Ok(None);
    }
    if template.closure.is_some() {
        return Err(Error::new(
            "unsupported: physical opaque closure owner requires parent specialization replay",
        ));
    }
    if publication.facts().declaration() != target.declaration {
        return Err(Error::new(
            "physical opaque object changed exact source owner",
        ));
    }
    let admission = cx
        .request(crate::compiler::keys::item(SOURCE_ADMISSION, source_index))
        .get(0);
    let count = source
        .exprs
        .iter()
        .filter(|expression| {
            matches!(
                &expression.kind,
                ir::ExprKind::Call {
                    target: ir::Callee::TraitItem(_),
                    ..
                }
            )
        })
        .count();
    let (mut requests, _requests_storage) = provider.scratch_vec(count)?;
    for (index, expression) in source.exprs.iter().enumerate() {
        provider.0.budget.charge_work(1).map_err(Error::new)?;
        if let ir::ExprKind::Call {
            target: ir::Callee::TraitItem(_),
            ..
        } = &expression.kind
        {
            let site = ir::ExprId(index);
            let (callee, _) = specialization.call(site)?;
            let ir::Callee::TraitItem(specialized) = callee else {
                continue;
            };
            if opaque_dispatch::opaque_receiver(&specialized.trait_ref.self_ty, &provider.0.budget)?
                .is_none()
            {
                continue;
            }
            let request = cx.request(opaque_dispatch::key(
                &target,
                site,
                &specialized,
                &provider.0.budget,
            )?);
            requests.push((site, request.get(0), request.get(1)));
        }
    }
    let root_count = source
        .locals
        .len()
        .checked_add(source.exprs.len())
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| Error::new("physical opaque type frontier overflow"))?;
    let (mut roots, _roots_storage) = provider.scratch_vec(root_count)?;
    for ty in source
        .locals
        .iter()
        .map(|local| &local.ty)
        .chain(source.exprs.iter().map(|expression| &expression.ty))
        .chain(std::iter::once(&source.result))
    {
        roots.push(specialization.ty(ty)?);
    }
    let original = cfg::lower_source_template(publication, &provider.0.budget)
        .map_err(|error| Error::new(error.to_string()))?;
    // Unchanged owning locals retain their original cleanup. Demand the same
    // owner-local frontier used by ordinary objects before any independent
    // representation read suspends this producer.
    let cleanup = if has_drop(original.function()) {
        // Opaque roots are published universally even when they have no type
        // arguments. Their canonical INSTANCE frontier, not the closed-source
        // root-only publication protocol, owns the destructor dependencies.
        Some(
            cx.request(keys::words(INSTANCE, key.params.clone())?)
                .get(0),
        )
    } else {
        None
    };
    // All independent source and call-authority producers are requested before
    // representation reads suspend this branch. No global hidden-type inventory.
    let representations = opaque_instances::read(&facts, &roots, &provider.0.budget).await?;
    cx.read(admission).await?;
    let (mut calls, _calls_storage) = provider.scratch_vec(count)?;
    for (site, authorization, ready) in requests {
        let authorization: Arc<opaque_dispatch::OpaqueCallAuthorization> =
            cx.read_fact(authorization).await?;
        authorization.replay(
            publication,
            &target,
            site,
            &representations,
            &types,
            &provider.0.budget,
        )?;
        let ready: Arc<opaque_dispatch::PhysicalCalleeReady> = cx.read_fact(ready).await?;
        if ready.target() != authorization.target() {
            return Err(Error::new(
                "physical opaque callee readiness changed authorized target",
            ));
        }
        let params = instances::graph_key::encode(authorization.target(), &provider.0.budget)
            .map_err(Error::new)?;
        // The INSTANCE frontier reads the same authorization and retains this
        // exact target for linking. Only demand the object here, never await it.
        cx.request(crate::compiler::keys::words(OBJECT, params)?);
        calls.push(authorization);
    }
    let closed = types.instance_facts(&target).map_err(Error::new)?;
    let mut physical = opaque_physical::normalize(
        publication,
        &target,
        original.function(),
        &representations,
        &calls,
        &types,
        &provider.0.budget,
    )?;
    let validation = physical
        .seal(
            publication,
            original.function(),
            &representations,
            &types,
            &closed,
            &provider.0.budget,
        )
        .map_err(Error::new)?;
    // Only unchanged source owning types may reuse their original cleanup.
    // A hidden representation or generic substitution that introduces Drop
    // still requires a separate exact cleanup-elaboration protocol.
    if closed.has_normal_drop_impls() {
        for (index, local) in physical.function().locals.iter().enumerate() {
            provider.0.budget.charge_work(1).map_err(Error::new)?;
            match closed
                .normal_drop_plan(&local.ty)
                .map_err(|error| source_fact_error(error, &provider.0.budget))?
            {
                traits::NormalDropPlan::Trivial => {}
                traits::NormalDropPlan::Destructor(_) => {
                    let source = original.function().locals.get(index).ok_or_else(|| {
                        Error::new("physical opaque cleanup has no original owning local")
                    })?;
                    if !ir::identical_type(&source.ty, &local.ty, &provider.0.budget)
                        .map_err(Error::new)?
                    {
                        return Err(Error::new(
                            "unsupported: physical opaque cleanup requires exact realized destructor dependencies",
                        ));
                    }
                }
                _ => {
                    return Err(Error::new(
                        "unsupported: physical opaque cleanup requires exact realized destructor dependencies",
                    ));
                }
            }
        }
    }
    if let Some(handle) = cleanup {
        let demand: Arc<instances::DirectDemands> = cx.read_fact(handle).await?;
        demand
            .validate_cleanup(physical.function(), &provider.0.budget)
            .map_err(Error::new)?;
    }
    scale_task_work_by_cfg(physical.function());
    let concrete =
        ownership::analyze_physical(Arc::new(physical.function().clone()), closed, &validation)
            .map_err(|analysis| {
                Error::new(analysis_diagnostic(
                    &provider.0.budget,
                    "physical opaque instance",
                    instance_identity(key, source_index, &physical.function().name),
                    &analysis,
                ))
            })?;
    let object = backend::emit_physical_cfg_function_with_profile(
        &concrete,
        &|target| instances::symbol_of(target, &types, &provider.0.budget),
        provider.0.source_options.overflow_checks,
        provider.0.source_options.panic_unwind,
    )
    .map_err(Error::new)?;
    Ok(Some(object))
}
