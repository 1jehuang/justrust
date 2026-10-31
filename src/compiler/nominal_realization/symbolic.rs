//! Candidate-only symbolic closure. Never creates executable types or receipts.
use super::{
    Context, Error, FieldCoordinate, ReadSource, RealizationKey, coordinates::Term, ir, producer,
    read_sources, topology,
};

#[derive(Clone)]
pub(super) struct CandidateMember {
    pub(super) key: RealizationKey,
    pub(super) source: ReadSource,
    pub(super) actuals: Vec<Term>,
    pub(super) fields: Vec<(FieldCoordinate, Term)>,
}

pub(super) struct Discovery {
    pub(super) members: Vec<CandidateMember>,
    pub(super) components: Vec<topology::Component>,
}

fn copy(term: &Term, b: &ir::PlanBudget) -> Result<Term, Error> {
    Term::decode(&term.encode(b)?, b)
}

pub(super) fn close_term(
    ty: &ir::Ty,
    env: &ir::GenericEnv,
    actuals: &[Term],
    b: &ir::PlanBudget,
    depth: usize,
) -> Result<Term, Error> {
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    let child = |ty: &ir::Ty| close_term(ty, env, actuals, b, depth + 1);
    let children = |types: &[ir::Ty]| -> Result<Vec<Term>, Error> {
        let mut out = topology::storage(types.len(), b)?;
        for ty in types {
            out.push(child(ty)?);
        }
        Ok(out)
    };
    Ok(match ty {
        ir::Ty::Param(p) => {
            let index = env
                .ordered_params()
                .position(|bound| bound == *p)
                .ok_or_else(|| Error::new("symbolic parameter outside exact nominal binder"))?;
            copy(
                actuals
                    .get(index)
                    .ok_or_else(|| Error::new("symbolic nominal actual arity changed"))?,
                b,
            )?
        }
        ir::Ty::Struct(n) => Term::Nominal(n.def().source_decl, children(&n.def().type_args)?),
        ir::Ty::Enum(n) => Term::Nominal(n.def().source_decl, children(&n.def().type_args)?),
        ir::Ty::Ref { kind, referent } => Term::Ref(*kind, Box::new(child(referent)?)),
        ir::Ty::RawPtr { kind, pointee } => Term::RawPtr(*kind, Box::new(child(pointee)?)),
        ir::Ty::Array { element, len } => Term::Array(Box::new(child(element)?), *len),
        ir::Ty::Slice { element } => Term::Slice(Box::new(child(element)?)),
        ir::Ty::PhantomData(marker) => {
            b.retain_bytes(std::mem::size_of::<Term>())
                .map_err(Error::new)?;
            Term::PhantomData(marker.declaration(), Box::new(child(marker.argument())?))
        }
        ir::Ty::FnPointer(pointer) => {
            b.retain_bytes(std::mem::size_of::<Term>())
                .map_err(Error::new)?;
            Term::FnPointer(
                children(&pointer.inputs)?,
                Box::new(child(&pointer.output)?),
            )
        }
        ir::Ty::Tuple(types) => Term::Tuple(children(types)?),
        ir::Ty::AssociatedType(_) => {
            return Err(Error::new(
                "unsupported: recursive symbolic associated projection evidence",
            ));
        }
        other => Term::from_ty(other, b, depth + 1)?,
    })
}

type Dependency = (ir::DeclId, Vec<Term>, bool);

/// Replay consumers only need an exact key while this source term is alive.
/// Borrow its immutable actuals instead of retaining an owned term inventory.
/// Every occurrence is visited, including duplicate and non-owning edges.
pub(super) fn visit_dependencies(
    term: &Term,
    by_value: bool,
    b: &ir::PlanBudget,
    depth: usize,
    visit: &mut impl FnMut(ir::DeclId, &[Term], bool) -> Result<(), Error>,
) -> Result<(), Error> {
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    match term {
        Term::Nominal(declaration, actuals) => {
            visit(*declaration, actuals, by_value)?;
            for actual in actuals {
                visit_dependencies(actual, false, b, depth + 1, visit)?;
            }
        }
        Term::Ref(_, child) | Term::RawPtr(_, child) | Term::Slice(child) => {
            visit_dependencies(child, false, b, depth + 1, visit)?;
        }
        Term::PhantomData(_, child) => visit_dependencies(child, false, b, depth + 1, visit)?,
        Term::Array(child, _) => visit_dependencies(child, by_value, b, depth + 1, visit)?,
        Term::FnPointer(inputs, output) => {
            for child in inputs.iter().chain(std::iter::once(output.as_ref())) {
                visit_dependencies(child, false, b, depth + 1, visit)?;
            }
        }
        Term::Tuple(children) => {
            for child in children {
                visit_dependencies(child, by_value, b, depth + 1, visit)?;
            }
        }
        Term::Dyn(..) => {
            return Err(Error::new(
                "unsupported: recursive symbolic dynamic metadata equations",
            ));
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn dependencies(
    term: &Term,
    by_value: bool,
    out: &mut Vec<Dependency>,
    b: &ir::PlanBudget,
    depth: usize,
) -> Result<(), Error> {
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    match term {
        Term::Nominal(declaration, actuals) => {
            let mut args = topology::storage(actuals.len(), b)?;
            for actual in actuals {
                args.push(copy(actual, b)?);
            }
            b.retain_bytes(std::mem::size_of::<Dependency>())
                .map_err(Error::new)?;
            out.try_reserve_exact(1)
                .map_err(|_| Error::new("symbolic dependency allocation failed"))?;
            out.push((*declaration, args, by_value));
            for actual in actuals {
                dependencies(actual, false, out, b, depth + 1)?;
            }
        }
        Term::Ref(_, child) | Term::RawPtr(_, child) | Term::Slice(child) => {
            dependencies(child, false, out, b, depth + 1)?
        }
        Term::PhantomData(_, child) => dependencies(child, false, out, b, depth + 1)?,
        Term::Array(child, _) => dependencies(child, by_value, out, b, depth + 1)?,
        Term::FnPointer(inputs, output) => {
            for child in inputs.iter().chain(std::iter::once(output.as_ref())) {
                dependencies(child, false, out, b, depth + 1)?;
            }
        }
        Term::Tuple(children) => {
            for child in children {
                dependencies(child, by_value, out, b, depth + 1)?;
            }
        }
        Term::Dyn(..) => {
            return Err(Error::new(
                "unsupported: recursive symbolic dynamic metadata equations",
            ));
        }
        _ => {}
    }
    Ok(())
}

/// Discover only reachable exact applications. Each frontier batches all schema
/// requests before awaiting any, and every loop/allocation is locally charged.
/// Declaration recursion alone is not a cycle: actuals are part of every key.
pub(super) async fn discover(
    cx: &Context,
    root: RealizationKey,
    b: &ir::PlanBudget,
    validated: &super::traits::ValidatedTraitWorld,
) -> Result<Discovery, Error> {
    let (world, _, _) = producer::decode(&root.0, b)?;
    b.charge_application().map_err(Error::new)?;
    let mut keys = topology::storage(1, b)?;
    keys.push(root);
    let mut members = Vec::new();
    let mut applications = Vec::new();
    let mut cursor = 0usize;
    while cursor < keys.len() {
        let end = keys.len();
        let mut decoded = topology::storage(end - cursor, b)?;
        let mut declarations = topology::storage(end - cursor, b)?;
        for key in &keys[cursor..end] {
            let (member_world, declaration, actuals) = producer::decode(&key.0, b)?;
            if member_world != world {
                return Err(Error::new("foreign symbolic application world"));
            }
            declarations.push(declaration);
            decoded.push(actuals);
        }
        let sources = read_sources(cx, &declarations).await?;
        for ((source, actuals), key_index) in sources.into_iter().zip(decoded).zip(cursor..end) {
            let env = &source.proof().schema().env;
            if actuals.len() != env.total_arity() {
                return Err(Error::new("symbolic source application arity mismatch"));
            }
            let requirements = validated
                .symbolic_nominal_requirements(
                    source.declaration(),
                    source.proof(),
                    source.template(),
                )
                .map_err(|error| Error::new(error.into_legacy_diagnostic(b)))?;
            let inventory = producer::source_fields(source.template(), b)?;
            let mut fields = topology::storage(inventory.len(), b)?;
            let mut needed = Vec::new();
            for requirement in &requirements {
                let term = close_term(requirement, env, &actuals, b, 0)?;
                dependencies(&term, false, &mut needed, b, 0)?;
            }
            for actual in &actuals {
                dependencies(actual, false, &mut needed, b, 0)?;
            }
            for (coordinate, ty) in inventory {
                let term = close_term(&ty, env, &actuals, b, 0)?;
                dependencies(&term, true, &mut needed, b, 0)?;
                fields.push((coordinate, term));
            }
            let mut edges = topology::storage(needed.len(), b)?;
            for (declaration, args, by_value) in needed {
                let key = producer::key(world, declaration, &args, b)?;
                let mut found = None;
                for (index, previous) in keys.iter().enumerate() {
                    b.charge_work(previous.0.params.len().max(key.0.params.len()))
                        .map_err(Error::new)?;
                    if previous == &key {
                        found = Some(index);
                        break;
                    }
                }
                let target = if let Some(index) = found {
                    index
                } else {
                    b.charge_application().map_err(Error::new)?;
                    b.retain_bytes(std::mem::size_of::<RealizationKey>())
                        .map_err(Error::new)?;
                    keys.try_reserve_exact(1)
                        .map_err(|_| Error::new("symbolic frontier allocation failed"))?;
                    let index = keys.len();
                    keys.push(key);
                    index
                };
                edges.push(topology::Edge { target, by_value });
            }
            b.retain_bytes(
                std::mem::size_of::<CandidateMember>()
                    + std::mem::size_of::<topology::Application>(),
            )
            .map_err(Error::new)?;
            members
                .try_reserve_exact(1)
                .map_err(|_| Error::new("symbolic member allocation failed"))?;
            applications
                .try_reserve_exact(1)
                .map_err(|_| Error::new("symbolic topology allocation failed"))?;
            let key = super::component_key::copy_application(&keys[key_index], b)?;
            applications.push(topology::Application {
                key: topology::ApplicationWords(super::component_key::application_words(&key, b)?),
                edges,
            });
            members.push(CandidateMember {
                key,
                source,
                actuals,
                fields,
            });
        }
        cursor = end;
    }
    let components = topology::components(&applications, b)?;
    Ok(Discovery {
        members,
        components,
    })
}
