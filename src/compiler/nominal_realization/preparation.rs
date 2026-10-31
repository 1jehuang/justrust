//! Node-local adapters from typed graph reads to source solver descriptions.
//! These callbacks are borrowed for one computation and are never published.
use super::super::frontend::{
    self, NominalDeclarationFields as Fields, NominalDeclarationRecipe as Recipe,
    NominalDemand as Demand, NominalShape as Shape,
};
use super::*;

pub(super) struct Capabilities {
    pub(super) sources: Vec<ReadSource>,
    pub(super) nominals: Vec<ReadNominal>,
    // Retained only by this consumer computation across missing-input retries.
    // Replay still checks the exact current definition and world before a hit.
    replays: std::cell::RefCell<traits::NominalReplays>,
}
impl Capabilities {
    pub(super) fn new() -> Self {
        Self {
            sources: Vec::new(),
            nominals: Vec::new(),
            replays: std::cell::RefCell::new(traits::NominalReplays::default()),
        }
    }

    pub(super) fn with_inputs<T>(
        &self,
        types: &frontend::PlanCtx,
        declaration: ir::DeclId,
        f: impl FnOnce(frontend::NominalPreparationInputs<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let budget = types.trait_world()?.budget().clone();
        let world = types.trait_world()?;
        let schema = |declaration| {
            self.source_recipe(declaration, &budget)
                .map_err(|e| e.to_string())
        };
        let source = |ty: &frontend::Type| types.nominal_source_shape(declaration, ty);
        let caller = types.callable_facts(declaration)?;
        let nominal = |demand: &Demand| {
            self.lookup(demand, &world, &caller, &budget)
                .map_err(|e| e.to_string())
        };
        f(frontend::NominalPreparationInputs::new(
            &schema, &source, &nominal,
        ))
    }

    pub(super) fn source_recipe(
        &self,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<Option<Recipe>, Error> {
        let Some(source) = self.sources.iter().find(|s| s.declaration() == declaration) else {
            return Ok(None);
        };
        let scopes = source.proof().schema().env.scopes();
        if scopes.len() != 1 || scopes[0].binder().declaration != declaration {
            return Err(Error::new(
                "nominal source recipe binder is not its exact declaration",
            ));
        }
        let shapes = |fields: &ir::NominalFields| -> Result<Arc<[Shape]>, Error> {
            let fields = fields.read(budget).map_err(Error::new)?;
            let mut values = Vec::new();
            values
                .try_reserve_exact(fields.len())
                .map_err(|_| Error::new("nominal field recipe allocation failed"))?;
            for field in fields.iter() {
                values.push(shape(&field.ty, budget, 0)?);
            }
            Ok(values.into())
        };
        let fields = match source.template() {
            ir::Ty::Struct(n) => Fields::Struct(shapes(&n.def().fields)?),
            ir::Ty::Enum(n) => {
                let mut variants = Vec::new();
                variants
                    .try_reserve_exact(n.def().variants.len())
                    .map_err(|_| Error::new("nominal variant recipe allocation failed"))?;
                for variant in n.def().variants.iter() {
                    variants.push(shapes(&variant.fields)?);
                }
                Fields::Enum(variants.into())
            }
            _ => {
                return Err(Error::new(
                    "typed source recipe output changed nominal kind",
                ));
            }
        };
        Ok(Some(Recipe {
            env: scopes[0].clone(),
            fields,
        }))
    }

    pub(super) fn lookup(
        &self,
        demand: &Demand,
        world: &traits::ValidatedTraitWorld,
        caller: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Option<ir::Ty>, Error> {
        let Some(key) = self.demand_key(demand, world, caller, budget)? else {
            return Ok(None);
        };
        let Some(capability) = self.nominals.iter().find(|n| n.application_key() == &key) else {
            return Ok(None);
        };
        let definition = capability.definition()?;
        capability.replay_with(&definition, world, &mut self.replays.borrow_mut())?;
        Ok(Some(definition))
    }
    pub(super) fn demand_key(
        &self,
        demand: &Demand,
        world: &traits::ValidatedTraitWorld,
        caller: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Option<RealizationKey>, Error> {
        self.key_with_missing(demand, world, caller, budget, &mut Vec::new(), 0)
    }

    fn key_with_missing(
        &self,
        demand: &Demand,
        world: &traits::ValidatedTraitWorld,
        caller: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
        missing: &mut Vec<RealizationKey>,
        depth: usize,
    ) -> Result<Option<RealizationKey>, Error> {
        budget.check_depth(depth).map_err(Error::new)?;
        let mut arguments = Vec::new();
        arguments
            .try_reserve_exact(demand.arguments.len())
            .map_err(|_| Error::new("nominal argument allocation failed"))?;
        let mut ready = true;
        for argument in demand.arguments.iter() {
            match self.shape_ty(argument, world, caller, budget, missing, depth + 1)? {
                Some(ty) => arguments
                    .push(coordinates::Term::from_ty(&ty, budget, depth + 1)?.encode(budget)?),
                None => ready = false,
            }
        }
        if !ready {
            return Ok(None);
        }
        let count = arguments
            .iter()
            .try_fold(6usize, |n, a| n.checked_add(a.len() + 1))
            .ok_or_else(|| Error::new("nominal demand coordinate overflow"))?;
        RealizationKey::encode(
            world.nominal_realization_world(),
            demand.declaration,
            &arguments,
            count,
        )
        .map(Some)
    }

    fn shape_ty(
        &self,
        value: &Shape,
        world: &traits::ValidatedTraitWorld,
        caller: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
        missing: &mut Vec<RealizationKey>,
        depth: usize,
    ) -> Result<Option<ir::Ty>, Error> {
        budget.check_depth(depth).map_err(Error::new)?;
        budget.charge_work(1).map_err(Error::new)?;
        let mut child =
            |value: &Shape| self.shape_ty(value, world, caller, budget, missing, depth + 1);
        Ok(Some(match value {
            Shape::Leaf(ty) => {
                coordinates::Term::from_ty(ty, budget, depth + 1)?;
                ty.clone()
            }
            Shape::Nominal(declaration, arguments) => {
                let demand = Demand {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                };
                let Some(key) =
                    self.key_with_missing(&demand, world, caller, budget, missing, depth + 1)?
                else {
                    return Ok(None);
                };
                let Some(cap) = self
                    .nominals
                    .iter()
                    .find(|cap| cap.application_key() == &key)
                else {
                    if !missing.contains(&key) {
                        missing.push(key);
                    }
                    return Ok(None);
                };
                let ty = cap.definition()?;
                cap.replay_with(&ty, world, &mut self.replays.borrow_mut())?;
                ty
            }
            Shape::Ref(kind, value) => {
                let Some(ty) = child(value)? else {
                    return Ok(None);
                };
                ir::Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(ty),
                }
            }
            Shape::RawPtr(kind, value) => {
                let Some(ty) = child(value)? else {
                    return Ok(None);
                };
                ir::Ty::RawPtr {
                    kind: *kind,
                    pointee: Arc::new(ty),
                }
            }
            Shape::Array(value, len) => {
                let Some(ty) = child(value)? else {
                    return Ok(None);
                };
                ir::Ty::Array {
                    element: Arc::new(ty),
                    len: *len,
                }
            }
            Shape::Slice(value) => {
                let Some(ty) = child(value)? else {
                    return Ok(None);
                };
                ir::Ty::Slice {
                    element: Arc::new(ty),
                }
            }
            Shape::ManuallyDrop(declaration, argument) => {
                let Some(argument) = child(argument)? else {
                    return Ok(None);
                };
                let marker = ir::ManuallyDropTy::new(world.canonical_catalog(), argument, budget)
                    .map_err(Error::new)?;
                if marker.declaration() != *declaration {
                    return Err(Error::new(
                        "nominal marker source shape changed canonical catalog authority",
                    ));
                }
                ir::Ty::ManuallyDrop(marker)
            }
            Shape::PhantomData(declaration, argument) => {
                let Some(argument) = child(argument)? else {
                    return Ok(None);
                };
                let marker = ir::PhantomDataTy::new(world.canonical_catalog(), argument, budget)
                    .map_err(Error::new)?;
                if marker.declaration() != *declaration {
                    return Err(Error::new(
                        "nominal marker source shape changed canonical catalog authority",
                    ));
                }
                ir::Ty::PhantomData(marker)
            }
            Shape::FnPointer(inputs, output) => {
                let mut types =
                    ir::FnPointerTy::storage(inputs.len(), budget).map_err(Error::new)?;
                let mut ready = true;
                for value in inputs.iter() {
                    if let Some(ty) = child(value)? {
                        types.push(ty);
                    } else {
                        ready = false;
                    }
                }
                let output = child(output)?;
                if !ready || output.is_none() {
                    return Ok(None);
                }
                ir::Ty::FnPointer(
                    ir::FnPointerTy::new(
                        types,
                        output.ok_or_else(|| Error::new("missing function pointer result"))?,
                        budget,
                    )
                    .map_err(Error::new)?,
                )
            }
            Shape::Tuple(values) => {
                let mut types = Vec::new();
                types
                    .try_reserve_exact(values.len())
                    .map_err(|_| Error::new("nominal tuple allocation failed"))?;
                let mut ready = true;
                for value in values.iter() {
                    if let Some(ty) = child(value)? {
                        types.push(ty)
                    } else {
                        ready = false
                    }
                }
                if !ready {
                    return Ok(None);
                }
                ir::Ty::Tuple(types.into())
            }
            Shape::Projection {
                declaration,
                item,
                receiver,
                arguments,
            } => {
                let receiver = child(receiver)?;
                let mut actuals = Vec::new();
                actuals
                    .try_reserve_exact(arguments.len())
                    .map_err(|_| Error::new("projection actual allocation failed"))?;
                let mut ready = receiver.is_some();
                for value in arguments.iter() {
                    if let Some(ty) = child(value)? {
                        actuals.push(ty)
                    } else {
                        ready = false
                    }
                }
                if !ready {
                    return Ok(None);
                }
                let projection = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: ir::TraitRef {
                        declaration: *declaration,
                        self_ty: receiver.expect("checked readiness"),
                        args: actuals.into(),
                    },
                    item: *item,
                }));
                let normalized = caller
                    .normalize_substituted_projection_type(&projection, budget)
                    .map_err(|e| Error::new(e.to_string()))?;
                coordinates::Term::from_ty(&normalized, budget, depth + 1)?;
                normalized
            }
        }))
    }

    pub(super) async fn satisfy(
        &mut self,
        cx: &Context,
        types: &frontend::PlanCtx,
        declaration: ir::DeclId,
        preparation: frontend::NominalPreparation,
    ) -> Result<(), Error> {
        let world = types.trait_world().map_err(Error::new)?;
        let budget = world.budget();
        let caller = types.callable_facts(declaration).map_err(Error::new)?;
        let (demands, declarations, needs_source_schemas, unresolved_constraints) =
            match preparation {
                frontend::NominalPreparation::Solved(manifest) => {
                    (manifest.demands().to_vec(), Vec::new(), false, false)
                }
                frontend::NominalPreparation::Blocked {
                    nominals,
                    declarations,
                    needs_source_schemas,
                    unresolved_constraints,
                } => (
                    nominals.to_vec(),
                    declarations.to_vec(),
                    needs_source_schemas,
                    unresolved_constraints,
                ),
            };
        let sources: Vec<_> = declarations
            .into_iter()
            .filter(|d| !self.sources.iter().any(|s| s.declaration() == *d))
            .collect();
        let keys = self.missing_keys(&demands, &world, &caller, budget)?;
        if sources.is_empty() && keys.is_empty() {
            let owner = types.declaration(declaration).map_err(Error::new)?;
            return Err(Error::new(format!(
                "unsupported: nominal source constraints cannot progress in {} from the exact available graph inputs (source schemas needed: {needs_source_schemas}, unresolved constraints: {unresolved_constraints})",
                owner.name,
            )));
        }
        // Demand both independent inventories before awaiting either typed read.
        for declaration in &sources {
            cx.request(source_key(*declaration));
        }
        for key in &keys {
            cx.request(key.0.clone());
        }
        let sources = read_sources(cx, &sources).await?;
        let nominals = read_manifest(cx, &keys, budget).await?;
        budget
            .retain_bytes(
                sources
                    .len()
                    .checked_mul(std::mem::size_of::<ReadSource>())
                    .and_then(|n| {
                        nominals
                            .len()
                            .checked_mul(std::mem::size_of::<ReadNominal>())
                            .and_then(|m| n.checked_add(m))
                    })
                    .ok_or_else(|| Error::new("nominal consumer capability size overflow"))?,
            )
            .map_err(Error::new)?;
        self.sources
            .try_reserve_exact(sources.len())
            .map_err(|_| Error::new("nominal source read inventory allocation failed"))?;
        self.nominals
            .try_reserve_exact(nominals.len())
            .map_err(|_| Error::new("nominal realization read inventory allocation failed"))?;
        self.sources.extend(sources);
        self.nominals.extend(nominals);
        // A demand's key can depend on realized nominal arguments. Reading one
        // inner frontier may therefore expose the next key without requiring
        // any new source inference. Saturate only these already-declared body
        // demands here, rather than restarting the entire source checker for
        // every layer. All authority still arrives through typed graph reads.
        loop {
            let keys = self.missing_keys(&demands, &world, &caller, budget)?;
            if keys.is_empty() {
                break;
            }
            for key in &keys {
                cx.request(key.0.clone());
            }
            let nominals = read_manifest(cx, &keys, budget).await?;
            if nominals.is_empty() {
                return Err(Error::new("nominal prerequisite frontier made no progress"));
            }
            budget
                .retain_bytes(
                    nominals
                        .len()
                        .checked_mul(std::mem::size_of::<ReadNominal>())
                        .ok_or_else(|| Error::new("nominal consumer capability size overflow"))?,
                )
                .map_err(Error::new)?;
            self.nominals
                .try_reserve_exact(nominals.len())
                .map_err(|_| Error::new("nominal realization read inventory allocation failed"))?;
            self.nominals.extend(nominals);
        }
        Ok(())
    }

    pub(super) fn missing_keys(
        &self,
        demands: &[Demand],
        world: &traits::ValidatedTraitWorld,
        caller: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Vec<RealizationKey>, Error> {
        let mut missing = Vec::new();
        for demand in demands {
            if let Some(key) =
                self.key_with_missing(demand, world, caller, budget, &mut missing, 0)?
                && !self.nominals.iter().any(|n| n.application_key() == &key)
                && !missing.contains(&key)
            {
                missing.push(key)
            }
        }
        Ok(missing)
    }
}

pub(in crate::compiler) fn shape(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Shape, Error> {
    budget.check_depth(depth).map_err(Error::new)?;
    budget.charge_work(1).map_err(Error::new)?;
    let children = |types: &[ir::Ty]| -> Result<Arc<[Shape]>, Error> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(types.len())
            .map_err(|_| Error::new("nominal source term allocation failed"))?;
        for ty in types {
            values.push(shape(ty, budget, depth + 1)?);
        }
        Ok(values.into())
    };
    Ok(match ty {
        ir::Ty::Struct(n) => Shape::Nominal(n.def().source_decl, children(&n.def().type_args)?),
        ir::Ty::Enum(n) => Shape::Nominal(n.def().source_decl, children(&n.def().type_args)?),
        ir::Ty::ManuallyDrop(marker) => {
            budget
                .retain_bytes(std::mem::size_of::<Shape>() + 2 * std::mem::size_of::<usize>())
                .map_err(Error::new)?;
            Shape::ManuallyDrop(
                marker.declaration(),
                Arc::new(shape(marker.argument(), budget, depth + 1)?),
            )
        }
        ir::Ty::PhantomData(marker) => {
            budget
                .retain_bytes(std::mem::size_of::<Shape>() + 2 * std::mem::size_of::<usize>())
                .map_err(Error::new)?;
            Shape::PhantomData(
                marker.declaration(),
                Arc::new(shape(marker.argument(), budget, depth + 1)?),
            )
        }
        ir::Ty::FnPointer(pointer) => Shape::FnPointer(
            children(&pointer.inputs)?,
            Arc::new(shape(&pointer.output, budget, depth + 1)?),
        ),
        ir::Ty::Tuple(types) => Shape::Tuple(children(types)?),
        ir::Ty::Ref { kind, referent } => {
            Shape::Ref(*kind, Arc::new(shape(referent, budget, depth + 1)?))
        }
        ir::Ty::RawPtr { kind, pointee } => {
            Shape::RawPtr(*kind, Arc::new(shape(pointee, budget, depth + 1)?))
        }
        ir::Ty::Slice { element } => Shape::Slice(Arc::new(shape(element, budget, depth + 1)?)),
        ir::Ty::Array { element, len } => {
            Shape::Array(Arc::new(shape(element, budget, depth + 1)?), *len)
        }
        ir::Ty::AssociatedType(p) => Shape::Projection {
            declaration: p.trait_ref.declaration,
            item: p.item,
            receiver: Arc::new(shape(&p.trait_ref.self_ty, budget, depth + 1)?),
            arguments: children(&p.trait_ref.args)?,
        },
        // Closed object identities were lowered by the canonical dynamic path.
        ir::Ty::Dyn(object) if object.is_closed_leaf() => Shape::Leaf(ty.clone()),
        ir::Ty::Closure(_) | ir::Ty::Dyn(_) => {
            return Err(Error::new(
                "unsupported: nominal source recipe needs independently authenticated closure or dynamic metadata",
            ));
        }
        primitive => Shape::Leaf(primitive.clone()),
    })
}
