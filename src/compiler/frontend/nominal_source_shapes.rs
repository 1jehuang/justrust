//! Source-only demand descriptions. These are not type or region permissions.
use super::NominalShape as Shape;
use super::*;

impl PlanCtx {
    pub(in crate::compiler) fn nominal_source_shape(
        &self,
        declaration: ir::DeclId,
        ty: &Type,
    ) -> Result<Option<Shape>, String> {
        let info = self.registry.declaration(declaration)?;
        let bindings = generic_bindings(&self.registry, &info.env, &self.budget)?;
        let parameters = HashMap::new();
        let active = std::cell::RefCell::new(Vec::new());
        self.nominal_source_shape_inner(ty, &bindings, &parameters, declaration, 0, &active)
            .map(Some)
            .map_err(|error| {
                format!(
                    "{error}; source recipe callable={} ({declaration:?})",
                    info.name
                )
            })
    }

    /// Complete demand arguments, not source syntax or type permissions. The
    /// existing declaration proof and realization reads remain mandatory.
    fn nominal_source_recipe_arguments(
        &self,
        declaration: ir::DeclId,
        mut args: Vec<Shape>,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        depth: usize,
        active: &std::cell::RefCell<Vec<(ir::DeclId, usize)>>,
    ) -> Result<Vec<Shape>, String> {
        self.budget.check_depth(depth)?;
        self.registry
            .nominal_source_arity(declaration, args.len())?;
        let info = self.registry.declaration(declaration)?;
        if args.len() == info.type_params.len() {
            return Ok(args);
        }
        let schema = self
            .nominal_schemas
            .get(&declaration)
            .ok_or("missing authenticated source recipe declaration")?;
        let defaults = schema.defaults();
        nominal_defaults::validate_declaration(
            self.registry.names.as_ref(),
            &info.type_params,
            defaults,
        )?;
        self.budget.retain_bytes(
            info.type_params
                .len()
                .checked_mul(std::mem::size_of::<Shape>())
                .ok_or("nominal default recipe actual size overflow")?,
        )?;
        args.try_reserve_exact(info.type_params.len() - args.len())
            .map_err(|_| "infrastructure: nominal default recipe actual allocation failed")?;
        self.budget.retain_bytes(
            info.type_params
                .len()
                .checked_mul(4 * std::mem::size_of::<(GenericBindingKey, Shape)>())
                .ok_or("nominal default recipe binding size overflow")?,
        )?;
        let mut parameters = HashMap::new();
        parameters
            .try_reserve(info.type_params.len())
            .map_err(|_| "infrastructure: nominal default recipe binding allocation failed")?;
        for (parameter, actual) in info.type_params.iter().zip(&args) {
            self.budget.charge_work(1)?;
            parameters.insert(self.registry.parameter_key(*parameter)?, actual.clone());
        }
        while args.len() < info.type_params.len() {
            self.budget.charge_work(1)?;
            let index = args.len();
            let default = defaults[index]
                .as_ref()
                .ok_or("missing original source recipe default slot")?;
            default.syntax(self.registry.names.as_ref())?;
            {
                let previous = active.borrow();
                for slot in previous.iter() {
                    self.budget.charge_work(1)?;
                    if *slot == (declaration, index) {
                        return Err("cyclic nominal generic default source recipe".into());
                    }
                }
            }
            self.budget
                .retain_bytes(std::mem::size_of::<(ir::DeclId, usize)>())?;
            {
                let mut stack = active.borrow_mut();
                stack.try_reserve_exact(1).map_err(
                    |_| "infrastructure: nominal default recipe cycle allocation failed",
                )?;
                stack.push((declaration, index));
            }
            let actual = self.nominal_source_shape_inner(
                &default.ty,
                bindings,
                &parameters,
                declaration,
                depth + 1,
                active,
            );
            active.borrow_mut().pop();
            let actual = actual?;
            reject_default_recipe_carriers(&actual, &self.budget, depth + 1)?;
            parameters.insert(
                self.registry.parameter_key(info.type_params[index])?,
                actual.clone(),
            );
            args.push(actual);
        }
        Ok(args)
    }

    fn nominal_source_shape_inner(
        &self,
        ty: &Type,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        parameters: &HashMap<GenericBindingKey, Shape>,
        declaration: ir::DeclId,
        depth: usize,
        active: &std::cell::RefCell<Vec<(ir::DeclId, usize)>>,
    ) -> Result<Shape, String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        let child = |ty: &Type| {
            self.nominal_source_shape_inner(
                ty,
                bindings,
                parameters,
                declaration,
                depth + 1,
                active,
            )
        };
        Ok(match ty {
            Type::Int(n) => Shape::Leaf(ir::Ty::Int(*n)),
            Type::Float(n) => Shape::Leaf(ir::Ty::Float(*n)),
            Type::Bool => Shape::Leaf(ir::Ty::Bool),
            Type::Unit => Shape::Leaf(ir::Ty::Unit),
            Type::Never => Shape::Leaf(ir::Ty::Never),
            Type::Char => Shape::Leaf(ir::Ty::Char),
            Type::Str => Shape::Leaf(ir::Ty::Str),
            Type::Param(name) => {
                let key = self.registry.parameter_key(*name)?;
                if let Some(actual) = parameters.get(&key) {
                    actual.clone()
                } else {
                    Shape::Leaf(
                        bindings
                            .get(&key)
                            .cloned()
                            .ok_or("source nominal recipe has a foreign generic parameter")?,
                    )
                }
            }
            Type::SelfType => Shape::Leaf(
                bindings
                    .get(&GenericBindingKey::SelfType)
                    .cloned()
                    .ok_or("unsupported: source nominal recipe needs exact impl Self metadata")?,
            ),
            Type::Ref { kind, referent, .. } => Shape::Ref(*kind, Arc::new(child(referent)?)),
            Type::RawPtr { kind, pointee } => Shape::RawPtr(*kind, Arc::new(child(pointee)?)),
            Type::Array(element, len) => Shape::Array(Arc::new(child(element)?), *len),
            Type::Slice(element) => Shape::Slice(Arc::new(child(element)?)),
            Type::PhantomData(source) => {
                source
                    .original
                    .checked_path(self.registry.names.as_ref())
                    .map_err(native_adapter::name_diagnostic)?;
                let argument = source
                    .argument
                    .as_ref()
                    .ok_or("missing PhantomData source argument")?;
                self.budget.retain_bytes(
                    std::mem::size_of::<Shape>() + 2 * std::mem::size_of::<usize>(),
                )?;
                Shape::PhantomData(
                    self.catalog.id(tr::CatalogItem::PhantomData),
                    Arc::new(child(argument)?),
                )
            }
            Type::ManuallyDrop(source) => {
                source
                    .original
                    .checked_path(self.registry.names.as_ref())
                    .map_err(native_adapter::name_diagnostic)?;
                let argument = source
                    .argument
                    .as_ref()
                    .ok_or("missing ManuallyDrop source argument")?;
                self.budget.retain_bytes(
                    std::mem::size_of::<Shape>() + 2 * std::mem::size_of::<usize>(),
                )?;
                Shape::ManuallyDrop(
                    self.catalog.id(tr::CatalogItem::ManuallyDrop),
                    Arc::new(child(argument)?),
                )
            }
            Type::Cell(source) => {
                source
                    .original
                    .checked_path(self.registry.names.as_ref())
                    .map_err(native_adapter::name_diagnostic)?;
                let argument = source
                    .argument
                    .as_ref()
                    .ok_or("missing Cell source argument")?;
                self.budget.retain_bytes(
                    std::mem::size_of::<Shape>() + 2 * std::mem::size_of::<usize>(),
                )?;
                Shape::Cell(
                    self.catalog.id(tr::CatalogItem::Cell),
                    Arc::new(child(argument)?),
                )
            }
            Type::FnPointer { inputs, output } => {
                let mut mapped = ir::FnPointerTy::storage(inputs.len(), &self.budget)?;
                for input in inputs {
                    mapped.push(child(input)?);
                }
                let output = child(output)?;
                self.budget.retain_bytes(
                    std::mem::size_of::<Shape>() + 2 * std::mem::size_of::<usize>(),
                )?;
                Shape::FnPointer(
                    ir::FnPointerTy::slice(mapped, &self.budget)?,
                    Arc::new(output),
                )
            }
            Type::Tuple(types) => {
                let mut fields = Vec::new();
                fields
                    .try_reserve_exact(types.len())
                    .map_err(|_| "source recipe tuple allocation failed")?;
                self.budget.retain_bytes(
                    types
                        .len()
                        .checked_mul(std::mem::size_of::<Shape>())
                        .ok_or("source recipe tuple size overflow")?,
                )?;
                for ty in types {
                    fields.push(child(ty)?);
                }
                Shape::Tuple(fields.into())
            }
            Type::Named(path) => {
                let declaration=*self.registry.nominals.get(&name_scope::path_key(self.registry.names.as_ref(),path,Namespace::Type)?)
                    .ok_or("unsupported: source recipe path is not an authenticated nominal declaration")?;
                let args = self.nominal_source_recipe_arguments(
                    declaration,
                    Vec::new(),
                    bindings,
                    depth + 1,
                    active,
                )?;
                Shape::Nominal(
                    declaration,
                    source_recipe_argument_backing(args, &self.budget)?,
                )
            }
            Type::Apply(path) => {
                if path.segments.iter().any(|s| !s.associated_types.is_empty()) {
                    return Err(
                        "unsupported: associated equalities in nominal source recipe".into(),
                    );
                }
                let (at, segment) = path
                    .segments
                    .iter()
                    .enumerate()
                    .find(|(_, s)| !s.args.is_empty() || !s.region_args.is_empty())
                    .ok_or("nominal source recipe lacks its written argument segment")?;
                if at + 1 != path.segments.len() {
                    return Err("unsupported: source recipe generic associated type".into());
                }
                let declaration = *self
                    .registry
                    .nominals
                    .get(&name_scope::path_key(
                        self.registry.names.as_ref(),
                        &path.path(self.registry.names.as_ref())?,
                        Namespace::Type,
                    )?)
                    .ok_or(
                        "unsupported: source recipe generic path is not a nominal declaration",
                    )?;
                self.registry
                    .nominal_source_arity(declaration, segment.args.len())?;
                self.budget.retain_bytes(
                    segment
                        .args
                        .len()
                        .checked_mul(std::mem::size_of::<Shape>())
                        .ok_or("nominal recipe actual size overflow")?,
                )?;
                let mut args = Vec::new();
                args.try_reserve_exact(segment.args.len())
                    .map_err(|_| "infrastructure: nominal recipe actual allocation failed")?;
                for (index, ty) in segment.args.iter().enumerate() {
                    args.push(child(ty).map_err(|error| {
                        format!("{error}; nominal={declaration:?} argument={index}")
                    })?);
                }
                let args = self.nominal_source_recipe_arguments(
                    declaration,
                    args,
                    bindings,
                    depth + 1,
                    active,
                )?;
                Shape::Nominal(
                    declaration,
                    source_recipe_argument_backing(args, &self.budget)?,
                )
            }
            Type::Projection(_) => {
                // Reuse retained-occurrence authentication. The resulting shape
                // describes a demand only: realization still requires graph reads.
                let mut state = self.scratch();
                state.source_context = Some(self.registry.source_type_context(declaration)?);
                let info = self.registry.declaration(declaration)?;
                let lowered = state.source_ty(
                    &self.registry,
                    ty,
                    bindings,
                    ir::FactsCtx::Universal(&info.env),
                    &self.budget,
                    depth + 1,
                    true,
                )?;
                super::super::nominal_realization::source_shape(&lowered, &self.budget, depth + 1)
                    .map_err(|error| error.to_string())?
            }
            Type::Dyn(path) => {
                // A closed principal is a rigid leaf: its actuals and associated
                // assignments are lowered through the canonical dynamic metadata
                // path and admitted only when no parameter or inference remains.
                let lowered = {
                    let mut state = self.scratch();
                    state.source_dynamic_type(
                        &self.registry,
                        path,
                        bindings,
                        ir::FactsCtx::Concrete,
                        &self.budget,
                    )?
                };
                match lowered {
                    ir::Ty::Dyn(object) if object.is_closed_leaf() => {
                        Shape::Leaf(ir::Ty::Dyn(object))
                    }
                    _ => return Err("unsupported: generic dynamic source nominal recipe".into()),
                }
            }
            Type::Opaque(_) => return Err("unsupported: opaque source nominal recipe".into()),
            Type::Infer => {
                return Err(
                    "unsupported: unresolved source type recipe requires solver-owned inference"
                        .into(),
                );
            }
        })
    }
}

// Vec storage and its immutable transfer coexist. Even an empty slice pays
// the Arc header, rather than assuming zero-element means zero allocation.
fn source_recipe_argument_backing(
    args: Vec<Shape>,
    budget: &ir::PlanBudget,
) -> Result<Arc<[Shape]>, String> {
    let bytes = args
        .len()
        .checked_mul(std::mem::size_of::<Shape>())
        .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
        .ok_or("nominal source recipe Arc storage overflow")?;
    budget.retain_bytes(bytes)?;
    Ok(args.into())
}

/// This rejects visible default carrier demands only. Nominal field carriers
/// still require the unchanged source-region and realization proof services.
fn reject_default_recipe_carriers(
    shape: &Shape,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    fn visit(
        shape: &Shape,
        budget: &ir::PlanBudget,
        depth: usize,
        owning: bool,
    ) -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        match shape {
            Shape::Leaf(ty) => {
                if owning && !ir::carrier_locations(ty, budget)?.is_empty() {
                    return Err("unsupported: nominal default recipe requires original carrier-family source transport".into());
                }
            }
            Shape::Nominal(_, arguments) | Shape::Tuple(arguments) => {
                for argument in arguments.iter() {
                    visit(argument, budget, depth + 1, owning)?;
                }
            }
            Shape::PhantomData(_, argument) => visit(argument, budget, depth + 1, false)?,
            Shape::ManuallyDrop(_, argument) | Shape::Cell(_, argument) => {
                visit(argument, budget, depth + 1, owning)?
            }
            Shape::FnPointer(inputs, output) => {
                // Signature types are metadata, not stored values owned by the
                // pointer. Still visit every occurrence for bounded traversal.
                // Actual ingress and pointer validation remain mandatory.
                for input in inputs.iter() {
                    visit(input, budget, depth + 1, false)?;
                }
                visit(output, budget, depth + 1, false)?;
            }
            Shape::Array(element, _) => visit(element, budget, depth + 1, owning)?,
            Shape::Ref(_, inner) | Shape::RawPtr(_, inner) | Shape::Slice(inner) if !owning => {
                visit(inner, budget, depth + 1, false)?;
            }
            Shape::Projection {
                receiver,
                arguments,
                ..
            } if !owning => {
                visit(receiver, budget, depth + 1, false)?;
                for argument in arguments.iter() {
                    visit(argument, budget, depth + 1, false)?;
                }
            }
            Shape::Ref(..) | Shape::RawPtr(..) | Shape::Slice(_) | Shape::Projection { .. } => {
                return Err("unsupported: nominal default recipe requires broader original source transport".into());
            }
        }
        Ok(())
    }
    visit(shape, budget, depth, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pointer_signature_walk_is_nonowning_but_visits_inputs_and_output() {
        let raw = Shape::RawPtr(ir::BorrowKind::Shared, Arc::new(Shape::Leaf(ir::Ty::Bool)));
        let shape = Shape::FnPointer(Arc::from([raw.clone()]), Arc::new(raw.clone()));
        let budget = ir::PlanBudget::default();
        reject_default_recipe_carriers(&shape, &budget, 0).unwrap();
        assert!(
            reject_default_recipe_carriers(&raw, &budget, 0)
                .unwrap_err()
                .contains("unsupported")
        );
        // Root=0, raw input=1, its primitive=2, raw output=1 and
        // its primitive=2. Neither signature side may skip the meter.
        let shallow = ir::PlanBudget::new(ir::PlanLimits {
            depth: 1,
            ..Default::default()
        });
        assert!(reject_default_recipe_carriers(&shape, &shallow, 0).is_err());
        let output_only = Shape::FnPointer(Arc::from([]), Arc::new(raw));
        let shallow = ir::PlanBudget::new(ir::PlanLimits {
            depth: 1,
            ..Default::default()
        });
        assert!(reject_default_recipe_carriers(&output_only, &shallow, 0).is_err());
    }

    #[test]
    fn source_recipe_arc_backing_is_admitted_independently_even_when_empty() {
        for count in [0, 2] {
            let bytes = count * std::mem::size_of::<Shape>() + 2 * std::mem::size_of::<usize>();
            let budget = ir::PlanBudget::new(ir::PlanLimits {
                bytes: bytes - 1,
                ..Default::default()
            });
            let args = vec![Shape::Leaf(ir::Ty::Bool); count];
            assert!(source_recipe_argument_backing(args, &budget).is_err());
            let failure = budget.first_resource_failure().unwrap();
            assert_eq!(failure.kind, ir::ResourceBudgetKind::Bytes);
            assert_eq!((failure.used, failure.requested), (0, bytes));
        }
    }

    #[test]
    fn source_default_recipes_preserve_nominal_identity_and_order_without_raw_arity_relaxation() {
        let source = parse("struct Pair<A=i32,B=A,C=B>{a:A,b:B,c:C}struct Required<T,U=i32>{a:T,b:U}fn inputs(a:Pair,b:Pair<i32>,c:Pair<i32,i32,i32>){}fn context<T>(x:T){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let owner = plan.function_decl(0).unwrap();
        let recipes = source.functions[0]
            .params
            .iter()
            .map(|(_, ty)| plan.nominal_source_shape(owner, ty).unwrap().unwrap())
            .collect::<Vec<_>>();
        let mut meter = |depth| {
            budget.check_depth(depth)?;
            budget.charge_work(1)
        };
        assert!(recipes[0].same_with(&recipes[1], 0, &mut meter).unwrap());
        assert!(recipes[1].same_with(&recipes[2], 0, &mut meter).unwrap());
        let pair = plan.nominal_declaration(0).unwrap();
        let Shape::Nominal(actual, arguments) = &recipes[0] else {
            panic!("lost nominal recipe")
        };
        assert_eq!(*actual, pair);
        assert_eq!(arguments.len(), 3);
        assert!(
            arguments
                .iter()
                .all(|argument| matches!(argument, Shape::Leaf(ir::Ty::Int(ir::IntTy::I32))))
        );
        let bindings = HashMap::new();
        let active = std::cell::RefCell::new(Vec::new());
        assert!(
            plan.nominal_source_recipe_arguments(
                pair,
                vec![Shape::Leaf(ir::Ty::Bool); 4],
                &bindings,
                0,
                &active
            )
            .is_err()
        );
        let required = plan.nominal_declaration(1).unwrap();
        assert!(
            plan.nominal_source_recipe_arguments(required, Vec::new(), &bindings, 0, &active)
                .is_err()
        );
        let facts = plan.callable_facts(plan.function_decl(2).unwrap()).unwrap();
        assert!(
            plan.intern_nominal(
                pair,
                &[ir::Ty::Int(ir::IntTy::I32)],
                ir::FactsCtx::Validated(&facts),
                &budget
            )
            .is_err()
        );
        let generic = plan
            .function_schema(plan.function_decl(1).unwrap())
            .unwrap();
        let rigid = Shape::Leaf(generic.signature.params[0].clone());
        let error = plan
            .nominal_source_recipe_arguments(pair, vec![rigid], &bindings, 0, &active)
            .unwrap_err();
        assert!(error.contains("unsupported"), "{error}");
        let foreign = parse("struct Pair<A=i32>{a:A}fn input(x:Pair){}fn main(){}").unwrap();
        assert!(
            plan.nominal_source_shape(owner, &foreign.functions[0].params[0].1)
                .is_err()
        );
    }

    #[test]
    fn source_default_recipe_does_not_grant_hidden_nominal_carrier_permissions() {
        let source = parse("struct Borrowed<'a>{value:&'a i32}struct Cell<T=Borrowed<'static>>{value:T}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let declaration = plan.nominal_declaration(1).unwrap();
        let bindings = HashMap::new();
        let active = std::cell::RefCell::new(Vec::new());
        let demand = plan
            .nominal_source_recipe_arguments(declaration, Vec::new(), &bindings, 0, &active)
            .unwrap();
        assert!(matches!(demand.as_slice(), [Shape::Nominal(_, _)]));
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let error = plan
            .complete_nominal_source_arguments(
                declaration,
                Vec::new(),
                ir::FactsCtx::Validated(&facts),
                &budget,
            )
            .unwrap_err();
        assert!(
            error.contains("unsupported") && error.contains("carrier"),
            "{error}"
        );
    }
}
