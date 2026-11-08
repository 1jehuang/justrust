//! Closure coordinates are never closure metadata. Only the successful keyed
//! template read can supply owner/site, captures, signature and mode evidence.
use super::*;

pub(in crate::compiler::nominal_realization) fn replay_template(
    types: &crate::compiler::frontend::PlanCtx,
    provider: ir::DeclId,
    template: &Arc<ir::ClosureTy>,
    arguments: &[ir::Ty],
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, Error> {
    budget.check_depth(0).map_err(Error::new)?;
    if template.provider() != provider
        || types.stored_closure_owner(provider).map_err(Error::new)?
            != Some((template.owner(), template.site()))
    {
        return Err(Error::new(
            "nominal closure coordinate changed its canonical source owner",
        ));
    }
    budget
        .retain_bytes(
            arguments
                .len()
                .saturating_mul(std::mem::size_of::<ir::Ty>()),
        )
        .map_err(Error::new)?;
    let owner = types
        .instance_facts(&ir::CallTarget {
            declaration: template.owner(),
            type_args: Arc::from(arguments),
        })
        .map_err(Error::new)?;
    owner
        .instantiate_closure_owner(template, arguments)
        .map(ir::Ty::Closure)
        .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))
}

enum Input {
    Template(Arc<crate::compiler::frontend::PlanCtx>, Arc<ir::ClosureTy>),
    Actual(ir::Ty),
}

pub(super) fn resolve_actual<'a>(
    cx: &'a Context,
    provider: ir::DeclId,
    arguments: Vec<Term>,
    world: u64,
    catalog: &'a traits::CoreCatalog,
    budget: &'a ir::PlanBudget,
    depth: usize,
) -> Pending<'a, ir::Ty> {
    Box::pin(async move {
        budget.check_depth(depth).map_err(Error::new)?;
        let count = arguments
            .len()
            .checked_add(1)
            .ok_or_else(|| Error::new("closure actual prerequisite count overflow"))?;
        let mut pending: Vec<Pending<'a, Input>> = topology::storage(count, budget)?;
        pending.push(Box::pin(async move {
            let types = crate::compiler::Facts { cx }.types().await?;
            if types
                .trait_world()
                .map_err(Error::new)?
                .nominal_realization_world()
                != world
            {
                return Err(Error::new(
                    "closure nominal coordinate changed its graph world",
                ));
            }
            let index = types.callable_index(provider).map_err(Error::new)?;
            let handle = cx
                .request(crate::compiler::keys::item(
                    crate::compiler::TEMPLATE_BODY,
                    index,
                ))
                .get(0);
            let template: Arc<crate::compiler::TemplateBodyFact> = cx.read_fact(handle).await?;
            let checked = template.closure.as_ref().ok_or_else(|| {
                Error::new("nominal closure coordinate named a nonclosure provider")
            })?;
            if checked.ty.provider() != provider {
                return Err(Error::new(
                    "nominal closure template changed its canonical provider",
                ));
            }
            // This is extracted only from the successful canonical typed read.
            // The join needs the genuine type, not the whole recursive body and
            // publication proof. Dropping that payload here gives its Send
            // proof a finite boundary without erasing or inventing authority.
            Ok(Input::Template(types, checked.ty.clone()))
        }));
        for argument in arguments {
            pending.push(Box::pin(async move {
                resolve(cx, argument, world, catalog, budget, depth + 1)
                    .await
                    .map(Input::Actual)
            }));
        }
        // Every owner argument and the provider template are requested before
        // awaiting any. No worker blocks and no closure body is reconstructed.
        let mut values = all(pending).await?.into_iter();
        let Some(Input::Template(types, template)) = values.next() else {
            return Err(Error::new(
                "closure actual omitted its typed template prerequisite",
            ));
        };
        let mut actuals = topology::storage(count - 1, budget)?;
        for value in values {
            let Input::Actual(ty) = value else {
                return Err(Error::new("closure actual prerequisite ordering changed"));
            };
            actuals.push(ty);
        }
        replay_template(&types, provider, &template, &actuals, budget)
    })
}
