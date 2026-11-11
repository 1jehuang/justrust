//! Layout-free prerequisites of authored associated-call candidate frontiers.
//! A recipe requests a graph input only. It never selects or proves an impl.
use super::super::associated_nominal_inputs::{OriginalPredicates, substitute_recipe};
use super::*;
use nominal_manifest::{Demand, Shape};

impl Solver<'_, '_, '_> {
    pub(super) fn associated_nominal_demands(&self) -> Result<Vec<Demand>, String> {
        let mut demands = Vec::new();
        let Some(inputs) = self.preparation else {
            return Ok(demands);
        };
        for (owner, name) in &self.associated_nominal_sites {
            self.checker.infer_work(0)?;
            if self.resolve(*owner, 0)?.is_none() {
                continue;
            }
            for (header, _) in self.checker.source_associated_trait_candidates(name)? {
                let Some(bindings) = self.associated_owner_bindings(*owner, &header)? else {
                    continue;
                };
                let mut shapes = HashMap::new();
                let mut actuals = HashMap::new();
                for (parameter, term) in &bindings {
                    self.checker.infer_work(0)?;
                    if let Some(actual) = self.resolve(*term, 0)? {
                        self.checker.infer_storage(2)?;
                        shapes.insert(*parameter, self.checker.nominal_recipe_shape(&actual, 0)?);
                        actuals.insert(*parameter, actual);
                    }
                }
                let mut ordered = Vec::new();
                for parameter in header.schema_env.ordered_params() {
                    let Some(actual) = actuals.get(&parameter) else {
                        break;
                    };
                    self.checker.infer_storage(1)?;
                    ordered.push(actual.clone());
                }
                if ordered.len() == header.schema_env.total_arity()
                    && self.checker.original_candidate_predicates(
                        header.declaration,
                        &ordered,
                        inputs,
                    )? == OriginalPredicates::NotProven
                {
                    continue;
                }
                for argument in header.trait_ref.args.iter() {
                    let recipe = self.checker.nominal_recipe_shape(argument, 0)?;
                    if let Some(recipe) =
                        substitute_recipe(self.checker, &recipe, &header.schema_env, &shapes, 0)?
                    {
                        // Child demands are independently caller-wellformed.
                        // A pending parent never grants a parent demand.
                        let mut argument_demands = Vec::new();
                        self.collect_associated_demands(&recipe, inputs, &mut argument_demands, 0)?;
                        for demand in argument_demands {
                            self.retain_associated_demand(demand, &mut demands, 0)?;
                        }
                    }
                }
            }
        }
        Ok(demands)
    }

    fn retain_associated_demand(
        &self,
        demand: Demand,
        demands: &mut Vec<Demand>,
        depth: usize,
    ) -> Result<(), String> {
        for previous in demands.iter() {
            self.checker.infer_work(depth)?;
            if previous.declaration == demand.declaration
                && previous.arguments.len() == demand.arguments.len()
            {
                let mut same = true;
                for (left, right) in previous.arguments.iter().zip(demand.arguments.iter()) {
                    same &=
                        left.same_with(right, depth + 1, &mut |d| self.checker.infer_work(d))?;
                }
                if same {
                    return Ok(());
                }
            }
        }
        self.checker.infer_storage(1)?;
        demands.push(demand);
        Ok(())
    }

    fn collect_associated_demands(
        &self,
        shape: &Shape,
        inputs: &nominal_manifest::Inputs<'_>,
        demands: &mut Vec<Demand>,
        depth: usize,
    ) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        let mut children_ready = true;
        macro_rules! child {
            ($value:expr) => {
                children_ready &=
                    self.collect_associated_demands($value, inputs, demands, depth + 1)?;
            };
        }
        match shape {
            Shape::Leaf(_) => {}
            Shape::Nominal(_, args) | Shape::FnItem(_, args) | Shape::Tuple(args) => {
                for argument in args.iter() {
                    child!(argument);
                }
            }
            Shape::Dyn(_, args, assignments) => {
                for argument in args.iter() {
                    child!(argument);
                }
                for (_, value) in assignments.iter() {
                    child!(value);
                }
            }
            Shape::FnPointer(args, result) => {
                for argument in args.iter() {
                    child!(argument);
                }
                child!(result);
            }
            Shape::PhantomData(_, value)
            | Shape::ManuallyDrop(_, value)
            | Shape::Cell(_, value)
            | Shape::Ref(_, value)
            | Shape::RawPtr(_, value)
            | Shape::Array(value, _)
            | Shape::DependentArray(value, _)
            | Shape::Slice(value) => {
                child!(value);
            }
            Shape::Projection { .. } => return Ok(false),
        }
        if !children_ready {
            return Ok(false);
        }
        if let Shape::Nominal(declaration, arguments) = shape {
            let plan = self.checker.signatures.plan()?;
            let schema = plan
                .trait_world()?
                .original_predicate_schema(*declaration)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            if schema.env.total_arity() != arguments.len() {
                return Err("associated nominal demand arity changed".into());
            }
            if !schema.predicates.is_empty() {
                let mut actuals = Vec::new();
                for argument in arguments.iter() {
                    let Some(actual) = self.checker.existing_nominal_recipe(argument, inputs)?
                    else {
                        return Ok(false);
                    };
                    self.checker.infer_storage(1)?;
                    actuals.push(actual);
                }
                if self
                    .checker
                    .original_candidate_predicates(*declaration, &actuals, inputs)?
                    != OriginalPredicates::Proved
                {
                    return Ok(false);
                }
            }
            self.retain_associated_demand(
                Demand {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                },
                demands,
                depth,
            )?;
        }
        Ok(true)
    }
}
