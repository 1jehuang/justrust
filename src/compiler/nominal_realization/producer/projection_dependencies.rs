//! Closed projection dependencies are discovered before physical child waits.
//! This inventory is candidate metadata, never a successful nominal read.
use super::*;

struct Application {
    key: RealizationKey,
    declaration: ir::DeclId,
    actuals: Arc<[ir::Ty]>,
}

fn append<T>(out: &mut Vec<T>, value: T, budget: &ir::PlanBudget) -> Result<(), Error> {
    budget
        .retain_bytes(std::mem::size_of::<T>())
        .map_err(Error::new)?;
    out.try_reserve_exact(1)
        .map_err(|_| Error::new("closed projection dependency allocation failed"))?;
    out.push(value);
    Ok(())
}

/// Visit semantic coordinates only, never unfold a nominal's fields here.
/// Those fields require the next frontier's canonical typed source read.
fn dependencies(
    ty: &ir::Ty,
    by_value: bool,
    world: u64,
    apps: &mut Vec<Application>,
    edges: &mut Vec<topology::Edge>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), Error> {
    budget.check_depth(depth).map_err(Error::new)?;
    budget.charge_work(1).map_err(Error::new)?;
    let nominal = match ty {
        ir::Ty::Struct(n) => Some((n.def().source_decl, &n.def().type_args)),
        ir::Ty::Enum(n) => Some((n.def().source_decl, &n.def().type_args)),
        _ => None,
    };
    if let Some((declaration, actuals)) = nominal {
        let mut terms = topology::storage(actuals.len(), budget)?;
        for actual in actuals.iter() {
            terms.push(Term::from_ty(actual, budget, depth + 1)?);
        }
        let key = key(world, declaration, &terms, budget)?;
        let mut found = None;
        for (index, prior) in apps.iter().enumerate() {
            budget
                .charge_work(prior.key.0.params.len().max(key.0.params.len()))
                .map_err(Error::new)?;
            if prior.key == key {
                found = Some(index);
                break;
            }
        }
        let target = if let Some(index) = found {
            index
        } else {
            budget.charge_application().map_err(Error::new)?;
            let index = apps.len();
            append(
                apps,
                Application {
                    key,
                    declaration,
                    actuals: actuals.clone(),
                },
                budget,
            )?;
            index
        };
        append(edges, topology::Edge { target, by_value }, budget)?;
        for actual in actuals.iter() {
            dependencies(actual, false, world, apps, edges, budget, depth + 1)?;
        }
        return Ok(());
    }
    match ty {
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                dependencies(field, by_value, world, apps, edges, budget, depth + 1)?;
            }
        }
        ir::Ty::Ref { referent: ty, .. }
        | ir::Ty::RawPtr { pointee: ty, .. }
        | ir::Ty::Slice { element: ty } => {
            dependencies(ty, false, world, apps, edges, budget, depth + 1)?
        }
        ir::Ty::Array { element, .. } => {
            dependencies(element, by_value, world, apps, edges, budget, depth + 1)?
        }
        ir::Ty::Cell(marker) => dependencies(
            marker.argument(),
            by_value,
            world,
            apps,
            edges,
            budget,
            depth + 1,
        )?,
        ir::Ty::ManuallyDrop(marker) => dependencies(
            marker.argument(),
            by_value,
            world,
            apps,
            edges,
            budget,
            depth + 1,
        )?,
        ir::Ty::PhantomData(marker) => dependencies(
            marker.argument(),
            false,
            world,
            apps,
            edges,
            budget,
            depth + 1,
        )?,
        ir::Ty::FnPointer(pointer) => {
            for ty in pointer
                .inputs
                .iter()
                .chain(std::iter::once(&pointer.output))
            {
                dependencies(ty, false, world, apps, edges, budget, depth + 1)?;
            }
        }
        ir::Ty::Closure(closure) => {
            // Captures are genuine parent-published metadata. No closure recipe
            // or source-template body can be synthesized by this inventory.
            for ty in closure
                .owner_args()
                .iter()
                .chain(closure.params())
                .chain(std::iter::once(closure.result()))
            {
                dependencies(ty, false, world, apps, edges, budget, depth + 1)?;
            }
            for ty in closure.captures() {
                dependencies(ty, by_value, world, apps, edges, budget, depth + 1)?;
            }
        }
        ir::Ty::AssociatedType(_) | ir::Ty::Param(_) => {
            return Err(Error::new(
                "unsupported: unresolved closed projection dependency coordinate",
            ));
        }
        ir::Ty::Opaque(_) | ir::Ty::FnItem(_) | ir::Ty::Dyn(_) => {
            return Err(Error::new(
                "unsupported: projected nominal dependency needs explicit semantic metadata",
            ));
        }
        _ => {}
    }
    Ok(())
}

pub(super) async fn validate(
    cx: &Context,
    root: RealizationKey,
    actuals: Arc<[ir::Ty]>,
    world: &traits::ValidatedTraitWorld,
) -> Result<(), Error> {
    let budget = world.budget();
    let (identity, declaration, _) = decode(&root.0, budget)?;
    if identity != world.nominal_realization_world() {
        return Err(Error::new("closed projection dependency changed world"));
    }
    let mut terms = topology::storage(actuals.len(), budget)?;
    for actual in actuals.iter() {
        terms.push(Term::from_ty(actual, budget, 0)?);
    }
    if key(identity, declaration, &terms, budget)? != root {
        return Err(Error::new(
            "closed projection dependency changed ordered actuals",
        ));
    }
    let mut apps = topology::storage(1, budget)?;
    apps.push(Application {
        key: root,
        declaration,
        actuals,
    });
    let mut topology = Vec::new();
    let mut cursor = 0;
    while cursor < apps.len() {
        let end = apps.len();
        let mut declarations = topology::storage(end - cursor, budget)?;
        for app in &apps[cursor..end] {
            declarations.push(app.declaration);
        }
        // read_sources requests all independent schemas before awaiting any.
        let sources = read_sources(cx, &declarations).await?;
        for (index, source) in (cursor..end).zip(sources) {
            let input = world
                .close_nominal_input(
                    apps[index].declaration,
                    source.proof(),
                    &apps[index].actuals,
                )
                .map_err(|e| Error::new(e.into_legacy_diagnostic(budget)))?;
            let mut edges = Vec::new();
            let actuals = apps[index].actuals.clone();
            for actual in actuals.iter() {
                dependencies(actual, false, identity, &mut apps, &mut edges, budget, 0)?;
            }
            for (_, source_field) in source_fields(source.template(), budget)? {
                let closed = world
                    .close_nominal_dependency_field(&input, &source_field)
                    .map_err(|e| Error::new(e.into_legacy_diagnostic(budget)))?;
                dependencies(&closed, true, identity, &mut apps, &mut edges, budget, 0)?;
            }
            let words = super::super::component_key::application_words(&apps[index].key, budget)?;
            append(
                &mut topology,
                topology::Application {
                    key: topology::ApplicationWords(words),
                    edges,
                },
                budget,
            )?;
        }
        cursor = end;
    }
    let components = topology::components(&topology, budget)?;
    for component in components {
        if component.members.len() != 1
            || topology[component.members[0]]
                .edges
                .iter()
                .any(|edge| edge.target == component.members[0])
        {
            return Err(Error::new(
                "unsupported: recursive associated nominal dependencies require component projection evidence",
            ));
        }
    }
    Ok(())
}
