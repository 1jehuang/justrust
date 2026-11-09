use super::*;

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
    charged(world.budget(), world.budget().check_depth(depth))?;
    charged(world.budget(), world.budget().charge_work(1))?;
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
        storage_occurrences(world, env, arguments, a, b, found, visited, depth + 1)
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
                storage_occurrences(world, env, arguments, a, b, found, visited, depth + 1)?;
            }
            for (a, b) in nominal_fields(&a.def().fields, world.budget())?
                .iter()
                .zip(nominal_fields(&b.def().fields, world.budget())?.iter())
            {
                storage_occurrences(
                    world,
                    env,
                    arguments,
                    &a.ty,
                    &b.ty,
                    found,
                    visited,
                    depth + 1,
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
                storage_occurrences(world, env, arguments, a, b, found, visited, depth + 1)?;
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
                    storage_occurrences(
                        world,
                        env,
                        arguments,
                        &a.ty,
                        &b.ty,
                        found,
                        visited,
                        depth + 1,
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
        (
            Ty::DependentArray {
                element: a,
                length: x,
            },
            Ty::DependentArray {
                element: b,
                length: y,
            },
        ) if x == y => recurse(a, b)?,
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
