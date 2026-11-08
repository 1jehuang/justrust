//! Bootstrap admission authenticates a nominal head, never its executable layout.
use super::*;

pub(super) fn source_header(
    plan: &PlanCtx,
    implementation: usize,
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, String> {
    plan.check_budget(budget)?;
    let block = plan
        .program
        .impls
        .get(implementation)
        .ok_or("missing original impl header")?;
    let declaration = *plan
        .registry
        .impl_ids
        .get(implementation)
        .ok_or("missing original impl declaration")?;
    let info = plan.declaration(declaration)?;
    if info.kind != ir::DeclKind::TraitImpl || block.trait_path.is_none() {
        return Err("source trait header requires original trait impl".into());
    }
    let bindings = generic_bindings(&plan.registry, &info.env, budget)?;
    let caller = if info.env.is_generic() {
        ir::FactsCtx::Universal(&info.env)
    } else {
        ir::FactsCtx::Concrete
    };
    let mut state = plan.scratch();
    state.source_context = Some(plan.registry.source_type_context(declaration)?);
    let original = state.source_ty(
        &plan.registry,
        &block.self_ty,
        &bindings,
        caller,
        budget,
        0,
        true,
    )?;
    authenticate(&mut state, &plan.registry, &original, caller, budget)?;
    caller.validate_ty_metadata_with_budget(&original, budget)?;
    // This is retained header metadata only. build_with must still validate
    // the exact owning predicates and well-formedness before any callable is
    // published. Public source_ty admission remains projection-free without
    // an authoritative world normalization context.
    Ok(original)
}

pub(super) fn authenticate(
    state: &mut PlanState,
    registry: &DeclRegistry,
    original: &ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    if !raw_type_has_projection(original, budget, 0)? {
        return Ok(());
    }
    let (declaration, actuals, id) = match original {
        ir::Ty::Struct(value) => (
            value.def().source_decl,
            &value.def().type_args,
            value.def().id,
        ),
        ir::Ty::Enum(value) => (
            value.def().source_decl,
            &value.def().type_args,
            value.def().id,
        ),
        _ => return Err("unsupported: projection-bearing impl heads".into()),
    };
    for actual in actuals.iter() {
        budget.charge_work(1)?;
        if raw_type_has_projection(actual, budget, 0)? {
            return Err("unsupported: projection-bearing impl head actuals".into());
        }
    }
    let info = registry.declaration(declaration)?;
    budget.charge_work(actuals.len())?;
    let _actual_storage = budget.reserve_bytes(
        actuals
            .len()
            .checked_mul(std::mem::size_of::<ir::Ty>())
            .ok_or("staged impl actual storage overflow")?,
    )?;
    if !matches!(info.kind, ir::DeclKind::Struct | ir::DeclKind::Enum)
        || info.type_params.len() != actuals.len()
        || id.snapshot != declaration.snapshot
        || id.index.source_slot()?
            != nominal_schema::canonical_slot(&(declaration, actuals.to_vec()))
    {
        return Err("noncanonical staged impl nominal identity".into());
    }
    // Reapply only the original declaration and ordered actuals. No supplied
    // field can become a seed or an executable type proof through this hook.
    let canonical = state.intern(registry, declaration, actuals, caller, budget, 0)?;
    let same = match (original, &canonical) {
        (ir::Ty::Struct(a), ir::Ty::Struct(b)) => same_struct_def(a.def(), b.def(), budget)?,
        (ir::Ty::Enum(a), ir::Ty::Enum(b)) => same_enum_def(a.def(), b.def(), budget)?,
        _ => false,
    };
    if !same {
        return Err("staged impl nominal changed original source fields".into());
    }
    Ok(())
}

impl PlanCtx {
    pub(super) fn match_inherent_head(
        &self,
        item: ir::DeclId,
        receiver: &ir::Ty,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<Option<HashMap<ir::ParamId, ir::Ty>>, String> {
        self.check_budget(budget)?;
        let associated = self
            .declaration(item)?
            .path
            .associated
            .as_ref()
            .ok_or("inherent inference requires original associated declaration")?;
        if associated.trait_decl.is_some() {
            return Err("inherent inference changed original implementation kind".into());
        }
        let env = self.param_env(associated.impl_decl)?;
        let pattern = &associated.self_pattern;
        let mut bindings = HashMap::new();
        if !raw_type_has_projection(pattern, budget, 0)?
            && !raw_type_has_projection(receiver, budget, 0)?
        {
            return Ok(
                match_impl_type(pattern, receiver, &env, &mut bindings, budget, 0)?
                    .then_some(bindings),
            );
        }
        let facts = caller
            .validated()
            .ok_or("unsupported: projected inherent receiver requires validated caller facts")?;
        if !facts.budget().same_authority(budget)
            || !Arc::ptr_eq(facts.world(), &self.trait_world()?)
        {
            return Err("inherent inference changed validated caller authority".into());
        }
        facts
            .validate_ty_metadata(receiver)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        authenticate(
            &mut self.scratch(),
            &self.registry,
            pattern,
            ir::FactsCtx::Universal(&self.declaration(associated.impl_decl)?.env),
            budget,
        )?;
        let mut solver =
            super::super::source_replay::CallInferenceState::new(Some(env.clone()), budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let schema = solver
            .schema(pattern, 0)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let actual = solver
            .type_term(receiver, None, 0)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        if let Err(error) = solver.constrain(&schema, &actual, 0) {
            if error.kind() == tr::FactsErrorKind::InvalidMetadata {
                return Ok(None);
            }
            return Err(error.into_legacy_diagnostic(budget));
        }
        // A receiver constrains existing input subterms only. Retain their
        // authenticated handles rather than issuing a replacement layout or
        // reconstructing a parent-local receipt while resolving a slot.
        let values = solver
            .finish(false, &mut |declaration, arguments| {
                original_receiver_nominal(receiver, declaration, arguments, budget, 0)
                .map_err(|e| tr::FactsError::from_message(budget, &e))?
                .ok_or_else(|| tr::FactsError::invalid(budget,
                    "inherent inference requested a nominal absent from its original receiver"))
            })
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let Some(values) = values else {
            return Err(
                "unsupported: inherent impl arguments are not determined by its receiver".into(),
            );
        };
        if values.len() != env.arity() {
            return Err("inherent inference changed original impl argument arity".into());
        }
        for (index, value) in values.into_iter().enumerate() {
            let parameter = ir::ParamId {
                owner: env.binder(),
                index: index as u16,
            };
            caller.validate_ty_metadata_with_budget(&value, budget)?;
            budget.retain_bytes(256)?;
            bindings.insert(parameter, value);
        }
        Ok(Some(bindings))
    }
}

fn original_receiver_nominal(
    receiver: &ir::Ty,
    declaration: ir::DeclId,
    arguments: &[ir::Ty],
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<ir::Ty>, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let application = match receiver {
        ir::Ty::Struct(value) => Some((value.def().source_decl, value.def().type_args.as_ref())),
        ir::Ty::Enum(value) => Some((value.def().source_decl, value.def().type_args.as_ref())),
        ir::Ty::Cell(value) => Some((value.declaration(), std::slice::from_ref(value.argument()))),
        ir::Ty::ManuallyDrop(value) => {
            Some((value.declaration(), std::slice::from_ref(value.argument())))
        }
        ir::Ty::PhantomData(value) => {
            Some((value.declaration(), std::slice::from_ref(value.argument())))
        }
        _ => None,
    };
    if let Some((owner, actuals)) = application {
        if owner == declaration && actuals.len() == arguments.len() {
            let mut equal = true;
            for (a, b) in actuals.iter().zip(arguments) {
                equal &= metered_type_equal(a, b, budget, depth + 1)?;
            }
            if equal {
                return Ok(Some(receiver.clone()));
            }
        }
        return original_receiver_nominals(
            actuals.iter(),
            declaration,
            arguments,
            budget,
            depth + 1,
        );
    }
    match receiver {
        ir::Ty::Ref {
            referent: child, ..
        }
        | ir::Ty::RawPtr { pointee: child, .. }
        | ir::Ty::Array { element: child, .. }
        | ir::Ty::Slice { element: child }
        | ir::Ty::DependentArray { element: child, .. } => {
            original_receiver_nominal(child, declaration, arguments, budget, depth + 1)
        }
        ir::Ty::Tuple(children) => {
            original_receiver_nominals(children.iter(), declaration, arguments, budget, depth + 1)
        }
        ir::Ty::AssociatedType(projection) => original_receiver_nominals(
            std::iter::once(&projection.trait_ref.self_ty).chain(projection.trait_ref.args.iter()),
            declaration,
            arguments,
            budget,
            depth + 1,
        ),
        ir::Ty::Closure(value) => {
            original_receiver_nominals(value.types(), declaration, arguments, budget, depth + 1)
        }
        ir::Ty::Opaque(value) => {
            original_receiver_nominals(value.types(), declaration, arguments, budget, depth + 1)
        }
        ir::Ty::FnPointer(value) => original_receiver_nominals(
            value.inputs.iter().chain(std::iter::once(&value.output)),
            declaration,
            arguments,
            budget,
            depth + 1,
        ),
        ir::Ty::FnItem(value) => original_receiver_nominals(
            value.type_args.iter(),
            declaration,
            arguments,
            budget,
            depth + 1,
        ),
        _ => Ok(None),
    }
}

fn original_receiver_nominals<'a>(
    children: impl Iterator<Item = &'a ir::Ty>,
    declaration: ir::DeclId,
    arguments: &[ir::Ty],
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<ir::Ty>, String> {
    for child in children {
        if let Some(value) =
            original_receiver_nominal(child, declaration, arguments, budget, depth)?
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_inherent_receiver_retains_checked_inputs_and_rejects_foreign_facts() {
        let program = parse("trait Has{type Out;}struct Wrap<T>{value:T}impl<T> Wrap<T>{fn keep(self)->Self{self}}fn original<U:Has>(value:Wrap<U::Out>){}fn other<U:Has>(value:Wrap<U::Out>){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let owner = plan.function_decl(0).unwrap();
        let facts = plan.callable_facts(owner).unwrap();
        let schema = plan.raw_schema(owner).unwrap();
        let receiver = &schema.signature.params[0];
        let item = *plan
            .registry
            .function_ids
            .iter()
            .find(|item| plan.declaration(**item).unwrap().path.associated.is_some())
            .unwrap();
        let associated = plan
            .declaration(item)
            .unwrap()
            .path
            .associated
            .as_ref()
            .unwrap();
        let env = plan.param_env(associated.impl_decl).unwrap();
        assert!(
            match_impl_type(
                &associated.self_pattern,
                receiver,
                &env,
                &mut HashMap::new(),
                &budget,
                0
            )
            .is_err()
        );
        let bindings = plan
            .match_inherent_head(item, receiver, ir::FactsCtx::Validated(&facts), &budget)
            .unwrap()
            .unwrap();
        let (_, actuals) = nominal_parts(receiver).unwrap();
        assert_eq!(
            bindings.get(&ir::ParamId {
                owner: env.binder(),
                index: 0
            }),
            actuals.first()
        );
        assert!(
            plan.match_inherent_head(
                item,
                receiver,
                ir::FactsCtx::Universal(&schema.env),
                &budget
            )
            .is_err()
        );
        let other = plan.callable_facts(plan.function_decl(1).unwrap()).unwrap();
        assert!(
            plan.match_inherent_head(item, receiver, ir::FactsCtx::Validated(&other), &budget)
                .is_err()
        );
        let ir::Ty::Struct(value) = receiver else {
            panic!("missing Wrap receiver")
        };
        let mut forged = value.def().clone();
        let mut fields = forged.fields.read(&budget).unwrap().to_vec();
        fields[0].ty = ir::Ty::Bool;
        forged.fields = Arc::<[ir::FieldDef]>::from(fields).into();
        let forged = ir::Ty::Struct(ir::StructTy::new(forged));
        assert!(
            plan.match_inherent_head(item, &forged, ir::FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
        assert!(budget.charge_work(usize::MAX).is_err());
        assert!(
            plan.match_inherent_head(item, receiver, ir::FactsCtx::Validated(&facts), &budget)
                .is_err()
        );
    }

    #[test]
    fn source_header_requires_original_trait_impl_and_planning_authority() {
        let program = parse("trait Mark{}struct Store<T>{value:T}impl<T> Mark for Store<T>{}impl<T> Store<T>{fn keep(self)->Self{self}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let header = source_header(&plan, 0, &budget).unwrap();
        assert!(matches!(header, ir::Ty::Struct(_)));
        assert!(source_header(&plan, 1, &budget).is_err());
        assert!(source_header(&plan, usize::MAX, &budget).is_err());
        assert!(source_header(&plan, 0, &ir::PlanBudget::default()).is_err());
    }

    #[test]
    fn staged_projected_storage_requires_original_source_fields_and_identity() {
        let program = parse("trait Has{type Out;type Other;}struct Store<T:Has>{value:T::Out}fn original<T:Has>(x:Store<T>){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let schema = plan.raw_schema(plan.function_decl(0).unwrap()).unwrap();
        let original = &schema.signature.params[0];
        let caller = ir::FactsCtx::Universal(&schema.env);
        authenticate(
            &mut plan.scratch(),
            &plan.registry,
            original,
            caller,
            &budget,
        )
        .unwrap();
        let foreign_program = parse("trait Has{type Out;type Other;}struct Store<T:Has>{value:T::Out}fn original<T:Has>(x:Store<T>){}fn main(){}").unwrap();
        let foreign_budget = ir::PlanBudget::default();
        let foreign_plan = PlanCtx::from_program(&foreign_program, &foreign_budget).unwrap();
        let foreign_schema = foreign_plan
            .raw_schema(foreign_plan.function_decl(0).unwrap())
            .unwrap();
        assert!(
            authenticate(
                &mut plan.scratch(),
                &plan.registry,
                &foreign_schema.signature.params[0],
                caller,
                &budget,
            )
            .is_err()
        );
        assert!(
            authenticate(
                &mut plan.scratch(),
                &plan.registry,
                original,
                ir::FactsCtx::Universal(&foreign_schema.env),
                &budget,
            )
            .is_err()
        );
        let ir::Ty::Struct(value) = original else {
            panic!("missing Store application")
        };
        let fields = value.def().fields.read(&budget).unwrap();
        let ir::Ty::AssociatedType(projection) = &fields[0].ty else {
            panic!("missing source projection")
        };
        let mut altered_projection = projection.as_ref().clone();
        let trait_index = plan
            .registry
            .trait_ids
            .iter()
            .position(|id| *id == projection.trait_ref.declaration)
            .unwrap();
        altered_projection.item = plan.registry.trait_associated_types[trait_index][1];
        let mut definition = value.def().clone();
        let mut altered_fields = fields.to_vec();
        altered_fields[0].ty = ir::Ty::AssociatedType(Arc::new(altered_projection));
        definition.fields = Arc::<[ir::FieldDef]>::from(altered_fields).into();
        let forged = ir::Ty::Struct(ir::StructTy::new(definition));
        assert!(
            authenticate(
                &mut plan.scratch(),
                &plan.registry,
                &forged,
                caller,
                &budget
            )
            .is_err()
        );
        let mut definition = value.def().clone();
        definition.type_args = Arc::from([ir::Ty::Bool]);
        let forged = ir::Ty::Struct(ir::StructTy::new(definition));
        assert!(
            authenticate(
                &mut plan.scratch(),
                &plan.registry,
                &forged,
                caller,
                &budget
            )
            .is_err()
        );
        let mut definition = value.def().clone();
        definition.source_decl = plan.function_decl(0).unwrap();
        let forged = ir::Ty::Struct(ir::StructTy::new(definition));
        assert!(
            authenticate(
                &mut plan.scratch(),
                &plan.registry,
                &forged,
                caller,
                &budget
            )
            .is_err()
        );
    }
}
