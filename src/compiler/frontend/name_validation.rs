//! Universal checked-handle admission for the complete published source AST.
//! This walk never resolves printable names or grants semantic publication.
use super::*;
use crate::compiler::expansion_budget::{Counter, Storage};
use crate::compiler::module_identity::ModuleError;
use crate::compiler::token_session::{BudgetVec, TokenError};

fn transport(error: TokenError) -> String {
    super::super::source::SourceError::from_token(error).to_string()
}

/// Every child is visited, including unused declarations, unreachable bodies,
/// unmatched patterns, public fields, and unselected generic arguments.
/// No recursion or unaccounted traversal storage is used here. Existing source
/// depth validation remains a separate gate, with its existing policy unchanged.
pub(super) fn validate_program_names(program: &Program) -> Result<(), String> {
    let mut validator = Validator::new(program)?;
    validator.many(&program.functions, |value| Visit::Function(value, None))?;
    validator.many(&program.structs, Visit::Struct)?;
    validator.many(&program.enums, Visit::Enum)?;
    validator.many(&program.impls, Visit::Impl)?;
    validator.many(&program.traits, Visit::Trait)?;
    validator.work(program.trait_imports.len())?;
    for (module, path) in &program.trait_imports {
        validator.source_module(*module)?;
        validator.push(Visit::TraitPath(path))?;
    }
    validator.run()
}

/// Active-callable ownership admission after the plan's exact Arc guard.
/// This bounded walk does not recertify unrelated declarations or compare the
/// callable with frozen registry metadata. The caller must perform that typed
/// metadata/provenance check independently before dispatching a body or recipe.
pub(super) fn validate_function_names(
    program: &Program,
    function: &Function,
    containing_module: Option<ModuleId>,
) -> Result<(), String> {
    let mut validator = Validator::new(program)?;
    validator.push(Visit::Function(function, containing_module))?;
    validator.run()
}

/// A trait declaration's head is mutable separately from its default body.
/// Validate it before comparing against the retained immutable signature; this
/// does not by itself certify equality or select a callable declaration.
pub(super) fn validate_function_head_names(
    program: &Program,
    head: &FunctionHead,
) -> Result<(), String> {
    let mut validator = Validator::new(program)?;
    validator.push(Visit::Head(head))?;
    validator.run()
}

enum Visit<'a> {
    Function(&'a Function, Option<ModuleId>),
    Head(&'a FunctionHead),
    Struct(&'a StructDecl),
    Enum(&'a EnumDecl),
    Variant(&'a VariantDecl, ModuleId),
    Field(&'a FieldDecl, ModuleId),
    Impl(&'a ImplDecl),
    Associated(&'a AssociatedFunction, ModuleId),
    Trait(&'a TraitDecl),
    TraitItem(&'a TraitItemDecl, ModuleId),
    Bound(&'a TypeBound),
    Regions(&'a RegionDecls),
    Type(&'a Type),
    GenericPath(&'a GenericPath),
    TraitPath(&'a TraitPath),
    Block(&'a Block),
    Stmt(&'a Stmt),
    Expr(&'a Expr),
    Pattern(&'a Pattern),
    Place(&'a Place),
    Selector(&'a FieldSelector),
    Path(&'a AstPath),
    Ident(AstIdent, LookupClass),
    Region(RegionName),
    Binding(BindingName),
    FieldName(FieldName),
    LocalName(LocalName),
}
struct Validator<'a> {
    program: &'a Program,
    root: ModuleId,
    pending: BudgetVec<Visit<'a>>,
}
impl<'a> Validator<'a> {
    fn new(program: &'a Program) -> Result<Self, String> {
        let names = program.names.as_ref();
        let modules = program.modules.as_ref();
        program.symbols.check(names)?;
        modules.check_names(names).map_err(module_diagnostic)?;
        if !modules.budget().same_authority(names.budget()) {
            return Err(module_diagnostic(ModuleError::ForeignNames));
        }
        Ok(Self {
            program,
            root: modules.source_root().map_err(module_diagnostic)?,
            pending: BudgetVec::new(names.budget(), Storage::Temporary).map_err(transport)?,
        })
    }
    fn run(mut self) -> Result<(), String> {
        while let Some(node) = self.pending.pop() {
            self.work(1)?;
            self.visit(node)?;
        }
        Ok(())
    }
    fn work(&self, amount: usize) -> Result<(), String> {
        self.program
            .names
            .budget()
            .charge(Counter::Work, amount as u64)
            .map_err(|error| transport(TokenError::Budget(error)))
    }
    fn push(&mut self, node: Visit<'a>) -> Result<(), String> {
        self.work(1)?;
        self.pending.push(node).map_err(transport)
    }
    // Precharge the whole flat scan before reading or enqueueing its children.
    // BudgetVec additionally admits actual allocator-rounded capacity.
    fn many<T>(
        &mut self,
        values: &'a [T],
        mut node: impl FnMut(&'a T) -> Visit<'a>,
    ) -> Result<(), String> {
        self.work(values.len())?;
        for value in values {
            self.push(node(value))?;
        }
        Ok(())
    }
    fn source_module(&self, module: ModuleId) -> Result<(), String> {
        let root = self
            .program
            .modules
            .root_of(module)
            .map_err(module_diagnostic)?;
        if root != self.root {
            return Err(module_diagnostic(ModuleError::InvalidParent));
        }
        Ok(())
    }
    fn visibility(&self, visibility: &ir::Visibility, module: ModuleId) -> Result<(), String> {
        self.source_module(module)?;
        visibility
            .validate_for(self.program.modules.as_ref(), module)
            .map_err(module_diagnostic)
    }
    fn declaration_path(&self, path: &AstPath, module: ModuleId) -> Result<(), String> {
        self.work(path.len())?;
        path.check(self.program.names.as_ref())
            .map_err(native_adapter::name_diagnostic)?;
        if path.is_empty() || path.anchor() != PathAnchor::Crate {
            return Err(module_diagnostic(ModuleError::InvalidParent));
        }
        // Verify a canonical path against the already authoritative module,
        // never infer or replace that authority from a printable name.
        let prefix = &path.segments()[..path.len() - 1];
        self.work(prefix.len())?;
        let mut expected = self.root;
        for segment in prefix {
            expected = self
                .program
                .modules
                .lookup_child(self.program.names.as_ref(), expected, *segment)
                .map_err(module_diagnostic)?
                .ok_or_else(|| module_diagnostic(ModuleError::InvalidParent))?;
        }
        if expected != module {
            return Err(module_diagnostic(ModuleError::InvalidParent));
        }
        Ok(())
    }
    fn parameters(&mut self, values: &'a [(BindingName, Type)]) -> Result<(), String> {
        self.work(values.len())?;
        for (name, ty) in values {
            self.push(Visit::Binding(*name))?;
            self.push(Visit::Type(ty))?;
        }
        Ok(())
    }
    fn identifiers(&mut self, values: &'a [AstIdent], class: LookupClass) -> Result<(), String> {
        self.many(values, |name| Visit::Ident(*name, class))
    }
    fn pattern_fields(&mut self, fields: &'a [(FieldSelector, Pattern)]) -> Result<(), String> {
        self.work(fields.len())?;
        for (selector, pattern) in fields {
            self.push(Visit::Selector(selector))?;
            self.push(Visit::Pattern(pattern))?;
        }
        Ok(())
    }
    fn expression_fields(&mut self, fields: &'a [(FieldSelector, Expr)]) -> Result<(), String> {
        self.work(fields.len())?;
        for (selector, expression) in fields {
            self.push(Visit::Selector(selector))?;
            self.push(Visit::Expr(expression))?;
        }
        Ok(())
    }
    fn visit(&mut self, node: Visit<'a>) -> Result<(), String> {
        match node {
            Visit::Function(function, containing_module) => {
                self.visibility(&function.visibility, function.module)?;
                if let Some(module) = containing_module {
                    self.source_module(module)?;
                    if module != function.module {
                        return Err(module_diagnostic(ModuleError::InvalidParent));
                    }
                }
                if containing_module.is_some() {
                    self.push(Visit::Path(&function.name))?;
                } else {
                    self.declaration_path(&function.name, function.module)?;
                }
                self.push(Visit::Regions(&function.regions))?;
                self.identifiers(&function.type_params, LookupClass::TypeParameter)?;
                self.many(&function.bounds, Visit::Bound)?;
                self.parameters(&function.params)?;
                self.push(Visit::Type(&function.result))?;
                self.push(Visit::Block(&function.body))?;
            }
            Visit::Head(head) => {
                self.push(Visit::Ident(head.name, LookupClass::Member))?;
                self.push(Visit::Regions(&head.regions))?;
                self.identifiers(&head.type_params, LookupClass::TypeParameter)?;
                self.many(&head.bounds, Visit::Bound)?;
                self.parameters(&head.params)?;
                self.push(Visit::Type(&head.result))?;
            }
            Visit::Struct(declaration) => {
                self.visibility(&declaration.visibility, declaration.module)?;
                self.declaration_path(&declaration.name, declaration.module)?;
                self.identifiers(&declaration.type_params, LookupClass::TypeParameter)?;
                self.many(&declaration.fields, |field| {
                    Visit::Field(field, declaration.module)
                })?;
            }
            Visit::Enum(declaration) => {
                self.visibility(&declaration.visibility, declaration.module)?;
                self.declaration_path(&declaration.name, declaration.module)?;
                self.identifiers(&declaration.type_params, LookupClass::TypeParameter)?;
                self.many(&declaration.variants, |variant| {
                    Visit::Variant(variant, declaration.module)
                })?;
            }
            Visit::Variant(variant, module) => {
                self.push(Visit::Ident(
                    variant.name,
                    LookupClass::Item(Namespace::Value),
                ))?;
                self.many(&variant.fields, |field| Visit::Field(field, module))?;
            }
            Visit::Field(field, module) => {
                self.visibility(&field.visibility, module)?;
                self.push(Visit::FieldName(field.name))?;
                self.push(Visit::Type(&field.ty))?;
            }
            Visit::Impl(implementation) => {
                self.source_module(implementation.module)?;
                if let Some(path) = &implementation.trait_path {
                    self.push(Visit::TraitPath(path))?;
                }
                self.identifiers(&implementation.type_params, LookupClass::TypeParameter)?;
                self.many(&implementation.bounds, Visit::Bound)?;
                self.push(Visit::Type(&implementation.self_ty))?;
                self.many(&implementation.items, |item| {
                    Visit::Associated(item, implementation.module)
                })?;
            }
            Visit::Associated(item, module) => {
                self.visibility(&item.visibility, module)?;
                self.push(Visit::Function(&item.function, Some(module)))?;
            }
            Visit::Trait(declaration) => {
                self.visibility(&declaration.visibility, declaration.module)?;
                self.declaration_path(&declaration.name, declaration.module)?;
                self.many(&declaration.supertraits, Visit::TraitPath)?;
                self.many(&declaration.items, |item| {
                    Visit::TraitItem(item, declaration.module)
                })?;
            }
            Visit::TraitItem(item, module) => {
                self.push(Visit::Head(&item.signature))?;
                if let Some(function) = &item.default {
                    self.push(Visit::Function(function, Some(module)))?;
                }
            }
            Visit::Bound(bound) => {
                self.push(Visit::Ident(bound.parameter, LookupClass::TypeParameter))?;
                self.many(&bound.traits, Visit::TraitPath)?;
            }
            Visit::Regions(regions) => {
                if let Some(region) = regions.receiver {
                    self.push(Visit::Region(region))?;
                }
                self.identifiers(&regions.params, LookupClass::LifetimeParameter)?;
                self.work(regions.outlives.len())?;
                for (left, right) in &regions.outlives {
                    self.push(Visit::Region(*left))?;
                    self.push(Visit::Region(*right))?;
                }
            }
            Visit::Type(ty) => match ty {
                Type::Param(name) => self.push(Visit::Ident(*name, LookupClass::TypeParameter))?,
                Type::Named(path) => self.push(Visit::Path(path))?,
                Type::Apply(path) => self.push(Visit::GenericPath(path))?,
                Type::Ref {
                    region, referent, ..
                } => {
                    if let Some(region) = region {
                        self.push(Visit::Region(*region))?;
                    }
                    self.push(Visit::Type(referent))?;
                }
                Type::Array(element, _) | Type::Slice(element) => {
                    self.push(Visit::Type(element))?
                }
                Type::Tuple(elements) => self.many(elements, Visit::Type)?,
                Type::SelfType
                | Type::Int(_)
                | Type::Float(_)
                | Type::Bool
                | Type::Unit
                | Type::Never
                | Type::Infer => (),
            },
            Visit::GenericPath(path) => {
                self.work(path.segments.len())?;
                for segment in &path.segments {
                    self.push(Visit::Ident(
                        segment.name,
                        LookupClass::Item(Namespace::Type),
                    ))?;
                    self.many(&segment.args, Visit::Type)?;
                }
            }
            Visit::TraitPath(path) => match path {
                TraitPath::Source(path) => self.push(Visit::Path(path))?,
                TraitPath::Builtin(
                    BuiltinTrait::Sized
                    | BuiltinTrait::Copy
                    | BuiltinTrait::Clone
                    | BuiltinTrait::Drop,
                ) => (),
            },
            Visit::Block(block) => {
                self.many(&block.statements, Visit::Stmt)?;
                if let Some(tail) = &block.tail {
                    self.push(Visit::Expr(tail))?;
                }
            }
            Visit::Stmt(statement) => match statement {
                Stmt::Declare { name, ty, .. } => {
                    self.push(Visit::Binding(*name))?;
                    self.push(Visit::Type(ty))?;
                }
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    self.push(Visit::Binding(*name))?;
                    self.push(Visit::Type(ty))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::LetPattern { pattern, ty, value } => {
                    self.push(Visit::Pattern(pattern))?;
                    self.push(Visit::Type(ty))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::Assign { name, value } => {
                    self.push(Visit::LocalName(*name))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::AssignDeref { carrier, value } => {
                    self.push(Visit::Expr(carrier))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::AssignIndex {
                    array,
                    index,
                    value,
                }
                | Stmt::AssignIndexOp {
                    array,
                    index,
                    value,
                    ..
                } => {
                    self.push(Visit::Place(array))?;
                    self.push(Visit::Expr(index))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::AssignOp { place, value, .. } | Stmt::AssignPlace { place, value } => {
                    self.push(Visit::Place(place))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::Expr(expression) | Stmt::ImplicitExpr(expression) => {
                    self.push(Visit::Expr(expression))?
                }
                Stmt::Return(value) => {
                    if let Some(value) = value {
                        self.push(Visit::Expr(value))?;
                    }
                }
                Stmt::While {
                    label,
                    condition,
                    body,
                } => {
                    if let Some(label) = label {
                        self.push(Visit::Ident(*label, LookupClass::Label))?;
                    }
                    self.push(Visit::Expr(condition))?;
                    self.push(Visit::Block(body))?;
                }
            },
            Visit::Expr(expression) => match expression {
                Expr::TraitCall {
                    owner,
                    trait_path,
                    name,
                    args,
                } => {
                    self.push(Visit::Type(owner))?;
                    self.push(Visit::TraitPath(trait_path))?;
                    self.push(Visit::Ident(*name, LookupClass::Member))?;
                    self.many(args, Visit::Expr)?;
                }
                Expr::MethodCall {
                    receiver,
                    name,
                    type_args,
                    args,
                } => {
                    self.push(Visit::Expr(receiver))?;
                    self.push(Visit::Ident(*name, LookupClass::Member))?;
                    self.many(type_args, Visit::Type)?;
                    self.many(args, Visit::Expr)?;
                }
                Expr::AssociatedCall {
                    owner,
                    name,
                    type_args,
                    args,
                } => {
                    self.push(Visit::Type(owner))?;
                    self.push(Visit::Ident(*name, LookupClass::Member))?;
                    self.many(type_args, Visit::Type)?;
                    self.many(args, Visit::Expr)?;
                }
                Expr::Apply { path, value } => {
                    self.push(Visit::GenericPath(path))?;
                    self.push(Visit::Expr(value))?;
                }
                Expr::Cast { value, target } => {
                    self.push(Visit::Expr(value))?;
                    self.push(Visit::Type(target))?;
                }
                Expr::Borrow { place, .. } => self.push(Visit::Place(place))?,
                Expr::Deref { value }
                | Expr::Repeat { value, .. }
                | Expr::Index { base: value, .. }
                | Expr::Unary { value, .. } => self.push(Visit::Expr(value))?,
                Expr::DynamicIndex { base, index } => {
                    self.push(Visit::Expr(base))?;
                    self.push(Visit::Expr(index))?;
                }
                Expr::Array(elements) | Expr::Tuple(elements) => {
                    self.many(elements, Visit::Expr)?
                }
                Expr::EnumVariant { name, fields, .. } => {
                    self.push(Visit::Path(name))?;
                    self.expression_fields(fields)?;
                }
                Expr::Match { scrutinee, arms } => {
                    self.push(Visit::Expr(scrutinee))?;
                    self.work(arms.len())?;
                    for arm in arms {
                        self.push(Visit::Pattern(&arm.pattern))?;
                        self.push(Visit::Expr(&arm.body))?;
                    }
                }
                Expr::Struct { name, fields } => {
                    self.push(Visit::Path(name))?;
                    self.work(fields.len())?;
                    for (name, value) in fields {
                        self.push(Visit::Ident(*name, LookupClass::Member))?;
                        self.push(Visit::Expr(value))?;
                    }
                }
                Expr::Field { base, field } => {
                    self.push(Visit::Expr(base))?;
                    self.push(Visit::Selector(field))?;
                }
                Expr::Var(path) => self.push(Visit::Path(path))?,
                Expr::Binary { left, right, .. } => {
                    self.push(Visit::Expr(left))?;
                    self.push(Visit::Expr(right))?;
                }
                Expr::Call { name, args } => {
                    self.push(Visit::Path(name))?;
                    self.many(args, Visit::Expr)?;
                }
                Expr::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.push(Visit::Expr(condition))?;
                    self.push(Visit::Block(then_block))?;
                    self.push(Visit::Block(else_block))?;
                }
                Expr::Block(block) => self.push(Visit::Block(block))?,
                Expr::LabeledBlock { label, body } => {
                    self.push(Visit::Ident(*label, LookupClass::Label))?;
                    self.push(Visit::Block(body))?;
                }
                Expr::Loop { label, body } => {
                    if let Some(label) = label {
                        self.push(Visit::Ident(*label, LookupClass::Label))?;
                    }
                    self.push(Visit::Block(body))?;
                }
                Expr::Break { label, value } => {
                    if let Some(label) = label {
                        self.push(Visit::Ident(*label, LookupClass::Label))?;
                    }
                    if let Some(value) = value {
                        self.push(Visit::Expr(value))?;
                    }
                }
                Expr::Continue { label } => {
                    if let Some(label) = label {
                        self.push(Visit::Ident(*label, LookupClass::Label))?;
                    }
                }
                Expr::Integer(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Println(_) => (),
            },
            Visit::Pattern(pattern) => match pattern {
                Pattern::Apply { path, pattern } => {
                    self.push(Visit::GenericPath(path))?;
                    self.push(Visit::Pattern(pattern))?;
                }
                Pattern::Variant { name, fields, .. } => {
                    self.push(Visit::Path(name))?;
                    self.pattern_fields(fields)?;
                }
                Pattern::Path(path) => self.push(Visit::Path(path))?,
                Pattern::Binding { name, .. } => {
                    self.push(Visit::Ident(*name, LookupClass::Local))?
                }
                Pattern::Tuple(elements) => self.many(elements, Visit::Pattern)?,
                Pattern::Struct { name, fields, .. } => {
                    self.push(Visit::Path(name))?;
                    self.work(fields.len())?;
                    for (name, pattern) in fields {
                        self.push(Visit::Ident(*name, LookupClass::Member))?;
                        self.push(Visit::Pattern(pattern))?;
                    }
                }
                Pattern::Bool(_) | Pattern::Int { .. } | Pattern::Wildcard => (),
            },
            Visit::Place(place) => {
                self.push(Visit::LocalName(place.local))?;
                self.many(&place.fields, Visit::Selector)?;
            }
            Visit::Selector(selector) => match selector {
                FieldSelector::Named(name) => {
                    self.push(Visit::Ident(*name, LookupClass::Member))?
                }
                FieldSelector::Deref
                | FieldSelector::ConstantIndex(_)
                | FieldSelector::Tuple(_) => (),
            },
            Visit::Path(path) => {
                self.work(path.len())?;
                path.check(self.program.names.as_ref())
                    .map_err(native_adapter::name_diagnostic)?;
            }
            Visit::Ident(name, class) => {
                self.program
                    .names
                    .key(name, class)
                    .map_err(native_adapter::name_diagnostic)?;
            }
            Visit::Region(region) => match region {
                RegionName::Named(name) => {
                    self.push(Visit::Ident(name, LookupClass::LifetimeParameter))?
                }
                RegionName::Static | RegionName::Infer => (),
            },
            Visit::Binding(binding) => match binding {
                BindingName::Named(name) => self.push(Visit::Ident(name, LookupClass::Local))?,
                BindingName::Wildcard => (),
            },
            Visit::FieldName(field) => match field {
                FieldName::Named(name) => self.push(Visit::Ident(name, LookupClass::Member))?,
                FieldName::Tuple(_) => (),
            },
            Visit::LocalName(local) => match local {
                LocalName::Named(name) => self.push(Visit::Ident(name, LookupClass::Local))?,
                LocalName::Receiver => (),
            },
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;

    fn builder() -> ProgramBuilder {
        ProgramBuilder::new(
            std::path::Path::new("name-validation.rs"),
            "",
            &ExpansionBudget::default(),
        )
        .unwrap()
    }
    fn function(builder: &mut ProgramBuilder) -> Function {
        Function {
            module: builder.module_root().unwrap(),
            constness: Constness::Runtime,
            visibility: ir::Visibility::Public,
            bounds: Vec::new(),
            name: builder.path("crate::unused").unwrap(),
            regions: RegionDecls::default(),
            type_params: Vec::new(),
            params: Vec::new(),
            result: Type::Unit,
            body: Block {
                statements: Vec::new(),
                tail: None,
            },
        }
    }
    fn program() -> Program {
        let mut builder = builder();
        let function = function(&mut builder);
        builder.functions.push(function);
        builder.finish().unwrap()
    }
    #[test]
    fn empty_program_still_checks_exact_name_arena_association() {
        let mut left = builder().finish().unwrap();
        let right = builder().finish().unwrap();
        left.names = right.names;
        left.symbols = right.symbols;
        assert!(validate_program_names(&left).is_err());
    }
    #[test]
    fn empty_program_checks_symbol_owner_before_optional_symbol_use() {
        let mut input = builder().finish().unwrap();
        let same_owner_symbols = input.symbols.clone();
        input.symbols = same_owner_symbols;
        validate_program_names(&input).unwrap();
        let foreign = builder().finish().unwrap();
        input.symbols = foreign.symbols;
        assert!(validate_program_names(&input).is_err());
    }
    #[test]
    fn unused_public_declarations_cannot_use_catalog_or_foreign_modules() {
        let mut input = program();
        input.functions[0].module = input.modules.core_root();
        assert!(validate_program_names(&input).is_err());
        let foreign = program();
        input.functions[0].module = foreign.modules.source_root().unwrap();
        assert!(validate_program_names(&input).is_err());
    }
    #[test]
    fn all_type_children_regions_and_unreachable_body_handles_are_checked() {
        let mut foreign = builder();
        let name = foreign.ident("foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        let valid = program();
        let mut input = valid.clone();
        input.functions[0].result = Type::Tuple(vec![Type::Unit, Type::Param(name)]);
        assert!(validate_program_names(&input).is_err());
        let mut input = valid.clone();
        input.functions[0]
            .regions
            .outlives
            .push((RegionName::Static, RegionName::Named(name)));
        assert!(validate_program_names(&input).is_err());
        let mut input = valid.clone();
        input.functions[0]
            .body
            .statements
            .push(Stmt::Expr(Expr::Integer(IntegerLiteral {
                magnitude: 0,
                suffix: None,
            })));
        input.functions[0].body.statements.push(Stmt::Return(None));
        input.functions[0].body.statements.push(Stmt::LetPattern {
            pattern: Pattern::Tuple(vec![
                Pattern::Wildcard,
                Pattern::Binding {
                    name,
                    mutable: false,
                },
            ]),
            ty: Type::Unit,
            value: Expr::Bool(false),
        });
        assert!(validate_program_names(&input).is_err());
        let budget = valid.names.budget();
        let before = budget.snapshot().temporary_bytes;
        validate_program_names(&valid).unwrap();
        assert_eq!(budget.snapshot().temporary_bytes, before);
    }
    #[test]
    fn public_field_names_and_visibility_scopes_are_not_skipped() {
        let mut foreign = builder();
        let field_name = foreign.ident("field").unwrap();
        let _foreign = foreign.finish().unwrap();
        let mut input = builder();
        let module = input.module_root().unwrap();
        let path = input.path("crate::S").unwrap();
        input.structs.push(StructDecl {
            module,
            type_params: Vec::new(),
            name: path,
            visibility: ir::Visibility::Public,
            fields: vec![FieldDecl {
                name: FieldName::Named(field_name),
                ty: Type::Unit,
                visibility: ir::Visibility::Public,
            }],
        });
        assert!(input.finish().is_err());
    }
    #[test]
    fn canonical_prefix_cannot_be_paired_with_another_source_module() {
        let mut input = builder();
        let child_name = input.ident("child").unwrap();
        let root = input.module_root().unwrap();
        let child = input.module_child(root, child_name).unwrap();
        let mut declaration = function(&mut input);
        declaration.module = child;
        input.functions.push(declaration);
        assert!(input.finish().is_err());
    }
    #[test]
    fn poisoned_name_budget_preserves_central_resource_diagnostic() {
        let input = program();
        let budget = input.names.budget();
        let used = budget.snapshot().counters[Counter::Work as usize];
        let limit = budget.limits().counters[Counter::Work as usize];
        budget.charge(Counter::Work, limit - used).unwrap();
        let error = validate_program_names(&input).unwrap_err();
        let failure = budget.snapshot().failure.unwrap();
        assert_eq!(error, transport(TokenError::Budget(failure)));
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
    #[test]
    fn receiver_syntax_has_no_name_handle_but_named_local_sites_are_checked() {
        let mut input = program();
        input.functions[0].body.statements.push(Stmt::Assign {
            name: LocalName::Receiver,
            value: Expr::Bool(false),
        });
        input.functions[0].body.tail = Some(Box::new(Expr::Borrow {
            kind: ir::BorrowKind::Shared,
            place: Place {
                local: LocalName::Receiver,
                fields: Vec::new(),
            },
        }));
        // Structural ownership admission is not receiver-binding resolution.
        validate_program_names(&input).unwrap();
        let mut foreign = builder();
        let name = foreign.ident("foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        let mut assignment = input.clone();
        assignment.functions[0].body.statements[0] = Stmt::Assign {
            name: LocalName::Named(name),
            value: Expr::Bool(false),
        };
        assert!(validate_program_names(&assignment).is_err());
        input.functions[0].body.tail = Some(Box::new(Expr::Borrow {
            kind: ir::BorrowKind::Shared,
            place: Place {
                local: LocalName::Named(name),
                fields: Vec::new(),
            },
        }));
        assert!(validate_program_names(&input).is_err());
    }
    #[test]
    fn active_function_validation_checks_owners_without_rescanning_other_functions() {
        let mut input = program();
        let foreign = program();
        input.functions.push(foreign.functions[0].clone());
        let before = input.names.budget().snapshot().temporary_bytes;
        validate_function_names(&input, &input.functions[0], None).unwrap();
        assert_eq!(input.names.budget().snapshot().temporary_bytes, before);
        assert!(validate_program_names(&input).is_err());
        assert!(validate_function_names(&input, &input.functions[1], None).is_err());
        input.symbols = foreign.symbols;
        assert!(validate_function_names(&input, &input.functions[0], None).is_err());
    }
    #[test]
    fn active_associated_function_checks_explicit_containing_module_authority() {
        let mut input = builder();
        let child_name = input.ident("child").unwrap();
        let root = input.module_root().unwrap();
        let child = input.module_child(root, child_name).unwrap();
        let mut declaration = function(&mut input);
        declaration.name = input.path("member").unwrap();
        // The retained callable context supplies its lexical module. A member
        // name is not required to be a crate-rooted free declaration path.
        let input = input.finish().unwrap();
        validate_function_names(&input, &declaration, Some(root)).unwrap();
        assert!(validate_function_names(&input, &declaration, Some(child)).is_err());
        let foreign = program();
        assert!(
            validate_function_names(
                &input,
                &declaration,
                Some(foreign.modules.source_root().unwrap())
            )
            .is_err()
        );
        assert!(validate_function_names(&input, &declaration, None).is_err());
    }
    #[test]
    fn active_function_head_checks_all_signature_handles_independently() {
        let input = program();
        let mut head = FunctionHead {
            constness: Constness::Runtime,
            name: input.functions[0].name.segments()[0],
            regions: RegionDecls::default(),
            type_params: Vec::new(),
            bounds: Vec::new(),
            params: Vec::new(),
            result: Type::Unit,
        };
        validate_function_head_names(&input, &head).unwrap();
        let mut foreign = builder();
        let name = foreign.ident("foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        head.result = Type::Tuple(vec![Type::Unit, Type::Param(name)]);
        assert!(validate_function_head_names(&input, &head).is_err());
        head.result = Type::Unit;
        head.params.push((BindingName::Named(name), Type::Unit));
        assert!(validate_function_head_names(&input, &head).is_err());
    }
}
