//! Direct AST emission for the supported built-in generated derive slice.
//! No expanded input is printed, reparsed, or assigned fabricated source names.
use super::*;

/// Parser-local emission request. The scope arena seals the exact original
/// metadata owner, derive occurrence and nominal declaration before publication.
#[derive(Clone, Debug)]
pub(super) enum EmissionProof {
    Copy(crate::compiler::semantic_scope::ItemDerivedCopyId),
    Clone(crate::compiler::semantic_scope::DerivedMemberAdmission),
}

#[derive(Clone, Debug)]
pub(super) struct CopyEmission {
    pub(super) proof: EmissionProof,
    pub(super) nominal: usize,
    pub(super) implementation: usize,
    pub(super) derive: AstIdent,
}

impl Parser<'_, '_> {
    pub(super) fn unit_clone(
        &mut self,
        declaration: &StructDecl,
        admission: crate::compiler::semantic_scope::DerivedMemberAdmission,
    ) -> Result<ImplDecl, ParseError> {
        use crate::compiler::semantic_name::DerivedAtom;
        let mut implementation =
            unit_copy(declaration).map_err(|message| self.unsupported(message))?;
        let method = self
            .names
            .derived_identifier(&admission, DerivedAtom::Method)
            .map_err(scope_parse_error)?;
        let receiver = self
            .names
            .derived_identifier(&admission, DerivedAtom::Receiver)
            .map_err(scope_parse_error)?;
        let proof = self
            .item_expansion
            .ok_or_else(|| self.error("derived Clone outside item expansion"))?;
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(
                crate::compiler::token_session::TokenError::InvalidHandle,
            )
        })?;
        let frontier = scopes
            .generated_receiver_function_root(
                self.names,
                &*self.modules,
                &proof,
                method,
                Some(receiver),
            )
            .map_err(scope_parse_error)?;
        let (frontier, _) = scopes
            .bind(self.names, frontier, receiver)
            .map_err(scope_parse_error)?;
        let lexical_scope = Some(scopes.function(frontier).map_err(scope_parse_error)?);
        let name = AstPath::new(self.names, PathAnchor::Relative, &[method])
            .map_err(native_adapter::name_error)?;
        implementation.trait_path = Some(TraitPath::Builtin(BuiltinTrait::Clone));
        implementation.items.push(AssociatedFunction {
            receiver: ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Shared,
            },
            visibility: ir::Visibility::Restricted(declaration.module),
            function: Function {
                lexical_scope,
                module: declaration.module,
                constness: Constness::Runtime,
                unsafety: false,
                static_mut: false,
                visibility: ir::Visibility::Restricted(declaration.module),
                bounds: Vec::new(),
                name,
                regions: RegionDecls::default(),
                type_params: Vec::new(),
                params: Vec::new(),
                param_mut: Vec::new(),
                result: Type::SelfType,
                impl_params: 0,
                body: Block {
                    statements: Vec::new(),
                    tail: Some(Box::new(Expr::Struct {
                        name: declaration.name.clone(),
                        fields: Vec::new(),
                    })),
                },
            },
        });
        Ok(implementation)
    }
}

pub(super) fn validate_unit_clone(
    names: &impl crate::compiler::semantic_path::NameAccess,
    declaration: &StructDecl,
    implementation: &ImplDecl,
    admission: &crate::compiler::semantic_scope::DerivedMemberAdmission,
    scopes: &crate::compiler::semantic_scope::ScopeArena,
) -> Result<(), String> {
    use crate::compiler::semantic_name::{DerivedAtom, validate_derived_identifier};
    if admission.kind() != crate::compiler::semantic_scope::DerivedMemberKind::Clone {
        return Err("generated Clone member kind changed".into());
    }
    let mut shape = implementation.clone();
    shape.trait_path = Some(TraitPath::Builtin(BuiltinTrait::Copy));
    shape.items.clear();
    validate_unit_copy(names, declaration, &shape)?;
    if !matches!(
        implementation.trait_path,
        Some(TraitPath::Builtin(BuiltinTrait::Clone))
    ) || implementation.items.len() != 1
    {
        return Err("generated Clone implementation inventory changed".into());
    }
    let item = &implementation.items[0];
    let function = &item.function;
    let method = function
        .name
        .single()
        .ok_or("generated Clone method path changed")?;
    validate_derived_identifier(names, admission, DerivedAtom::Method, method)
        .map_err(|error| error.to_string())?;
    let scope = function
        .lexical_scope
        .ok_or("generated Clone function scope missing")?;
    let receiver = scopes
        .receiver_binding(names, scope)
        .map_err(|error| error.to_string())?;
    let receiver = scopes
        .binding(receiver)
        .map_err(|error| error.to_string())?
        .declaration;
    validate_derived_identifier(names, admission, DerivedAtom::Receiver, receiver)
        .map_err(|error| error.to_string())?;
    let Some(Expr::Struct { name, fields }) = function.body.tail.as_deref() else {
        return Err("generated Clone body changed".into());
    };
    let mut constructed = shape.clone();
    constructed.self_ty = Type::Named(name.clone());
    validate_unit_copy(names, declaration, &constructed)?;
    if !function.params.is_empty()
        || !matches!(function.result, Type::SelfType)
        || !matches!(
            item.receiver,
            ReceiverKind::Borrowed {
                kind: ir::BorrowKind::Shared
            }
        )
        || item.visibility != ir::Visibility::Restricted(declaration.module)
        || function.visibility != ir::Visibility::Restricted(declaration.module)
        || function.lexical_scope.is_none()
        || function.module != declaration.module
        || function.constness != Constness::Runtime
        || function.unsafety
        || function.static_mut
        || !function.bounds.is_empty()
        || !function.type_params.is_empty()
        || !function.param_mut.is_empty()
        || function.impl_params != 0
        || function.regions.receiver.is_some()
        || !function.regions.params.is_empty()
        || !function.regions.outlives.is_empty()
        || !function.body.statements.is_empty()
        || !fields.is_empty()
    {
        return Err("generated Clone signature/body recipe changed".into());
    }
    Ok(())
}

pub(super) fn unit_copy(declaration: &StructDecl) -> Result<ImplDecl, &'static str> {
    if declaration.shape != StructShape::Unit
        || !declaration.fields.is_empty()
        || !declaration.type_params.is_empty()
        || !declaration.region_params.is_empty()
        || !declaration.bounds.is_empty()
    {
        return Err("generated Copy currently requires a nongeneric unit struct");
    }
    Ok(ImplDecl {
        trait_path: Some(TraitPath::Builtin(BuiltinTrait::Copy)),
        bounds: Vec::new(),
        module: declaration.module,
        type_params: Vec::new(),
        self_ty: Type::Named(declaration.name.clone()),
        items: Vec::new(),
        associated_types: Vec::new(),
        associated_constants: Vec::new(),
    })
}

pub(super) fn validate_unit_copy(
    names: &impl crate::compiler::semantic_path::NameAccess,
    declaration: &StructDecl,
    implementation: &ImplDecl,
) -> Result<(), String> {
    unit_copy(declaration).map_err(str::to_owned)?;
    let Type::Named(actual) = &implementation.self_ty else {
        return Err("generated Copy self type changed".into());
    };
    if !matches!(
        implementation.trait_path,
        Some(TraitPath::Builtin(BuiltinTrait::Copy))
    ) || implementation.module != declaration.module
        || !implementation.bounds.is_empty()
        || !implementation.type_params.is_empty()
        || !implementation.items.is_empty()
        || !implementation.associated_types.is_empty()
        || !implementation.associated_constants.is_empty()
        || actual.anchor() != declaration.name.anchor()
        || actual.segments().len() != declaration.name.segments().len()
    {
        return Err("generated Copy implementation shape changed".into());
    }
    actual.check(names).map_err(|error| error.to_string())?;
    declaration
        .name
        .check(names)
        .map_err(|error| error.to_string())?;
    for (actual, expected) in actual.segments().iter().zip(declaration.name.segments()) {
        if names.span(*actual).map_err(|error| error.to_string())?
            != names.span(*expected).map_err(|error| error.to_string())?
            || names
                .key(*actual, LookupClass::Item(Namespace::Type))
                .map_err(|error| error.to_string())?
                != names
                    .key(*expected, LookupClass::Item(Namespace::Type))
                    .map_err(|error| error.to_string())?
        {
            return Err("generated Copy nominal occurrence changed".into());
        }
    }
    Ok(())
}
