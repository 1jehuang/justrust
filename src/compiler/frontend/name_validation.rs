//! Universal checked-handle admission for the complete published source AST.
//! This walk never resolves printable names or grants semantic publication.
use super::*;
use crate::compiler::expansion_budget::{Counter, Storage};
use crate::compiler::module_identity::ModuleError;
use crate::compiler::semantic_scope::{
    BindingId, OccurrenceRole, ScopeError, ScopeId, scope_diagnostic,
};
use crate::compiler::token_session::{BudgetVec, TokenError};

fn transport(error: TokenError) -> String {
    super::super::source::SourceError::from_token(error).to_string()
}

/// Construction-final syntax authority. Empty records and units share storage,
/// but never a source constructor. Only trusted publication can mint these rows.
#[derive(Debug)]
pub(super) struct StructSources {
    rows: Vec<StructSource>,
    fields: Vec<StructFieldSource>,
    _storage: [crate::compiler::expansion_budget::RetainedBytes; 3],
}
#[derive(Debug)]
struct StructSource {
    module: ModuleId,
    name: PathKey,
    shape: StructShape,
    fields: std::ops::Range<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StructFieldSource {
    Named(NameKey),
    Tuple(usize),
}
fn field_source(
    names: &NameStore,
    field: FieldName,
) -> Result<StructFieldSource, super::super::semantic_name::NameError> {
    Ok(match field {
        FieldName::Named(name) => StructFieldSource::Named(names.key(name, LookupClass::Member)?),
        FieldName::Tuple(index) => StructFieldSource::Tuple(index),
    })
}
pub(super) fn capture_struct_sources(
    names: &NameStore,
    declarations: &[StructDecl],
) -> Result<Arc<StructSources>, ParseError> {
    let budget = names.budget();
    budget
        .charge(Counter::Work, declarations.len() as u64)
        .map_err(|error| native_adapter::transport_error(error.into()))?;
    let count = declarations
        .iter()
        .try_fold(0usize, |count, declaration| {
            count
                .checked_add(declaration.fields.len())
                .ok_or(TokenError::IdExhausted)
        })
        .map_err(native_adapter::transport_error)?;
    budget
        .charge(Counter::Work, count as u64)
        .map_err(|error| native_adapter::transport_error(error.into()))?;
    let mut row_lease = budget
        .reserve_array(
            Storage::Retained,
            declarations.len() as u64,
            std::mem::size_of::<StructSource>() as u64,
        )
        .map_err(|error| native_adapter::transport_error(error.into()))?;
    let mut field_lease = budget
        .reserve_array(
            Storage::Retained,
            count as u64,
            std::mem::size_of::<StructFieldSource>() as u64,
        )
        .map_err(|error| native_adapter::transport_error(error.into()))?;
    let owner_lease = budget
        .reserve(
            Storage::Retained,
            (std::mem::size_of::<StructSources>() + 2 * std::mem::size_of::<usize>()) as u64,
        )
        .map_err(|error| native_adapter::transport_error(error.into()))?;
    let mut rows = Vec::new();
    let mut fields = Vec::new();
    rows.try_reserve_exact(declarations.len())
        .map_err(|_| native_adapter::transport_error(budget.poison_allocation().into()))?;
    fields
        .try_reserve_exact(count)
        .map_err(|_| native_adapter::transport_error(budget.poison_allocation().into()))?;
    if let Err(error) = row_lease.resize_array(
        rows.capacity() as u64,
        std::mem::size_of::<StructSource>() as u64,
        0,
    ) {
        drop(rows);
        return Err(native_adapter::transport_error(error.into()));
    }
    if let Err(error) = field_lease.resize_array(
        fields.capacity() as u64,
        std::mem::size_of::<StructFieldSource>() as u64,
        0,
    ) {
        drop(fields);
        return Err(native_adapter::transport_error(error.into()));
    }
    for declaration in declarations {
        let start = fields.len();
        for field in &declaration.fields {
            fields.push(field_source(names, field.name).map_err(native_adapter::name_error)?);
        }
        declaration
            .name
            .check(names)
            .map_err(native_adapter::name_error)?;
        budget
            .charge(Counter::Work, declaration.name.len() as u64)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        rows.push(StructSource {
            module: declaration.module,
            name: declaration.name.key(Namespace::Type),
            shape: declaration.shape,
            fields: start..fields.len(),
        });
    }
    Ok(Arc::new(StructSources {
        rows,
        fields,
        _storage: [
            row_lease
                .retain()
                .map_err(|error| native_adapter::transport_error(error.into()))?,
            field_lease
                .retain()
                .map_err(|error| native_adapter::transport_error(error.into()))?,
            owner_lease
                .retain()
                .map_err(|error| native_adapter::transport_error(error.into()))?,
        ],
    }))
}

/// Every child is visited, including unused declarations, unreachable bodies,
/// unmatched patterns, public fields, and unselected generic arguments.
/// No recursion or unaccounted traversal storage is used here. Existing source
/// depth validation remains a separate gate, with its existing policy unchanged.
pub(super) fn validate_program_names(program: &Program) -> Result<(), String> {
    if program.struct_sources.rows.len() != program.structs.len() {
        return Err("contract: original struct declaration count changed".into());
    }
    generated_owners::validate(program)?;
    generated_metadata::validate(program)?;
    let mut validator = Validator::new(program)?;
    validator.require_coverage = true;
    for recipe in generated_metadata::debug_recipes(&program.generated_metadata) {
        if recipe.implementation.is_none() {
            validator.work(1)?;
            validator.generated.push(recipe.scope).map_err(transport)?;
        }
    }
    if let Some(scopes) = &program.scopes {
        scopes
            .check_item_authority(program.names.as_ref(), program.modules.as_ref())
            .map_err(scope_diagnostic)?;
    }
    validator.many(&program.functions, |value| Visit::Function(value, None))?;
    validator.many(&program.constants, Visit::Const)?;
    for (declaration, original) in program.structs.iter().zip(&program.struct_sources.rows) {
        validator.work(declaration.fields.len() + declaration.name.len() + 1)?;
        if declaration.module != original.module
            || declaration.shape != original.shape
            || declaration.name.key(Namespace::Type) != original.name
            || declaration.fields.len() != original.fields.len()
        {
            return Err("contract: original struct source shape changed".into());
        }
        for (field, expected) in declaration
            .fields
            .iter()
            .zip(&program.struct_sources.fields[original.fields.clone()])
        {
            if field_source(&program.names, field.name).map_err(native_adapter::name_diagnostic)?
                != *expected
            {
                return Err("contract: original struct field identity changed".into());
            }
        }
        validator.push(Visit::Struct(declaration))?;
    }
    validator.many(&program.enums, Visit::Enum)?;
    validator.many(&program.impls, Visit::Impl)?;
    validator.many(&program.traits, Visit::Trait)?;
    validator.work(program.trait_imports.len())?;
    for (module, path) in &program.trait_imports {
        validator.source_module(*module)?;
        validator.push(Visit::Import(*module, path))?;
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
    generated_owners::validate_callable(program, function)?;
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

/// Admission for the owning trait's associated declaration metadata during
/// callable replay. This does not inspect unrelated declarations or bodies.
pub(super) fn validate_associated_bound_names(
    program: &Program,
    owner: &TraitDecl,
) -> Result<(), String> {
    let mut validator = Validator::new(program)?;
    validator.enter_scope(None, owner.module)?;
    validator.push(Visit::Path(&owner.name))?;
    validator.work(owner.associated_types.len())?;
    for member in &owner.associated_types {
        validator.push(Visit::Ident(member.name, LookupClass::Member))?;
        validator.many(&member.bounds, Visit::TraitPath)?;
    }
    validator.run()
}

enum Visit<'a> {
    Const(&'a ConstDecl),
    Function(&'a Function, Option<ModuleId>),
    RestoreScope(Option<ScopeId>, ModuleId),
    RestoreBindings(usize),
    MatchArm(&'a MatchArm),
    IfLetThen(&'a Pattern, &'a Block),
    Import(ModuleId, &'a TraitPath),
    ImportShape(&'a GenericPath),
    Head(&'a FunctionHead),
    Struct(&'a StructDecl),
    StructShape(&'a StructDecl),
    Enum(&'a EnumDecl),
    Variant(&'a VariantDecl, ModuleId),
    Field(&'a FieldDecl, ModuleId),
    Impl(&'a ImplDecl),
    AssociatedConst(&'a AssociatedConstDecl, ModuleId),
    Associated(&'a AssociatedFunction, ModuleId),
    Trait(&'a TraitDecl),
    TraitItem(&'a TraitItemDecl, ModuleId),
    Bound(&'a TypeBound),
    Regions(&'a RegionDecls),
    Type(&'a Type),
    DerivedSignatureType(&'a Type, ModuleId),
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
    LocalDeclaration(AstIdent),
    LocalReference(AstIdent),
}
struct Validator<'a> {
    program: &'a Program,
    root: ModuleId,
    pending: BudgetVec<Visit<'a>>,
    scope: Option<ScopeId>,
    module: ModuleId,
    generated: BudgetVec<ScopeId>,
    // All still-live declaration identities, including shadowed bindings that
    // a definition-site macro capture may legitimately reference.
    bindings: BudgetVec<BindingId>,
    require_coverage: bool,
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
        if let Some(scopes) = &program.scopes {
            scopes.check_names(names).map_err(scope_diagnostic)?;
            if !scopes.budget().same_authority(names.budget()) {
                return Err(scope_diagnostic(ScopeError::ForeignNames));
            }
        }
        let root = modules.source_root().map_err(module_diagnostic)?;
        Ok(Self {
            program,
            root,
            pending: BudgetVec::new(names.budget(), Storage::Temporary).map_err(transport)?,
            scope: None,
            module: root,
            generated: BudgetVec::new(names.budget(), Storage::Temporary).map_err(transport)?,
            bindings: BudgetVec::new(names.budget(), Storage::Temporary).map_err(transport)?,
            require_coverage: false,
        })
    }
    fn run(mut self) -> Result<(), String> {
        while let Some(node) = self.pending.pop() {
            self.work(1)?;
            self.visit(node)?;
        }
        if self.require_coverage
            && let Some(scopes) = &self.program.scopes
        {
            scopes
                .check_item_function_coverage(self.generated.as_slice())
                .map_err(scope_diagnostic)?;
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
    fn occurrence(&self, name: AstIdent) -> Result<(), String> {
        let role = match self.scope {
            Some(function) => OccurrenceRole::Function {
                module: self.module,
                function,
            },
            None => OccurrenceRole::OriginalModule(self.module),
        };
        self.occurrence_in(name, role)
    }
    fn occurrence_in(&self, name: AstIdent, role: OccurrenceRole) -> Result<(), String> {
        if let Some(scopes) = &self.program.scopes {
            scopes
                .check_occurrence_in(
                    self.program.names.as_ref(),
                    self.program.modules.as_ref(),
                    name,
                    role,
                )
                .map_err(scope_diagnostic)?;
        } else {
            let data = self
                .program
                .names
                .data(name)
                .map_err(native_adapter::name_diagnostic)?;
            let session = self.program.names.session();
            if data.context != session.root_context()
                || session
                    .span(data.span)
                    .map_err(transport)?
                    .expansion
                    .is_some()
            {
                return Err(scope_diagnostic(ScopeError::UnregisteredExpansion));
            }
        }
        Ok(())
    }
    fn declaration_path(&self, path: &AstPath, module: ModuleId) -> Result<(), String> {
        self.declaration_path_in(path, module, OccurrenceRole::OriginalModule(module))
    }
    fn declaration_path_in(
        &self,
        path: &AstPath,
        module: ModuleId,
        role: OccurrenceRole,
    ) -> Result<(), String> {
        self.work(path.len())?;
        path.check(self.program.names.as_ref())
            .map_err(native_adapter::name_diagnostic)?;
        for (index, segment) in path.segments().iter().enumerate() {
            self.occurrence_in(
                *segment,
                if index + 1 == path.len() {
                    role
                } else {
                    OccurrenceRole::OriginalModule(module)
                },
            )?;
        }
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
    fn enter_scope(&mut self, scope: Option<ScopeId>, module: ModuleId) -> Result<(), String> {
        if self.scope != scope || self.module != module {
            self.push(Visit::RestoreScope(self.scope, self.module))?;
            self.scope = scope;
            self.module = module;
        }
        Ok(())
    }
    fn local(&mut self, name: AstIdent, declaration: bool) -> Result<(), String> {
        if self.scope.is_none() {
            self.push(Visit::Ident(name, LookupClass::Local))
        } else if declaration {
            self.push(Visit::LocalDeclaration(name))
        } else {
            self.push(Visit::LocalReference(name))
        }
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
    fn positional_patterns(&mut self, fields: &'a PositionalRest) -> Result<(), String> {
        let count = fields
            .prefix
            .len()
            .checked_add(fields.suffix.len())
            .ok_or_else(|| {
                transport(TokenError::InvalidInput(
                    "positional pattern child count overflow",
                ))
            })?;
        // Charge both explicit lists before reading or pushing either list.
        // Omitted fields have no source nodes or generated binding identities.
        self.work(count)?;
        for child in fields.prefix.iter().chain(&fields.suffix) {
            self.push(Visit::Pattern(child))?;
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
            Visit::Const(declaration) => {
                self.visibility(&declaration.visibility, declaration.module)?;
                self.declaration_path_in(
                    &declaration.name,
                    declaration.module,
                    OccurrenceRole::OriginalModule(declaration.module),
                )?;
                self.enter_scope(None, declaration.module)?;
                self.push(Visit::Expr(&declaration.initializer))?;
                self.push(Visit::Type(&declaration.ty))?;
            }
            Visit::Function(function, containing_module) => {
                let mut declaration_role = OccurrenceRole::OriginalModule(function.module);
                if let Some(scope) = function.lexical_scope {
                    let scopes = self
                        .program
                        .scopes
                        .as_ref()
                        .ok_or_else(|| scope_diagnostic(ScopeError::InvalidHandle))?;
                    scopes.check_function(scope).map_err(scope_diagnostic)?;
                    let generated = scopes
                        .generated_function_declaration(scope)
                        .map_err(scope_diagnostic)?;
                    if containing_module.is_some()
                        && generated.is_some()
                        && !generated_owners::contains(self.program, scope, function.module)
                    {
                        return Err(scope_diagnostic(ScopeError::FunctionMismatch));
                    }
                    if containing_module.is_none() || generated.is_some() {
                        let declaration = *function
                            .name
                            .segments()
                            .last()
                            .ok_or_else(|| scope_diagnostic(ScopeError::InvalidHandle))?;
                        scopes
                            .check_generated_function(
                                self.program.names.as_ref(),
                                self.program.modules.as_ref(),
                                scope,
                                declaration,
                                function.module,
                            )
                            .map_err(scope_diagnostic)?;
                        if generated.is_some() {
                            if self.require_coverage {
                                self.work(1)?;
                                self.generated.push(scope).map_err(transport)?;
                            }
                            declaration_role = OccurrenceRole::GeneratedDeclaration {
                                module: function.module,
                                function: scope,
                                declaration,
                            };
                        }
                    }
                }
                self.visibility(&function.visibility, function.module)?;
                if let Some(module) = containing_module {
                    self.source_module(module)?;
                    if module != function.module {
                        return Err(module_diagnostic(ModuleError::InvalidParent));
                    }
                }
                if containing_module.is_some() {
                    self.enter_scope(function.lexical_scope, function.module)?;
                    if matches!(
                        declaration_role,
                        OccurrenceRole::GeneratedDeclaration { .. }
                    ) {
                        function
                            .name
                            .check(self.program.names.as_ref())
                            .map_err(native_adapter::name_diagnostic)?;
                        let (PathAnchor::Relative, [member]) =
                            (function.name.anchor(), function.name.segments())
                        else {
                            return Err(scope_diagnostic(ScopeError::FunctionMismatch));
                        };
                        self.occurrence_in(*member, declaration_role)?;
                    } else {
                        self.push(Visit::Path(&function.name))?;
                    }
                } else {
                    self.declaration_path_in(&function.name, function.module, declaration_role)?;
                    self.enter_scope(function.lexical_scope, function.module)?;
                }
                self.push(Visit::RestoreBindings(self.bindings.len()))?;
                // LIFO tasks: signature declarations precede the body, whose
                // own restore marker leaves parameters live until this marker.
                self.push(Visit::Block(&function.body))?;
                self.push(Visit::Regions(&function.regions))?;
                self.identifiers(&function.type_params, LookupClass::TypeParameter)?;
                self.many(&function.bounds, Visit::Bound)?;
                if let Some(module) = generated_debug::signature_module(self.program, function)? {
                    self.work(function.params.len())?;
                    for (name, ty) in &function.params {
                        self.push(Visit::Binding(*name))?;
                        self.push(Visit::DerivedSignatureType(ty, module))?;
                    }
                    self.push(Visit::DerivedSignatureType(&function.result, module))?;
                } else {
                    self.parameters(&function.params)?;
                    self.push(Visit::Type(&function.result))?;
                }
            }
            Visit::RestoreScope(scope, module) => {
                self.scope = scope;
                self.module = module;
            }
            Visit::RestoreBindings(len) => self.bindings.truncate(len),
            Visit::MatchArm(arm) => {
                self.push(Visit::RestoreBindings(self.bindings.len()))?;
                self.push(Visit::Expr(&arm.body))?;
                if let Some(guard) = &arm.guard {
                    self.push(Visit::Expr(guard))?;
                }
                self.push(Visit::Pattern(&arm.pattern))?;
            }
            Visit::IfLetThen(pattern, block) => {
                self.push(Visit::RestoreBindings(self.bindings.len()))?;
                self.push(Visit::Block(block))?;
                self.push(Visit::Pattern(pattern))?;
            }
            Visit::Import(module, path) => {
                self.enter_scope(None, module)?;
                if let TraitPath::Source(path) = path {
                    // Check all argument handles before enforcing the import
                    // shape, including caller-mutated unused imports.
                    self.push(Visit::ImportShape(path))?;
                }
                self.push(Visit::TraitPath(path))?;
            }
            Visit::ImportShape(path) => {
                self.work(path.segments.len())?;
                if path
                    .segments
                    .iter()
                    .any(|segment| !segment.args.is_empty() || !segment.associated_types.is_empty())
                {
                    return Err(transport(TokenError::InvalidInput(
                        "trait imports cannot contain application arguments",
                    )));
                }
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
                self.enter_scope(None, declaration.module)?;
                // The worklist is LIFO: all field and name checks precede
                // shape integrity, including malformed caller-mutated ASTs.
                self.push(Visit::StructShape(declaration))?;
                self.many(&declaration.bounds, Visit::Bound)?;
                self.visibility(&declaration.visibility, declaration.module)?;
                self.declaration_path(&declaration.name, declaration.module)?;
                self.identifiers(&declaration.type_params, LookupClass::TypeParameter)?;
                self.identifiers(&declaration.region_params, LookupClass::LifetimeParameter)?;
                self.many(&declaration.fields, |field| {
                    Visit::Field(field, declaration.module)
                })?;
            }
            Visit::StructShape(declaration) => {
                self.work(declaration.fields.len())?;
                let valid = match declaration.shape {
                    StructShape::Unit => declaration.fields.is_empty(),
                    StructShape::Record => {
                        declaration.fields.iter().all(|field| matches!(field.name, FieldName::Named(_)))
                    }
                    StructShape::Tuple => {
                        !declaration.fields.is_empty()
                            && declaration.fields.iter().enumerate().all(|(index, field)| {
                                matches!(field.name, FieldName::Tuple(actual) if actual == index)
                            })
                    }
                };
                if !valid {
                    return Err(transport(TokenError::InvalidInput(
                        "struct shape disagrees with its fields",
                    )));
                }
            }
            Visit::Enum(declaration) => {
                self.enter_scope(None, declaration.module)?;
                self.many(&declaration.bounds, Visit::Bound)?;
                self.visibility(&declaration.visibility, declaration.module)?;
                self.declaration_path(&declaration.name, declaration.module)?;
                self.identifiers(&declaration.type_params, LookupClass::TypeParameter)?;
                self.identifiers(&declaration.region_params, LookupClass::LifetimeParameter)?;
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
                self.enter_scope(None, implementation.module)?;
                self.source_module(implementation.module)?;
                if let Some(path) = &implementation.trait_path {
                    self.push(Visit::TraitPath(path))?;
                }
                self.identifiers(&implementation.type_params, LookupClass::TypeParameter)?;
                self.many(&implementation.bounds, Visit::Bound)?;
                self.push(Visit::Type(&implementation.self_ty))?;
                self.work(implementation.associated_types.len())?;
                for associated in &implementation.associated_types {
                    self.push(Visit::Ident(associated.name, LookupClass::Member))?;
                    self.push(Visit::Type(&associated.value))?;
                }
                self.many(&implementation.associated_constants, |item| {
                    Visit::AssociatedConst(item, implementation.module)
                })?;
                self.many(&implementation.items, |item| {
                    Visit::Associated(item, implementation.module)
                })?;
            }
            Visit::AssociatedConst(item, module) => {
                self.visibility(&item.visibility, module)?;
                self.push(Visit::Ident(item.name, LookupClass::Member))?;
                self.push(Visit::Type(&item.ty))?;
                if let Some(value) = &item.initializer {
                    self.push(Visit::Expr(value))?;
                }
            }
            Visit::Associated(item, module) => {
                receiver_authority::validate_kind(self.program, &item.function, item.receiver)?;
                self.visibility(&item.visibility, module)?;
                self.push(Visit::Function(&item.function, Some(module)))?;
            }
            Visit::Trait(declaration) => {
                self.enter_scope(None, declaration.module)?;
                self.visibility(&declaration.visibility, declaration.module)?;
                self.declaration_path(&declaration.name, declaration.module)?;
                self.identifiers(&declaration.type_params, LookupClass::TypeParameter)?;
                self.many(&declaration.bounds, Visit::Bound)?;
                self.many(&declaration.supertraits, Visit::TraitPath)?;
                self.work(declaration.associated_types.len())?;
                for associated in &declaration.associated_types {
                    self.push(Visit::Ident(associated.name, LookupClass::Member))?;
                    self.many(&associated.bounds, Visit::TraitPath)?;
                }
                self.many(&declaration.associated_constants, |item| {
                    Visit::AssociatedConst(item, declaration.module)
                })?;
                self.many(&declaration.items, |item| {
                    Visit::TraitItem(item, declaration.module)
                })?;
            }
            Visit::TraitItem(item, module) => {
                self.push(Visit::Head(&item.signature))?;
                if let Some(function) = &item.default {
                    receiver_authority::validate_kind(self.program, function, item.receiver)?;
                    self.push(Visit::Function(function, Some(module)))?;
                }
            }
            Visit::Bound(bound) => {
                self.push(Visit::Type(&bound.subject))?;
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
            Visit::DerivedSignatureType(ty, module) => {
                self.source_module(module)?;
                self.enter_scope(None, module)?;
                self.push(Visit::Type(ty))?;
            }
            Visit::Type(ty) => match ty {
                Type::Opaque(opaque) => {
                    self.many(&opaque.bounds, Visit::TraitPath)?;
                    self.many(&opaque.regions, |region| Visit::Region(*region))?;
                }
                Type::Param(name) => self.push(Visit::Ident(*name, LookupClass::TypeParameter))?,
                Type::Named(path) => self.push(Visit::Path(path))?,
                Type::Apply(path) => self.push(Visit::GenericPath(path))?,
                Type::Projection(projection) => match projection.as_ref() {
                    SourceProjection::Qualified {
                        self_ty,
                        trait_path,
                        item,
                    } => {
                        self.push(Visit::Type(self_ty))?;
                        self.push(Visit::TraitPath(trait_path))?;
                        self.push(Visit::Ident(*item, LookupClass::Member))?;
                    }
                    SourceProjection::SelfItem { item } => {
                        self.push(Visit::Ident(*item, LookupClass::Member))?;
                    }
                },
                Type::Ref {
                    region, referent, ..
                } => {
                    if let Some(region) = region {
                        self.push(Visit::Region(*region))?;
                    }
                    self.push(Visit::Type(referent))?;
                }
                Type::Array(element, _)
                | Type::Slice(element)
                | Type::RawPtr {
                    pointee: element, ..
                } => self.push(Visit::Type(element))?,
                Type::Dyn(trait_path) => self.push(Visit::TraitPath(trait_path))?,
                Type::Tuple(elements) => self.many(elements, Visit::Type)?,
                Type::SelfType
                | Type::Int(_)
                | Type::Float(_)
                | Type::Char
                | Type::Str
                | Type::Bool
                | Type::Unit
                | Type::Never
                | Type::Infer => (),
            },
            Visit::GenericPath(path) => {
                self.work(path.segments.len())?;
                let checked = path
                    .checked_path(self.program.names.as_ref())
                    .map_err(native_adapter::name_diagnostic)?;
                let occurrences = checked
                    .source_occurrences(self.program.names.as_ref())
                    .map_err(native_adapter::name_diagnostic)?;
                self.work(occurrences.len())?;
                for occurrence in occurrences {
                    self.occurrence(*occurrence)?;
                }
                for segment in &path.segments {
                    self.many(&segment.region_args, |region| Visit::Region(*region))?;
                    self.many(&segment.args, Visit::Type)?;
                    self.work(segment.associated_types.len())?;
                    for binding in &segment.associated_types {
                        self.push(Visit::Ident(binding.name, LookupClass::Member))?;
                        self.push(Visit::Type(&binding.value))?;
                    }
                }
            }
            Visit::TraitPath(path) => match path {
                TraitPath::Source(path) => self.push(Visit::GenericPath(path))?,
                TraitPath::Builtin(
                    BuiltinTrait::Sized
                    | BuiltinTrait::Copy
                    | BuiltinTrait::Clone
                    | BuiltinTrait::Drop,
                ) => (),
            },
            Visit::Block(block) => {
                self.push(Visit::RestoreBindings(self.bindings.len()))?;
                if let Some(tail) = &block.tail {
                    self.push(Visit::Expr(tail))?;
                }
                self.work(block.statements.len())?;
                for statement in block.statements.iter().rev() {
                    self.push(Visit::Stmt(statement))?;
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
                Stmt::LetElse {
                    pattern,
                    ty,
                    value,
                    else_block,
                } => {
                    // The initializer and failure block see the old bindings.
                    // Success declarations then belong to the enclosing block,
                    // unlike conditional or match-arm pattern declarations.
                    self.push(Visit::Pattern(pattern))?;
                    self.push(Visit::Block(else_block))?;
                    self.push(Visit::Type(ty))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::Assign { name, value } => {
                    self.push(Visit::LocalName(*name))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::AssignNestedIndex { target, value, .. } => {
                    self.push(Visit::Expr(target))?;
                    self.push(Visit::Expr(value))?;
                }
                Stmt::AssignDeref {
                    carrier,
                    fields,
                    index,
                    value,
                    ..
                } => {
                    self.push(Visit::Expr(carrier))?;
                    self.many(fields, Visit::Selector)?;
                    if let Some(index) = index {
                        self.push(Visit::Expr(index))?;
                    }
                    self.push(Visit::Expr(value))?;
                }
                Stmt::AssignIndex {
                    array,
                    index,
                    value,
                    ..
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
                Stmt::AssignOp { place, value, .. } | Stmt::AssignPlace { place, value, .. } => {
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
                Stmt::WhileLet {
                    label,
                    pattern,
                    scrutinee,
                    body,
                    ..
                } => {
                    if let Some(label) = label {
                        self.push(Visit::Ident(*label, LookupClass::Label))?;
                    }
                    self.push(Visit::IfLetThen(pattern, body))?;
                    self.push(Visit::Expr(scrutinee))?;
                }
                Stmt::For {
                    label,
                    pattern,
                    source,
                    body,
                    ..
                } => {
                    if let Some(label) = label {
                        self.push(Visit::Ident(*label, LookupClass::Label))?;
                    }
                    self.push(Visit::IfLetThen(pattern, body))?;
                    match &source.iterable {
                        ForIterable::Range { start, end, .. } => {
                            self.push(Visit::Expr(end))?;
                            self.push(Visit::Expr(start))?;
                        }
                        ForIterable::Value(value) => self.push(Visit::Expr(value))?,
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
                    self.push(Visit::Block(body))?;
                    self.push(Visit::Expr(condition))?;
                }
            },
            Visit::Expr(expression) => match expression {
                Expr::AssociatedConst(value) => {
                    self.push(Visit::Ident(value.name, LookupClass::Member))?;
                    self.push(Visit::Type(&value.owner))?;
                    if let Some(path) = &value.trait_path {
                        self.push(Visit::TraitPath(path))?;
                    }
                }
                Expr::Try(inner) => self.push(Visit::Expr(inner))?,
                Expr::UnexpandedMacro(_) => {
                    let mut error = crate::compiler::source::SourceError::syntax(
                        None,
                        "unexpanded macro syntax cannot be admitted as an active program",
                    );
                    error.kind = crate::compiler::source::SourceErrorKind::Unsupported;
                    return Err(error.to_string());
                }
                Expr::TraitCall {
                    owner,
                    trait_path,
                    name,
                    type_args,
                    args,
                } => {
                    self.push(Visit::Type(owner))?;
                    self.push(Visit::TraitPath(trait_path))?;
                    self.many(type_args, Visit::Type)?;
                    self.push(Visit::Ident(*name, LookupClass::Member))?;
                    self.many(args, Visit::Expr)?;
                }
                Expr::Invoke { receiver, args } => {
                    self.push(Visit::Expr(receiver))?;
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
                Expr::TypeAlignment(target) => self.push(Visit::Type(target))?,
                Expr::Borrow { place, .. } => self.push(Visit::Place(place))?,
                Expr::Deref { value }
                | Expr::BorrowValue { value, .. }
                | Expr::Repeat { value, .. }
                | Expr::Index { base: value, .. }
                | Expr::Unary { value, .. } => self.push(Visit::Expr(value))?,
                Expr::DynamicIndex { base, index, .. } => {
                    self.push(Visit::Expr(base))?;
                    self.push(Visit::Expr(index))?;
                }
                Expr::RangeIndex {
                    base, start, end, ..
                } => {
                    self.push(Visit::Expr(base))?;
                    for bound in [start, end].into_iter().flatten() {
                        self.push(Visit::Expr(bound))?;
                    }
                }
                Expr::NativeSyscall(elements)
                | Expr::RawSlice {
                    arguments: elements,
                    ..
                }
                | Expr::Array(elements)
                | Expr::Tuple(elements) => self.many(elements, Visit::Expr)?,
                Expr::EnumVariant { name, fields, .. } => {
                    self.push(Visit::Path(name))?;
                    self.expression_fields(fields)?;
                }
                Expr::Match { scrutinee, arms } => {
                    self.work(arms.len())?;
                    for arm in arms.iter().rev() {
                        self.push(Visit::MatchArm(arm))?;
                    }
                    self.push(Visit::Expr(scrutinee))?;
                }
                Expr::TupleStruct { name, fields } => {
                    self.push(Visit::Path(name))?;
                    self.many(fields, Visit::Expr)?;
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
                Expr::ConstItem(path) => self.push(Visit::Path(path))?,
                Expr::Var(path) => {
                    self.push(Visit::Path(path))?;
                    if path.anchor() == PathAnchor::SelfModule && path.is_empty() {
                        self.push(Visit::LocalName(receiver_authority::local(path)))?;
                    }
                    if self.scope.is_some()
                        && path.anchor() == PathAnchor::Relative
                        && path.len() == 1
                    {
                        self.push(Visit::LocalReference(path.segments()[0]))?;
                    }
                }
                Expr::Binary { left, right, .. } => {
                    self.push(Visit::Expr(left))?;
                    self.push(Visit::Expr(right))?;
                }
                Expr::Call { name, args } => {
                    self.push(Visit::Path(name))?;
                    if self.scope.is_some()
                        && name.anchor() == PathAnchor::Relative
                        && name.len() == 1
                    {
                        self.push(Visit::LocalReference(name.segments()[0]))?;
                    }
                    self.many(args, Visit::Expr)?;
                }
                Expr::IfLet {
                    pattern,
                    scrutinee,
                    then_block,
                    else_branch,
                } => {
                    if let Some(branch) = else_branch {
                        self.push(Visit::Expr(branch))?;
                    }
                    self.push(Visit::IfLetThen(pattern, then_block))?;
                    self.push(Visit::Expr(scrutinee))?;
                }
                Expr::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.push(Visit::Block(else_block))?;
                    self.push(Visit::Block(then_block))?;
                    self.push(Visit::Expr(condition))?;
                }
                Expr::Block(block) | Expr::Unsafe(block) => self.push(Visit::Block(block))?,
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
                Expr::Output(output) => validate_output_site(output, &self.program.names)?,
                Expr::Format { arguments, .. } => {
                    for argument in arguments {
                        self.push(Visit::Expr(argument))?;
                    }
                }
                Expr::FormatInto(source) => {
                    for argument in source.arguments.iter().chain(source.rest.iter().flatten()) {
                        self.push(Visit::Expr(argument))?;
                    }
                }
                Expr::Panic(panic) => validate_panic_site(panic, &self.program.names)?,
                Expr::Integer(_)
                | Expr::Float(_)
                | Expr::Char(_)
                | Expr::Str(_)
                | Expr::ByteStr(_)
                | Expr::Bool(_)
                | Expr::Println(_) => (),
            },
            Visit::Pattern(pattern) => match pattern {
                Pattern::Apply { path, pattern } => {
                    self.push(Visit::GenericPath(path))?;
                    self.push(Visit::Pattern(pattern))?;
                }
                Pattern::Ref { pattern, .. } => self.push(Visit::Pattern(pattern))?,
                Pattern::At { name, pattern, .. } => {
                    self.local(*name, true)?;
                    self.push(Visit::Pattern(pattern))?;
                }
                Pattern::Variant { name, fields, .. } => {
                    self.push(Visit::Path(name))?;
                    self.pattern_fields(fields)?;
                }
                Pattern::VariantRest { name, fields } => {
                    self.push(Visit::Path(name))?;
                    self.positional_patterns(fields)?;
                }
                Pattern::TupleRest(fields) | Pattern::ArrayRest(fields) => {
                    self.positional_patterns(fields)?;
                }
                Pattern::Path(path) => self.push(Visit::Path(path))?,
                Pattern::Binding { name, .. } => self.local(*name, true)?,
                Pattern::Tuple(elements)
                | Pattern::Array(elements)
                | Pattern::Alternatives(elements) => {
                    self.many(elements, Visit::Pattern)?;
                }
                Pattern::Struct { name, fields, .. } => {
                    self.push(Visit::Path(name))?;
                    self.work(fields.len())?;
                    for (name, pattern) in fields {
                        self.push(Visit::Ident(*name, LookupClass::Member))?;
                        self.push(Visit::Pattern(pattern))?;
                    }
                }
                Pattern::Bool(_)
                | Pattern::Int { .. }
                | Pattern::Char(_)
                | Pattern::Range { .. }
                | Pattern::Wildcard => (),
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
                let occurrences = path
                    .source_occurrences(self.program.names.as_ref())
                    .map_err(native_adapter::name_diagnostic)?;
                self.work(occurrences.len())?;
                for occurrence in occurrences {
                    self.occurrence(*occurrence)?;
                }
            }
            Visit::Ident(name, class) => {
                self.program
                    .names
                    .key(name, class)
                    .map_err(native_adapter::name_diagnostic)?;
                self.occurrence(name)?;
            }
            Visit::Region(region) => match region {
                RegionName::Named(name) => {
                    self.push(Visit::Ident(name, LookupClass::LifetimeParameter))?
                }
                RegionName::Static | RegionName::Infer => (),
            },
            Visit::Binding(binding) => match binding {
                BindingName::Named(name) => self.local(name, true)?,
                BindingName::Wildcard => (),
            },
            Visit::FieldName(field) => match field {
                FieldName::Named(name) => self.push(Visit::Ident(name, LookupClass::Member))?,
                FieldName::Tuple(_) => (),
            },
            Visit::LocalName(local) => match local {
                LocalName::Named(name) => self.local(name, false)?,
                LocalName::ReceiverOccurrence(name) => {
                    if let Some(function) = self.scope {
                        self.program
                            .scopes
                            .as_ref()
                            .ok_or_else(|| scope_diagnostic(ScopeError::InvalidHandle))?
                            .check_receiver_reference(&*self.program.names, function, name.ident())
                            .map_err(scope_diagnostic)?;
                    }
                    self.occurrence(name.ident())?;
                }
                LocalName::Receiver => {
                    if self.scope.is_some() {
                        return Err(scope_diagnostic(ScopeError::UnrecordedOccurrence));
                    }
                }
            },
            Visit::LocalDeclaration(name) => {
                self.program
                    .names
                    .key(name, LookupClass::Local)
                    .map_err(native_adapter::name_diagnostic)?;
                self.occurrence(name)?;
                if let Some(scope) = self.scope {
                    let scopes = self
                        .program
                        .scopes
                        .as_ref()
                        .ok_or_else(|| scope_diagnostic(ScopeError::InvalidHandle))?;
                    let binding = scopes
                        .declaration_for(self.program.names.as_ref(), name)
                        .map_err(scope_diagnostic)?;
                    if scopes.binding_function(binding).map_err(scope_diagnostic)? != scope {
                        return Err(scope_diagnostic(ScopeError::FunctionMismatch));
                    }
                    self.work(1)?;
                    self.bindings.push(binding).map_err(transport)?;
                }
            }
            Visit::LocalReference(name) => {
                self.program
                    .names
                    .key(name, LookupClass::Local)
                    .map_err(native_adapter::name_diagnostic)?;
                self.occurrence(name)?;
                if let Some(scope) = self.scope {
                    let scopes = self
                        .program
                        .scopes
                        .as_ref()
                        .ok_or_else(|| scope_diagnostic(ScopeError::InvalidHandle))?;
                    if scopes
                        .reference_function(self.program.names.as_ref(), name)
                        .map_err(scope_diagnostic)?
                        != scope
                    {
                        return Err(scope_diagnostic(ScopeError::FunctionMismatch));
                    }
                    if let Some(binding) = scopes
                        .reference_for(self.program.names.as_ref(), name)
                        .map_err(scope_diagnostic)?
                    {
                        if scopes.binding_function(binding).map_err(scope_diagnostic)? != scope {
                            return Err(scope_diagnostic(ScopeError::FunctionMismatch));
                        }
                        self.work(self.bindings.len())?;
                        if !self.bindings.as_slice().contains(&binding) {
                            return Err(scope_diagnostic(ScopeError::UnrecordedOccurrence));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;

    #[test]
    fn nominal_bound_parameters_reject_foreign_name_handles() {
        let valid = super::super::parse_unresolved(
            "trait Mark {} struct S<T: Mark> { v: T } enum E<T: Mark> { A(T) } fn main() {}",
        )
        .unwrap()
        .program;
        validate_program_names(&valid).unwrap();
        let mut foreign = builder();
        let foreign_name = foreign.ident("T").unwrap();
        let _foreign = foreign.finish().unwrap();
        for site in 0..2 {
            let mut malformed = valid.clone();
            if site == 0 {
                malformed.structs[0].bounds[0].subject = Type::Param(foreign_name);
            } else {
                malformed.enums[0].bounds[0].subject = Type::Param(foreign_name);
            }
            assert!(validate_program_names(&malformed).is_err());
        }
    }

    #[test]
    fn unused_associated_sources_validate_every_name_owner_and_projection_child() {
        let parsed = super::super::parse_unresolved(
            "mod inner{trait Family<T>{type Out;fn unused()->Self::Out;}\
             struct S;impl Family<bool> for S{type Out=<S as Family<(bool,[i32;7])>>::Out;}}\
             fn main(){}",
        )
        .unwrap();
        let valid = parsed.program;
        validate_program_names(&valid).unwrap();
        let mut foreign = builder();
        let foreign_name = foreign.ident("Out").unwrap();
        let foreign_path = foreign.path("crate::Family").unwrap();
        let foreign = foreign.finish().unwrap();
        let baseline = valid.names.budget().snapshot().temporary_bytes;
        for site in 0..10 {
            let mut malformed = valid.clone();
            match site {
                0 => malformed.traits[0].associated_types[0].name = foreign_name,
                1 => malformed.impls[0].associated_types[0].name = foreign_name,
                2 => malformed.impls[0].module = foreign.modules.source_root().unwrap(),
                3 => malformed.traits[0].module = foreign.modules.source_root().unwrap(),
                4 => {
                    let Type::Projection(projection) =
                        &mut malformed.traits[0].items[0].signature.result
                    else {
                        panic!("expected written Self::Out");
                    };
                    let SourceProjection::SelfItem { item } = projection.as_mut() else {
                        panic!("expected owner-relative projection");
                    };
                    *item = foreign_name;
                }
                _ => {
                    let Type::Projection(projection) =
                        &mut malformed.impls[0].associated_types[0].value
                    else {
                        panic!("expected written qualified RHS");
                    };
                    let SourceProjection::Qualified {
                        self_ty,
                        trait_path,
                        item,
                    } = projection.as_mut()
                    else {
                        panic!("expected qualified projection");
                    };
                    match site {
                        5 => *item = foreign_name,
                        6 => *self_ty = Type::Param(foreign_name),
                        _ => {
                            let TraitPath::Source(path) = trait_path else {
                                unreachable!()
                            };
                            if site == 7 {
                                path.segments.last_mut().unwrap().args[0] = Type::Tuple(vec![
                                    Type::Bool,
                                    Type::Array(Box::new(Type::Param(foreign_name)), 7),
                                ]);
                            } else if site == 8 {
                                path.resolved_use = Some(foreign_path.clone());
                            } else {
                                // Original spelling is a separate admission site
                                // from the resolved-use certificate above.
                                path.segments[0].name = foreign_name;
                            }
                        }
                    }
                }
            }
            assert!(
                validate_program_names(&malformed)
                    .unwrap_err()
                    .starts_with("contract:"),
                "site {site}"
            );
            assert_eq!(valid.names.budget().snapshot().temporary_bytes, baseline);
        }
        assert!(valid.names.budget().snapshot().failure.is_none());
    }

    #[test]
    fn equality_bindings_validate_later_foreign_children_after_unsupported_rhs() {
        let parsed = super::super::parse_unresolved(
            "trait Family{type Out;type Other;}\
             fn unused<T:Family<Out=&i32,Other=(bool,[i32;7])>>(){}fn main(){}",
        )
        .unwrap();
        let valid = parsed.program;
        validate_program_names(&valid).unwrap();
        let mut foreign = builder();
        let foreign_name = foreign.ident("Other").unwrap();
        let foreign_path = foreign.path("crate::Family").unwrap();
        let _foreign = foreign.finish().unwrap();
        let baseline = valid.names.budget().snapshot().temporary_bytes;
        for site in 0..4 {
            let mut malformed = valid.clone();
            let TraitPath::Source(path) = &mut malformed.functions[0].bounds[0].traits[0] else {
                panic!("expected written equality application")
            };
            match site {
                0 => path.segments.last_mut().unwrap().associated_types[1].name = foreign_name,
                1 => {
                    path.segments.last_mut().unwrap().associated_types[1].value =
                        Type::Tuple(vec![
                            Type::Bool,
                            Type::Array(Box::new(Type::Param(foreign_name)), 7),
                        ]);
                }
                2 => path.resolved_use = Some(foreign_path.clone()),
                _ => path.segments.last_mut().unwrap().name = foreign_name,
            }
            let error = validate_program_names(&malformed).unwrap_err();
            assert!(error.starts_with("contract:"), "site {site}: {error}");
            assert_eq!(valid.names.budget().snapshot().temporary_bytes, baseline);
        }
        assert!(valid.names.budget().snapshot().failure.is_none());
    }

    #[test]
    fn equality_exact_source_rejects_same_owner_member_rhs_and_row_mutations() {
        let parsed = super::super::parse_unresolved(
            "trait Family{type Out;}\
             fn unused<T:Family<Out=(bool,[i32;7]),Out=(bool,[i32;7])>>(){}fn main(){}",
        )
        .unwrap();
        let valid = parsed.program;
        let TraitPath::Source(original) = &valid.functions[0].bounds[0].traits[0] else {
            panic!("expected written equality application")
        };
        let budget = ir::PlanBudget::default();
        assert!(exact_source_generic_path(&valid.names, original, original, &budget, 0).unwrap());
        for site in 0..4 {
            let mut changed = original.clone();
            let rows = &mut changed.segments.last_mut().unwrap().associated_types;
            match site {
                // The spelling and owner agree, but the written occurrence does not.
                0 => rows[0].name = rows[1].name,
                1 => rows[0].value = Type::Bool,
                2 => rows.swap(0, 1),
                _ => {
                    rows.pop();
                }
            }
            assert!(
                !exact_source_generic_path(&valid.names, original, &changed, &budget, 0).unwrap(),
                "site {site}"
            );
        }
    }

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
            embedded_source: None,
            lexical_scope: None,
            module: builder.module_root().unwrap(),
            constness: Constness::Runtime,
            unsafety: false,
            static_mut: false,
            visibility: ir::Visibility::Public,
            bounds: Vec::new(),
            name: builder.path("crate::unused").unwrap(),
            regions: RegionDecls::default(),
            type_params: Vec::new(),
            params: Vec::new(),
            param_mut: Vec::new(),
            result: Type::Unit,
            impl_params: 0,
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

    fn trait_application(builder: &mut ProgramBuilder, name: &str, args: Vec<Type>) -> TraitPath {
        let path = builder.path(name).unwrap();
        let mut segments: Vec<_> = path
            .segments()
            .iter()
            .map(|name| PathSegment {
                name: *name,
                region_args: Vec::new(),
                args: Vec::new(),
                associated_types: Vec::new(),
            })
            .collect();
        segments.last_mut().unwrap().args = args;
        TraitPath::Source(GenericPath {
            anchor: path.anchor(),
            segments,
            resolved_use: None,
        })
    }

    #[test]
    fn unused_trait_parameters_bounds_and_all_application_arguments_are_checked() {
        let mut foreign = builder();
        let foreign_name = foreign.ident("Foreign").unwrap();
        let foreign_path = foreign.path("crate::Other").unwrap();
        let _foreign = foreign.finish().unwrap();
        let mut input = builder();
        let module = input.module_root().unwrap();
        let parameter = input.ident("T").unwrap();
        let name = input.path("crate::Unused").unwrap();
        let bound = trait_application(
            &mut input,
            "crate::Other",
            vec![
                Type::Bool,
                Type::Tuple(vec![Type::Unit, Type::Param(parameter)]),
            ],
        );
        let parent = trait_application(&mut input, "crate::Parent", vec![Type::Param(parameter)]);
        input.traits.push(TraitDecl {
            module,
            type_params: vec![parameter],
            bounds: vec![TypeBound {
                subject: Type::Param(parameter),
                traits: vec![bound],
                maybe_unsized: false,
            }],
            supertraits: vec![parent],
            name,
            visibility: ir::Visibility::Public,
            associated_constants: Vec::new(),
            associated_types: Vec::new(),
            self_defaults: Vec::new(),
            items: Vec::new(),
        });
        let valid = input.finish().unwrap();
        let budget = valid.names.budget();
        let baseline = budget.snapshot().temporary_bytes;
        validate_program_names(&valid).unwrap();
        assert_eq!(budget.snapshot().temporary_bytes, baseline);
        for site in 0..5 {
            let mut malformed = valid.clone();
            let declaration = &mut malformed.traits[0];
            match site {
                0 => declaration.type_params[0] = foreign_name,
                1 => declaration.bounds[0].subject = Type::Param(foreign_name),
                2 => {
                    let TraitPath::Source(path) = &mut declaration.bounds[0].traits[0] else {
                        unreachable!()
                    };
                    path.segments.last_mut().unwrap().args[1] =
                        Type::Tuple(vec![Type::Unit, Type::Param(foreign_name)]);
                }
                3 => {
                    let TraitPath::Source(path) = &mut declaration.supertraits[0] else {
                        unreachable!()
                    };
                    path.segments.last_mut().unwrap().args[0] = Type::Param(foreign_name);
                }
                _ => {
                    let TraitPath::Source(path) = &mut declaration.supertraits[0] else {
                        unreachable!()
                    };
                    path.resolved_use = Some(foreign_path.clone());
                }
            }
            assert!(
                validate_program_names(&malformed)
                    .unwrap_err()
                    .starts_with("contract:"),
                "site {site}"
            );
            assert_eq!(budget.snapshot().temporary_bytes, baseline);
        }
        assert!(budget.snapshot().failure.is_none());
    }

    #[test]
    fn generic_trait_imports_check_all_handles_then_reject_application_shape() {
        let mut input = builder();
        let module = input.module_root().unwrap();
        let import = trait_application(&mut input, "crate::Imported", Vec::new());
        input.trait_imports.push((module, import));
        let valid = input.finish().unwrap();
        validate_program_names(&valid).unwrap();
        let mut malformed = valid.clone();
        let TraitPath::Source(path) = &mut malformed.trait_imports[0].1 else {
            unreachable!()
        };
        path.segments.last_mut().unwrap().args.push(Type::Bool);
        assert!(
            validate_program_names(&malformed)
                .unwrap_err()
                .contains("trait imports cannot contain application arguments")
        );
        let mut foreign = builder();
        let foreign_name = foreign.ident("Foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        let TraitPath::Source(path) = &mut malformed.trait_imports[0].1 else {
            unreachable!()
        };
        path.segments
            .last_mut()
            .unwrap()
            .args
            .push(Type::Param(foreign_name));
        let error = validate_program_names(&malformed).unwrap_err();
        assert!(error.starts_with("contract:"));
        assert!(!error.contains("trait imports cannot contain application arguments"));
    }

    fn generated_program(captured: bool) -> Program {
        let source = if captured {
            "macro_rules! make{($name:ident)=>{pub fn $name(x:i32)->i32{let local=x+1;local}};}make!(generated);fn original()->i32{0}"
        } else {
            "macro_rules! make{()=>{pub fn generated(x:i32)->i32{let local=x+1;local}};}make!();fn original()->i32{0}"
        };
        super::super::parse_syntax_unprepared(source)
            .unwrap()
            .program
    }

    #[test]
    fn generated_function_products_bind_exact_declarations_and_cover_even_unused_public_items() {
        for captured in [true, false] {
            let input = generated_program(captured);
            validate_program_names(&input).unwrap();
            let scopes = input.scopes.as_ref().unwrap();
            let index = input
                .functions
                .iter()
                .position(|function| {
                    function.lexical_scope.is_some_and(|scope| {
                        scopes
                            .generated_function_declaration(scope)
                            .unwrap()
                            .is_some()
                    })
                })
                .unwrap();
            let original = (0..input.functions.len())
                .find(|other| *other != index)
                .unwrap();
            let mut missing = input.clone();
            missing.functions.remove(index);
            assert!(
                validate_program_names(&missing)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            // Bounded active validation is not a second whole-program coverage scan.
            validate_function_names(&missing, &input.functions[original], None).unwrap();
            let mut empty = input.clone();
            empty.functions.clear();
            assert!(
                validate_program_names(&empty)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            let mut duplicate = input.clone();
            duplicate.functions.push(input.functions[index].clone());
            assert!(
                validate_program_names(&duplicate)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            let mut changed_name = input.clone();
            changed_name.functions[index].name = input.functions[original].name.clone();
            assert!(
                validate_program_names(&changed_name)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            assert!(
                validate_function_names(&changed_name, &changed_name.functions[index], None)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            // Use certificates are not declaration-product authority. Even a
            // real generated source occurrence cannot authorize another target.
            let mut certified_wrong_declaration = input.clone();
            certified_wrong_declaration.functions[index].name = input.functions[original]
                .name
                .with_resolved_use(input.names.as_ref(), &input.functions[index].name)
                .unwrap();
            assert!(
                validate_program_names(&certified_wrong_declaration)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            let mut detached = input.clone();
            detached.functions[index].lexical_scope = None;
            assert!(
                validate_program_names(&detached)
                    .unwrap_err()
                    .starts_with("contract:")
            );
        }
    }

    #[test]
    fn inactive_macro_syntax_cannot_be_inserted_into_an_unused_active_body() {
        let mut builder = builder();
        let path = builder.path("crate::unknown").unwrap();
        let span = builder.names().data(path.segments()[0]).unwrap().span;
        let names = builder.names();
        let stream = crate::compiler::token_tree::group(
            names.session(),
            &[],
            crate::compiler::token_tree::GroupPolicy::OriginalSource,
        )
        .unwrap();
        let args =
            crate::compiler::macro_cursor::TokenSlice::roots(names.session(), stream).unwrap();
        let lease = names
            .budget()
            .reserve(
                Storage::Retained,
                std::mem::size_of::<UnexpandedMacroSyntax>() as u64,
            )
            .unwrap();
        let syntax = Arc::new(UnexpandedMacroSyntax {
            _path: path,
            _args: args,
            span,
            _lease: lease,
        });
        let function = function(&mut builder);
        builder.functions.push(function);
        let mut input = builder.finish().unwrap();
        input.functions[0].body.tail = Some(Box::new(Expr::UnexpandedMacro(syntax)));
        assert!(
            validate_program_names(&input)
                .unwrap_err()
                .starts_with("unsupported:")
        );
        assert!(
            validate_function_names(&input, &input.functions[0], None)
                .unwrap_err()
                .starts_with("unsupported:")
        );
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
                    by_ref: None,
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
            shape: StructShape::Record,
            type_params: Vec::new(),
            bounds: Vec::new(),
            region_params: Vec::new(),
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
    fn struct_shape_integrity_does_not_reject_genuine_unit_names() {
        let mut input = builder();
        let module = input.module_root().unwrap();
        let name = input.path("crate::Unit").unwrap();
        input.structs.push(StructDecl {
            module,
            shape: StructShape::Unit,
            type_params: Vec::new(),
            bounds: Vec::new(),
            region_params: Vec::new(),
            name,
            visibility: ir::Visibility::Public,
            fields: Vec::new(),
        });
        let mut program = input.finish().unwrap();
        validate_program_names(&program).unwrap();
        program.structs[0].shape = StructShape::Record;
        assert!(
            validate_program_names(&program)
                .unwrap_err()
                .starts_with("contract:")
        );
        program.structs[0].shape = StructShape::Unit;
        program.structs[0].fields.push(FieldDecl {
            name: FieldName::Tuple(0),
            ty: Type::Unit,
            visibility: ir::Visibility::Public,
        });
        assert!(
            validate_program_names(&program)
                .unwrap_err()
                .starts_with("contract:")
        );
    }
    #[test]
    fn unused_let_else_checks_all_actual_handles_before_semantic_admission() {
        let mut foreign = builder();
        let foreign_name = foreign.ident("foreign").unwrap();
        let foreign_path = foreign.path("crate::Foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        let mut valid = program();
        // This is handle admission only; divergence is checked separately by
        // source typing and the checked IR/CFG boundary.
        valid.functions[0].body.statements.push(Stmt::LetElse {
            pattern: Box::new(Pattern::Wildcard),
            ty: Type::Unit,
            value: Box::new(Expr::Bool(true)),
            else_block: Block {
                statements: Vec::new(),
                tail: None,
            },
        });
        validate_program_names(&valid).unwrap();
        for child in 0..4 {
            let mut malformed = valid.clone();
            let Stmt::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } = malformed.functions[0].body.statements.last_mut().unwrap()
            else {
                unreachable!()
            };
            match child {
                0 => {
                    **pattern = Pattern::Binding {
                        name: foreign_name,
                        mutable: false,
                        by_ref: None,
                    }
                }
                1 => *ty = Type::Param(foreign_name),
                2 => **value = Expr::Var(foreign_path.clone()),
                3 => else_block.tail = Some(Box::new(Expr::Var(foreign_path.clone()))),
                _ => unreachable!(),
            }
            assert!(validate_program_names(&malformed).is_err());
            assert!(validate_function_names(&malformed, &malformed.functions[0], None).is_err());
        }
    }

    #[test]
    fn let_else_success_binding_persists_but_cannot_move_into_initializer_or_failure() {
        let source = "enum E{A(i32),B}fn f(p:E)->i32{macro_rules! marker{()=>{0}}let n=9;let E::A(n)=p else{return n;};n}fn main(){}";
        for target in 0..3 {
            let mut input = parse(source).unwrap();
            validate_program_names(&input).unwrap();
            validate_function_names(&input, &input.functions[0], None).unwrap();
            let donor = input.functions[0].body.tail.take().unwrap();
            let Stmt::LetElse {
                pattern,
                value,
                else_block,
                ..
            } = &mut input.functions[0].body.statements[1]
            else {
                unreachable!()
            };
            match target {
                0 => *value = donor,
                1 => else_block.statements.push(Stmt::Expr(*donor)),
                2 => {
                    **pattern = Pattern::Wildcard;
                    input.functions[0].body.tail = Some(donor);
                }
                _ => unreachable!(),
            }
            let before = input.names.budget().snapshot();
            assert!(
                validate_program_names(&input)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            assert!(validate_function_names(&input, &input.functions[0], None).is_err());
            let after = input.names.budget().snapshot();
            assert!(after.failure.is_none());
            assert_eq!(after.temporary_bytes, before.temporary_bytes);
            assert_eq!(after.retained_bytes, before.retained_bytes);
        }
    }

    #[test]
    fn unused_while_let_checks_every_actual_handle_even_in_dead_body() {
        let mut foreign = builder();
        let foreign_name = foreign.ident("foreign").unwrap();
        let foreign_path = foreign.path("crate::Foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        let mut valid = program();
        valid.functions[0].body.statements.push(Stmt::WhileLet {
            label: None,
            pattern: Box::new(Pattern::Bool(true)),
            scrutinee: Box::new(Expr::Bool(false)),
            has_semicolon: false,
            body: Block {
                statements: Vec::new(),
                tail: None,
            },
        });
        validate_program_names(&valid).unwrap();
        for child in 0..4 {
            let mut malformed = valid.clone();
            let Stmt::WhileLet {
                label,
                pattern,
                scrutinee,
                body,
                ..
            } = malformed.functions[0].body.statements.last_mut().unwrap()
            else {
                unreachable!()
            };
            match child {
                0 => *label = Some(foreign_name),
                1 => {
                    **pattern = Pattern::Binding {
                        name: foreign_name,
                        mutable: false,
                        by_ref: None,
                    }
                }
                2 => **scrutinee = Expr::Var(foreign_path.clone()),
                3 => body.tail = Some(Box::new(Expr::Var(foreign_path.clone()))),
                _ => unreachable!(),
            }
            assert!(validate_program_names(&malformed).is_err());
            assert!(validate_function_names(&malformed, &malformed.functions[0], None).is_err());
        }
    }

    #[test]
    fn while_let_body_binding_cannot_move_to_scrutinee_or_after_iteration_scope() {
        let source = "enum E{A(i32),B}fn f(p:E)->i32{macro_rules! marker{()=>{0}}let mut state=p;let n=9;while let E::A(n)=state{let x=n;state=E::B;}n}fn main(){}";
        for target in 0..3 {
            let mut input = parse(source).unwrap();
            validate_program_names(&input).unwrap();
            let Stmt::WhileLet {
                pattern,
                scrutinee,
                body,
                ..
            } = &mut input.functions[0].body.statements[2]
            else {
                unreachable!()
            };
            if target == 2 {
                **pattern = Pattern::Wildcard;
            } else {
                let Stmt::Let { value, .. } = &mut body.statements[0] else {
                    unreachable!()
                };
                let donor = Box::new(std::mem::replace(value, Expr::Bool(false)));
                if target == 0 {
                    *scrutinee = donor;
                } else {
                    input.functions[0].body.tail = Some(donor);
                }
            }
            let budget = input.names.budget();
            let before = budget.snapshot().temporary_bytes;
            assert!(
                validate_program_names(&input)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            assert!(validate_function_names(&input, &input.functions[0], None).is_err());
            assert!(budget.snapshot().failure.is_none());
            assert_eq!(budget.snapshot().temporary_bytes, before);
        }
    }

    #[test]
    fn lexical_liveness_keeps_outer_hygienic_ids_and_source_order_bindings() {
        for source in [
            "fn f(p:i32)->i32{let x=p;macro_rules! old{()=>{x}}let x=2;old!()+x}fn main(){}",
            "fn f(p:i32)->i32{macro_rules! marker{()=>{0}}let x=p;if let x=x{let y=x;y}else{x}}fn main(){}",
            "enum E{A(i32),B}fn f(p:E)->i32{macro_rules! marker{()=>{0}}match p{E::A(x)=>{let y=x;y},E::B=>0}}fn main(){}",
            "fn f(p:i32)->i32{macro_rules! marker{()=>{0}}let x:i32; x=p;{let y=x;let (a,b)=(y,x);a+b}}fn main(){}",
        ] {
            let input = parse(source).unwrap();
            validate_program_names(&input).unwrap();
            validate_function_names(&input, &input.functions[0], None).unwrap();
        }
    }

    #[test]
    fn moved_if_let_reference_requires_actual_live_pattern_not_occurrence_uniqueness() {
        let source = "enum E{A(i32),B}fn f(p:E)->i32{macro_rules! marker{()=>{0}}if let E::A(n)=p{n}else{0}}fn main(){}";
        for target in 0..4 {
            let mut input = parse(source).unwrap();
            let expression = input.functions[0].body.tail.as_deref_mut().unwrap();
            let Expr::IfLet {
                pattern,
                scrutinee,
                then_block,
                else_branch,
            } = expression
            else {
                unreachable!()
            };
            // Move the only use, rather than duplicating an occurrence. A
            // uniqueness check cannot establish branch-local binding authority.
            if target == 3 {
                **pattern = Pattern::Wildcard;
            } else {
                let donor = then_block.tail.take().unwrap();
                match target {
                    0 => {
                        *else_branch = Some(Box::new(Expr::Block(Block {
                            statements: Vec::new(),
                            tail: Some(donor),
                        })))
                    }
                    1 => {
                        let original = input.functions[0].body.tail.take().unwrap();
                        input.functions[0]
                            .body
                            .statements
                            .push(Stmt::Expr(*original));
                        input.functions[0].body.tail = Some(donor);
                    }
                    2 => *scrutinee = donor,
                    _ => unreachable!(),
                }
            }
            let budget = input.names.budget();
            let before = budget.snapshot();
            assert!(
                validate_program_names(&input)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            assert!(
                validate_function_names(&input, &input.functions[0], None)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            let after = budget.snapshot();
            assert!(after.failure.is_none());
            assert_eq!(after.temporary_bytes, before.temporary_bytes);
            assert_eq!(after.retained_bytes, before.retained_bytes);
        }
    }

    #[test]
    fn moved_match_arm_and_nested_block_references_cannot_escape_their_bindings() {
        for matching in [false, true] {
            let source = if matching {
                "enum E{A(i32),B}fn f(p:E)->i32{macro_rules! marker{()=>{0}}match p{E::A(x)=>x,E::B=>0}}fn main(){}"
            } else {
                "fn f()->i32{macro_rules! marker{()=>{0}}{let x=1;x}}fn main(){}"
            };
            let mut input = parse(source).unwrap();
            let expression = input.functions[0].body.tail.as_deref_mut().unwrap();
            let donor = match expression {
                Expr::Match { arms, .. } => {
                    Box::new(std::mem::replace(&mut arms[0].body, Expr::Bool(false)))
                }
                Expr::Block(block) => block.tail.take().unwrap(),
                _ => unreachable!(),
            };
            let original = input.functions[0].body.tail.take().unwrap();
            input.functions[0]
                .body
                .statements
                .push(Stmt::Expr(*original));
            input.functions[0].body.tail = Some(donor);
            assert!(
                validate_program_names(&input)
                    .unwrap_err()
                    .starts_with("contract:")
            );
            assert!(validate_function_names(&input, &input.functions[0], None).is_err());
        }
    }

    #[test]
    fn live_binding_storage_is_released_on_exact_latched_resource_failure() {
        let input = parse("fn f(p:i32)->i32{macro_rules! marker{()=>{0}}p}fn main(){}").unwrap();
        let budget = input.names.budget();
        let baseline = budget.snapshot().temporary_bytes;
        let mut validator = Validator::new(&input).unwrap();
        validator.scope = input.functions[0].lexical_scope;
        validator.module = input.functions[0].module;
        let BindingName::Named(name) = input.functions[0].params[0].0 else {
            unreachable!()
        };
        validator.visit(Visit::LocalDeclaration(name)).unwrap();
        assert_eq!(validator.bindings.len(), 1);
        assert!(budget.snapshot().temporary_bytes > baseline);
        validator
            .push(Visit::Block(&input.functions[0].body))
            .unwrap();
        let used = budget.snapshot().counters[Counter::Work as usize];
        let limit = budget.limits().counters[Counter::Work as usize];
        budget.charge(Counter::Work, limit - used).unwrap();
        let error = validator.run().unwrap_err();
        let failure = budget.snapshot().failure.unwrap();
        assert_eq!(error, transport(TokenError::Budget(failure)));
        assert_eq!(budget.snapshot().temporary_bytes, baseline);
    }

    #[test]
    fn unused_if_let_checks_pattern_scrutinee_then_and_actual_else_names() {
        let mut foreign = builder();
        let foreign_name = foreign.ident("foreign").unwrap();
        let foreign_path = foreign.path("crate::Foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        for has_else in [false, true] {
            let mut valid = program();
            valid.functions[0]
                .body
                .statements
                .push(Stmt::Expr(Expr::IfLet {
                    pattern: Box::new(Pattern::Wildcard),
                    scrutinee: Box::new(Expr::Bool(true)),
                    then_block: Block {
                        statements: Vec::new(),
                        tail: None,
                    },
                    else_branch: has_else.then(|| Box::new(Expr::Bool(false))),
                }));
            validate_program_names(&valid).unwrap();
            validate_function_names(&valid, &valid.functions[0], None).unwrap();
            for child in 0..if has_else { 4 } else { 3 } {
                let mut malformed = valid.clone();
                let Stmt::Expr(Expr::IfLet {
                    pattern,
                    scrutinee,
                    then_block,
                    else_branch,
                }) = malformed.functions[0].body.statements.last_mut().unwrap()
                else {
                    unreachable!()
                };
                match child {
                    0 => {
                        **pattern = Pattern::Binding {
                            name: foreign_name,
                            mutable: false,
                            by_ref: None,
                        }
                    }
                    1 => **scrutinee = Expr::Var(foreign_path.clone()),
                    2 => then_block.tail = Some(Box::new(Expr::Var(foreign_path.clone()))),
                    3 => **else_branch.as_mut().unwrap() = Expr::Var(foreign_path.clone()),
                    _ => unreachable!(),
                }
                assert!(validate_program_names(&malformed).is_err());
                assert!(
                    validate_function_names(&malformed, &malformed.functions[0], None).is_err()
                );
            }
        }
    }

    #[test]
    fn unused_array_patterns_check_every_explicit_child_including_rest_suffix() {
        let mut foreign = builder();
        let foreign_name = foreign.ident("foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        for rest in [false, true] {
            let mut valid = program();
            let pattern = if rest {
                Pattern::ArrayRest(PositionalRest {
                    prefix: vec![Pattern::Wildcard],
                    suffix: vec![Pattern::Wildcard],
                })
            } else {
                Pattern::Array(vec![Pattern::Wildcard, Pattern::Wildcard])
            };
            valid.functions[0].body.statements.push(Stmt::LetPattern {
                pattern,
                ty: Type::Unit,
                value: Expr::Bool(true),
            });
            validate_program_names(&valid).unwrap();
            for last in [false, true] {
                let mut malformed = valid.clone();
                let Stmt::LetPattern { pattern, .. } =
                    malformed.functions[0].body.statements.last_mut().unwrap()
                else {
                    unreachable!()
                };
                let target = match pattern {
                    Pattern::Array(fields) => &mut fields[usize::from(last)],
                    Pattern::ArrayRest(fields) => {
                        if last {
                            &mut fields.suffix[0]
                        } else {
                            &mut fields.prefix[0]
                        }
                    }
                    _ => unreachable!(),
                };
                *target = Pattern::Binding {
                    name: foreign_name,
                    mutable: false,
                    by_ref: None,
                };
                assert!(validate_program_names(&malformed).is_err());
                assert!(
                    validate_function_names(&malformed, &malformed.functions[0], None).is_err()
                );
            }
        }
    }

    #[test]
    fn positional_rest_checks_both_explicit_lists_and_variant_path() {
        let mut foreign = builder();
        let foreign_name = foreign.ident("foreign").unwrap();
        let foreign_path = foreign.path("crate::Foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        for variant in [false, true] {
            let mut input = builder();
            let path = input.path("crate::Pair").unwrap();
            let declaration = function(&mut input);
            input.functions.push(declaration);
            let mut valid = input.finish().unwrap();
            let fields = PositionalRest {
                prefix: vec![Pattern::Wildcard],
                suffix: vec![Pattern::Wildcard],
            };
            let pattern = if variant {
                Pattern::VariantRest { name: path, fields }
            } else {
                Pattern::TupleRest(fields)
            };
            valid.functions[0].body.statements.push(Stmt::LetPattern {
                pattern,
                ty: Type::Unit,
                value: Expr::Bool(true),
            });
            validate_program_names(&valid).unwrap();
            for suffix in [false, true] {
                let mut malformed = valid.clone();
                let Stmt::LetPattern { pattern, .. } =
                    malformed.functions[0].body.statements.last_mut().unwrap()
                else {
                    unreachable!()
                };
                let fields = match pattern {
                    Pattern::TupleRest(fields) | Pattern::VariantRest { fields, .. } => fields,
                    _ => unreachable!(),
                };
                let target = if suffix {
                    &mut fields.suffix
                } else {
                    &mut fields.prefix
                };
                target[0] = Pattern::Binding {
                    name: foreign_name,
                    mutable: false,
                    by_ref: None,
                };
                assert!(validate_program_names(&malformed).is_err());
                assert!(
                    validate_function_names(&malformed, &malformed.functions[0], None).is_err()
                );
            }
            if variant {
                let Stmt::LetPattern {
                    pattern: Pattern::VariantRest { name, .. },
                    ..
                } = valid.functions[0].body.statements.last_mut().unwrap()
                else {
                    unreachable!()
                };
                *name = foreign_path.clone();
                assert!(validate_program_names(&valid).is_err());
            }
        }
    }

    #[test]
    fn tuple_struct_shape_requires_contiguous_real_tuple_fields() {
        let mut input = builder();
        let module = input.module_root().unwrap();
        let name = input.path("crate::Pair").unwrap();
        input.structs.push(StructDecl {
            module,
            shape: StructShape::Tuple,
            type_params: Vec::new(),
            bounds: Vec::new(),
            region_params: Vec::new(),
            name,
            visibility: ir::Visibility::Public,
            fields: (0..2)
                .map(|index| FieldDecl {
                    name: FieldName::Tuple(index),
                    ty: Type::Unit,
                    visibility: ir::Visibility::Public,
                })
                .collect(),
        });
        let valid = input.finish().unwrap();
        validate_program_names(&valid).unwrap();
        for index in [0, 2, usize::MAX] {
            let mut malformed = valid.clone();
            malformed.structs[0].fields[1].name = FieldName::Tuple(index);
            assert!(
                validate_program_names(&malformed)
                    .unwrap_err()
                    .starts_with("contract:")
            );
        }
        let mut malformed = valid.clone();
        malformed.structs[0].shape = StructShape::Record;
        assert!(
            validate_program_names(&malformed)
                .unwrap_err()
                .starts_with("contract:")
        );
        let mut malformed = valid.clone();
        malformed.structs[0].fields.clear();
        assert!(
            validate_program_names(&malformed)
                .unwrap_err()
                .starts_with("contract:")
        );
    }

    #[test]
    fn unused_tuple_constructor_checks_path_and_every_child() {
        let mut input = builder();
        let declaration = function(&mut input);
        let path = input.path("crate::Pair").unwrap();
        input.functions.push(declaration);
        let mut valid = input.finish().unwrap();
        valid.functions[0]
            .body
            .statements
            .push(Stmt::Expr(Expr::TupleStruct {
                name: path,
                fields: vec![Expr::Bool(true), Expr::Bool(false)],
            }));
        validate_program_names(&valid).unwrap();
        let mut foreign = builder();
        let foreign_path = foreign.path("crate::Foreign").unwrap();
        let _foreign = foreign.finish().unwrap();
        let mut malformed = valid.clone();
        let Stmt::Expr(Expr::TupleStruct { fields, .. }) =
            malformed.functions[0].body.statements.last_mut().unwrap()
        else {
            unreachable!()
        };
        fields.push(Expr::Var(foreign_path.clone()));
        assert!(validate_program_names(&malformed).is_err());
        let Stmt::Expr(Expr::TupleStruct { name, .. }) =
            valid.functions[0].body.statements.last_mut().unwrap()
        else {
            unreachable!()
        };
        *name = foreign_path;
        assert!(validate_program_names(&valid).is_err());
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
            unsafety: false,
            name: input.functions[0].name.segments()[0],
            regions: RegionDecls::default(),
            type_params: Vec::new(),
            bounds: Vec::new(),
            params: Vec::new(),
            param_mut: Vec::new(),
            result: Type::Unit,
            impl_params: 0,
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

    #[test]
    fn empty_program_checks_optional_scope_owner_before_any_occurrence_use() {
        use crate::compiler::semantic_scope::ScopeArenaBuilder;
        let mut input = builder().finish().unwrap();
        let foreign = builder().finish().unwrap();
        input.scopes = Some(
            ScopeArenaBuilder::for_names(foreign.names.as_ref())
                .unwrap()
                .freeze()
                .unwrap(),
        );
        assert!(validate_program_names(&input).is_err());
        input.scopes = Some(
            ScopeArenaBuilder::for_names(input.names.as_ref())
                .unwrap()
                .freeze()
                .unwrap(),
        );
        validate_program_names(&input).unwrap();
        let clone = input.clone();
        assert!(Arc::ptr_eq(
            input.scopes.as_ref().unwrap(),
            clone.scopes.as_ref().unwrap()
        ));
        validate_program_names(&clone).unwrap();
    }

    #[test]
    fn unused_function_scope_checks_all_declaration_and_reference_associations() {
        use crate::compiler::semantic_scope::ScopeArenaBuilder;
        let mut source = builder();
        let mut declaration = function(&mut source);
        let binder = source.ident("local").unwrap();
        let reference = source.path("local").unwrap();
        let unrecorded = source.path("local").unwrap();
        let unresolved_elsewhere = source.path("missing").unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(source.names()).unwrap();
        let root = scopes.function_root(source.names()).unwrap();
        let function_scope = scopes.function(root).unwrap();
        let nested = scopes.enter(source.names(), root).unwrap();
        let (frontier, _) = scopes.bind(source.names(), nested, binder).unwrap();
        assert!(
            scopes
                .resolve_reference(source.names(), frontier, reference.segments()[0])
                .unwrap()
                .is_some()
        );
        let other_function = scopes.function_root(source.names()).unwrap();
        assert!(
            scopes
                .resolve_reference(
                    source.names(),
                    other_function,
                    unresolved_elsewhere.segments()[0]
                )
                .unwrap()
                .is_none()
        );
        declaration.body.statements.push(Stmt::Declare {
            name: BindingName::Named(binder),
            ty: Type::Unit,
            mutable: false,
        });
        declaration.body.tail = Some(Box::new(Expr::Var(reference)));
        source.functions.push(declaration);
        let mut input = source.finish().unwrap();
        input.scopes = Some(scopes.freeze().unwrap());
        input.functions[0].lexical_scope = Some(function_scope);
        validate_program_names(&input).unwrap();
        validate_function_names(&input, &input.functions[0], None).unwrap();
        input.functions[0].body.tail = Some(Box::new(Expr::Var(unrecorded)));
        assert!(validate_program_names(&input).is_err());
        assert!(validate_function_names(&input, &input.functions[0], None).is_err());
        input.functions[0].body.tail = Some(Box::new(Expr::Var(unresolved_elsewhere)));
        assert!(validate_program_names(&input).is_err());
        assert!(validate_function_names(&input, &input.functions[0], None).is_err());
        input.functions[0].body.tail = None;
        input.scopes = None;
        assert!(validate_program_names(&input).is_err());
    }
}
