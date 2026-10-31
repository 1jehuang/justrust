//! Source declaration admission for acyclic forwarded opaque returns.
//! Each header seals only its own final TEMPLATE_BODY outputs 0 and 2.
//! Validation requests the finite referenced-owner closure in complete waves,
//! never another validation node, transitive source body or physical fact.
use super::{
    CompilerProvider, Context, Error, Facts, OpaqueHiddenTypesFact, TEMPLATE_BODY,
    TemplateBodyFact, ir, keys, scale_task_work_by_callable, source_fact_error, traits,
};
use keys::opaque_declaration::OpaqueDeclaration as Declaration;
use std::sync::Arc;

struct Header {
    owner: ir::DeclId,
    definitions: Arc<[ir::OpaqueDefinitionProof]>,
    dependencies: Arc<[Arc<ir::OpaqueTy>]>,
}
impl crate::engine::Fact for Header {}

pub(super) struct Admission {
    owner: ir::DeclId,
}
impl crate::engine::Fact for Admission {}
impl Admission {
    pub(super) fn validate_owner(&self, owner: ir::DeclId) -> Result<(), Error> {
        if self.owner != owner {
            return Err(Error::new(
                "opaque declaration admission changed source owner",
            ));
        }
        Ok(())
    }
}

// Charge before requesting capacity, then reconcile any allocator over-allocation.
// These node-local buffers retain conservative charges through task completion.
fn reserve<T>(
    values: &mut Vec<T>,
    additional: usize,
    budget: &ir::PlanBudget,
) -> Result<(), Error> {
    let required = values
        .len()
        .checked_add(additional)
        .ok_or_else(|| Error::new("opaque declaration buffer extent overflow"))?;
    if required <= values.capacity() {
        return Ok(());
    }
    let old_capacity = values.capacity();
    let bytes = |slots: usize| {
        slots
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| Error::new("opaque declaration buffer storage overflow"))
    };
    budget
        .retain_bytes(bytes(required - old_capacity)?)
        .map_err(Error::new)?;
    values
        .try_reserve_exact(additional)
        .map_err(|_| Error::new("opaque declaration buffer allocation failed"))?;
    budget
        .retain_bytes(bytes(values.capacity() - required)?)
        .map_err(Error::new)
}

// Traverse type structure without allocating iterator trait objects. Full
// descriptors stay on edges, but owner identity bounds the graph exploration:
// growing generic actuals never cause instance expansion. Public projection
// inputs are NOT a completeness proof of their normalized hidden output.
fn collect(
    ty: &ir::Ty,
    output: &mut Vec<Arc<ir::OpaqueTy>>,
    budget: &ir::PlanBudget,
    facts: &traits::ValidatedFacts,
    depth: usize,
) -> Result<(), Error> {
    budget.check_depth(depth).map_err(Error::new)?;
    budget.charge_work(1).map_err(Error::new)?;
    match ty {
        ir::Ty::Opaque(opaque) => {
            let mut present = false;
            for prior in output.iter() {
                if ir::identical_type(&ir::Ty::Opaque(prior.clone()), ty, budget)
                    .map_err(Error::new)?
                {
                    present = true;
                    break;
                }
            }
            if !present {
                reserve(output, 1, budget)?;
                output.push(opaque.clone());
            }
            for child in opaque.owner_args() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        ir::Ty::Ref { referent, .. } => collect(referent, output, budget, facts, depth + 1)?,
        ir::Ty::RawPtr { pointee, .. } => collect(pointee, output, budget, facts, depth + 1)?,
        ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
            collect(element, output, budget, facts, depth + 1)?;
        }
        ir::Ty::Tuple(fields) => {
            for child in fields.iter() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        ir::Ty::Struct(value) => {
            for child in value.def().type_args.iter() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        ir::Ty::Enum(value) => {
            for child in value.def().type_args.iter() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        // Zero runtime storage does not erase declaration metadata dependencies.
        ir::Ty::PhantomData(value) => {
            collect(value.argument(), output, budget, facts, depth + 1)?;
        }
        ir::Ty::Cell(cell) => {
            collect(cell.argument(), output, budget, facts, depth + 1)?;
        }
        ir::Ty::ManuallyDrop(wrapper) => {
            collect(wrapper.argument(), output, budget, facts, depth + 1)?;
        }
        ir::Ty::FnPointer(value) => {
            for child in value.types() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        ir::Ty::Closure(value) => {
            for child in value.types() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        ir::Ty::FnItem(value) => {
            for child in value.type_args.iter() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        ir::Ty::Dyn(value) => {
            for child in value.types() {
                collect(child, output, budget, facts, depth + 1)?;
            }
        }
        ir::Ty::AssociatedType(_) => {
            // Canonical normalization validates this exact projection against
            // its defining publication environment, phase and equalities. It
            // follows selected/public assignments, never a physical hidden fact.
            let normal = facts
                .world()
                .normalize_ty(ty, facts)
                .map_err(|error| source_fact_error(error, budget))?;
            if matches!(normal, ir::Ty::AssociatedType(_)) {
                // Admissible symbolic membership is not complete hidden-output
                // dependency evidence. Without exact substitution obligations
                // this bounded protocol cannot certify the unresolved family.
                // Never silently replace it with its trait inputs or no edges.
                return Err(Error::new(
                    "unsupported: unresolved opaque hidden-output projection completeness requires exact substitution obligations",
                ));
            }
            collect(&normal, output, budget, facts, depth + 1)?;
        }
        _ => {}
    }
    Ok(())
}

pub(super) async fn produce(
    provider: &CompilerProvider,
    request: Declaration,
    facts: &Facts<'_>,
    cx: &Context,
    budget: &ir::PlanBudget,
) -> Result<(), Error> {
    let types = facts.types().await?;
    let owner = match request {
        Declaration::Header(owner) | Declaration::Validate(owner) => owner,
    };
    let index = types.callable_index(owner).map_err(Error::new)?;
    scale_task_work_by_callable(&types, owner);
    match request {
        Declaration::Header(_) => {
            let outputs = cx.request_item(&keys::Item::<TEMPLATE_BODY>(index));
            let body = outputs.get(0);
            let hidden = outputs.get(2);
            let body: Arc<TemplateBodyFact> = cx.read_fact(body).await?;
            let hidden: Arc<OpaqueHiddenTypesFact> = cx.read_fact(hidden).await?;
            provider.validated_body(&types, index, &body)?;
            let publication = body.publication.as_ref().ok_or_else(|| {
                Error::new("opaque declaration header requires original source publication")
            })?;
            let mut definitions: Vec<ir::OpaqueDefinitionProof> = Vec::new();
            reserve(&mut definitions, hidden.representations.len(), budget)?;
            for record in hidden.representations.iter() {
                budget
                    .charge_work(definitions.len() + 1)
                    .map_err(Error::new)?;
                if record.opaque().owner() != owner
                    || definitions.iter().any(|prior| {
                        prior.declaration_record().opaque().site() == record.opaque().site()
                    })
                {
                    return Err(Error::new(
                        "opaque declaration header changed owner or duplicated occurrence",
                    ));
                }
                definitions.push(
                    ir::OpaqueDefinitionProof::seal(publication.clone(), record, budget)
                        .map_err(Error::new)?,
                );
            }
            // Output 2 must cover every original defining wrapper, not merely
            // contain some individually authentic records. Never retain the body.
            for expression in &publication.source().exprs {
                budget
                    .charge_work(definitions.len() + 1)
                    .map_err(Error::new)?;
                if let ir::ExprKind::OpaqueWrap { opaque, .. } = &expression.kind
                    && (opaque.owner() != owner
                        || !definitions.iter().any(|definition| {
                            definition.declaration_record().opaque().site() == opaque.site()
                        }))
                {
                    return Err(Error::new(
                        "opaque declaration header omitted an original defining occurrence",
                    ));
                }
            }
            budget
                .retain_bytes(
                    std::mem::size_of::<Header>()
                        + 2 * std::mem::size_of::<usize>()
                        + definitions
                            .len()
                            .saturating_mul(std::mem::size_of::<ir::OpaqueDefinitionProof>()),
                )
                .map_err(Error::new)?;
            let mut dependencies = Vec::new();
            for definition in &definitions {
                collect(
                    definition.declaration_record().hidden(),
                    &mut dependencies,
                    budget,
                    publication.facts(),
                    0,
                )?;
            }
            budget
                .retain_bytes(
                    2 * std::mem::size_of::<usize>()
                        + dependencies
                            .len()
                            .saturating_mul(std::mem::size_of::<Arc<ir::OpaqueTy>>()),
                )
                .map_err(Error::new)?;
            cx.publish_fact(
                0,
                Header {
                    owner,
                    definitions: definitions.into(),
                    dependencies: dependencies.into(),
                },
            )
        }
        Declaration::Validate(_) => {
            let mut pending = Vec::new();
            reserve(&mut pending, 1, budget)?;
            pending.push(owner);
            let mut headers: Vec<(ir::DeclId, Arc<Header>)> = Vec::new();
            let mut edges: Vec<(ir::DeclId, Arc<[Arc<ir::OpaqueTy>]>)> = Vec::new();
            while !pending.is_empty() {
                budget
                    .charge_work(pending.len().saturating_mul(headers.len() + pending.len()))
                    .map_err(Error::new)?;
                pending.sort_unstable();
                pending.dedup();
                pending.retain(|candidate| !headers.iter().any(|(seen, _)| seen == candidate));
                let mut requests = Vec::new();
                reserve(&mut requests, pending.len(), budget)?;
                for declaration in pending.drain(..) {
                    let request = Declaration::Header(declaration);
                    request.charge_key(budget).map_err(Error::new)?;
                    requests.push(request);
                }
                budget
                    .retain_bytes(
                        requests
                            .len()
                            .saturating_mul(std::mem::size_of::<crate::engine::Handle>()),
                    )
                    .map_err(Error::new)?;
                let handles = cx.request_all(&requests, 0);
                reserve(&mut headers, requests.len(), budget)?;
                reserve(&mut edges, requests.len(), budget)?;
                for (request, handle) in requests.into_iter().zip(handles) {
                    let Declaration::Header(declaration) = request else {
                        unreachable!()
                    };
                    let header: Arc<Header> = cx.read_fact(handle).await?;
                    if header.owner != declaration || header.definitions.is_empty() {
                        return Err(Error::new(
                            "opaque declaration dependency has no original defining occurrence",
                        ));
                    }
                    let children = header.dependencies.clone();
                    for definition in header.definitions.iter() {
                        let record = definition.declaration_record();
                        if record.opaque().owner() != declaration {
                            return Err(Error::new(
                                "opaque declaration header contains a foreign source proof",
                            ));
                        }
                        budget.charge_work(1).map_err(Error::new)?;
                    }
                    reserve(&mut pending, children.len(), budget)?;
                    pending.extend(children.iter().map(|child| child.owner()));
                    headers.push((declaration, header));
                    edges.push((declaration, children));
                }
            }
            // Full original actuals, bounds and captures were authenticated by
            // the referring sealed source proof. Headers certify exact owner/site
            // completeness, not equality of open and instantiated descriptors.
            for (_, children) in &edges {
                for child in children.iter() {
                    budget.charge_work(headers.len()).map_err(Error::new)?;
                    let header = &headers
                        .iter()
                        .find(|(declaration, _)| *declaration == child.owner())
                        .ok_or_else(|| {
                            Error::new("opaque declaration frontier omitted a referenced owner")
                        })?
                        .1;
                    budget
                        .charge_work(header.definitions.len())
                        .map_err(Error::new)?;
                    if !header.definitions.iter().any(|definition| {
                        definition.declaration_record().opaque().site() == child.site()
                    }) {
                        return Err(Error::new(
                            "opaque declaration edge changed its return occurrence",
                        ));
                    }
                }
            }
            let mut colors = Vec::new();
            reserve(&mut colors, edges.len(), budget)?;
            colors.resize(edges.len(), 0u8);
            let mut stack = Vec::new();
            reserve(&mut stack, edges.len(), budget)?;
            stack.push((0usize, 0usize));
            colors[0] = 1;
            while !stack.is_empty() {
                budget.check_depth(stack.len()).map_err(Error::new)?;
                let (node, next) = stack
                    .last_mut()
                    .ok_or_else(|| Error::new("opaque declaration walk lost its active node"))?;
                budget.charge_work(edges.len() + 1).map_err(Error::new)?;
                if let Some(child) = edges[*node].1.get(*next) {
                    *next += 1;
                    let target = edges
                        .iter()
                        .position(|(declaration, _)| *declaration == child.owner())
                        .ok_or_else(|| Error::new("opaque declaration cycle walk lost an owner"))?;
                    match colors[target] {
                        1 => {
                            return Err(Error::new(
                                "unsupported: recursive opaque declaration requires grounded fixed-point inference",
                            ));
                        }
                        0 => {
                            colors[target] = 1;
                            stack.push((target, 0));
                        }
                        _ => {}
                    }
                } else {
                    colors[*node] = 2;
                    stack.pop();
                }
            }
            cx.publish_fact(0, Admission { owner })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn original(source: &str, workers: usize) -> (Arc<TemplateBodyFact>, ir::PlanBudget) {
        let snapshot = Arc::new(super::super::Snapshot {
            input: super::super::Input::Text(source.into()),
            source_options: super::super::source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let budget = snapshot.budget.clone();
        let engine = super::super::Engine::new(CompilerProvider(snapshot));
        let key = keys::Item::<TEMPLATE_BODY>(0).key();
        let requested = engine.request(key.clone());
        let report = engine.run(workers).unwrap();
        requested.completion().unwrap();
        assert!(report.capture_complete);
        assert!(!report.node_evaluations.iter().any(|(key, _)| matches!(
            key.kind,
            super::super::OPAQUE_REPRESENTATION
                | super::super::OPAQUE_CALL_DISPATCH
                | super::super::INSTANCE
                | super::super::OBJECT
                | super::super::BUILD
        )));
        (engine.fact_of::<TemplateBodyFact>(&key, 0).unwrap(), budget)
    }

    #[test]
    fn hidden_projection_inputs_do_not_certify_normalized_output_completeness() {
        for workers in [1, 8] {
            let (body, budget) = original(
                "trait Has{type Out;}fn owner<T:Has>(x:T::Out)->T::Out{x}fn main(){}",
                workers,
            );
            let publication = body.publication.as_ref().unwrap();
            assert!(matches!(body.checked.result, ir::Ty::AssociatedType(_)));
            let mut dependencies = Vec::new();
            let error = collect(
                &body.checked.result,
                &mut dependencies,
                &budget,
                publication.facts(),
                0,
            )
            .unwrap_err();
            assert!(
                error.to_string().contains(
                    "unsupported: unresolved opaque hidden-output projection completeness"
                )
            );
            assert!(dependencies.is_empty());
            let (body, budget) = original(
                "trait Has{type Out;}fn owner<T:Has<Out=i32>>(x:T::Out)->T::Out{x}fn main(){}",
                workers,
            );
            let publication = body.publication.as_ref().unwrap();
            collect(
                &body.checked.result,
                &mut Vec::new(),
                &budget,
                publication.facts(),
                0,
            )
            .unwrap();
        }
    }

    #[test]
    fn normalized_public_associated_output_retains_its_exact_opaque_owner() {
        for workers in [1, 8] {
            let (body, budget) = original(
                "trait Has<T>{type Out;}struct V;impl<T> Has<T> for V{type Out=T;}fn witness(){let value=outer(make());}fn make()->impl Sized{1}fn outer<T>(x:T)->impl Has<T,Out=T>{V}fn main(){}",
                workers,
            );
            let publication = body.publication.as_ref().unwrap();
            let facts = publication.facts();
            // Read the exact public assignment from the original source value.
            // Its generic actual is itself make's opaque identity. No callable
            // value capability, hidden representation or fabricated fact is used.
            let (opaque, bound, item, expected) = body
                .checked
                .exprs
                .iter()
                .find_map(|expression| {
                    let ir::Ty::Opaque(opaque) = &expression.ty else {
                        return None;
                    };
                    opaque.bounds().iter().find_map(|bound| {
                        bound.equalities().iter().find_map(|(item, rhs)| {
                            let ir::Ty::Opaque(expected) = rhs else {
                                return None;
                            };
                            Some((opaque.clone(), bound.clone(), *item, expected.clone()))
                        })
                    })
                })
                .expect("source value lost its public opaque associated assignment");
            assert_ne!(expected.owner(), opaque.owner());
            assert!(
                opaque
                    .owner_args()
                    .contains(&ir::Ty::Opaque(expected.clone()))
            );
            let projection = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                trait_ref: bound.application(ir::Ty::Opaque(opaque)),
                item,
            }));
            let normalized = facts.world().normalize_ty(&projection, facts).unwrap();
            assert_eq!(normalized, ir::Ty::Opaque(expected.clone()));
            let mut dependencies = Vec::new();
            collect(&projection, &mut dependencies, &budget, facts, 0).unwrap();
            assert_eq!(dependencies.as_slice(), &[expected]);
        }
    }

    #[test]
    fn declaration_buffers_and_walks_fail_before_unmetered_growth() {
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        let mut values: Vec<ir::DeclId> = Vec::new();
        assert!(reserve(&mut values, 1, &denied).is_err());
        assert_eq!(values.capacity(), 0);
        assert!(reserve(&mut values, usize::MAX, &ir::PlanBudget::default()).is_err());
    }
}
