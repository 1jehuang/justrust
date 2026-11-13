//! Constant patterns of aggregate type (`match w { WidthInfo::LF => .. }`,
//! `match e { Q => .. }` with `const Q: E = E::B(P { .. })`).
//!
//! The constant's initializer is evaluated during resolution into the
//! equivalent constructor pattern (struct, tuple struct, enum variant, tuple
//! and array patterns over scalar literal leaves), exactly as rustc lowers a
//! constant's value tree to a pattern. Every nominal type that occurs in the
//! value must be structural-match: its `PartialEq` must come from
//! `#[derive(PartialEq)]`. Otherwise this is stock's "constant of
//! non-structural type" error. Initializers outside this pure constructor
//! form (calls, field reads, floats, generic nominals) keep an explicit
//! unsupported diagnostic.
use super::*;

const DEPTH_LIMIT: usize = 64;

/// The definition context an initializer is evaluated in.
#[derive(Clone)]
struct Context {
    module: ModuleId,
    self_nominal: Option<AstPath>,
}

/// One retained constant initializer selected for evaluation.
struct Selected {
    context: Context,
    ty: Option<Type>,
    initializer: Expr,
}

/// A resolved value constructor and its declared fields.
struct Constructor {
    /// Canonical declaration path of the struct or enum variant.
    path: AstPath,
    /// Canonical path of the owning nominal (struct or enum).
    nominal: AstPath,
    shape: StructShape,
    fields: Vec<(FieldName, Type)>,
    variant: bool,
}

fn unsupported(message: &str) -> String {
    format!("unsupported: constant patterns: {message}")
}

fn scalar_type(ty: &Type) -> bool {
    matches!(ty, Type::Int(_) | Type::Bool | Type::Char)
}

impl Resolver<'_> {
    /// `Some(pattern)` when `path` names an inherent associated constant
    /// (`Owner::NAME` or `Self::NAME`) whose value becomes a pattern.
    pub(super) fn associated_const_pattern(
        &mut self,
        path: &AstPath,
        module: ModuleId,
    ) -> Result<Option<Pattern>, String> {
        let self_nominal = if self.self_allowed {
            self.self_nominal.clone()
        } else {
            None
        };
        let context = Context {
            module,
            self_nominal,
        };
        let Some(selected) = self.associated_constant_initializer(path, &context)? else {
            return Ok(None);
        };
        self.selected_pattern(selected, 0).map(Some)
    }

    /// `Some(pattern)` when the free `const` item at `source` has a
    /// non-scalar type. Scalar constants keep the literal evaluator.
    pub(super) fn aggregate_const_item_pattern(
        &mut self,
        source: &AstPath,
        depth: usize,
    ) -> Result<Option<Pattern>, String> {
        let key = self.key(source)?;
        let mut found = None;
        for declaration in &self.program.constants {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? == key {
                found = Some(declaration);
                break;
            }
        }
        let Some(declaration) = found else {
            return Ok(None);
        };
        if scalar_type(&declaration.ty) {
            return Ok(None);
        }
        let selected = Selected {
            context: Context {
                module: declaration.module,
                self_nominal: None,
            },
            ty: Some(declaration.ty.clone()),
            initializer: declaration.initializer.clone(),
        };
        self.selected_pattern(selected, depth + 1).map(Some)
    }

    fn selected_pattern(&mut self, selected: Selected, depth: usize) -> Result<Pattern, String> {
        let Selected {
            context,
            ty,
            initializer,
        } = selected;
        self.value_pattern(&initializer, &context, ty.as_ref(), depth + 1)
    }

    /// The owner nominal and member name of `Owner::NAME` / `Self::NAME`,
    /// unless the last segment names an enum variant.
    fn constant_owner(
        &mut self,
        path: &AstPath,
        context: &Context,
    ) -> Result<Option<(AstPath, AstIdent)>, String> {
        let Some(&name) = path.segments().last() else {
            return Ok(None);
        };
        let nominal = if path.anchor() == PathAnchor::SelfType && path.len() == 1 {
            let Some(nominal) = context.self_nominal.clone() else {
                return Ok(None);
            };
            nominal
        } else if path.len() >= 2 {
            let prefix = path
                .semantic_slice(self.names(), 0..path.len() - 1, path.anchor())
                .map_err(native_adapter::name_diagnostic)?;
            if prefix.anchor() == PathAnchor::SelfType {
                return Ok(None);
            }
            let Some(item) = self.path(&prefix, context.module, true, false)? else {
                return Ok(None);
            };
            if !matches!(item.kind, ResolvedKind::Struct | ResolvedKind::Enum) {
                return Ok(None);
            }
            item.source()?.clone()
        } else {
            return Ok(None);
        };
        let member = self.semantic_append(&nominal, name)?;
        if self.variant_shape(&member)?.is_some() {
            return Ok(None);
        }
        Ok(Some((nominal, name)))
    }

    /// Canonical nominal path of an inherent impl's self type, if it is a
    /// resolvable source struct or enum.
    fn impl_nominal(&mut self, block: &ImplDecl) -> Result<Option<AstPath>, String> {
        let path = match &block.self_ty {
            Type::Named(path) => path.clone(),
            Type::Apply(path) => path.path(self.names())?,
            _ => return Ok(None),
        };
        if path.anchor() == PathAnchor::SelfType {
            return Ok(None);
        }
        Ok(match self.path(&path, block.module, true, false) {
            Ok(Some(item)) if matches!(item.kind, ResolvedKind::Struct | ResolvedKind::Enum) => {
                Some(item.source()?.clone())
            }
            _ => None,
        })
    }

    fn associated_constant_initializer(
        &mut self,
        path: &AstPath,
        context: &Context,
    ) -> Result<Option<Selected>, String> {
        let Some((nominal, name)) = self.constant_owner(path, context)? else {
            return Ok(None);
        };
        let key = self.key(&nominal)?;
        let program = self.program;
        let mut selected = None;
        for block in &program.impls {
            self.alias_budget.charge_work(1)?;
            if block.trait_path.is_some() {
                continue;
            }
            let Some(owner) = self.impl_nominal(block)? else {
                continue;
            };
            if self.key(&owner)? != key {
                continue;
            }
            let mut found: Option<(ir::Visibility, Option<Type>, Expr)> = None;
            for item in &block.items {
                self.alias_budget.charge_work(1)?;
                if item.lowered_constant
                    && let Some(member) = item.function.name.segments().last()
                    && name_scope::ident_eq(self.names(), *member, name, LookupClass::Member)?
                {
                    let initializer = item
                        .function
                        .body
                        .tail
                        .as_deref()
                        .ok_or("lowered associated constant lost its initializer")?;
                    found = Some((
                        item.visibility.clone(),
                        Some(item.function.result.clone()),
                        initializer.clone(),
                    ));
                }
            }
            for constant in &block.associated_constants {
                self.alias_budget.charge_work(1)?;
                if name_scope::ident_eq(self.names(), constant.name, name, LookupClass::Member)? {
                    let initializer = constant
                        .initializer
                        .clone()
                        .ok_or("inherent associated constant has no initializer")?;
                    found = Some((
                        constant.visibility.clone(),
                        Some(constant.ty.clone()),
                        initializer,
                    ));
                }
            }
            let Some((visibility, ty, initializer)) = found else {
                continue;
            };
            if !block.type_params.is_empty() || !block.const_params.is_empty() {
                return Err(unsupported("associated constants of generic impls"));
            }
            if !visibility
                .accessible_from(&self.program.modules, context.module)
                .map_err(module_diagnostic)?
            {
                return Err(format!(
                    "E0624: associated constant `{}` is private",
                    self.ident_display(name)?
                ));
            }
            if selected.is_some() {
                return Err("E0034: multiple applicable inherent associated constants".into());
            }
            selected = Some(Selected {
                context: Context {
                    module: block.module,
                    self_nominal: Some(owner),
                },
                ty,
                initializer,
            });
        }
        Ok(selected)
    }

    /// Whether `nominal` is structural-match: its `PartialEq` impl was
    /// produced by `#[derive(PartialEq)]`.
    fn structural_nominal(&mut self, nominal: &AstPath) -> Result<bool, String> {
        let key = self.key(nominal)?;
        let program = self.program;
        for block in &program.impls {
            self.alias_budget.charge_work(1)?;
            if !block.derived {
                continue;
            }
            let Some(TraitPath::Source(path)) = &block.trait_path else {
                continue;
            };
            let Some(last) = path.segments.last() else {
                continue;
            };
            if self.ident_display(last.name)? != "PartialEq" {
                continue;
            }
            if let Some(owner) = self.impl_nominal(block)?
                && self.key(&owner)? == key
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn require_structural(&mut self, nominal: &AstPath) -> Result<(), String> {
        if self.structural_nominal(nominal)? {
            return Ok(());
        }
        Err(format!(
            "constant of non-structural type `{}` in a pattern",
            self.display(nominal)?
        ))
    }

    fn struct_constructor(&self, path: &AstPath) -> Result<Constructor, String> {
        let declaration = self.source_struct(path)?;
        if !declaration.type_params.is_empty() || !declaration.const_params.is_empty() {
            return Err(unsupported("values of generic structs"));
        }
        Ok(Constructor {
            path: declaration.name.clone(),
            nominal: declaration.name.clone(),
            shape: declaration.shape,
            fields: declaration
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect(),
            variant: false,
        })
    }

    fn variant_constructor(&self, path: &AstPath) -> Result<Constructor, String> {
        let key = self.key(path)?;
        let owner = path
            .semantic_parent(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        let index = self
            .declaration_index(&owner, |kind| match kind {
                ItemKind::Enum(index) => Some(*index),
                _ => None,
            })?
            .ok_or("resolved variant has no enum declaration")?;
        let declaration = self
            .program
            .enums
            .get(index)
            .ok_or("enum binding names no declaration")?;
        if !declaration.type_params.is_empty() || !declaration.const_params.is_empty() {
            return Err(unsupported("values of generic enums"));
        }
        for variant in &declaration.variants {
            self.alias_budget.charge_work(1)?;
            let candidate = self.semantic_append(&declaration.name, variant.name)?;
            if self.key(&candidate)? != key {
                continue;
            }
            return Ok(Constructor {
                path: candidate,
                nominal: declaration.name.clone(),
                shape: match variant.shape {
                    ir::VariantShape::Unit => StructShape::Unit,
                    ir::VariantShape::Tuple => StructShape::Tuple,
                    ir::VariantShape::Struct => StructShape::Record,
                },
                fields: variant
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect(),
                variant: true,
            });
        }
        Err("resolved variant is not declared by its enum".into())
    }

    /// The constructor named by an initializer path (`W`, `E::V`, `Self`,
    /// `Self::V`), resolved in the initializer's definition context.
    fn value_constructor(
        &mut self,
        name: &AstPath,
        context: &Context,
        type_namespace: bool,
    ) -> Result<Option<Constructor>, String> {
        if name.anchor() == PathAnchor::SelfType {
            let Some(nominal) = context.self_nominal.clone() else {
                return Ok(None);
            };
            return match name.segments() {
                [] => self.struct_constructor(&nominal).map(Some),
                [variant] => {
                    let path = self.semantic_append(&nominal, *variant)?;
                    if self.variant_shape(&path)?.is_none() {
                        return Ok(None);
                    }
                    self.variant_constructor(&path).map(Some)
                }
                _ => Ok(None),
            };
        }
        let Some(item) = self.path(name, context.module, type_namespace, false)? else {
            return Ok(None);
        };
        match item.kind {
            ResolvedKind::Struct => self.struct_constructor(item.source()?).map(Some),
            ResolvedKind::Variant => self.variant_constructor(item.source()?).map(Some),
            _ => Ok(None),
        }
    }

    fn constructor_value_pattern(
        &mut self,
        constructor: Constructor,
        values: ConstructorValues<'_>,
        context: &Context,
        depth: usize,
    ) -> Result<Pattern, String> {
        self.require_structural(&constructor.nominal)?;
        let name = self.resolved_use(&constructor.path, &constructor.path)?;
        let mut positional = Vec::new();
        let mut named = Vec::new();
        match (constructor.shape, values) {
            (StructShape::Unit, ConstructorValues::Unit) => {}
            (StructShape::Tuple, ConstructorValues::Positional(args)) => {
                if args.len() != constructor.fields.len() {
                    return Err("E0061: constructor argument count mismatch in constant".into());
                }
                for (index, (value, (_, ty))) in args.iter().zip(&constructor.fields).enumerate() {
                    let pattern = self.value_pattern(value, context, Some(ty), depth + 1)?;
                    positional.push((FieldSelector::Tuple(index), pattern));
                }
            }
            (StructShape::Record, ConstructorValues::Named(args)) => {
                if args.len() != constructor.fields.len() {
                    return Err(unsupported("struct literals with missing or extra fields"));
                }
                for (field, ty) in &constructor.fields {
                    let FieldName::Named(field) = field else {
                        return Err("named struct declares a positional field".into());
                    };
                    let mut value = None;
                    for (written, expr) in args {
                        self.alias_budget.charge_work(1)?;
                        if name_scope::ident_eq(
                            self.names(),
                            *written,
                            *field,
                            LookupClass::Member,
                        )? {
                            value = Some(expr);
                        }
                    }
                    let value = value.ok_or_else(|| {
                        unsupported("struct literals with missing or extra fields")
                    })?;
                    let pattern = self.value_pattern(value, context, Some(ty), depth + 1)?;
                    named.push((*field, pattern));
                }
            }
            _ => return Err("constructor shape mismatch in constant initializer".into()),
        }
        Ok(match (constructor.shape, constructor.variant) {
            (StructShape::Unit, true) => Pattern::Path(name),
            (StructShape::Unit, false) => Pattern::Struct {
                name,
                fields: Vec::new(),
                rest: false,
            },
            (StructShape::Tuple, _) => Pattern::Variant {
                name,
                shape: ir::VariantShape::Tuple,
                fields: positional,
                rest: false,
            },
            (StructShape::Record, true) => Pattern::Variant {
                name,
                shape: ir::VariantShape::Struct,
                fields: named
                    .into_iter()
                    .map(|(field, pattern)| (FieldSelector::Named(field), pattern))
                    .collect(),
                rest: false,
            },
            (StructShape::Record, false) => Pattern::Struct {
                name,
                fields: named,
                rest: false,
            },
        })
    }

    /// Evaluate one initializer expression into its pattern.
    fn value_pattern(
        &mut self,
        value: &Expr,
        context: &Context,
        ty: Option<&Type>,
        depth: usize,
    ) -> Result<Pattern, String> {
        if depth > DEPTH_LIMIT {
            return Err(format!(
                "constant pattern evaluation depth exceeds limit {DEPTH_LIMIT}"
            ));
        }
        self.alias_budget.charge_work(1)?;
        match value {
            Expr::Block(block) if block.statements.is_empty() => {
                let tail = block
                    .tail
                    .as_deref()
                    .ok_or_else(|| unsupported("empty block initializers"))?;
                return self.value_pattern(tail, context, ty, depth + 1);
            }
            Expr::Call { name, args } => {
                if let Some(constructor) = self.value_constructor(name, context, false)? {
                    return self.constructor_value_pattern(
                        constructor,
                        ConstructorValues::Positional(args),
                        context,
                        depth,
                    );
                }
                return Err(unsupported("function calls in constant initializers"));
            }
            Expr::TupleStruct { name, fields } => {
                let constructor = self
                    .value_constructor(name, context, false)?
                    .ok_or_else(|| unsupported("unresolved tuple constructor"))?;
                return self.constructor_value_pattern(
                    constructor,
                    ConstructorValues::Positional(fields),
                    context,
                    depth,
                );
            }
            Expr::Struct { name, fields } => {
                let constructor = self
                    .value_constructor(name, context, true)?
                    .ok_or_else(|| unsupported("unresolved struct literal"))?;
                return self.constructor_value_pattern(
                    constructor,
                    ConstructorValues::Named(fields),
                    context,
                    depth,
                );
            }
            Expr::Tuple(items) => {
                let types = match ty {
                    Some(Type::Tuple(types)) if types.len() == items.len() => Some(types),
                    _ => None,
                };
                let mut patterns = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    let ty = types.map(|types| &types[index]);
                    patterns.push(self.value_pattern(item, context, ty, depth + 1)?);
                }
                return Ok(Pattern::Tuple(patterns));
            }
            Expr::Array(items) => {
                let element = match ty {
                    Some(Type::Array(element, _)) => Some(element.as_ref()),
                    _ => None,
                };
                let mut patterns = Vec::new();
                for item in items {
                    patterns.push(self.value_pattern(item, context, element, depth + 1)?);
                }
                return Ok(Pattern::Array(patterns));
            }
            Expr::Str(text) => return Ok(Pattern::Str(text.clone())),
            Expr::Var(path) | Expr::ConstItem(path) => {
                if let Some(selected) = self.associated_constant_initializer(path, context)? {
                    return self.selected_pattern(selected, depth + 1);
                }
                if path.anchor() == PathAnchor::SelfType {
                    if let Some(constructor) = self.value_constructor(path, context, false)? {
                        return self.constructor_value_pattern(
                            constructor,
                            ConstructorValues::Unit,
                            context,
                            depth,
                        );
                    }
                    return Err(unsupported("unresolved Self path"));
                }
                if let Some(item) = self.path(path, context.module, false, false)? {
                    match item.kind {
                        ResolvedKind::Const => {
                            let source = item.source()?.clone();
                            if let Some(pattern) =
                                self.aggregate_const_item_pattern(&source, depth + 1)?
                            {
                                return Ok(pattern);
                            }
                        }
                        ResolvedKind::Variant => {
                            let constructor = self.variant_constructor(item.source()?)?;
                            return self.constructor_value_pattern(
                                constructor,
                                ConstructorValues::Unit,
                                context,
                                depth,
                            );
                        }
                        _ => {}
                    }
                } else if let Some(item) = self.path(path, context.module, true, false)?
                    && self.is_unit_struct(&item)?
                {
                    let constructor = self.struct_constructor(item.source()?)?;
                    return self.constructor_value_pattern(
                        constructor,
                        ConstructorValues::Unit,
                        context,
                        depth,
                    );
                }
            }
            _ => {}
        }
        if let Some(ty) = ty
            && !scalar_type(ty)
        {
            return Err(unsupported(
                "initializers that are not constructors, tuples, arrays or scalar literals",
            ));
        }
        let hint = match ty {
            Some(Type::Int(ty)) => Some(*ty),
            _ => None,
        };
        self.scalar_value_pattern(value, context.module, hint, depth + 1)
    }
}

enum ConstructorValues<'a> {
    Unit,
    Positional(&'a [Expr]),
    Named(&'a [(AstIdent, Expr)]),
}
