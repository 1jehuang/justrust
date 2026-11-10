use super::*;

/// One typed signature reconstruction owns these occurrence correspondences.
/// Original families remain semantic identities. Only their carrier expansion
/// uses the independently validated mapped occurrence, never a reminted field.
#[derive(Default)]
pub(in crate::compiler::traits) struct CarrierFamilyActuals {
    values: Vec<(ir::CarrierFamily, Ty)>,
}

impl CarrierFamilyActuals {
    pub(in crate::compiler::traits) fn has_array_correspondence(
        &self,
        world: &ValidatedTraitWorld,
        original: &Ty,
        mapped: &Ty,
        depth: usize,
    ) -> Result<bool> {
        charged(world.budget(), world.budget().check_depth(depth))?;
        for (family, actual) in &self.values {
            charged(world.budget(), world.budget().charge_work(1))?;
            let ir::CarrierFamily::Array(source) = family else {
                continue;
            };
            if world.eq_ty(source, original, depth + 1)?
                && world.eq_ty(actual, mapped, depth + 1)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(in crate::compiler::traits) fn from_signature(
        world: &ValidatedTraitWorld,
        raw: &GenericSchema,
        arguments: &[Ty],
        signature: &Signature,
    ) -> Result<Self> {
        if raw.signature.params.len() != signature.params.len()
            || arguments.len() != raw.env.total_arity()
        {
            return Err(FactsError::invalid(
                world.budget(),
                "mapped family signature arity changed",
            ));
        }
        let mut result = Self::default();
        let mut found = Vec::new();
        for _ in arguments {
            push(world.budget(), &mut found, None)?;
        }
        let mut visited = Vec::new();
        for (original, mapped) in raw
            .signature
            .params
            .iter()
            .zip(&signature.params)
            .chain(std::iter::once((&raw.signature.result, &signature.result)))
        {
            storage_occurrences_in(
                world,
                &raw.env,
                arguments,
                original,
                mapped,
                &mut found,
                &mut visited,
                0,
                &mut |original, mapped| {
                    let key = match original {
                        Ty::AssociatedType(value) => ir::CarrierFamily::Projection(value.clone()),
                        Ty::Opaque(value) => ir::CarrierFamily::Opaque(value.clone()),
                        Ty::DependentArray { .. } => {
                            ir::CarrierFamily::Array(Arc::new(original.clone()))
                        }
                        _ => unreachable!(),
                    };
                    // Keep each typed occurrence until semantic realization.
                    // A source field may still spell the same instantiated
                    // associated type that another occurrence has normalized.
                    push(world.budget(), &mut result.values, (key, mapped.clone()))
                },
            )?;
        }
        Ok(result)
    }

    fn same_family(
        world: &ValidatedTraitWorld,
        left: &ir::CarrierFamily,
        right: &ir::CarrierFamily,
    ) -> Result<bool> {
        charged(world.budget(), world.budget().charge_work(1))?;
        match (left, right) {
            (ir::CarrierFamily::Projection(a), ir::CarrierFamily::Projection(b)) => {
                world.same_projection(a, b)
            }
            (ir::CarrierFamily::Opaque(a), ir::CarrierFamily::Opaque(b)) => {
                world.eq_ty(&Ty::Opaque(a.clone()), &Ty::Opaque(b.clone()), 0)
            }
            _ => Ok(left == right),
        }
    }

    pub(in crate::compiler::traits) fn actual(
        &self,
        world: &ValidatedTraitWorld,
        family: &ir::CarrierFamily,
        logical: Ty,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Ty> {
        world.check_facts(facts, true)?;
        let mut selected: Option<(usize, Ty)> = None;
        for (original, mapped) in &self.values {
            if Self::same_family(world, original, family)? {
                // The signature authenticates each occurrence, but source
                // fields can retain projections. Compare their normal forms
                // under this exact caller, not an uninstantiated spelling.
                let mapped = if world.type_contains_projection(mapped, 0)? {
                    world.normalize_carrier_equality_type_with_scratch(mapped, facts, scratch)?
                } else {
                    mapped.clone()
                };
                if !world.eq_ty(&logical, &mapped, 0)? {
                    return Err(FactsError::invalid(
                        world.budget(),
                        "mapped family has conflicting typed occurrences or changed its semantic actual",
                    ));
                }
                let mut mappings = Vec::new();
                world.collect_nominal_mappings_with_actuals(
                    &logical,
                    &mapped,
                    &mut mappings,
                    self,
                    0,
                )?;
                charged(world.budget(), world.budget().charge_work(mappings.len()))?;
                let graph_nodes = mappings
                    .iter()
                    .filter(|mapping| mapping.instantiated.index.is_graph())
                    .count();
                if selected
                    .as_ref()
                    .is_none_or(|(previous, _)| graph_nodes > *previous)
                {
                    selected = Some((graph_nodes, mapped));
                }
            }
        }
        Ok(selected.map_or(logical, |(_, mapped)| mapped))
    }
}

impl NominalCarrierAuthority {
    /// Recover storage representations only from occurrences in the exact
    /// authenticated mapped signature. Canonical declaration actuals remain
    /// source identities on the public view and on every entry receipt.
    pub(in crate::compiler::traits) fn storage_arguments(
        &self,
        facts: &ValidatedFacts,
    ) -> Result<Arc<[Ty]>> {
        facts.world().carrier_storage_arguments(
            self.canonical.declaration,
            &self.canonical.arguments,
            &self.signature,
        )
    }
}

impl ValidatedTraitWorld {
    pub(in crate::compiler::traits) fn carrier_storage_arguments(
        &self,
        declaration: DeclId,
        source_arguments: &[Ty],
        signature: &Signature,
    ) -> Result<Arc<[Ty]>> {
        let world = self;
        let raw = world.normalized_declaration_schema(declaration)?;
        let mut found = Vec::new();
        for _ in source_arguments {
            push(world.budget(), &mut found, None)?;
        }
        let mut visited = Vec::new();
        for (original, mapped) in raw
            .signature
            .params
            .iter()
            .zip(&signature.params)
            .chain(std::iter::once((&raw.signature.result, &signature.result)))
        {
            storage_occurrences(
                world,
                &raw.env,
                source_arguments,
                original,
                mapped,
                &mut found,
                &mut visited,
                0,
            )?;
        }
        let mut arguments = Vec::new();
        for (original, mapped) in source_arguments.iter().zip(found) {
            push(
                world.budget(),
                &mut arguments,
                mapped.unwrap_or_else(|| original.clone()),
            )?;
        }
        Ok(arguments.into())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn storage_occurrences(
    world: &ValidatedTraitWorld,
    env: &GenericEnv,
    arguments: &[Ty],
    original: &Ty,
    mapped: &Ty,
    found: &mut [Option<Ty>],
    visited: &mut Vec<(ir::NominalAppId, ir::NominalAppId)>,
    depth: usize,
) -> Result<()> {
    storage_occurrences_in(
        world,
        env,
        arguments,
        original,
        mapped,
        found,
        visited,
        depth,
        &mut |_, _| Ok(()),
    )
}

fn mapped_array_extent(
    env: &GenericEnv,
    arguments: &[Ty],
    length: ir::ConstParamId,
    mapped: &Ty,
    budget: &ir::PlanBudget,
) -> Result<bool> {
    let argument = env
        .array_length_argument(length, arguments, budget)
        .map_err(|error| FactsError::invalid(budget, &error))?;
    match (argument, mapped) {
        (Ty::Const(value), Ty::Array { len, .. }) => {
            value
                .closed_key()
                .map_err(|error| FactsError::invalid(budget, &error))?;
            Ok(
                value.closed_type() == Some(ir::ConstType::Int(ir::IntTy::Usize))
                    && value.as_usize() == Some(*len),
            )
        }
        (Ty::Param(actual), Ty::DependentArray { length: mapped, .. }) => {
            Ok(actual.owner == mapped.owner && actual.index == mapped.index)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
#[test]
fn const_impl_storage_extents_keep_source_owner_kind_slot_and_mapped_length() {
    let program = crate::compiler::frontend::parse("fn probe<T,const N:usize,const M:usize>(){}fn main(){}fn arrays<T,const N:usize>(x:[T;N])->[T;N]{x}").unwrap();
    let budget = ir::PlanBudget::default();
    let context = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let scope = context
        .param_env(context.callable_decl(0).unwrap())
        .unwrap();
    let env = GenericEnv::new(&[scope.clone()], &budget).unwrap();
    let length = ir::ConstParamId {
        owner: scope.binder(),
        index: 1,
    };
    let value = |bits| {
        Ty::Const(ir::ConstArgument::Integer {
            ty: ir::IntTy::Usize,
            bits,
        })
    };
    let arguments = [Ty::Bool, value(2), value(3)];
    let mapped = |len| Ty::Array {
        element: Arc::new(Ty::Bool),
        len,
    };
    assert!(mapped_array_extent(&env, &arguments, length, &mapped(2), &budget).unwrap());
    assert!(!mapped_array_extent(&env, &arguments, length, &mapped(3), &budget).unwrap());
    let mut wrong_kind = arguments.clone();
    wrong_kind[1] = Ty::Const(ir::ConstArgument::Integer {
        ty: ir::IntTy::U8,
        bits: 2,
    });
    assert!(!mapped_array_extent(&env, &wrong_kind, length, &mapped(2), &budget).unwrap());
    assert!(mapped_array_extent(&env, &arguments[..2], length, &mapped(2), &budget).is_err());
    assert!(
        mapped_array_extent(
            &env,
            &arguments,
            ir::ConstParamId { index: 0, ..length },
            &mapped(2),
            &budget
        )
        .is_err()
    );
    let foreign = context
        .param_env(context.callable_decl(1).unwrap())
        .unwrap();
    assert!(
        mapped_array_extent(
            &env,
            &arguments,
            ir::ConstParamId {
                owner: foreign.binder(),
                index: 0
            },
            &mapped(2),
            &budget
        )
        .is_err()
    );
    let mut forwarded = arguments.clone();
    forwarded[1] = Ty::Param(ir::ParamId {
        owner: scope.binder(),
        index: 2,
    });
    let symbolic = |index| Ty::DependentArray {
        element: Arc::new(Ty::Bool),
        length: ir::ConstParamId {
            owner: scope.binder(),
            index,
        },
    };
    assert!(mapped_array_extent(&env, &forwarded, length, &symbolic(2), &budget).unwrap());
    assert!(!mapped_array_extent(&env, &forwarded, length, &symbolic(1), &budget).unwrap());
    let world = context.trait_world().unwrap();
    let raw = world
        .schema
        .declaration_schema(context.function_decl(2).unwrap())
        .unwrap();
    let original = &raw.signature.params[0];
    let mut signature = raw.signature.clone();
    for len in [0, 2] {
        signature.params[0] = mapped(len);
        signature.result = mapped(len);
        let actuals = CarrierFamilyActuals::from_signature(
            &world,
            &raw,
            &[Ty::Bool, value(len as u128)],
            &signature,
        )
        .unwrap();
        assert!(
            actuals
                .has_array_correspondence(&world, original, &mapped(len), 0)
                .unwrap()
        );
        world
            .collect_nominal_mappings_with_actuals(
                original,
                &mapped(len),
                &mut Vec::new(),
                &actuals,
                0,
            )
            .unwrap();
        assert!(
            world
                .collect_nominal_mappings(original, &mapped(len), &mut Vec::new(), 0)
                .is_err()
        );
        for changed in [
            mapped(len + 1),
            Ty::Array {
                element: Arc::new(Ty::Int(ir::IntTy::I32)),
                len,
            },
        ] {
            assert!(
                !actuals
                    .has_array_correspondence(&world, original, &changed, 0)
                    .unwrap()
            );
            assert!(
                world
                    .collect_nominal_mappings_with_actuals(
                        original,
                        &changed,
                        &mut Vec::new(),
                        &actuals,
                        0,
                    )
                    .is_err()
            );
        }
        let Ty::DependentArray { element, length } = original else {
            panic!("expected source array")
        };
        for changed in [
            ir::ConstParamId {
                owner: scope.binder(),
                index: length.index,
            },
            ir::ConstParamId {
                index: 0,
                ..*length
            },
        ] {
            let changed = Ty::DependentArray {
                element: element.clone(),
                length: changed,
            };
            assert!(
                !actuals
                    .has_array_correspondence(&world, &changed, &mapped(len), 0)
                    .unwrap()
            );
            assert!(
                world
                    .collect_nominal_mappings_with_actuals(
                        &changed,
                        &mapped(len),
                        &mut Vec::new(),
                        &actuals,
                        0,
                    )
                    .is_err()
            );
        }
    }
    signature.params[0] = symbolic(2);
    signature.result = symbolic(2);
    let actuals = CarrierFamilyActuals::from_signature(
        &world,
        &raw,
        &[
            Ty::Bool,
            Ty::Param(ir::ParamId {
                owner: scope.binder(),
                index: 2,
            }),
        ],
        &signature,
    )
    .unwrap();
    assert!(
        actuals
            .has_array_correspondence(&world, original, &symbolic(2), 0)
            .unwrap()
    );
    assert!(
        !actuals
            .has_array_correspondence(&world, original, &symbolic(1), 0)
            .unwrap()
    );
    world
        .collect_nominal_mappings_with_actuals(original, &symbolic(2), &mut Vec::new(), &actuals, 0)
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn storage_occurrences_in(
    world: &ValidatedTraitWorld,
    env: &GenericEnv,
    arguments: &[Ty],
    original: &Ty,
    mapped: &Ty,
    found: &mut [Option<Ty>],
    visited: &mut Vec<(ir::NominalAppId, ir::NominalAppId)>,
    depth: usize,
    family: &mut dyn FnMut(&Ty, &Ty) -> Result<()>,
) -> Result<()> {
    charged(world.budget(), world.budget().check_depth(depth))?;
    charged(world.budget(), world.budget().charge_work(1))?;
    if matches!(
        original,
        Ty::AssociatedType(_) | Ty::Opaque(_) | Ty::DependentArray { .. }
    ) {
        family(original, mapped)?;
    }
    if let Ty::Param(parameter) = original {
        let index = env
            .ordered_params()
            .position(|p| p == *parameter)
            .ok_or_else(|| {
                FactsError::invalid(
                    world.budget(),
                    "graph storage parameter is outside original binder",
                )
            })?;
        // This is a representation correspondence, not type-name equality:
        // authenticate the complete retained source/mapped aggregate structure.
        world.collect_nominal_mappings(&arguments[index], mapped, &mut Vec::new(), depth)?;
        if let Some(previous) = &found[index] {
            let graph = |ty: &Ty| match ty {
                Ty::Struct(n) => n.id().index.is_graph(),
                Ty::Enum(n) => n.id().index.is_graph(),
                _ => false,
            };
            if graph(previous) && graph(mapped) && !world.eq_ty(previous, mapped, depth)? {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage parameter has conflicting typed occurrences",
                ));
            }
            if graph(previous) {
                return Ok(());
            }
            if !graph(mapped) {
                world.collect_nominal_mappings(previous, mapped, &mut Vec::new(), depth)?;
                return Ok(());
            }
        }
        found[index] = Some(mapped.clone());
        return Ok(());
    }
    let mut recurse = |a: &Ty, b: &Ty| {
        storage_occurrences_in(
            world,
            env,
            arguments,
            a,
            b,
            found,
            visited,
            depth + 1,
            family,
        )
    };
    match (original, mapped) {
        (
            Ty::Ref {
                kind: a,
                referent: x,
            },
            Ty::Ref {
                kind: b,
                referent: y,
            },
        )
        | (
            Ty::RawPtr {
                kind: a,
                pointee: x,
            },
            Ty::RawPtr {
                kind: b,
                pointee: y,
            },
        ) if a == b => recurse(x, y)?,
        (Ty::Array { element: a, len: x }, Ty::Array { element: b, len: y }) if x == y => {
            recurse(a, b)?
        }
        (Ty::Slice { element: a }, Ty::Slice { element: b }) => recurse(a, b)?,
        (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b.iter()) {
                recurse(a, b)?;
            }
        }
        (Ty::Struct(a), Ty::Struct(b)) if a.def().source_decl == b.def().source_decl => {
            let pair = (a.id(), b.id());
            charged(world.budget(), world.budget().charge_work(visited.len()))?;
            if visited.contains(&pair) {
                return Ok(());
            }
            push(world.budget(), visited, pair)?;
            if a.def().type_args.len() != b.def().type_args.len()
                || a.def().fields.len() != b.def().fields.len()
            {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage struct correspondence changed",
                ));
            }
            for (a, b) in a.def().type_args.iter().zip(b.def().type_args.iter()) {
                storage_occurrences_in(
                    world,
                    env,
                    arguments,
                    a,
                    b,
                    found,
                    visited,
                    depth + 1,
                    family,
                )?;
            }
            for (a, b) in nominal_fields(&a.def().fields, world.budget())?
                .iter()
                .zip(nominal_fields(&b.def().fields, world.budget())?.iter())
            {
                storage_occurrences_in(
                    world,
                    env,
                    arguments,
                    &a.ty,
                    &b.ty,
                    found,
                    visited,
                    depth + 1,
                    family,
                )?;
            }
        }
        (Ty::Enum(a), Ty::Enum(b)) if a.def().source_decl == b.def().source_decl => {
            let pair = (a.id(), b.id());
            charged(world.budget(), world.budget().charge_work(visited.len()))?;
            if visited.contains(&pair) {
                return Ok(());
            }
            push(world.budget(), visited, pair)?;
            if a.def().type_args.len() != b.def().type_args.len()
                || a.def().variants.len() != b.def().variants.len()
            {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage enum correspondence changed",
                ));
            }
            for (a, b) in a.def().type_args.iter().zip(b.def().type_args.iter()) {
                storage_occurrences_in(
                    world,
                    env,
                    arguments,
                    a,
                    b,
                    found,
                    visited,
                    depth + 1,
                    family,
                )?;
            }
            for (a, b) in a.def().variants.iter().zip(b.def().variants.iter()) {
                if a.fields.len() != b.fields.len() {
                    return Err(FactsError::invalid(
                        world.budget(),
                        "graph storage variant correspondence changed",
                    ));
                }
                for (a, b) in nominal_fields(&a.fields, world.budget())?
                    .iter()
                    .zip(nominal_fields(&b.fields, world.budget())?.iter())
                {
                    storage_occurrences_in(
                        world,
                        env,
                        arguments,
                        &a.ty,
                        &b.ty,
                        found,
                        visited,
                        depth + 1,
                        family,
                    )?;
                }
            }
        }
        (Ty::PhantomData(a), Ty::PhantomData(b)) => {
            if !a.try_eq(b, |a, b| {
                recurse(a, b)?;
                Ok::<bool, FactsError>(true)
            })? {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage PhantomData declaration changed",
                ));
            }
        }
        (Ty::ManuallyDrop(a), Ty::ManuallyDrop(b)) => {
            if !a.try_eq(b, |a, b| {
                recurse(a, b)?;
                Ok::<bool, FactsError>(true)
            })? {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage ManuallyDrop declaration changed",
                ));
            }
        }
        (Ty::Cell(a), Ty::Cell(b)) => {
            if !a.try_eq(b, |a, b| {
                recurse(a, b)?;
                Ok::<bool, FactsError>(true)
            })? {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage Cell declaration changed",
                ));
            }
        }
        (Ty::FnPointer(a), Ty::FnPointer(b)) => {
            if !a.try_eq(b, |a, b| {
                recurse(a, b)?;
                Ok::<bool, FactsError>(true)
            })? {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage function pointer signature changed",
                ));
            }
        }
        (Ty::FnItem(a), Ty::FnItem(b)) => {
            if a.declaration != b.declaration || a.type_args.len() != b.type_args.len() {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage function item identity changed",
                ));
            }
            for (a, b) in a.type_args.iter().zip(b.type_args.iter()) {
                recurse(a, b)?;
            }
        }
        (Ty::Closure(a), Ty::Closure(b)) => {
            if !a.try_eq(b, |a, b| {
                recurse(a, b)?;
                Ok::<bool, FactsError>(true)
            })? {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage closure identity or seal changed",
                ));
            }
        }
        (Ty::Opaque(a), Ty::Opaque(b)) => {
            if !a.try_eq(b, |a, b| {
                recurse(a, b)?;
                Ok::<bool, FactsError>(true)
            })? {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage opaque declaration identity changed",
                ));
            }
        }
        (Ty::Dyn(a), Ty::Dyn(b)) => {
            if a.principal != b.principal
                || a.args.len() != b.args.len()
                || a.associated_types.len() != b.associated_types.len()
                || a.associated_types
                    .iter()
                    .zip(b.associated_types.iter())
                    .any(|(a, b)| a.0 != b.0)
            {
                return Err(FactsError::invalid(
                    world.budget(),
                    "graph storage dynamic trait identity changed",
                ));
            }
            for (a, b) in a.types().zip(b.types()) {
                recurse(a, b)?;
            }
        }
        (Ty::DependentArray { element: a, length }, Ty::Array { element: b, .. })
            if mapped_array_extent(env, arguments, *length, mapped, world.budget())? =>
        {
            recurse(a, b)?;
        }
        (Ty::DependentArray { element: a, length }, Ty::DependentArray { element: b, .. })
            if mapped_array_extent(env, arguments, *length, mapped, world.budget())? =>
        {
            recurse(a, b)?
        }
        // Projection normalization is authenticated by the mapped signature.
        // It provides no invertible occurrence of an original binder parameter.
        (Ty::AssociatedType(_), _) => {}
        _ if world.eq_ty(original, mapped, depth)? => {}
        _ => {
            return Err(FactsError::invalid(
                world.budget(),
                "graph storage correspondence changed canonical structure",
            ));
        }
    }
    Ok(())
}
