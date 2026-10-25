//! Parser-issued container certificates for generated callable products.
//! The certificate contains headers, never bodies or mutable result slots.
use super::super::semantic_scope::ScopeId;
use super::*;

#[derive(Clone, Debug)]
pub(super) struct Certificate {
    scope: ScopeId,
    member: AstIdent,
    receiver: ReceiverKind,
    header: Arc<Header>,
    signature: FunctionHead,
    owner: Owner,
    item: usize,
    visibility: ir::Visibility,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Implementation(usize),
    Trait(usize),
}
#[derive(Clone, Debug)]
struct Member {
    name: AstIdent,
    scope: Option<ScopeId>,
    receiver: ReceiverKind,
    signature: FunctionHead,
    visibility: ir::Visibility,
}
#[derive(Clone, Debug)]
struct Header {
    module: ModuleId,
    name: Option<AstPath>,
    self_ty: Option<Type>,
    trait_path: Option<TraitPath>,
    parameters: Vec<AstIdent>,
    bounds: Vec<TypeBound>,
    supertraits: Vec<TraitPath>,
    assignments: Vec<AssociatedTypeValue>,
    members: Vec<AssociatedTypeDecl>,
    visibility: Option<ir::Visibility>,
    self_defaults: Vec<usize>,
    callables: Vec<Member>,
}
fn preflight_trait(path: &TraitPath, budget: &ir::PlanBudget) -> Result<(), String> {
    if let TraitPath::Source(path) = path {
        source_clone::clone_path(path, budget)?;
        for segment in &path.segments {
            for argument in &segment.args {
                walk_depth(DepthNode::Type(argument), Some(budget))?;
            }
            for binding in &segment.associated_types {
                walk_depth(DepthNode::Type(&binding.value), Some(budget))?;
            }
        }
    }
    Ok(())
}
fn preflight_bounds(bounds: &[TypeBound], budget: &ir::PlanBudget) -> Result<(), String> {
    source_clone::clone_bounds(bounds, budget)?;
    for bound in bounds {
        walk_depth(DepthNode::Type(&bound.subject), Some(budget))?;
    }
    Ok(())
}
fn preflight_signature(
    regions: &RegionDecls,
    parameters: &[AstIdent],
    bounds: &[TypeBound],
    arguments: &[(BindingName, Type)],
    result: &Type,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    source_clone::clone_regions(regions, budget)?;
    source_clone::clone_names(parameters, budget)?;
    preflight_bounds(bounds, budget)?;
    source_clone::clone_parameters(arguments, budget)?;
    walk_depth(DepthNode::Type(result), Some(budget))
}
impl Header {
    fn implementation(owner: &ImplDecl) -> Result<Self, String> {
        let budget = ir::PlanBudget::default();
        source_clone::clone_names(&owner.type_params, &budget)?;
        preflight_bounds(&owner.bounds, &budget)?;
        walk_depth(DepthNode::Type(&owner.self_ty), Some(&budget))?;
        if let Some(path) = &owner.trait_path {
            preflight_trait(path, &budget)?;
        }
        for member in &owner.associated_types {
            walk_depth(DepthNode::Type(&member.value), Some(&budget))?;
        }
        for item in &owner.items {
            preflight_signature(
                &item.function.regions,
                &item.function.type_params,
                &item.function.bounds,
                &item.function.params,
                &item.function.result,
                &budget,
            )?;
        }
        Ok(Self {
            module: owner.module,
            name: None,
            self_ty: Some(owner.self_ty.clone()),
            trait_path: owner.trait_path.clone(),
            parameters: owner.type_params.clone(),
            bounds: owner.bounds.clone(),
            supertraits: Vec::new(),
            assignments: owner.associated_types.clone(),
            members: Vec::new(),
            visibility: None,
            self_defaults: Vec::new(),
            callables: owner
                .items
                .iter()
                .map(|item| {
                    let name = *item
                        .function
                        .name
                        .segments()
                        .last()
                        .ok_or("generated container has an empty member path")?;
                    Ok(Member {
                        name,
                        scope: item.function.lexical_scope,
                        receiver: item.receiver,
                        signature: signature(&item.function, name),
                        visibility: item.visibility.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        })
    }
    fn declaration(owner: &TraitDecl) -> Result<Self, String> {
        let budget = ir::PlanBudget::default();
        source_clone::clone_names(&owner.type_params, &budget)?;
        preflight_bounds(&owner.bounds, &budget)?;
        for path in &owner.supertraits {
            preflight_trait(path, &budget)?;
        }
        for member in &owner.associated_types {
            for path in &member.bounds {
                preflight_trait(path, &budget)?;
            }
        }
        for item in &owner.items {
            preflight_signature(
                &item.signature.regions,
                &item.signature.type_params,
                &item.signature.bounds,
                &item.signature.params,
                &item.signature.result,
                &budget,
            )?;
        }
        Ok(Self {
            module: owner.module,
            name: Some(owner.name.clone()),
            self_ty: None,
            trait_path: None,
            parameters: owner.type_params.clone(),
            bounds: owner.bounds.clone(),
            supertraits: owner.supertraits.clone(),
            assignments: Vec::new(),
            members: owner.associated_types.clone(),
            visibility: Some(owner.visibility.clone()),
            self_defaults: owner.self_defaults.clone(),
            callables: owner
                .items
                .iter()
                .map(|item| Member {
                    name: item.signature.name,
                    scope: item
                        .default
                        .as_ref()
                        .and_then(|function| function.lexical_scope),
                    receiver: item.receiver,
                    signature: item.signature.clone(),
                    visibility: owner.visibility.clone(),
                })
                .collect(),
        })
    }
}
fn signature(function: &Function, member: AstIdent) -> FunctionHead {
    FunctionHead {
        constness: function.constness,
        unsafety: function.unsafety,
        name: member,
        regions: function.regions.clone(),
        type_params: function.type_params.clone(),
        bounds: function.bounds.clone(),
        params: function.params.clone(),
        param_mut: function.param_mut.clone(),
        result: function.result.clone(),
        impl_params: function.impl_params,
    }
}
fn generated(
    program: &Program,
    function: &Function,
) -> Result<Option<(ScopeId, AstIdent)>, String> {
    let Some(scope) = function.lexical_scope else {
        return Ok(None);
    };
    let Some(scopes) = &program.scopes else {
        return Err("generated product has no scope authority".into());
    };
    let Some(member) = scopes
        .generated_function_declaration(scope)
        .map_err(super::super::semantic_scope::scope_diagnostic)?
    else {
        return Ok(None);
    };
    scopes
        .check_generated_function(
            program.names.as_ref(),
            program.modules.as_ref(),
            scope,
            member,
            function.module,
        )
        .map_err(super::super::semantic_scope::scope_diagnostic)?;
    Ok(Some((scope, member)))
}

/// Called only by the authenticated token parser, or by canonical resolution
/// after validating the original certificate. Public AST construction cannot
/// supply or refresh this private field.
pub(super) fn capture(program: &Program) -> Result<Arc<[Certificate]>, String> {
    let mut certificates = Vec::new();
    let mut add =
        |function: &Function, receiver, header: Arc<Header>, owner, item| -> Result<(), String> {
            if let Some((scope, member)) = generated(program, function)? {
                let budget = ir::PlanBudget::default();
                preflight_signature(
                    &function.regions,
                    &function.type_params,
                    &function.bounds,
                    &function.params,
                    &function.result,
                    &budget,
                )?;
                if certificates
                    .iter()
                    .any(|row: &Certificate| row.scope == scope)
                {
                    return Err("duplicate generated container product".into());
                }
                certificates
                    .try_reserve(1)
                    .map_err(|_| "generated container certificate allocation failed")?;
                certificates.push(Certificate {
                    scope,
                    member,
                    receiver,
                    header,
                    signature: signature(function, member),
                    owner,
                    item,
                    visibility: function.visibility.clone(),
                });
            }
            Ok(())
        };
    for (index, owner) in program.impls.iter().enumerate() {
        let mut has_product = false;
        for item in &owner.items {
            has_product |= generated(program, &item.function)?.is_some();
        }
        if !has_product {
            continue;
        }
        let header = Arc::new(Header::implementation(owner)?);
        for (item, value) in owner.items.iter().enumerate() {
            add(
                &value.function,
                value.receiver,
                header.clone(),
                Owner::Implementation(index),
                item,
            )?;
        }
    }
    for (index, owner) in program.traits.iter().enumerate() {
        let mut has_product = false;
        for function in owner.items.iter().filter_map(|item| item.default.as_ref()) {
            has_product |= generated(program, function)?.is_some();
        }
        if !has_product {
            continue;
        }
        let header = Arc::new(Header::declaration(owner)?);
        for (item, value) in owner.items.iter().enumerate() {
            if let Some(function) = &value.default {
                add(
                    function,
                    value.receiver,
                    header.clone(),
                    Owner::Trait(index),
                    item,
                )?;
            }
        }
    }
    Ok(certificates.into())
}
fn paths(
    names: &NameStore,
    a: &TraitPath,
    b: &TraitPath,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    match (a, b) {
        (TraitPath::Builtin(a), TraitPath::Builtin(b)) => Ok(a == b),
        (TraitPath::Source(a), TraitPath::Source(b)) => {
            exact_source_generic_path(names, a, b, budget, 0)
        }
        _ => Ok(false),
    }
}
fn identifiers(names: &NameStore, a: &[AstIdent], b: &[AstIdent]) -> Result<bool, String> {
    let mut equal = a.len() == b.len();
    for (a, b) in a.iter().zip(b) {
        equal &= exact_source_ident(names, *a, *b)?;
    }
    Ok(equal)
}
fn bounds(
    names: &NameStore,
    a: &[TypeBound],
    b: &[TypeBound],
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let mut equal = a.len() == b.len();
    for (a, b) in a.iter().zip(b) {
        equal &= a.maybe_unsized == b.maybe_unsized && a.traits.len() == b.traits.len();
        equal &= exact_source_type(names, &a.subject, &b.subject, budget, 0)?;
        for (a, b) in a.traits.iter().zip(&b.traits) {
            equal &= paths(names, a, b, budget)?;
        }
    }
    Ok(equal)
}
fn headers(
    names: &NameStore,
    a: &Header,
    b: &Header,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let mut equal =
        a.module == b.module && a.visibility == b.visibility && a.self_defaults == b.self_defaults;
    equal &= match (&a.name, &b.name) {
        (None, None) => true,
        (Some(a), Some(b)) => exact_source_path(names, a, b, budget)?,
        _ => false,
    };
    equal &= match (&a.self_ty, &b.self_ty) {
        (None, None) => true,
        (Some(a), Some(b)) => exact_source_type(names, a, b, budget, 0)?,
        _ => false,
    };
    equal &= match (&a.trait_path, &b.trait_path) {
        (None, None) => true,
        (Some(a), Some(b)) => paths(names, a, b, budget)?,
        _ => false,
    };
    equal &= identifiers(names, &a.parameters, &b.parameters)?;
    equal &= bounds(names, &a.bounds, &b.bounds, budget)?;
    equal &= a.supertraits.len() == b.supertraits.len();
    for (a, b) in a.supertraits.iter().zip(&b.supertraits) {
        equal &= paths(names, a, b, budget)?;
    }
    equal &= a.assignments.len() == b.assignments.len();
    for (a, b) in a.assignments.iter().zip(&b.assignments) {
        equal &= exact_source_ident(names, a.name, b.name)?;
        equal &= exact_source_type(names, &a.value, &b.value, budget, 0)?;
    }
    equal &= a.members.len() == b.members.len();
    for (a, b) in a.members.iter().zip(&b.members) {
        equal &= exact_source_ident(names, a.name, b.name)? && a.bounds.len() == b.bounds.len();
        for (a, b) in a.bounds.iter().zip(&b.bounds) {
            equal &= paths(names, a, b, budget)?;
        }
    }
    equal &= a.callables.len() == b.callables.len();
    for (a, b) in a.callables.iter().zip(&b.callables) {
        equal &= a.scope == b.scope && a.receiver == b.receiver && a.visibility == b.visibility;
        equal &= exact_source_ident(names, a.name, b.name)?;
        equal &= signatures(names, &a.signature, &b.signature, budget)?;
    }
    Ok(equal)
}
fn signatures(
    names: &NameStore,
    a: &FunctionHead,
    b: &FunctionHead,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let mut equal = a.constness == b.constness
        && a.unsafety == b.unsafety
        && a.impl_params == b.impl_params
        && a.param_mut == b.param_mut;
    equal &= exact_source_ident(names, a.name, b.name)?;
    equal &= identifiers(names, &a.type_params, &b.type_params)?;
    equal &= bounds(names, &a.bounds, &b.bounds, budget)?;
    equal &= a.params.len() == b.params.len();
    for ((an, at), (bn, bt)) in a.params.iter().zip(&b.params) {
        equal &= match (an, bn) {
            (BindingName::Named(a), BindingName::Named(b)) => exact_source_ident(names, *a, *b)?,
            (BindingName::Wildcard, BindingName::Wildcard) => true,
            _ => false,
        };
        equal &= exact_source_type(names, at, bt, budget, 0)?;
    }
    equal &= exact_source_type(names, &a.result, &b.result, budget, 0)?;
    equal &= identifiers(names, &a.regions.params, &b.regions.params)?;
    equal &= a.regions.outlives.len() == b.regions.outlives.len();
    let region = |a, b| -> Result<bool, String> {
        match (a, b) {
            (RegionName::Named(a), RegionName::Named(b)) => exact_source_ident(names, a, b),
            (RegionName::Static, RegionName::Static) | (RegionName::Infer, RegionName::Infer) => {
                Ok(true)
            }
            _ => Ok(false),
        }
    };
    equal &= match (a.regions.receiver, b.regions.receiver) {
        (None, None) => true,
        (Some(a), Some(b)) => region(a, b)?,
        _ => false,
    };
    for ((a, b), (c, d)) in a.regions.outlives.iter().zip(&b.regions.outlives) {
        equal &= region(*a, *c)? && region(*b, *d)?;
    }
    Ok(equal)
}
pub(super) fn validate(program: &Program) -> Result<(), String> {
    let actual = capture(program)?;
    let budget = ir::PlanBudget::default();
    if actual.len() != program.generated_owners.len() {
        return Err("generated container product coverage differs from original source".into());
    }
    for row in actual.iter() {
        let Some(original) = program
            .generated_owners
            .iter()
            .find(|original| original.scope == row.scope)
        else {
            return Err("generated product has no original container certificate".into());
        };
        if row.owner != original.owner
            || row.item != original.item
            || row.receiver != original.receiver
            || row.visibility != original.visibility
            || !exact_source_ident(&program.names, row.member, original.member)?
            || !headers(&program.names, &row.header, &original.header, &budget)?
            || !signatures(&program.names, &row.signature, &original.signature, &budget)?
        {
            return Err(format!(
                "generated callable differs from its original container or signature: owner {:?}/{:?}, item {}/{}, receiver {:?}/{:?}, visibility {:?}/{:?}, member {}, header {}, signature {}",
                row.owner,
                original.owner,
                row.item,
                original.item,
                row.receiver,
                original.receiver,
                row.visibility,
                original.visibility,
                exact_source_ident(&program.names, row.member, original.member)?,
                headers(&program.names, &row.header, &original.header, &budget)?,
                signatures(&program.names, &row.signature, &original.signature, &budget)?,
            ));
        }
    }
    Ok(())
}

/// File merging relocates only container ordinals, after validating the
/// original file certificate. Scope, header and exact occurrence stay sealed.
pub(super) fn rebase(
    certificates: &mut [Certificate],
    implementation_offset: usize,
    trait_offset: usize,
) -> Result<(), String> {
    for certificate in certificates {
        certificate.owner = match certificate.owner {
            Owner::Implementation(index) => Owner::Implementation(
                index
                    .checked_add(implementation_offset)
                    .ok_or("generated implementation index overflow")?,
            ),
            Owner::Trait(index) => Owner::Trait(
                index
                    .checked_add(trait_offset)
                    .ok_or("generated trait index overflow")?,
            ),
        };
    }
    Ok(())
}
pub(super) fn contains(program: &Program, scope: ScopeId, module: ModuleId) -> bool {
    program
        .generated_owners
        .iter()
        .any(|row| row.scope == scope && row.header.module == module)
}

pub(super) fn validate_callable(program: &Program, function: &Function) -> Result<(), String> {
    let Some((scope, member)) = generated(program, function)? else {
        return Ok(());
    };
    let Some(row) = program
        .generated_owners
        .iter()
        .find(|row| row.scope == scope)
    else {
        // Free generated functions use the existing item-product certificate.
        return Ok(());
    };
    let (header, selected, receiver) = match row.owner {
        Owner::Implementation(index) => {
            let owner = program
                .impls
                .get(index)
                .ok_or("missing original generated impl")?;
            let item = owner
                .items
                .get(row.item)
                .ok_or("missing original generated member")?;
            (
                Header::implementation(owner)?,
                &item.function,
                item.receiver,
            )
        }
        Owner::Trait(index) => {
            let owner = program
                .traits
                .get(index)
                .ok_or("missing original generated trait")?;
            let item = owner
                .items
                .get(row.item)
                .ok_or("missing original generated member")?;
            (
                Header::declaration(owner)?,
                item.default.as_ref().ok_or("missing generated default")?,
                item.receiver,
            )
        }
    };
    let budget = ir::PlanBudget::default();
    preflight_signature(
        &function.regions,
        &function.type_params,
        &function.bounds,
        &function.params,
        &function.result,
        &budget,
    )?;
    if selected.lexical_scope != Some(scope)
        || receiver != row.receiver
        || function.visibility != row.visibility
        || !exact_source_ident(&program.names, member, row.member)?
        || !headers(&program.names, &header, &row.header, &budget)?
        || !signatures(
            &program.names,
            &signature(function, member),
            &row.signature,
            &budget,
        )?
    {
        return Err("selected generated callable has a foreign container or schema".into());
    }
    Ok(())
}
