use super::*;

/// A source cycle selects the symbolic protocol, but never defines its
/// identity. Only exact closed application discovery determines SCC members.
fn source_cycle<'a>(
    cx: &'a Context,
    source: ReadSource,
    mut path: Vec<ir::DeclId>,
    b: &'a ir::PlanBudget,
) -> Pending<'a, bool> {
    Box::pin(async move {
        b.check_depth(path.len()).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        if path.contains(&source.declaration()) {
            return Ok(true);
        }
        b.retain_bytes(std::mem::size_of::<ir::DeclId>())
            .map_err(Error::new)?;
        path.try_reserve_exact(1)
            .map_err(|_| Error::new("source path allocation failed"))?;
        path.push(source.declaration());
        let mut needed = Vec::new();
        for (_, field) in source_fields(source.template(), b)? {
            declarations(&field, &mut needed, b, 0)?;
        }
        let sources = read_sources(cx, &needed).await?;
        let mut branches = topology::storage(sources.len(), b)?;
        for source in sources {
            let mut child_path = topology::storage(path.len(), b)?;
            child_path.extend_from_slice(&path);
            branches.push(source_cycle(cx, source, child_path, b));
        }
        Ok(all(branches).await?.into_iter().any(|cycle| cycle))
    })
}

/// Publish an immutable member alias only after the actual successful typed
/// component read. No executable field layout is required by this alias node.
pub(super) async fn publish_component_alias(
    cx: &Context,
    key: RealizationKey,
    world: &traits::ValidatedTraitWorld,
) -> Result<(), Error> {
    let b = world.budget();
    let discovery =
        symbolic::discover(cx, component_key::copy_application(&key, b)?, b, world).await?;
    let group = discovery
        .components
        .iter()
        .find(|group| group.members.contains(&0))
        .ok_or_else(|| Error::new("closed application discovery omitted root"))?;
    let mut members = topology::storage(group.members.len(), b)?;
    for &index in &group.members {
        members.push(component_key::copy_application(
            &discovery.members[index].key,
            b,
        )?);
    }
    let component_key =
        component_key::ComponentKey::encode(world.nominal_realization_world(), &members, b)?;
    let read = component::read(cx, &component_key, world).await?;
    let member = read.member_for(&key)?;
    cx.publish_fact(
        0,
        NominalPublication::Component {
            key,
            identity: cx.output_identity(0)?,
            read,
            member,
        },
    )
}

/// Routing inspects syntax only. Unsupported component equations are rejected
/// by the component producer, not while deciding whether acyclic input needs it.
fn probe_declarations(
    term: &Term,
    out: &mut Vec<ir::DeclId>,
    b: &ir::PlanBudget,
    depth: usize,
) -> Result<(), Error> {
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    match term {
        Term::Nominal(declaration, actuals) => {
            b.charge_work(out.len()).map_err(Error::new)?;
            if !out.contains(declaration) {
                b.retain_bytes(std::mem::size_of::<ir::DeclId>())
                    .map_err(Error::new)?;
                out.try_reserve_exact(1)
                    .map_err(|_| Error::new("routing declaration allocation failed"))?;
                out.push(*declaration);
            }
            for actual in actuals {
                probe_declarations(actual, out, b, depth + 1)?;
            }
        }
        Term::Dyn(_, actuals, assigned) => {
            for actual in actuals {
                probe_declarations(actual, out, b, depth + 1)?;
            }
            for (_, actual) in assigned {
                probe_declarations(actual, out, b, depth + 1)?;
            }
        }
        Term::Ref(_, child)
        | Term::RawPtr(_, child)
        | Term::Array(child, _)
        | Term::Slice(child) => {
            probe_declarations(child, out, b, depth + 1)?;
        }
        Term::Tuple(children) => {
            for child in children {
                probe_declarations(child, out, b, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) async fn needs_symbolic(
    cx: &Context,
    source: ReadSource,
    terms: &[Term],
    b: &ir::PlanBudget,
) -> Result<bool, Error> {
    let mut declarations = Vec::new();
    for term in terms {
        probe_declarations(term, &mut declarations, b, 0)?;
    }
    let sources = read_sources(cx, &declarations).await?;
    let mut branches = topology::storage(sources.len().saturating_add(1), b)?;
    branches.push(source_cycle(cx, source, Vec::new(), b));
    for source in sources {
        branches.push(source_cycle(cx, source, Vec::new(), b));
    }
    Ok(all(branches).await?.into_iter().any(|cycle| cycle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acyclic_dynamic_actual_probe_retains_nested_nominal_frontier() {
        let snapshot = ir::SnapshotId::from_graph_word(19);
        let principal = ir::DeclId { snapshot, index: 3 };
        let owner = ir::DeclId { snapshot, index: 7 };
        let associated = ir::DeclId { snapshot, index: 8 };
        let term = Term::Dyn(
            principal,
            vec![Term::Bool],
            vec![(associated, Term::Nominal(owner, vec![]))],
        );
        let budget = ir::PlanBudget::default();
        let mut declarations = Vec::new();
        probe_declarations(&term, &mut declarations, &budget, 0).unwrap();
        assert_eq!(declarations, [owner]);
        // Genuinely selected recursive components still reject equations not
        // supported by their proof language. Routing does not apply that ban.
        assert!(symbolic::dependencies(&term, false, &mut Vec::new(), &budget, 0).is_err());
    }
}
