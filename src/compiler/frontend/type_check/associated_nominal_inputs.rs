//! Existing-input preflight for authored associated-call nominal prerequisites.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OriginalPredicates {
    Pending,
    Proved,
    NotProven,
}

impl Checker<'_> {
    pub(super) fn original_candidate_predicates(
        &self,
        declaration: ir::DeclId,
        arguments: &[ir::Ty],
        inputs: &NominalPreparationInputs<'_>,
    ) -> Result<OriginalPredicates, String> {
        let plan = self.signatures.plan()?;
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Ok(OriginalPredicates::Pending);
        };
        let world = plan.trait_world()?;
        let schema = world
            .original_predicate_schema(declaration)
            .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
        if schema.env.total_arity() != arguments.len() {
            return Err("original candidate predicate arity changed".into());
        }
        let mut bindings = HashMap::new();
        for (parameter, argument) in schema.env.ordered_params().zip(arguments) {
            self.infer_storage(1)?;
            bindings.insert(parameter, self.nominal_recipe_shape(argument, 0)?);
        }
        // Do not let substitution mint missing nominal metadata or normalize
        // a projection whose only premise came from this candidate.
        for predicate in schema.predicates.iter() {
            for ty in predicate.metadata_types() {
                let recipe = self.nominal_recipe_shape(ty, 0)?;
                let Some(recipe) = substitute_recipe(self, &recipe, &schema.env, &bindings, 0)?
                else {
                    return Ok(OriginalPredicates::Pending);
                };
                if self.existing_nominal_recipe(&recipe, inputs)?.is_none() {
                    return Ok(OriginalPredicates::Pending);
                }
            }
        }
        Ok(
            if world
                .prove_original_declaration_predicates(
                    declaration,
                    arguments,
                    facts,
                    &mut self.trait_scratch.borrow_mut(),
                )
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
            {
                OriginalPredicates::Proved
            } else {
                OriginalPredicates::NotProven
            },
        )
    }
}

pub(super) fn substitute_recipe(
    checker: &Checker<'_>,
    shape: &NominalShape,
    env: &ir::GenericEnv,
    bindings: &HashMap<ir::ParamId, NominalShape>,
    depth: usize,
) -> Result<Option<NominalShape>, String> {
    recipe_inner(checker, shape, env, bindings, depth, true)
}

fn recipe_inner(
    checker: &Checker<'_>,
    shape: &NominalShape,
    env: &ir::GenericEnv,
    bindings: &HashMap<ir::ParamId, NominalShape>,
    depth: usize,
    substitute_parameters: bool,
) -> Result<Option<NominalShape>, String> {
    use NominalShape as Shape;
    checker.infer_work(depth)?;
    checker.infer_storage(1)?;
    macro_rules! ready {
        ($value:expr) => {
            match $value? {
                Some(value) => value,
                None => return Ok(None),
            }
        };
    }
    let child = |value: &Shape| {
        recipe_inner(
            checker,
            value,
            env,
            bindings,
            depth + 1,
            substitute_parameters,
        )
    };
    let children = |values: &[Shape]| -> Result<Option<Arc<[Shape]>>, String> {
        checker.infer_storage(values.len())?;
        let mut mapped = Vec::new();
        for value in values {
            mapped.push(ready!(child(value)));
        }
        Ok(Some(mapped.into()))
    };
    Ok(Some(match shape {
        Shape::Leaf(ir::Ty::Param(parameter))
            if substitute_parameters && env.contains(*parameter) =>
        {
            let Some(value) = bindings.get(parameter) else {
                return Ok(None);
            };
            // Owner substitutions themselves may still contain a projection.
            // Keep exact caller parameter identities, without rebinding them.
            ready!(recipe_inner(
                checker,
                value,
                env,
                bindings,
                depth + 1,
                false
            ))
        }
        Shape::Leaf(ty) => Shape::Leaf(ty.clone()),
        Shape::Nominal(id, args) => Shape::Nominal(*id, ready!(children(args))),
        Shape::FnItem(id, args) => Shape::FnItem(*id, ready!(children(args))),
        Shape::Dyn(id, args, assignments) => {
            let args = ready!(children(args));
            checker.infer_storage(assignments.len())?;
            let mut mapped = Vec::new();
            for (item, value) in assignments.iter() {
                mapped.push((*item, ready!(child(value))));
            }
            Shape::Dyn(*id, args, mapped.into())
        }
        Shape::Tuple(args) => Shape::Tuple(ready!(children(args))),
        Shape::FnPointer(args, result) => {
            Shape::FnPointer(ready!(children(args)), Arc::new(ready!(child(result))))
        }
        Shape::PhantomData(id, value) => Shape::PhantomData(*id, Arc::new(ready!(child(value)))),
        Shape::ManuallyDrop(id, value) => Shape::ManuallyDrop(*id, Arc::new(ready!(child(value)))),
        Shape::Cell(id, value) => Shape::Cell(*id, Arc::new(ready!(child(value)))),
        Shape::Ref(kind, value) => Shape::Ref(*kind, Arc::new(ready!(child(value)))),
        Shape::RawPtr(kind, value) => Shape::RawPtr(*kind, Arc::new(ready!(child(value)))),
        Shape::Array(value, length) => Shape::Array(Arc::new(ready!(child(value))), *length),
        Shape::DependentArray(value, length) => {
            let value = Arc::new(ready!(child(value)));
            let parameter = ir::ParamId {
                owner: length.owner,
                index: length.index,
            };
            if substitute_parameters && env.contains(parameter) {
                let Some(Shape::Leaf(actual)) = bindings.get(&parameter) else {
                    return Ok(None);
                };
                match const_parameters::array_type(
                    ir::Ty::Unit,
                    actual,
                    checker.facts(),
                    &checker.signatures.plan()?.budget,
                )? {
                    ir::Ty::Array { len, .. } => Shape::Array(value, len),
                    ir::Ty::DependentArray { length, .. } => Shape::DependentArray(value, length),
                    _ => return Err("invalid associated array demand extent".into()),
                }
            } else {
                Shape::DependentArray(value, *length)
            }
        }
        Shape::Slice(value) => Shape::Slice(Arc::new(ready!(child(value)))),
        Shape::Projection { .. } => return Ok(None),
    }))
}
