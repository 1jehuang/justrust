//! Connected closed nominal producer. All layouts come from successful typed
//! child reads. Recursive source schemas are explicitly unsupported before a
//! closed node can wait on itself, never hidden behind an interner lookup.
use super::*;
use coordinates::Term;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

type Pending<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

/// Poll every independent branch before suspending the runnable segment.
async fn all<'a, T: Send + 'a>(mut pending: Vec<Pending<'a, T>>) -> Result<Vec<T>, Error> {
    let mut results: Vec<Option<T>> = (0..pending.len()).map(|_| None).collect();
    std::future::poll_fn(move |cx| {
        let mut error = None;
        for (future, result) in pending.iter_mut().zip(results.iter_mut()) {
            if result.is_none() {
                match future.as_mut().poll(cx) {
                    Poll::Ready(Ok(value)) => *result = Some(value),
                    Poll::Ready(Err(e)) => {
                        if error.is_none() {
                            error = Some(e);
                        }
                    }
                    Poll::Pending => {}
                }
            }
        }
        if let Some(error) = error {
            return Poll::Ready(Err(error));
        }
        if results.iter().all(Option::is_some) {
            Poll::Ready(Ok(results
                .iter_mut()
                .map(|r| r.take().expect("complete branch"))
                .collect()))
        } else {
            Poll::Pending
        }
    })
    .await
}

pub(super) fn key(
    world: u64,
    declaration: ir::DeclId,
    actuals: &[Term],
    budget: &ir::PlanBudget,
) -> Result<RealizationKey, Error> {
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(actuals.len())
        .map_err(|_| Error::new("nominal actual key allocation failed"))?;
    for actual in actuals {
        encoded.push(actual.encode(budget)?);
    }
    // The structural terms are already charged to this input-scaled task.
    let words = encoded
        .iter()
        .try_fold(6usize, |n, t| n.checked_add(t.len() + 1))
        .ok_or_else(|| Error::new("nominal key size overflow"))?;
    RealizationKey::encode(world, declaration, &encoded, words)
}

fn decode(key: &Key, b: &ir::PlanBudget) -> Result<(u64, ir::DeclId, Vec<Term>), Error> {
    if key.kind != KIND
        || key.params.len() < 6
        || key.params[0] != VERSION
        || key.params[1] != CLOSED_REALIZATION
    {
        return Err(Error::new("invalid closed nominal key"));
    }
    let declaration = ir::DeclId {
        snapshot: ir::SnapshotId::from_graph_word(key.params[3]),
        index: key.params[4],
    };
    let count =
        usize::try_from(key.params[5]).map_err(|_| Error::new("nominal actual count overflow"))?;
    if count > key.params.len().saturating_sub(6) {
        return Err(Error::new("nominal actual count exceeds key"));
    }
    let mut at = 6usize;
    let mut actuals = Vec::new();
    actuals
        .try_reserve_exact(count)
        .map_err(|_| Error::new("nominal actual decode allocation failed"))?;
    for _ in 0..count {
        let length = usize::try_from(
            *key.params
                .get(at)
                .ok_or_else(|| Error::new("truncated nominal actual length"))?,
        )
        .map_err(|_| Error::new("nominal actual length overflow"))?;
        at += 1;
        let end = at
            .checked_add(length)
            .ok_or_else(|| Error::new("nominal actual end overflow"))?;
        actuals.push(Term::decode(
            key.params
                .get(at..end)
                .ok_or_else(|| Error::new("truncated nominal actual"))?,
            b,
        )?);
        at = end;
    }
    if at != key.params.len() || self::key(key.params[2], declaration, &actuals, b)?.0 != *key {
        return Err(Error::new("noncanonical closed nominal key"));
    }
    Ok((key.params[2], declaration, actuals))
}

fn source_fields(source: &ir::Ty) -> Result<Vec<(FieldCoordinate, ir::Ty)>, Error> {
    let mut out = Vec::new();
    match source {
        ir::Ty::Struct(n) => {
            out.try_reserve_exact(n.def().fields.len())
                .map_err(|_| Error::new("nominal field inventory allocation failed"))?;
            for (field, f) in n.def().fields.iter().enumerate() {
                out.push((
                    FieldCoordinate {
                        variant: None,
                        field,
                    },
                    f.ty.clone(),
                ));
            }
        }
        ir::Ty::Enum(n) => {
            for (variant, v) in n.def().variants.iter().enumerate() {
                out.try_reserve_exact(v.fields.len())
                    .map_err(|_| Error::new("nominal field inventory allocation failed"))?;
                for (field, f) in v.fields.iter().enumerate() {
                    out.push((
                        FieldCoordinate {
                            variant: Some(variant),
                            field,
                        },
                        f.ty.clone(),
                    ));
                }
            }
        }
        _ => {
            return Err(Error::new(
                "nominal source output is not a nominal declaration",
            ));
        }
    }
    Ok(out)
}

fn declarations(
    ty: &ir::Ty,
    out: &mut Vec<ir::DeclId>,
    b: &ir::PlanBudget,
    d: usize,
) -> Result<(), Error> {
    b.check_depth(d).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    match ty {
        ir::Ty::Struct(n) => {
            if !out.contains(&n.def().source_decl) {
                out.push(n.def().source_decl);
            }
            for t in n.def().type_args.iter() {
                declarations(t, out, b, d + 1)?;
            }
        }
        ir::Ty::Enum(n) => {
            if !out.contains(&n.def().source_decl) {
                out.push(n.def().source_decl);
            }
            for t in n.def().type_args.iter() {
                declarations(t, out, b, d + 1)?;
            }
        }
        ir::Ty::Ref { referent: t, .. }
        | ir::Ty::RawPtr { pointee: t, .. }
        | ir::Ty::Slice { element: t }
        | ir::Ty::Array { element: t, .. } => declarations(t, out, b, d + 1)?,
        ir::Ty::Tuple(ts) => {
            for t in ts.iter() {
                declarations(t, out, b, d + 1)?;
            }
        }
        ir::Ty::AssociatedType(p) => {
            declarations(&p.trait_ref.self_ty, out, b, d + 1)?;
            for t in p.trait_ref.args.iter() {
                declarations(t, out, b, d + 1)?;
            }
        }
        ir::Ty::Closure(_) => {
            return Err(Error::new(
                "unsupported: closure nominal source realization frontier",
            ));
        }
        _ => {}
    }
    Ok(())
}

/// A local dependency-path check over exact typed source schemas. No closed
/// child is requested until this walk excludes an unsupported source cycle.
fn acyclic<'a>(
    cx: &'a Context,
    source: ReadSource,
    mut path: Vec<ir::DeclId>,
    b: &'a ir::PlanBudget,
) -> Pending<'a, ()> {
    Box::pin(async move {
        b.check_depth(path.len()).map_err(Error::new)?;
        if path.contains(&source.declaration()) {
            return Err(Error::new(
                "unsupported: recursive nominal field realization requires an explicit recursive layout protocol",
            ));
        }
        path.push(source.declaration());
        let mut needed = Vec::new();
        for (_, field) in source_fields(source.template())? {
            declarations(&field, &mut needed, b, 0)?;
        }
        let sources = read_sources(cx, &needed).await?;
        let branches = sources
            .into_iter()
            .map(|s| acyclic(cx, s, path.clone(), b))
            .collect();
        all(branches).await?;
        Ok(())
    })
}

fn resolve<'a>(
    cx: &'a Context,
    term: Term,
    world: u64,
    b: &'a ir::PlanBudget,
    depth: usize,
) -> Pending<'a, ir::Ty> {
    Box::pin(async move {
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        Ok(match term {
            Term::Int(n) => ir::Ty::Int(n),
            Term::Float(n) => ir::Ty::Float(n),
            Term::Bool => ir::Ty::Bool,
            Term::Unit => ir::Ty::Unit,
            Term::Never => ir::Ty::Never,
            Term::Char => ir::Ty::Char,
            Term::Str => ir::Ty::Str,
            Term::Ref(kind, t) => ir::Ty::Ref {
                kind,
                referent: Arc::new(resolve(cx, *t, world, b, depth + 1).await?),
            },
            Term::RawPtr(kind, t) => ir::Ty::RawPtr {
                kind,
                pointee: Arc::new(resolve(cx, *t, world, b, depth + 1).await?),
            },
            Term::Slice(t) => ir::Ty::Slice {
                element: Arc::new(resolve(cx, *t, world, b, depth + 1).await?),
            },
            Term::Array(t, len) => ir::Ty::Array {
                element: Arc::new(resolve(cx, *t, world, b, depth + 1).await?),
                len,
            },
            Term::Tuple(ts) => ir::Ty::Tuple(
                all(ts
                    .into_iter()
                    .map(|t| resolve(cx, t, world, b, depth + 1))
                    .collect())
                .await?
                .into(),
            ),
            // Decoding grants no trait authority: the realized definition is
            // still validated (metadata, dyn compatibility) by its consumers.
            Term::Dyn(principal, args, assigned) => {
                let args = all(args
                    .into_iter()
                    .map(|t| resolve(cx, t, world, b, depth + 1))
                    .collect())
                .await?;
                let mut associated_types = Vec::new();
                for (item, t) in assigned {
                    associated_types.push((item, resolve(cx, t, world, b, depth + 1).await?));
                }
                let object = ir::DynTy {
                    principal,
                    args: args.into(),
                    associated_types: associated_types.into(),
                };
                if !object.is_closed_leaf() {
                    return Err(Error::new("invalid dynamic nominal coordinate"));
                }
                ir::Ty::Dyn(Arc::new(object))
            }
            Term::Nominal(declaration, args) => {
                let key = key(world, declaration, &args, b)?;
                let mut read = read_manifest(cx, &[key]).await?;
                read.pop()
                    .ok_or_else(|| Error::new("nominal child read omitted its output"))?
                    .definition()?
            }
        })
    })
}

fn field<'a>(
    cx: &'a Context,
    ty: ir::Ty,
    input: &'a traits::ClosedNominalInput,
    world: &'a traits::ValidatedTraitWorld,
    depth: usize,
) -> Pending<'a, (ir::Ty, Vec<traits::NormalizedAssociatedType>)> {
    Box::pin(async move {
        let b = world.budget();
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        let error = |e: traits::FactsError| Error::new(e.into_legacy_diagnostic(b));
        let mut dependencies = Vec::new();
        let result = match ty {
            ir::Ty::Param(p) => world
                .nominal_realization_parameter(input, &p)
                .map_err(error)?,
            ir::Ty::AssociatedType(p) => {
                let (t, proofs) = world
                    .normalize_nominal_field_projection(input, &p)
                    .map_err(error)?;
                dependencies = proofs;
                let term = Term::from_ty(&t, b, 0)?;
                if term.contains_nominal() {
                    return Err(Error::new(
                        "unsupported: nominal-valued associated field realization requires an expanded closed dependency cycle protocol",
                    ));
                }
                resolve(cx, term, world.nominal_realization_world(), b, depth + 1).await?
            }
            ir::Ty::Struct(n) => {
                let args = all(n
                    .def()
                    .type_args
                    .iter()
                    .cloned()
                    .map(|t| field(cx, t, input, world, depth + 1))
                    .collect())
                .await?;
                let mut coordinates = Vec::new();
                for (t, proofs) in args {
                    coordinates.push(Term::from_ty(&t, b, 0)?);
                    dependencies.extend(proofs);
                }
                resolve(
                    cx,
                    Term::Nominal(n.def().source_decl, coordinates),
                    world.nominal_realization_world(),
                    b,
                    depth + 1,
                )
                .await?
            }
            ir::Ty::Enum(n) => {
                let args = all(n
                    .def()
                    .type_args
                    .iter()
                    .cloned()
                    .map(|t| field(cx, t, input, world, depth + 1))
                    .collect())
                .await?;
                let mut coordinates = Vec::new();
                for (t, proofs) in args {
                    coordinates.push(Term::from_ty(&t, b, 0)?);
                    dependencies.extend(proofs);
                }
                resolve(
                    cx,
                    Term::Nominal(n.def().source_decl, coordinates),
                    world.nominal_realization_world(),
                    b,
                    depth + 1,
                )
                .await?
            }
            ir::Ty::Ref { kind, referent } => {
                let (t, p) = field(cx, (*referent).clone(), input, world, depth + 1).await?;
                dependencies = p;
                ir::Ty::Ref {
                    kind,
                    referent: Arc::new(t),
                }
            }
            ir::Ty::RawPtr { kind, pointee } => {
                let (t, p) = field(cx, (*pointee).clone(), input, world, depth + 1).await?;
                dependencies = p;
                ir::Ty::RawPtr {
                    kind,
                    pointee: Arc::new(t),
                }
            }
            ir::Ty::Slice { element } => {
                let (t, p) = field(cx, (*element).clone(), input, world, depth + 1).await?;
                dependencies = p;
                ir::Ty::Slice {
                    element: Arc::new(t),
                }
            }
            ir::Ty::Array { element, len } => {
                let (t, p) = field(cx, (*element).clone(), input, world, depth + 1).await?;
                dependencies = p;
                ir::Ty::Array {
                    element: Arc::new(t),
                    len,
                }
            }
            ir::Ty::Tuple(ts) => {
                let values = all(ts
                    .iter()
                    .cloned()
                    .map(|t| field(cx, t, input, world, depth + 1))
                    .collect())
                .await?;
                let mut out = Vec::new();
                for (t, p) in values {
                    out.push(t);
                    dependencies.extend(p);
                }
                ir::Ty::Tuple(out.into())
            }
            ir::Ty::Closure(_) => {
                return Err(Error::new(
                    "unsupported: nominal field closure realization needs early parent-solved metadata",
                ));
            }
            other => other,
        };
        Ok((result, dependencies))
    })
}

pub(super) async fn run(cx: &Context, key_words: Key) -> Result<(), Error> {
    let types_handle = cx
        .request(crate::compiler::keys::SignaturesDebtRoot.key())
        .get(0);
    let types: Arc<super::super::frontend::PlanCtx> = cx.read_fact(types_handle).await?;
    let world = types.trait_world().map_err(Error::new)?;
    let b = world.budget();
    let (world_key, declaration, terms) = decode(&key_words, b)?;
    if world_key != world.nominal_realization_world() {
        return Err(Error::new("foreign nominal realization world coordinate"));
    }
    let mut sources = read_sources(cx, &[declaration]).await?;
    let source = sources
        .pop()
        .ok_or_else(|| Error::new("nominal source read omitted its output"))?;
    acyclic(cx, source.clone(), Vec::new(), b).await?;
    let actuals: Arc<[ir::Ty]> = all(terms
        .into_iter()
        .map(|t| resolve(cx, t, world_key, b, 0))
        .collect())
    .await?
    .into();
    let input = world
        .close_nominal_input(declaration, source.proof(), &actuals)
        .map_err(|e| Error::new(e.into_legacy_diagnostic(b)))?;
    let inventory = source_fields(source.template())?;
    let normalized = all(inventory
        .iter()
        .map(|(_, t)| field(cx, t.clone(), &input, &world, 0))
        .collect())
    .await?;
    let mut rows = Vec::new();
    let mut dependencies = Vec::new();
    for ((coordinate, source), (normalized, proofs)) in inventory.into_iter().zip(normalized) {
        rows.push(FieldRealization {
            coordinate,
            source,
            normalized,
        });
        dependencies.extend(proofs);
    }
    let identity = RealizationIdentity(cx.output_identity(0)?);
    let definition =
        materialize_definition(source.template(), identity, actuals.clone(), &rows, b)?;
    let source_application = match source.template() {
        ir::Ty::Struct(n) => n.id(),
        ir::Ty::Enum(n) => n.id(),
        _ => return Err(Error::new("nominal source changed representation")),
    };
    cx.publish_fact(
        0,
        RealizedNominal {
            key: RealizationKey(key_words),
            identity,
            declaration,
            source_application,
            source_definition: source.template().clone(),
            definition,
            actuals,
            declaration_proof: source.proof().clone(),
            fields: rows.into(),
            normalization: dependencies.into(),
            input,
        },
    )
}
