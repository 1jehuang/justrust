//! Absence of native Vec element provenance, not initialized slot authority.
//! Roles are issued only from exact embedded original method recipes.
use super::*;

type Issuer = crate::compiler::source::EmbeddedOriginalFile;
#[derive(Clone, Debug)]
pub struct NativeEmptySource {
    declaration: ir::DeclId,
    constructor: ir::DeclId,
    owner: ir::DeclId,
    owner_issuer: Issuer,
    _method_issuer: Issuer,
    _source: crate::compiler::token_session::OriginalSpan,
}
impl NativeEmptySource {
    pub(in crate::compiler) fn matches_owner(
        &self,
        declaration: ir::DeclId,
        owner: &ir::StructTy,
    ) -> bool {
        self.declaration == declaration
            && self.owner == owner.def().source_decl
            && owner.native_sequence_binding().is_some_and(|binding| {
                binding.matches_declaration(owner.def())
                    && binding._issuer.same_issuer(&self.owner_issuer)
            })
    }
    pub(in crate::compiler) fn constructor(&self) -> ir::DeclId {
        self.constructor
    }
}
fn same(names: &NameStore, a: AstIdent, b: AstIdent, class: LookupClass) -> Result<bool, String> {
    Ok(names
        .key(a, class)
        .map_err(native_adapter::name_diagnostic)?
        == names
            .key(b, class)
            .map_err(native_adapter::name_diagnostic)?)
}
fn spelling(names: &NameStore, value: AstIdent, expected: &str) -> Result<bool, String> {
    Ok(names
        .spelling(value)
        .map_err(native_adapter::name_diagnostic)?
        == expected)
}
fn original(
    registry: &DeclRegistry,
    member: AstIdent,
    budget: &ir::PlanBudget,
) -> Result<Option<(crate::compiler::token_session::OriginalSpan, Issuer)>, String> {
    let names = registry.program.names.as_ref();
    let span = names
        .span(member)
        .map_err(native_adapter::name_diagnostic)?;
    let crate::compiler::expansion_span::SpanOrigin::Original(source) = names
        .session()
        .span(span)
        .map_err(|e| e.to_string())?
        .origin
    else {
        return Ok(None);
    };
    for issuer in registry.program.embedded_issuers.iter() {
        budget.charge_work(1)?;
        if issuer.authorizes(names.session().sources(), source) {
            return Ok(Some((source, issuer.clone())));
        }
    }
    Ok(None)
}
fn nominal_parameter(
    registry: &DeclRegistry,
    ty: &Type,
    owner: ir::DeclId,
    parameter: AstIdent,
) -> Result<bool, String> {
    let Type::Apply(path) = ty else {
        return Ok(false);
    };
    let (_, args) = path.application()?;
    let [Type::Param(actual)] = args else {
        return Ok(false);
    };
    let names = registry.program.names.as_ref();
    Ok(registry.nominals.get(&name_scope::path_key(
        names,
        &path.path(names)?,
        Namespace::Type,
    )?) == Some(&owner)
        && same(names, *actual, parameter, LookupClass::TypeParameter)?)
}
fn zero(value: &Expr) -> bool {
    matches!(value, Expr::Integer(value) if value.magnitude == 0 && value.suffix == Some(ir::IntTy::Usize))
}
/// The direct recipe has no calls, stores, branches or parameters. The delegate
/// has exactly one call to that same authenticated original recipe and no other
/// statements. Neither path can initialize a reference-bearing element.
pub(in crate::compiler::frontend) fn issue(
    registry: &DeclRegistry,
    declaration: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Option<Arc<NativeEmptySource>>, String> {
    budget.charge_work(64)?;
    let info = registry.declaration(declaration)?;
    let Some(associated) = &info.path.associated else {
        return Ok(None);
    };
    let Some(position) = registry.function_position(declaration) else {
        return Ok(None);
    };
    let Some(CallableSource::Associated {
        implementation,
        item,
    }) = registry.callable_sources.get(position).copied()
    else {
        return Ok(None);
    };
    let implementation = &registry.program.impls[implementation];
    let method = &implementation.items[item];
    let function = &method.function;
    let names = registry.program.names.as_ref();
    let Some(member) = function.name.segments().last().copied() else {
        return Ok(None);
    };
    let Some((source, issuer)) = original(registry, member, budget)? else {
        return Ok(None);
    };
    let Some(implementation_source) = implementation.original_source else {
        return Ok(None);
    };
    let [parameter] = implementation.type_params.as_slice() else {
        return Ok(None);
    };
    if associated.receiver != ReceiverKind::None
        || info.receiver != ReceiverKind::None
        || method.receiver != ReceiverKind::None
        || info.path.module != function.module
        || info.visibility != function.visibility
        || !issuer.authorizes(names.session().sources(), implementation_source)
        || implementation.unsafety
        || !implementation.bounds.is_empty()
        || !implementation.regions.params.is_empty()
        || !implementation.regions.outlives.is_empty()
        || !function.type_params.is_empty()
        || !function.bounds.is_empty()
        || !function.params.is_empty()
        || !function.param_mut.is_empty()
        || function.regions.receiver.is_some()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
        || function.unsafety
        || function.static_mut
        || function.constness != Constness::Runtime
        || function.impl_params != 0
        || !function.body.statements.is_empty()
        || !nominal_parameter(
            registry,
            &implementation.self_ty,
            associated.nominal,
            *parameter,
        )?
        || !nominal_parameter(registry, &function.result, associated.nominal, *parameter)?
    {
        return Ok(None);
    }
    let owner_info = registry.declaration(associated.nominal)?;
    let Some(owner) = registry.program.structs.get(owner_info.source_index) else {
        return Ok(None);
    };
    let Some(owner_member) = owner.name.segments().last().copied() else {
        return Ok(None);
    };
    let Some((_, owner_issuer)) = original(registry, owner_member, budget)? else {
        return Ok(None);
    };
    let expected = [
        crate::compiler::source::source_prelude::PRELUDE_MODULE,
        "vec",
        "Vec",
    ];
    if owner.name.segments().len() != expected.len() {
        return Ok(None);
    }
    for (&part, expected) in owner.name.segments().iter().zip(expected) {
        budget.charge_work(1)?;
        if !spelling(names, part, expected)? {
            return Ok(None);
        }
    }
    let [pointer, length, capacity] = owner.fields.as_slice() else {
        return Ok(None);
    };
    let (FieldName::Named(pointer), FieldName::Named(length), FieldName::Named(capacity)) =
        (&pointer.name, &length.name, &capacity.name)
    else {
        return Ok(None);
    };
    let constructor = if associated.trait_decl.is_none()
        && implementation.trait_path.is_none()
        && spelling(names, member, "new")?
    {
        if !owner_issuer.same_issuer(&issuer) {
            return Ok(None);
        }
        let Some(Expr::Struct { name, fields }) = function.body.tail.as_deref() else {
            return Ok(None);
        };
        let [
            (pointer_name, pointer_value),
            (length_name, length_value),
            (capacity_name, capacity_value),
        ] = fields.as_slice()
        else {
            return Ok(None);
        };
        let Expr::Cast {
            value,
            target:
                Type::RawPtr {
                    kind: ir::BorrowKind::Mut,
                    pointee,
                },
        } = pointer_value
        else {
            return Ok(None);
        };
        let Type::Param(actual) = pointee.as_ref() else {
            return Ok(None);
        };
        if registry
            .nominals
            .get(&name_scope::path_key(names, name, Namespace::Type)?)
            != Some(&associated.nominal)
            || !same(names, *pointer, *pointer_name, LookupClass::Member)?
            || !same(names, *length, *length_name, LookupClass::Member)?
            || !same(names, *capacity, *capacity_name, LookupClass::Member)?
            || !same(names, *actual, *parameter, LookupClass::TypeParameter)?
            || !zero(value)
            || !zero(length_value)
            || !zero(capacity_value)
        {
            return Ok(None);
        }
        declaration
    } else if spelling(names, member, "default")? {
        let Some(trait_declaration) = associated.trait_decl else {
            return Ok(None);
        };
        let Some(trait_path) = implementation.trait_path.as_ref() else {
            return Ok(None);
        };
        if registry.resolve_trait_path(trait_path)? != trait_declaration {
            return Ok(None);
        }
        let trait_info = registry.declaration(trait_declaration)?;
        let DeclItemName::Source(trait_member) = trait_info.path.item else {
            return Ok(None);
        };
        let Some((_, trait_issuer)) = original(registry, trait_member, budget)? else {
            return Ok(None);
        };
        if !trait_issuer.same_issuer(&issuer) || !spelling(names, trait_member, "Default")? {
            return Ok(None);
        }
        let Some(Expr::AssociatedCall {
            owner: call_owner,
            name,
            type_args,
            args,
        }) = function.body.tail.as_deref()
        else {
            return Ok(None);
        };
        if !args.is_empty() || !type_args.is_empty() {
            return Ok(None);
        }
        let owner_path = match call_owner {
            Type::Named(path) => path.clone(),
            // Resolution inserts exactly one Infer for the omitted Vec impl
            // argument. The original implementation and return type have already
            // been replayed as this owner's identical parameter above. This is
            // not permission for any concrete or differently shaped argument.
            Type::Apply(path)
                if nominal_parameter(registry, call_owner, associated.nominal, *parameter)?
                    || matches!(path.application()?.1, [Type::Infer]) =>
            {
                path.path(names)?
            }
            _ => return Ok(None),
        };
        if registry
            .nominals
            .get(&name_scope::path_key(names, &owner_path, Namespace::Type)?)
            != Some(&associated.nominal)
        {
            return Ok(None);
        }
        let mut selected = None;
        for candidate in registry
            .inherent_peers
            .get(&associated.nominal)
            .into_iter()
            .flatten()
        {
            budget.charge_work(1)?;
            let Some(candidate_source) = registry.source_callable(*candidate) else {
                continue;
            };
            let Some(candidate_member) = candidate_source.name.segments().last().copied() else {
                continue;
            };
            if same(names, candidate_member, *name, LookupClass::Member)?
                && selected.replace(*candidate).is_some()
            {
                return Ok(None);
            }
        }
        let Some(selected) = selected else {
            return Ok(None);
        };
        // Recursion is one exact inherent new recipe, never another delegate.
        let Some(selected_info) = registry.declaration(selected)?.path.associated.as_ref() else {
            return Ok(None);
        };
        if selected_info.trait_decl.is_some() || selected_info.nominal != associated.nominal {
            return Ok(None);
        }
        let Some(selected_source) = registry.source_callable(selected) else {
            return Ok(None);
        };
        let Some(selected_member) = selected_source.name.segments().last().copied() else {
            return Ok(None);
        };
        if !spelling(names, selected_member, "new")? {
            return Ok(None);
        }
        let Some(delegate) = issue(registry, selected, budget)? else {
            return Ok(None);
        };
        if delegate.constructor != selected
            || delegate.owner != associated.nominal
            || !delegate.owner_issuer.same_issuer(&owner_issuer)
        {
            return Ok(None);
        }
        selected
    } else {
        return Ok(None);
    };
    budget.charge_nodes(1)?;
    budget.retain_bytes(
        std::mem::size_of::<NativeEmptySource>() + 2 * std::mem::size_of::<usize>(),
    )?;
    Ok(Some(Arc::new(NativeEmptySource {
        declaration,
        constructor,
        owner: associated.nominal,
        owner_issuer,
        _method_issuer: issuer,
        _source: source,
    })))
}
