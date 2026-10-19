//! Encountered opaque source objects. All physical authority is borrowed within
//! this producer and only immutable machine-object bytes leave it.
use super::*;

pub(super) async fn produce(
    provider: &CompilerProvider,
    cx: &Context,
    index: usize,
) -> Result<Option<backend::Object>, Error> {
    let facts = Facts { cx };
    let types = facts.types().await?;
    let (target, source_index) = facts.instance(index).await?;
    let template = facts.template(source_index).await?;
    let source = provider.validated_body(&types, source_index, &template)?;
    let mut encountered = false;
    for ty in source
        .locals
        .iter()
        .map(|local| &local.ty)
        .chain(source.exprs.iter().map(|expression| &expression.ty))
        .chain(std::iter::once(&source.result))
    {
        if opaque_instances::contains(ty, &provider.0.budget)? {
            encountered = true;
            break;
        }
    }
    if !encountered {
        return Ok(None);
    }
    if !target.type_args.is_empty() || template.closure.is_some() {
        return Err(Error::new(
            "unsupported: physical opaque object requires nongeneric original source owner",
        ));
    }
    let publication = template
        .publication
        .as_ref()
        .ok_or_else(|| Error::new("physical opaque object omitted original publication"))?;
    if publication.facts().declaration() != target.declaration {
        return Err(Error::new(
            "physical opaque object changed exact source owner",
        ));
    }
    let admission = cx
        .request(Facts::item(SOURCE_ADMISSION, source_index))
        .get(0);
    let count = source
        .exprs
        .iter()
        .filter(|expression| {
            matches!(
                &expression.kind, ir::ExprKind::Call { target: ir::Callee::TraitItem(call), .. }
                    if matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_))
            )
        })
        .count();
    let (mut requests, _requests_storage) = provider.scratch_vec(count)?;
    for (index, expression) in source.exprs.iter().enumerate() {
        provider.0.budget.charge_work(1).map_err(Error::new)?;
        if let ir::ExprKind::Call {
            target: ir::Callee::TraitItem(call),
            ..
        } = &expression.kind
            && matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_))
        {
            let site = ir::ExprId(index);
            let request = cx.request(opaque_dispatch::key(
                &target,
                site,
                call,
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
    roots.extend(source.locals.iter().map(|local| local.ty.clone()));
    roots.extend(source.exprs.iter().map(|expression| expression.ty.clone()));
    roots.push(source.result.clone());
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
        if !authorization.target().type_args.is_empty() {
            return Err(Error::new(
                "unsupported: physical generic callee needs canonical object producer",
            ));
        }
        let callable = types
            .callable_index(authorization.target().declaration)
            .map_err(Error::new)?;
        let slot = types
            .closed_roots()
            .iter()
            .position(|candidate| *candidate == callable)
            .ok_or_else(|| {
                Error::new("unsupported: physical callee has no declaration-owned object root")
            })?;
        // Exact target replay precedes this demand. Object producers do not wait
        // for one another, so recursive calls cannot create object/body joins.
        // BUILD waits on every declared root object before linking any symbol.
        cx.request(Facts::item(OBJECT, slot));
        calls.push(authorization);
    }
    let original = cfg::lower_source_template(publication, &provider.0.budget)
        .map_err(|error| Error::new(error.to_string()))?;
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
    physical
        .seal(
            publication,
            original.function(),
            &representations,
            &types,
            &closed,
            &provider.0.budget,
        )
        .map_err(Error::new)?;
    // Cleanup of hidden owning representations needs its own exact dependency
    // protocol. The first slice proves trivial cleanup rather than silently
    // omitting a destructor from the sealed executable frontier.
    if closed.has_normal_drop_impls() {
        for local in &physical.function().locals {
            provider.0.budget.charge_work(1).map_err(Error::new)?;
            match closed
                .normal_drop_plan(&local.ty)
                .map_err(|error| source_fact_error(error, &provider.0.budget))?
            {
                traits::NormalDropPlan::Trivial => {}
                _ => {
                    return Err(Error::new(
                        "unsupported: physical opaque cleanup requires exact realized destructor dependencies",
                    ));
                }
            }
        }
    }
    let validation = physical.validation(
        publication,
        original.function(),
        &representations,
        &types,
        &provider.0.budget,
    );
    scale_task_work_by_cfg(physical.function());
    let concrete =
        ownership::analyze_physical(Arc::new(physical.function().clone()), closed, &validation)
            .map_err(|analysis| {
                Error::new(analysis_diagnostic(
                    &provider.0.budget,
                    "physical opaque instance",
                    index,
                    &analysis,
                ))
            })?;
    let object = backend::emit_physical_cfg_function(&concrete, &|target| {
        instances::symbol_of(target, &types, &provider.0.budget)
    })
    .map_err(Error::new)?;
    Ok(Some(object))
}
