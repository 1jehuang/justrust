//! `Self` inside a struct or enum definition (`next: Option<Box<Self>>`,
//! `where Self: Trait`) denotes the nominal applied to its own declared
//! parameters, exactly as if that application had been written out.
use super::*;

/// The nominal's identity applied to its own region and type parameters.
/// `None` for nominals with const parameters: `Self` there is reported as an
/// explicit unsupported construct instead of being approximated.
/// The resolver's `type_params` must already be the nominal's parameters.
pub(super) fn nominal_self_type(
    resolver: &Resolver<'_>,
    name: &AstPath,
    region_params: &[AstIdent],
    type_params: &[AstIdent],
    has_const_params: bool,
) -> Result<Option<Type>, String> {
    // The definition is resolved in its own module, where its own name
    // denotes it. Absolute paths would name the mount root in libraries.
    let ident = *name
        .segments()
        .last()
        .ok_or("contract: nominal declaration path is empty")?;
    if has_const_params || resolver.param_contains(ident)? {
        return Ok(None);
    }
    let names = resolver.names();
    if region_params.is_empty() && type_params.is_empty() {
        return Ok(Some(Type::Named(
            AstPath::new(names, PathAnchor::Relative, &[ident])
                .map_err(native_adapter::name_diagnostic)?,
        )));
    }
    Ok(Some(Type::Apply(GenericPath {
        anchor: PathAnchor::Relative,
        segments: vec![PathSegment {
            name: ident,
            region_args: region_params
                .iter()
                .map(|r| RegionName::Named(*r))
                .collect(),
            args: type_params.iter().map(|p| Type::Param(*p)).collect(),
            associated_types: Vec::new(),
        }],
        resolved_use: None,
    })))
}

/// Replace every structural `Self` in a nominal field or bound type.
pub(super) fn substitute_nominal_self(
    ty: &mut Type,
    self_ty: Option<&Type>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    if !source_type_mentions_self(ty, budget, 0)? {
        return Ok(());
    }
    let Some(self_ty) = self_ty else {
        return Err(
            "unsupported: `Self` inside the definition of a nominal with const parameters or a parameter named like the nominal".into(),
        );
    };
    replace_nominal_self(ty, self_ty, budget, 0)?;
    if source_type_mentions_self(ty, budget, 0)? {
        return Err(
            "unsupported: `Self` in this position of a struct or enum definition is unsupported"
                .into(),
        );
    }
    Ok(())
}

fn replace_nominal_self(
    ty: &mut Type,
    self_ty: &Type,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let mut each = |ty: &mut Type| replace_nominal_self(ty, self_ty, budget, depth + 1);
    match ty {
        Type::SelfType => *ty = self_ty.clone(),
        Type::Ref { referent, .. } => each(referent)?,
        Type::DependentArray(element, _)
        | Type::Array(element, _)
        | Type::Slice(element)
        | Type::RawPtr {
            pointee: element, ..
        } => each(element)?,
        Type::FnPointer { inputs, output } => {
            for field in inputs.iter_mut().chain(std::iter::once(output.as_mut())) {
                each(field)?;
            }
        }
        Type::Tuple(fields) => {
            for field in fields {
                each(field)?;
            }
        }
        Type::Apply(path) if path.anchor != PathAnchor::SelfType => {
            for segment in &mut path.segments {
                for argument in segment.args.iter_mut().chain(
                    segment
                        .associated_types
                        .iter_mut()
                        .map(|binding| &mut binding.value),
                ) {
                    each(argument)?;
                }
            }
        }
        Type::PhantomData(source) | Type::ManuallyDrop(source) | Type::Cell(source) => {
            if let Some(argument) = source.argument.as_mut() {
                each(argument)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Bound subjects and trait arguments of a nominal's declared predicates.
pub(super) fn substitute_nominal_self_bounds(
    bounds: &mut [TypeBound],
    self_ty: Option<&Type>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for bound in bounds {
        substitute_nominal_self(&mut bound.subject, self_ty, budget)?;
        for path in &mut bound.traits {
            let TraitPath::Source(path) = path else {
                continue;
            };
            for segment in &mut path.segments {
                for argument in segment.args.iter_mut().chain(
                    segment
                        .associated_types
                        .iter_mut()
                        .map(|binding| &mut binding.value),
                ) {
                    substitute_nominal_self(argument, self_ty, budget)?;
                }
            }
        }
    }
    Ok(())
}
