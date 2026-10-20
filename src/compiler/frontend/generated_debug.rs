//! Immutable pending derive recipe. The canonical source Debug declaration supplies
//! the signature only after source resolution, never a placeholder type or getter.
use super::*;
use crate::compiler::semantic_name::{DerivedAtom, validate_derived_identifier};
use crate::compiler::semantic_scope::{DerivedMemberAdmission, DerivedMemberKind, ScopeId};

#[derive(Clone, Debug)]
pub(super) struct DebugRecipe {
    pub(super) admission: DerivedMemberAdmission,
    pub(super) nominal: usize,
    pub(super) scope: ScopeId,
    pub(super) method: AstIdent,
    pub(super) receiver: AstIdent,
    pub(super) formatter: AstIdent,
    pub(super) formatter_use: AstIdent,
    pub(super) write: AstIdent,
    pub(super) implementation: Option<usize>,
    pub(super) source_trait: Option<usize>,
    pub(super) source_member: Option<usize>,
}
impl Parser<'_, '_> {
    pub(super) fn unit_debug_recipe(
        &mut self,
        nominal: usize,
        admission: DerivedMemberAdmission,
    ) -> Result<DebugRecipe, ParseError> {
        let method = self
            .names
            .derived_identifier(&admission, DerivedAtom::Method)
            .map_err(scope_parse_error)?;
        let receiver = self
            .names
            .derived_identifier(&admission, DerivedAtom::Receiver)
            .map_err(scope_parse_error)?;
        let formatter = self
            .names
            .derived_identifier(&admission, DerivedAtom::FormatterParameter)
            .map_err(scope_parse_error)?;
        let formatter_use = self
            .names
            .derived_identifier(&admission, DerivedAtom::FormatterParameter)
            .map_err(scope_parse_error)?;
        let write = self
            .names
            .derived_identifier(&admission, DerivedAtom::FormatterWriteStr)
            .map_err(scope_parse_error)?;
        let expansion = self
            .item_expansion
            .ok_or_else(|| self.error("derived Debug outside item expansion"))?;
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(
                crate::compiler::token_session::TokenError::InvalidHandle,
            )
        })?;
        let frontier = scopes
            .generated_receiver_function_root(
                self.names,
                &*self.modules,
                &expansion,
                method,
                Some(receiver),
            )
            .map_err(scope_parse_error)?;
        // Register the actual formatter binding in this lexical product. No source
        // identifier is reconstructed from a string or attributed to another file.
        let (frontier, _) = scopes
            .bind(self.names, frontier, receiver)
            .map_err(scope_parse_error)?;
        let (frontier, _) = scopes
            .bind(self.names, frontier, formatter)
            .map_err(scope_parse_error)?;
        if scopes
            .resolve_reference(self.names, frontier, formatter_use)
            .map_err(scope_parse_error)?
            .is_none()
        {
            return Err(native_adapter::transport_error(
                crate::compiler::token_session::TokenError::InvalidHandle,
            ));
        }
        let scope = scopes.function(frontier).map_err(scope_parse_error)?;
        Ok(DebugRecipe {
            admission,
            nominal,
            scope,
            method,
            receiver,
            formatter,
            formatter_use,
            write,
            implementation: None,
            source_trait: None,
            source_member: None,
        })
    }
}

/// Called while the existing declaration-resolution loop visits each trait.
/// Only the embedded native prelude's immediate fmt module is eligible.
pub(super) fn canonical_source(program: &Program, declaration: &TraitDecl) -> Result<bool, String> {
    let Some(root) = program.prelude_module else {
        return Ok(false);
    };
    if program
        .modules
        .parent(declaration.module)
        .map_err(module_diagnostic)?
        != Some(root)
    {
        return Ok(false);
    }
    let Some(module) = program
        .modules
        .name(declaration.module)
        .map_err(module_diagnostic)?
    else {
        return Ok(false);
    };
    let Some(name) = declaration.name.segments().last() else {
        return Err("empty source Debug trait path".into());
    };
    Ok(program
        .names
        .spelling(module)
        .map_err(native_adapter::name_diagnostic)?
        == "fmt"
        && program
            .names
            .spelling(*name)
            .map_err(native_adapter::name_diagnostic)?
            == "Debug")
}

pub(super) fn construct(
    program: &Program,
    recipe: &DebugRecipe,
    source: usize,
    member: usize,
) -> Result<ImplDecl, String> {
    let declaration = program
        .structs
        .get(recipe.nominal)
        .ok_or("derived Debug nominal outside inventory")?;
    let mut implementation = generated_derive::unit_copy(declaration).map_err(str::to_owned)?;
    let owner = program
        .traits
        .get(source)
        .ok_or("derived Debug source trait outside inventory")?;
    if !canonical_source(program, owner)? || !owner.type_params.is_empty() {
        return Err("derived Debug requires the canonical native source trait".into());
    }
    let item = owner
        .items
        .get(member)
        .ok_or("derived Debug source member outside inventory")?;
    let signature = &item.signature;
    if program
        .names
        .spelling(signature.name)
        .map_err(native_adapter::name_diagnostic)?
        != "fmt"
        || signature.params.len() != 1
        || !signature.type_params.is_empty()
        || !signature.bounds.is_empty()
        || signature.constness != Constness::Runtime
        || signature.unsafety
        || signature.impl_params != 0
        || signature.regions.receiver.is_some()
        || !signature.regions.params.is_empty()
        || !signature.regions.outlives.is_empty()
        || !matches!(
            item.receiver,
            ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Shared
            }
        )
    {
        return Err(
            "unsupported: canonical Debug method signature is outside the unit derive contract"
                .into(),
        );
    }
    let method = AstPath::new(&*program.names, PathAnchor::Relative, &[recipe.method])
        .map_err(native_adapter::name_diagnostic)?;
    let formatter = AstPath::new(
        &*program.names,
        PathAnchor::Relative,
        &[recipe.formatter_use],
    )
    .map_err(native_adapter::name_diagnostic)?;
    let trait_path = GenericPath {
        anchor: owner.name.anchor(),
        segments: owner
            .name
            .segments()
            .iter()
            .map(|name| PathSegment {
                name: *name,
                region_args: Vec::new(),
                args: Vec::new(),
                associated_types: Vec::new(),
            })
            .collect(),
        resolved_use: None,
    };
    let name = declaration
        .name
        .segments()
        .last()
        .copied()
        .ok_or("derived Debug nominal lacks name")?;
    implementation.trait_path = Some(TraitPath::Source(trait_path));
    implementation.items.push(AssociatedFunction {
        receiver: item.receiver,
        visibility: ir::Visibility::Restricted(declaration.module),
        function: Function {
            embedded_source: None,
            lexical_scope: Some(recipe.scope),
            module: declaration.module,
            constness: Constness::Runtime,
            unsafety: false,
            static_mut: false,
            visibility: ir::Visibility::Restricted(declaration.module),
            bounds: Vec::new(),
            name: method,
            regions: signature.regions.clone(),
            type_params: Vec::new(),
            params: vec![(
                BindingName::Named(recipe.formatter),
                signature.params[0].1.clone(),
            )],
            param_mut: signature.param_mut.clone(),
            result: signature.result.clone(),
            impl_params: 0,
            body: Block {
                statements: Vec::new(),
                tail: Some(Box::new(Expr::MethodCall {
                    receiver: Box::new(Expr::Var(formatter)),
                    name: recipe.write,
                    type_args: Vec::new(),
                    args: vec![Expr::Str(
                        program
                            .names
                            .spelling(name)
                            .map_err(native_adapter::name_diagnostic)?
                            .to_owned(),
                    )],
                })),
            },
        },
    });
    Ok(implementation)
}

/// Replays the original derive/member pair before borrowing the canonical source
/// signature's occurrence domain. The generated binding and body keep their own
/// expansion domain. This never chooses authority from an identifier's origin.
pub(super) fn signature_module(
    program: &Program,
    function: &Function,
) -> Result<Option<ModuleId>, String> {
    let Some(scope) = function.lexical_scope else {
        return Ok(None);
    };
    for recipe in generated_metadata::debug_recipes(&program.generated_metadata) {
        if recipe.scope != scope {
            continue;
        }
        validate_recipe(program, recipe)?;
        let (Some(implementation), Some(source), Some(_)) = (
            recipe.implementation,
            recipe.source_trait,
            recipe.source_member,
        ) else {
            return Err("pending derived Debug cannot authorize a signature".into());
        };
        let canonical = &program.impls[implementation].items[0].function;
        let budget = ir::PlanBudget::default();
        if function.params.len() != canonical.params.len()
            || !exact_source_path(&program.names, &function.name, &canonical.name, &budget)?
            || !exact_source_type(
                &program.names,
                &function.result,
                &canonical.result,
                &budget,
                0,
            )?
        {
            return Err("derived Debug source signature occurrence changed".into());
        }
        for ((name, ty), (expected, expected_ty)) in function.params.iter().zip(&canonical.params) {
            let (BindingName::Named(name), BindingName::Named(expected)) = (name, expected) else {
                return Err("derived Debug signature binding changed".into());
            };
            if !exact_source_ident(&program.names, *name, *expected)?
                || !exact_source_type(&program.names, ty, expected_ty, &budget, 0)?
            {
                return Err("derived Debug signature parameter occurrence changed".into());
            }
        }
        return Ok(Some(program.traits[source].module));
    }
    Ok(None)
}

pub(super) fn validate_recipe(program: &Program, recipe: &DebugRecipe) -> Result<(), String> {
    if recipe.admission.kind() != DerivedMemberKind::Debug {
        return Err("derived Debug admission kind changed".into());
    }
    for (atom, ident) in [
        (DerivedAtom::Method, recipe.method),
        (DerivedAtom::Receiver, recipe.receiver),
        (DerivedAtom::FormatterParameter, recipe.formatter),
        (DerivedAtom::FormatterParameter, recipe.formatter_use),
        (DerivedAtom::FormatterWriteStr, recipe.write),
    ] {
        validate_derived_identifier(&*program.names, &recipe.admission, atom, ident)
            .map_err(|error| error.to_string())?;
    }
    let scopes = program
        .scopes
        .as_ref()
        .ok_or("derived Debug has no source scope authority")?;
    let declaration = program
        .structs
        .get(recipe.nominal)
        .ok_or("derived Debug nominal missing")?;
    scopes
        .check_generated_function(
            &*program.names,
            &*program.modules,
            recipe.scope,
            recipe.method,
            declaration.module,
        )
        .map_err(crate::compiler::semantic_scope::scope_diagnostic)?;
    let receiver = scopes
        .receiver_binding(&*program.names, recipe.scope)
        .and_then(|binding| scopes.binding(binding))
        .map_err(crate::compiler::semantic_scope::scope_diagnostic)?;
    if !exact_source_ident(&program.names, receiver.declaration, recipe.receiver)? {
        return Err("derived Debug receiver authority changed".into());
    }
    match (
        recipe.implementation,
        recipe.source_trait,
        recipe.source_member,
    ) {
        (None, None, None) => Ok(()),
        (Some(index), Some(source), Some(member)) => {
            let expected = construct(program, recipe, source, member)?;
            let actual = program
                .impls
                .get(index)
                .ok_or("derived Debug implementation missing")?;
            let budget = ir::PlanBudget::default();
            let Some(TraitPath::Source(path)) = &actual.trait_path else {
                return Err("derived Debug source trait changed".into());
            };
            let Some(TraitPath::Source(expected_path)) = &expected.trait_path else {
                unreachable!()
            };
            if !exact_source_generic_path(&program.names, path, expected_path, &budget, 0)?
                || actual.module != expected.module
                || !actual.bounds.is_empty()
                || !actual.type_params.is_empty()
                || !actual.associated_types.is_empty()
                || !actual.associated_constants.is_empty()
                || !exact_source_type(
                    &program.names,
                    &actual.self_ty,
                    &expected.self_ty,
                    &budget,
                    0,
                )?
                || actual.items.len() != 1
            {
                return Err("derived Debug owner recipe changed".into());
            }
            if actual.items[0].receiver != expected.items[0].receiver
                || actual.items[0].visibility != ir::Visibility::Restricted(declaration.module)
            {
                return Err("derived Debug receiver/visibility changed".into());
            }
            let function = &actual.items[0].function;
            let expected_function = &expected.items[0].function;
            if function.lexical_scope != Some(recipe.scope)
                || function.module != expected_function.module
                || !exact_source_path(
                    &program.names,
                    &function.name,
                    &expected_function.name,
                    &budget,
                )?
                || function.constness != Constness::Runtime
                || function.unsafety
                || function.static_mut
                || function.visibility != ir::Visibility::Restricted(declaration.module)
                || !function.bounds.is_empty()
                || !function.type_params.is_empty()
                || function.params.len() != 1
                || function.impl_params != 0
                || function.regions.receiver.is_some()
                || !function.regions.params.is_empty()
                || !function.regions.outlives.is_empty()
                || !function.body.statements.is_empty()
                || function.param_mut != expected_function.param_mut
                || !exact_source_type(
                    &program.names,
                    &function.result,
                    &expected_function.result,
                    &budget,
                    0,
                )?
            {
                return Err("derived Debug signature changed".into());
            }
            for ((name, ty), (expected_name, expected_ty)) in
                function.params.iter().zip(&expected_function.params)
            {
                let (BindingName::Named(name), BindingName::Named(expected_name)) =
                    (name, expected_name)
                else {
                    return Err("derived Debug parameter changed".into());
                };
                if !exact_source_ident(&program.names, *name, *expected_name)?
                    || !exact_source_type(&program.names, ty, expected_ty, &budget, 0)?
                {
                    return Err("derived Debug parameter authority changed".into());
                }
            }
            let Some(Expr::MethodCall {
                receiver,
                name,
                type_args,
                args,
            }) = function.body.tail.as_deref()
            else {
                return Err("derived Debug body changed".into());
            };
            let Expr::Var(receiver) = receiver.as_ref() else {
                return Err("derived Debug formatter changed".into());
            };
            let expected_receiver = AstPath::new(
                &*program.names,
                PathAnchor::Relative,
                &[recipe.formatter_use],
            )
            .map_err(native_adapter::name_diagnostic)?;
            let nominal = declaration
                .name
                .segments()
                .last()
                .copied()
                .ok_or("derived Debug nominal name missing")?;
            if !exact_source_path(&program.names, receiver, &expected_receiver, &budget)?
                || !exact_source_ident(&program.names, *name, recipe.write)?
                || !type_args.is_empty()
                || !matches!(args.as_slice(), [Expr::Str(text)] if text == program.names.spelling(nominal).map_err(native_adapter::name_diagnostic)?)
            {
                return Err("derived Debug body recipe changed".into());
            }
            Ok(())
        }
        _ => Err("partial derived Debug source binding".into()),
    }
}
