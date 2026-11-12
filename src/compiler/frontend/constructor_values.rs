//! Tuple constructors used as values (`.map(Some)`, `let f: fn(i32) -> E =
//! E::A`). Each tuple variant or tuple struct declaration owns one compiler
//! function item `$ctor$Owner$Variant<T..>(a0: F0, ..) -> Owner<T..>` emitted
//! beside the declaration itself, so the value is the exact zero-sized
//! `Ty::FnItem` of that declaration: Copy, coercible to a function pointer and
//! callable through `Fn*`. Constructor calls (`E::A(x)`) never use it.
use super::*;

/// The written owner shape a constructor function is emitted for.
pub(super) struct ConstructorOwner<'a> {
    pub(super) module: ModuleId,
    pub(super) path: &'a AstPath,
    pub(super) name: AstIdent,
    pub(super) type_params: &'a [AstIdent],
    pub(super) region_params: &'a [AstIdent],
    pub(super) has_const_params: bool,
    pub(super) bounds: &'a [TypeBound],
    pub(super) visibility: &'a ir::Visibility,
}

impl Parser<'_, '_> {
    /// Emits the constructor function for one tuple shape and returns its
    /// canonical path. Unsupported owners get no function: their value uses
    /// keep the explicit unsupported diagnostic in resolution.
    pub(super) fn constructor_value_function(
        &mut self,
        parsed: &mut ParsedDeclarations,
        owner: &ConstructorOwner<'_>,
        variant: Option<AstIdent>,
        fields: &[FieldDecl],
    ) -> Result<Option<AstPath>, ParseError> {
        if self.purpose == ParserPurpose::InactiveSyntax
            || self.expansion.is_some()
            || self.item_expansion.is_some()
            || fields.is_empty()
            || !owner.region_params.is_empty()
            || owner.has_const_params
        {
            return Ok(None);
        }
        let budget = ir::PlanBudget::default();
        for field in fields {
            if source_type_mentions_self(&field.ty, &budget, 0).map_err(|e| self.error(&e))? {
                return Ok(None);
            }
        }
        let anchor = variant.unwrap_or(owner.name);
        let origin = self
            .names
            .span(anchor)
            .map_err(native_adapter::name_error)?;
        let origin = match panic_callsite(self.names.session(), origin) {
            Ok(origin) => origin,
            Err(_) => return Ok(None),
        };
        let ident = match self
            .names
            .anonymous_constructor(origin, owner.name, variant)
        {
            Ok(ident) => ident,
            Err(super::super::semantic_name::NameError::Unsupported(_)) => return Ok(None),
            Err(error) => return Err(native_adapter::name_error(error)),
        };
        let module_path = owner
            .path
            .semantic_parent(self.names)
            .map_err(native_adapter::name_error)?;
        let name = module_path
            .push(self.names, ident)
            .map_err(native_adapter::name_error)?;
        let mut params = Vec::new();
        let mut args = Vec::new();
        self.reserve_parser_items(&mut params, fields.len())?;
        self.reserve_parser_items(&mut args, fields.len())?;
        for (ordinal, field) in fields.iter().enumerate() {
            let local = self
                .names
                .anonymous_parameter(origin, ordinal)
                .map_err(native_adapter::name_error)?;
            params.push((BindingName::Named(local), field.ty.clone()));
            args.push(Expr::Var(
                AstPath::new(self.names, PathAnchor::Relative, &[local])
                    .map_err(native_adapter::name_error)?,
            ));
        }
        // The function lives in the owner's module, where the owner's own
        // name resolves to it. Absolute `crate::` paths would instead name
        // the mount root inside library crates.
        let local_owner = AstPath::new(self.names, PathAnchor::Relative, &[owner.name])
            .map_err(native_adapter::name_error)?;
        let result = if owner.type_params.is_empty() {
            Type::Named(local_owner.clone())
        } else {
            Type::Apply(GenericPath {
                anchor: PathAnchor::Relative,
                segments: vec![PathSegment {
                    name: owner.name,
                    region_args: Vec::new(),
                    args: owner
                        .type_params
                        .iter()
                        .map(|param| Type::Param(*param))
                        .collect(),
                    associated_types: Vec::new(),
                }],
                resolved_use: None,
            })
        };
        let callee = match variant {
            Some(variant) => local_owner
                .push(self.names, variant)
                .map_err(native_adapter::name_error)?,
            None => local_owner,
        };
        // A constructor is only as visible as its least visible field.
        // Restricting to the declaring module is conservative for
        // `pub(crate)`/`pub(in ..)` fields: it may reject, never widen.
        let visibility = if fields
            .iter()
            .all(|field| field.visibility == ir::Visibility::Public)
        {
            owner.visibility.clone()
        } else {
            ir::Visibility::Restricted(owner.module)
        };
        let param_mut = vec![false; params.len()];
        let mut functions = std::mem::take(&mut parsed.program.functions);
        self.reserve_parser_items(&mut functions, 1)?;
        functions.push(Function {
            const_params: Vec::new(),
            target_feature: None,
            embedded_source: None,
            direct_return_source: None,
            suspended_source: None,
            lexical_scope: None,
            module: owner.module,
            constness: Constness::Runtime,
            unsafety: false,
            static_mut: false,
            visibility,
            bounds: owner.bounds.to_vec(),
            name: name.clone(),
            regions: RegionDecls::default(),
            type_params: owner.type_params.to_vec(),
            params,
            param_mut,
            result,
            impl_params: 0,
            body: Block {
                statements: Vec::new(),
                tail: Some(Box::new(Expr::Call { name: callee, args })),
            },
        });
        parsed.program.functions = functions;
        Ok(Some(name))
    }
}
