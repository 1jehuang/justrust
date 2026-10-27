//! Checked source namespace, import and alias resolution.
//! No display spelling is used as a lookup key or reconstructed into an AST.
use super::super::module_identity::{CatalogModule, ModuleSegment};
use super::super::semantic_name::BuiltinAtom;
use super::super::semantic_scope::{BindingId, ScopeId, scope_diagnostic};
use super::*;
mod capture_resolution;
mod const_inline;
mod const_pattern;
mod lexical_calls;
mod primitive_owners;
mod stored_closures;

#[derive(Clone, Debug)]
enum ResolvedPath {
    Source(AstPath),
    CatalogModule(CatalogModule),
    Builtin(BuiltinTrait),
}
#[derive(Clone, Debug)]
struct ResolvedItem {
    visibility: ir::Visibility,
    path: ResolvedPath,
    module: bool,
    kind: ResolvedKind,
}
impl ResolvedItem {
    fn source(&self) -> Result<&AstPath, String> {
        match &self.path {
            ResolvedPath::Source(path) => Ok(path),
            _ => Err("catalog target is not a source declaration".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedKind {
    TypeAlias,
    Trait,
    Module,
    Function,
    Const,
    Struct,
    Enum,
    Variant,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LocalKey {
    Named(NameKey),
    Anchored(BindingId),
    Receiver,
}

/// Resolver-only source ownership. These indices refer to the retained parsed
/// program, never to a printable trait name or an unchecked caller certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceOwnerContext {
    None,
    Trait { source_index: usize },
    TraitImpl { source_index: usize },
}

/// Implicit-prelude lookup memo key: path, requester, value/type namespace,
/// type-namespace first segment, and user-crate anchoring.
type PreludeMemoKey = (PathKey, ModuleId, bool, bool, bool);

struct Resolver<'a> {
    program: &'a Program,
    type_depth: usize,
    alias_cache: HashMap<PathKey, Type>,
    alias_active: std::collections::HashSet<PathKey>,
    alias_budget: ir::PlanBudget,
    nominal_arity: HashMap<PathKey, usize>,
    self_nominal: Option<AstPath>,
    self_allowed: bool,
    source_owner: SourceOwnerContext,
    items: &'a HashMap<PathKey, Vec<Item>>,
    imports: HashMap<(PathKey, usize, bool), Option<ResolvedItem>>,
    /// Memo of implicit-prelude fallback lookups, recorded only outside
    /// active import/glob cycles so cycle-cut `None` results never persist.
    prelude_memo: HashMap<PreludeMemoKey, Option<Option<ResolvedItem>>>,
    active: std::collections::HashSet<(PathKey, usize, bool)>,
    type_params: Vec<AstIdent>,
    lexical_function: Option<ScopeId>,
    /// Interpret `crate` as the user crate root even inside the prelude.
    user_crate_anchor: bool,
    anonymous_closure_body: bool,
    active_closure: Option<closures::ClosureSource>,
    resolved_closures: HashMap<PathKey, closures::ClosureSource>,
    resolved_closure_owners: HashMap<PathKey, Vec<PathKey>>,
    current_function_name: Option<AstPath>,
    current_callable: Option<CallableSource>,
    /// Keys of `static mut` accessor functions (see static_mut.rs).
    static_muts: std::collections::HashSet<PathKey>,
    /// `use prefix::*;` declarations, consulted after explicit bindings.
    globs: &'a [GlobImport],
    active_globs: std::collections::HashSet<(usize, PathKey, bool)>,
}
impl Resolver<'_> {
    fn with_source_owner<T>(
        &mut self,
        owner: SourceOwnerContext,
        run: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let previous = std::mem::replace(&mut self.source_owner, owner);
        let allowed = self.self_allowed;
        let nominal = self.self_nominal.clone();
        let lexical = self.lexical_function;
        let closure = self.anonymous_closure_body;
        if owner == SourceOwnerContext::None {
            self.self_allowed = false;
            self.self_nominal = None;
        }
        let result = run(self);
        self.source_owner = previous;
        self.self_allowed = allowed;
        self.self_nominal = nominal;
        self.lexical_function = lexical;
        self.anonymous_closure_body = closure;
        result
    }

    fn contains_projection(&self, ty: &Type, depth: usize) -> Result<bool, String> {
        self.alias_budget.check_depth(depth)?;
        self.alias_budget.charge_work(1)?;
        let mut found = false;
        match ty {
            Type::Projection(_) => found = true,
            Type::Apply(path) => {
                self.alias_budget.charge_work(path.segments.len())?;
                for segment in &path.segments {
                    for actual in segment.args.iter().chain(
                        segment
                            .associated_types
                            .iter()
                            .map(|binding| &binding.value),
                    ) {
                        found |= self.contains_projection(actual, depth + 1)?;
                    }
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    found |= self.contains_projection(field, depth + 1)?;
                }
            }
            Type::Ref { referent, .. }
            | Type::Array(referent, _)
            | Type::Slice(referent)
            | Type::RawPtr {
                pointee: referent, ..
            } => {
                found |= self.contains_projection(referent, depth + 1)?;
            }
            _ => {}
        }
        Ok(found)
    }

    #[inline(never)]
    fn source_trait_index(&mut self, path: &TraitPath, module: ModuleId) -> Result<usize, String> {
        let TraitPath::Source(path) = path else {
            return Err("builtin trait has no associated type declaration".into());
        };
        let original = path.path(self.names())?;
        let target = self
            .path(&original, module, true, false)?
            .ok_or("unresolved projection trait")?;
        if target.kind != ResolvedKind::Trait {
            return Err("associated projection target is not a trait".into());
        }
        let target = self.key(target.source()?)?;
        for (index, declaration) in self.program.traits.iter().enumerate() {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? == target {
                return Ok(index);
            }
        }
        Err("projection trait is not a retained source declaration".into())
    }

    fn check_associated_member(
        &mut self,
        trait_index: usize,
        item: AstIdent,
    ) -> Result<(), String> {
        self.check_inherited_associated_member(trait_index, item, &mut Vec::new())?
            .map(|_| ())
            .ok_or_else(|| "no associated type in the exact owning trait".into())
    }

    fn check_inherited_associated_member(
        &mut self,
        trait_index: usize,
        item: AstIdent,
        active: &mut Vec<usize>,
    ) -> Result<Option<usize>, String> {
        self.alias_budget.check_depth(active.len())?;
        if active.contains(&trait_index) {
            return Err("cyclic supertrait associated lookup".into());
        }
        let key = self.ident_key(item, LookupClass::Member)?;
        let declaration = self
            .program
            .traits
            .get(trait_index)
            .ok_or("foreign source trait owner")?;
        for associated in &declaration.associated_types {
            self.alias_budget.charge_work(1)?;
            if self.ident_key(associated.name, LookupClass::Member)? == key {
                return Ok(Some(trait_index));
            }
        }
        let module = declaration.module;
        let parents = declaration.supertraits.clone();
        active.push(trait_index);
        let mut found = None;
        for parent in parents {
            if matches!(parent, TraitPath::Builtin(_)) {
                continue;
            }
            let index = self.source_trait_index(&parent, module)?;
            if let Some(owner) = self.check_inherited_associated_member(index, item, active)? {
                if found.is_some_and(|previous| previous != owner) {
                    return Err("ambiguous associated type in supertraits".into());
                }
                found = Some(owner);
            }
        }
        active.pop();
        Ok(found)
    }

    #[inline(never)]
    fn projection(
        &mut self,
        projection: &mut SourceProjection,
        module: ModuleId,
    ) -> Result<(), String> {
        let mut boundary = None;
        let (trait_index, item) = match projection {
            SourceProjection::Qualified {
                self_ty,
                trait_path,
                item,
            } => {
                self.ty(self_ty, module)?;
                // Resolve the owning trait from the written path before
                // trait_path canonicalizes it in place.
                let index = self.source_trait_index(trait_path, module)?;
                match self.trait_path(trait_path, module) {
                    Err(error) if error.starts_with("unsupported:") => {
                        boundary = Some(error);
                    }
                    result => result?,
                }
                (index, *item)
            }
            SourceProjection::SelfItem { item } => {
                let index = match self.source_owner {
                    SourceOwnerContext::Trait { source_index } => source_index,
                    SourceOwnerContext::TraitImpl { source_index } => {
                        let implementation = self
                            .program
                            .impls
                            .get(source_index)
                            .ok_or("foreign source impl owner")?;
                        let path = implementation
                            .trait_path
                            .as_ref()
                            .ok_or("Self associated type outside trait impl")?;
                        self.source_trait_index(path, implementation.module)?
                    }
                    SourceOwnerContext::None => {
                        return Err(
                            "Self associated type outside an owning trait or trait impl".into()
                        );
                    }
                };
                (index, *item)
            }
        };
        self.check_associated_member(trait_index, item)?;
        if let Some(error) = boundary {
            return Err(error);
        }
        Ok(())
    }

    fn names(&self) -> &NameStore {
        self.program.names.as_ref()
    }
    /// True when some impl or trait declares an associated function with this
    /// member name and no associated constant shares it. Selection of the exact
    /// owner and declaration happens during checking.
    fn names_associated_function(&self, name: AstIdent) -> Result<bool, String> {
        let names = self.names();
        let key = names
            .key(name, LookupClass::Member)
            .map_err(native_adapter::name_diagnostic)?;
        let same = |other: AstIdent| -> Result<bool, String> {
            Ok(names
                .key(other, LookupClass::Member)
                .map_err(native_adapter::name_diagnostic)?
                == key)
        };
        let mut function = false;
        for block in &self.program.impls {
            for constant in &block.associated_constants {
                if same(constant.name)? {
                    return Ok(false);
                }
            }
            for item in &block.items {
                if let Some(last) = item.function.name.segments().last()
                    && same(*last)?
                {
                    function = true;
                }
            }
        }
        for declaration in &self.program.traits {
            for constant in &declaration.associated_constants {
                if same(constant.name)? {
                    return Ok(false);
                }
            }
            for item in &declaration.items {
                if same(item.signature.name)? {
                    function = true;
                }
            }
        }
        Ok(function)
    }
    fn key(&self, path: &AstPath) -> Result<PathKey, String> {
        name_scope::path_key(self.names(), path, Namespace::Type)
    }
    fn display(&self, path: &AstPath) -> Result<super::super::semantic_path::PathDisplay, String> {
        path.display(self.names())
            .map_err(native_adapter::name_diagnostic)
    }
    fn ident_display(&self, ident: AstIdent) -> Result<&str, String> {
        self.names()
            .spelling(ident)
            .map_err(native_adapter::name_diagnostic)
    }
    fn ident_key(&self, ident: AstIdent, class: LookupClass) -> Result<NameKey, String> {
        self.names()
            .key(ident, class)
            .map_err(native_adapter::name_diagnostic)
    }
    fn param_contains(&self, ident: AstIdent) -> Result<bool, String> {
        let key = self.ident_key(ident, LookupClass::TypeParameter)?;
        for parameter in &self.type_params {
            self.alias_budget.charge_work(1)?;
            if self.ident_key(*parameter, LookupClass::TypeParameter)? == key {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn atom_is(
        &self,
        ident: AstIdent,
        atom: BuiltinAtom,
        namespace: Namespace,
    ) -> Result<bool, String> {
        Ok(self.ident_key(ident, LookupClass::Item(namespace))?
            == self.program.symbols.key(self.names(), atom)?)
    }
    fn path_items(&self, path: &AstPath) -> Result<Option<&Vec<Item>>, String> {
        Ok(self.items.get(&self.key(path)?))
    }
    fn charge_entries<T>(&self, count: usize) -> Result<(), String> {
        self.alias_budget
            .charge_work(count.checked_add(1).ok_or("resolver work overflow")?)?;
        self.alias_budget.retain_bytes(
            count
                .checked_mul(
                    std::mem::size_of::<T>()
                        .checked_mul(2)
                        .and_then(|size| size.checked_add(128))
                        .ok_or("resolver storage overflow")?,
                )
                .ok_or("resolver storage overflow")?,
        )
    }
    fn module_path(&self, mut module: ModuleId) -> Result<AstPath, String> {
        self.program
            .modules
            .check(module)
            .map_err(module_diagnostic)?;
        let mut segments = Vec::new();
        loop {
            match self
                .program
                .modules
                .segment(module)
                .map_err(module_diagnostic)?
            {
                ModuleSegment::SourceRoot => break,
                ModuleSegment::Catalog(_) => {
                    return Err("catalog module is not a source lexical scope".into());
                }
                ModuleSegment::Source(ident) => {
                    self.charge_entries::<AstIdent>(1)?;
                    segments
                        .try_reserve(1)
                        .map_err(|_| "module path allocation failed")?;
                    segments.push(ident);
                }
            }
            module = self
                .program
                .modules
                .parent(module)
                .map_err(module_diagnostic)?
                .ok_or("source module has no lexical parent")?;
        }
        self.alias_budget.charge_work(segments.len())?;
        segments.reverse();
        AstPath::new(self.names(), PathAnchor::Crate, &segments)
            .map_err(native_adapter::name_diagnostic)
    }
    fn semantic_append(&self, path: &AstPath, ident: AstIdent) -> Result<AstPath, String> {
        // This is a canonical namespace-lookup intermediate, not a new source
        // occurrence. Never mutate or extend an original-use certificate.
        path.check(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        let canonical = AstPath::new(self.names(), path.anchor(), path.segments())
            .map_err(native_adapter::name_diagnostic)?;
        canonical
            .push(self.names(), ident)
            .map_err(native_adapter::name_diagnostic)
    }
    fn single_path(&self, ident: AstIdent) -> Result<AstPath, String> {
        AstPath::new(self.names(), PathAnchor::Relative, &[ident])
            .map_err(native_adapter::name_diagnostic)
    }
    fn resolved_use(&self, target: &AstPath, source: &AstPath) -> Result<AstPath, String> {
        target
            .with_resolved_use(self.names(), source)
            .map_err(native_adapter::name_diagnostic)
    }
    fn certify_generic(&self, target: &mut GenericPath, source: &AstPath) -> Result<(), String> {
        target.resolved_use = None;
        let canonical = target.path(self.names())?;
        target.resolved_use = Some(self.resolved_use(&canonical, source)?);
        Ok(())
    }
    /// `Trait::method(args)` is `<_ as Trait>::method(args)`: the Self type
    /// is inferred from the first argument during type checking.
    fn trait_path_call(
        &mut self,
        name: &AstPath,
        args: &mut Vec<Expr>,
        module: ModuleId,
    ) -> Result<Option<Expr>, String> {
        if name.len() < 2 || name.anchor() == PathAnchor::SelfType {
            return Ok(None);
        }
        let prefix = name
            .semantic_parent(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        let Ok(Some(item)) = self.path(&prefix, module, true, false) else {
            return Ok(None);
        };
        if item.kind != ResolvedKind::Trait {
            return Ok(None);
        }
        let method = *name.segments().last().ok_or("empty trait call path")?;
        let trait_path = TraitPath::Source(self.alias_source_path(&prefix)?);
        Ok(Some(Expr::TraitCall {
            owner: Type::Infer,
            trait_path: Box::new(trait_path),
            name: method,
            type_args: Vec::new(),
            args: std::mem::take(args),
        }))
    }
    fn alias_source_path(&self, path: &AstPath) -> Result<GenericPath, String> {
        path.check(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        self.charge_entries::<PathSegment>(path.len())?;
        let mut segments = Vec::new();
        segments
            .try_reserve_exact(path.len())
            .map_err(|_| "source path allocation failed")?;
        for ident in path.segments() {
            segments.push(PathSegment {
                name: *ident,
                region_args: Vec::new(),
                args: Vec::new(),
                associated_types: Vec::new(),
            });
        }
        Ok(GenericPath {
            anchor: path.anchor(),
            segments,
            resolved_use: Some(path.clone()),
        })
    }
    fn alias_decl(&self, name: &AstPath) -> Result<(&TypeAliasDecl, ModuleId), String> {
        self.path_items(name)?
            .and_then(|items| {
                items.iter().find_map(|item| {
                    if let ItemKind::TypeAlias(alias) = &item.kind {
                        Some((alias, item.module))
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| "missing checked type alias declaration".into())
    }
    fn resolve_aliases(&mut self) -> Result<(), String> {
        // Source spans supply deterministic traversal order, never name identity.
        let roots = self.ordered_paths()?;
        #[expect(
            clippy::mutable_key_type,
            reason = "PathKey hashes immutable owner/anchor/namespace/name keys only; its shared resource ledger is not semantic identity"
        )]
        let mut active = std::collections::HashSet::new();
        for root in roots {
            if !self.path_items(&root)?.is_some_and(|items| {
                items
                    .iter()
                    .any(|item| matches!(item.kind, ItemKind::TypeAlias(_)))
            }) {
                continue;
            }
            self.charge_entries::<(AstPath, bool)>(1)?;
            let mut stack = vec![(root, false)];
            while let Some((name, finish)) = stack.pop() {
                self.alias_budget.charge_work(1)?;
                let key = self.key(&name)?;
                if self.alias_cache.contains_key(&key) {
                    continue;
                }
                if finish {
                    self.alias_template(&name)?;
                    active.remove(&key);
                    continue;
                }
                self.charge_entries::<PathKey>(1)?;
                active
                    .try_reserve(1)
                    .map_err(|_| "alias active allocation failed")?;
                if !active.insert(key) {
                    return Err(format!(
                        "E0391: cycle in type alias {}",
                        self.display(&name)?
                    ));
                }
                let (alias, module) = self.alias_decl(&name)?;
                walk_depth(DepthNode::Type(&alias.target), Some(&self.alias_budget))?;
                let mut spellings = Vec::new();
                let mut pending = vec![&alias.target];
                while let Some(ty) = pending.pop() {
                    self.alias_budget.charge_work(1)?;
                    self.charge_entries::<&Type>(1)?;
                    match ty {
                        Type::Projection(projection) => {
                            if let SourceProjection::Qualified {
                                self_ty,
                                trait_path,
                                ..
                            } = projection.as_ref()
                            {
                                self.charge_entries::<&Type>(1)?;
                                pending.push(self_ty);
                                if let TraitPath::Source(path) = trait_path {
                                    for segment in &path.segments {
                                        self.charge_entries::<&Type>(segment.args.len())?;
                                        pending.extend(&segment.args);
                                        pending.extend(
                                            segment
                                                .associated_types
                                                .iter()
                                                .map(|binding| &binding.value),
                                        );
                                    }
                                }
                            }
                        }
                        Type::Named(path) => spellings.push(path.clone()),
                        Type::Apply(path) => {
                            spellings.push(path.path(self.names())?);
                            for segment in &path.segments {
                                pending.extend(&segment.args);
                                pending.extend(
                                    segment
                                        .associated_types
                                        .iter()
                                        .map(|binding| &binding.value),
                                );
                            }
                        }
                        Type::Tuple(fields) => pending.extend(fields),
                        Type::Ref { referent, .. }
                        | Type::Array(referent, _)
                        | Type::Slice(referent)
                        | Type::RawPtr {
                            pointee: referent, ..
                        } => pending.push(referent),
                        _ => {}
                    }
                }
                let mut dependencies = Vec::new();
                for spelling in spellings {
                    if let Some(item) = self.path(&spelling, module, true, false)?
                        && item.kind == ResolvedKind::TypeAlias
                    {
                        self.charge_entries::<AstPath>(1)?;
                        dependencies.push(item.source()?.clone());
                    }
                }
                self.charge_entries::<(AstPath, bool)>(dependencies.len() + 1)?;
                stack.push((name, true));
                for dependency in dependencies.into_iter().rev() {
                    stack.push((dependency, false));
                }
            }
        }
        Ok(())
    }
    fn alias_template(&mut self, name: &AstPath) -> Result<Type, String> {
        let key = self.key(name)?;
        if let Some(target) = self.alias_cache.get(&key) {
            walk_depth(DepthNode::Type(target), Some(&self.alias_budget))?;
            return Ok(target.clone());
        }
        if self.alias_active.contains(&key) {
            return Err(format!(
                "E0391: cycle in type alias {}",
                self.display(name)?
            ));
        }
        if self.alias_active.len() >= 8 {
            return Err("type alias resolution nesting exceeds resource limit 8".into());
        }
        let (alias, module) = self.alias_decl(name)?;
        clone_vector(&alias.params, &self.alias_budget)?;
        walk_depth(DepthNode::Type(&alias.target), Some(&self.alias_budget))?;
        let params = alias.params.clone();
        let mut target = alias.target.clone();
        fn check_target(ty: &Type) -> Result<(), String> {
            match ty {
                Type::Projection(projection) => match projection.as_ref() {
                    SourceProjection::SelfItem { .. } => {
                        return Err("Self associated type outside impl in type alias".into());
                    }
                    SourceProjection::Qualified {
                        self_ty,
                        trait_path,
                        ..
                    } => {
                        check_target(self_ty)?;
                        if let TraitPath::Source(path) = trait_path {
                            for segment in &path.segments {
                                for actual in segment.args.iter().chain(
                                    segment
                                        .associated_types
                                        .iter()
                                        .map(|binding| &binding.value),
                                ) {
                                    check_target(actual)?;
                                }
                            }
                        }
                    }
                },
                Type::SelfType => return Err("Self outside impl in type alias".into()),
                Type::Ref { region: None, .. } => {
                    return Err("E0106: missing lifetime specifier in type alias".into());
                }
                Type::Ref { .. } => {
                    return Err("explicit lifetime type aliases are unsupported".into());
                }
                Type::Slice(_) => return Err("unsized type aliases are unsupported".into()),
                Type::Dyn(_) => {
                    return Err("unsupported: trait object type aliases are unsupported".into());
                }
                Type::Array(t, _) => check_target(t)?,
                Type::Tuple(fields) => {
                    for field in fields {
                        check_target(field)?;
                    }
                }
                Type::Apply(path) => {
                    for segment in &path.segments {
                        for arg in segment.args.iter().chain(
                            segment
                                .associated_types
                                .iter()
                                .map(|binding| &binding.value),
                        ) {
                            check_target(arg)?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        check_target(&target)?;
        self.charge_entries::<PathKey>(1)?;
        self.alias_active
            .try_reserve(1)
            .map_err(|_| "alias active allocation failed")?;
        self.alias_active.insert(key.clone());
        let saved_params = std::mem::replace(&mut self.type_params, params);
        let saved_self = self.self_nominal.take();
        let saved_allowed = std::mem::replace(&mut self.self_allowed, false);
        let saved_owner = std::mem::replace(&mut self.source_owner, SourceOwnerContext::None);
        let result = self.ty(&mut target, module);
        let params = std::mem::replace(&mut self.type_params, saved_params);
        self.self_nominal = saved_self;
        self.self_allowed = saved_allowed;
        self.source_owner = saved_owner;
        self.alias_active.remove(&key);
        result?;
        validate_depth(DepthNode::Type(&target))?;
        let mut used = std::collections::HashSet::new();
        self.collect_params(&target, &mut used, 0)?;
        for param in params {
            if !used.contains(&self.ident_key(param, LookupClass::TypeParameter)?) {
                return Err(format!(
                    "E0091: type parameter {} is never used in type alias {}",
                    self.ident_display(param)?,
                    self.display(name)?
                ));
            }
        }
        walk_depth(DepthNode::Type(&target), Some(&self.alias_budget))?;
        self.charge_entries::<(PathKey, Type)>(1)?;
        self.alias_cache
            .try_reserve(1)
            .map_err(|_| "alias cache allocation failed")?;
        self.alias_cache.insert(key, target.clone());
        Ok(target)
    }
    fn collect_params(
        &self,
        ty: &Type,
        used: &mut std::collections::HashSet<NameKey>,
        depth: usize,
    ) -> Result<(), String> {
        self.alias_budget.check_depth(depth)?;
        self.alias_budget.charge_work(1)?;
        match ty {
            Type::Projection(projection) => {
                if let SourceProjection::Qualified {
                    self_ty,
                    trait_path,
                    ..
                } = projection.as_ref()
                {
                    self.collect_params(self_ty, used, depth + 1)?;
                    if let TraitPath::Source(path) = trait_path {
                        for segment in &path.segments {
                            for actual in segment.args.iter().chain(
                                segment
                                    .associated_types
                                    .iter()
                                    .map(|binding| &binding.value),
                            ) {
                                self.collect_params(actual, used, depth + 1)?;
                            }
                        }
                    }
                }
            }
            Type::Param(name) => {
                self.charge_entries::<NameKey>(1)?;
                used.try_reserve(1)
                    .map_err(|_| "type parameter set allocation failed")?;
                used.insert(self.ident_key(*name, LookupClass::TypeParameter)?);
            }
            Type::Apply(path) => {
                for segment in &path.segments {
                    for arg in segment.args.iter().chain(
                        segment
                            .associated_types
                            .iter()
                            .map(|binding| &binding.value),
                    ) {
                        self.collect_params(arg, used, depth + 1)?;
                    }
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.collect_params(field, used, depth + 1)?;
                }
            }
            Type::Ref { referent, .. }
            | Type::Array(referent, _)
            | Type::Slice(referent)
            | Type::RawPtr {
                pointee: referent, ..
            } => self.collect_params(referent, used, depth + 1)?,
            _ => {}
        }
        Ok(())
    }
    fn expand_alias(
        &mut self,
        name: &AstPath,
        args: &[Type],
        source: &AstPath,
    ) -> Result<Type, String> {
        let params = &self.alias_decl(name)?.0.params;
        if params.len() != args.len() {
            return Err(format!(
                "E0107: type alias {} expects {} type arguments, got {}",
                self.display(name)?,
                params.len(),
                args.len()
            ));
        }
        clone_vector(params, &self.alias_budget)?;
        let params = params.clone();
        let mut target = self.alias_template(name)?;
        // The source program is universally admitted before resolution. Thus
        // even aliases erased to primitives had their original use checked.
        // Surviving template paths retain that use, not the alias declaration's
        // occurrences. Annotate before substitution so caller arguments keep
        // their own independent provenance when they replace parameters.
        self.alias_use(&mut target, source, 0)?;
        self.substitute(&mut target, &params, args, 0)?;
        validate_depth(DepthNode::Type(&target))?;
        Ok(target)
    }
    fn alias_use(&self, ty: &mut Type, source: &AstPath, depth: usize) -> Result<(), String> {
        self.alias_budget.check_depth(depth)?;
        self.alias_budget.charge_work(1)?;
        match ty {
            Type::Projection(projection) => {
                if let SourceProjection::Qualified {
                    self_ty,
                    trait_path,
                    ..
                } = projection.as_mut()
                {
                    self.alias_use(self_ty, source, depth + 1)?;
                    // The trait and member keep their genuine definition-site
                    // occurrences. An alias invocation is not written trait
                    // syntax and must not be used to fabricate that authority.
                    if let TraitPath::Source(path) = trait_path {
                        for segment in &mut path.segments {
                            for actual in segment.args.iter_mut().chain(
                                segment
                                    .associated_types
                                    .iter_mut()
                                    .map(|binding| &mut binding.value),
                            ) {
                                self.alias_use(actual, source, depth + 1)?;
                            }
                        }
                    }
                }
            }
            Type::Named(path) => *path = self.resolved_use(path, source)?,
            Type::Apply(path) => {
                self.certify_generic(path, source)?;
                for segment in &mut path.segments {
                    for arg in segment.args.iter_mut().chain(
                        segment
                            .associated_types
                            .iter_mut()
                            .map(|binding| &mut binding.value),
                    ) {
                        self.alias_use(arg, source, depth + 1)?;
                    }
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.alias_use(field, source, depth + 1)?;
                }
            }
            Type::Ref { referent, .. }
            | Type::Array(referent, _)
            | Type::Slice(referent)
            | Type::RawPtr {
                pointee: referent, ..
            } => self.alias_use(referent, source, depth + 1)?,
            _ => {}
        }
        Ok(())
    }
    fn substitute(
        &self,
        ty: &mut Type,
        params: &[AstIdent],
        args: &[Type],
        depth: usize,
    ) -> Result<(), String> {
        self.alias_budget.check_depth(depth)?;
        self.alias_budget.charge_work(1)?;
        match ty {
            Type::Projection(projection) => {
                if let SourceProjection::Qualified {
                    self_ty,
                    trait_path,
                    ..
                } = projection.as_mut()
                {
                    self.substitute(self_ty, params, args, depth + 1)?;
                    if let TraitPath::Source(path) = trait_path {
                        for segment in &mut path.segments {
                            for actual in segment.args.iter_mut().chain(
                                segment
                                    .associated_types
                                    .iter_mut()
                                    .map(|binding| &mut binding.value),
                            ) {
                                self.substitute(actual, params, args, depth + 1)?;
                            }
                        }
                    }
                }
            }
            Type::Param(name) => {
                let key = self.ident_key(*name, LookupClass::TypeParameter)?;
                for (index, param) in params.iter().enumerate() {
                    self.alias_budget.charge_work(1)?;
                    if self.ident_key(*param, LookupClass::TypeParameter)? == key {
                        walk_depth(DepthNode::Type(&args[index]), Some(&self.alias_budget))?;
                        *ty = args[index].clone();
                        break;
                    }
                }
            }
            Type::Apply(path) => {
                for segment in &mut path.segments {
                    for arg in segment.args.iter_mut().chain(
                        segment
                            .associated_types
                            .iter_mut()
                            .map(|binding| &mut binding.value),
                    ) {
                        self.substitute(arg, params, args, depth + 1)?;
                    }
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.substitute(field, params, args, depth + 1)?;
                }
            }
            Type::Ref { referent, .. }
            | Type::Array(referent, _)
            | Type::Slice(referent)
            | Type::RawPtr {
                pointee: referent, ..
            } => self.substitute(referent, params, args, depth + 1)?,
            _ => {}
        }
        Ok(())
    }
    fn alias_constructor_path(
        &mut self,
        path: &GenericPath,
        module: ModuleId,
        named_constructor: bool,
    ) -> Result<Option<GenericPath>, String> {
        for end in 1..=path.segments.len() {
            if end == path.segments.len() && !named_constructor {
                return Ok(None);
            }
            let prefix = GenericPath {
                anchor: path.anchor,
                segments: path.segments[..end].to_vec(),
                resolved_use: None,
            }
            .path(self.names())?;
            let Some(item) = self.path(&prefix, module, true, false)? else {
                continue;
            };
            if item.kind != ResolvedKind::TypeAlias {
                if item.kind == ResolvedKind::Module && !path.segments[end - 1].args.is_empty() {
                    return Err("generic arguments are not allowed on modules".into());
                }
                if item.kind != ResolvedKind::Module {
                    return Ok(None);
                }
                continue;
            }
            if path.segments[end - 1].args.is_empty()
                && !self.alias_decl(item.source()?)?.0.params.is_empty()
            {
                return Err("inferred generic type alias constructors and patterns are unsupported; supply explicit type arguments".into());
            }
            clone_vector(&path.segments[end - 1].args, &self.alias_budget)?;
            let mut args = path.segments[end - 1].args.clone();
            for arg in &mut args {
                self.ty(arg, module)?;
            }
            let target = self.expand_alias(item.source()?, &args, &path.path(self.names())?)?;
            let mut expanded = match target {
                Type::Named(name) => self.alias_source_path(&name)?,
                Type::Apply(path) => path,
                _ => return Ok(None),
            };
            clone_vector(&path.segments[end..], &self.alias_budget)?;
            expanded
                .segments
                .try_reserve(path.segments.len() - end)
                .map_err(|_| "alias constructor path allocation failed")?;
            expanded.segments.extend_from_slice(&path.segments[end..]);
            self.certify_generic(&mut expanded, &path.path(self.names())?)?;
            return Ok(Some(expanded));
        }
        Ok(None)
    }
    fn builtin_trait(&self, ident: AstIdent) -> Result<Option<BuiltinTrait>, String> {
        for (atom, id) in [
            (BuiltinAtom::Sized, BuiltinTrait::Sized),
            (BuiltinAtom::Copy, BuiltinTrait::Copy),
            (BuiltinAtom::Clone, BuiltinTrait::Clone),
            (BuiltinAtom::Drop, BuiltinTrait::Drop),
        ] {
            if self.atom_is(ident, atom, Namespace::Type)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
    fn catalog_child(
        &self,
        module: CatalogModule,
        ident: AstIdent,
        namespace: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        if !namespace {
            return Ok(None);
        }
        let child = if module == CatalogModule::Root {
            if self.atom_is(ident, BuiltinAtom::Marker, Namespace::Type)? {
                Some(CatalogModule::Marker)
            } else if self.atom_is(ident, BuiltinAtom::CloneModule, Namespace::Type)? {
                Some(CatalogModule::Clone)
            } else if self.atom_is(ident, BuiltinAtom::Ops, Namespace::Type)? {
                Some(CatalogModule::Ops)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(child) = child {
            return Ok(Some(ResolvedItem {
                visibility: ir::Visibility::Public,
                path: ResolvedPath::CatalogModule(child),
                module: true,
                kind: ResolvedKind::Module,
            }));
        }
        let Some(trait_id) = self.builtin_trait(ident)? else {
            return Ok(None);
        };
        let valid = matches!(
            (module, trait_id),
            (
                CatalogModule::Marker,
                BuiltinTrait::Sized | BuiltinTrait::Copy
            ) | (CatalogModule::Clone, BuiltinTrait::Clone)
                | (CatalogModule::Ops, BuiltinTrait::Drop)
        );
        Ok(valid.then_some(ResolvedItem {
            visibility: ir::Visibility::Public,
            path: ResolvedPath::Builtin(trait_id),
            module: true,
            kind: ResolvedKind::Trait,
        }))
    }
    fn trait_path(&mut self, path: &mut TraitPath, module: ModuleId) -> Result<(), String> {
        self.trait_application(path, module, false)
    }
    /// Unlike ordinary trait uses, declaration bounds retain the complete
    /// original occurrence even for catalog targets. The target is kept in a
    /// separate private record and is never inferred later from display text.
    fn associated_bound(
        &mut self,
        path: &mut TraitPath,
        module: ModuleId,
    ) -> Result<TraitPath, String> {
        let TraitPath::Source(source) = path else {
            return Err(
                "unsupported: associated bound requires an original source occurrence".into(),
            );
        };
        let mut boundary = None;
        for segment in &mut source.segments {
            for ty in segment.args.iter_mut().chain(
                segment
                    .associated_types
                    .iter_mut()
                    .map(|binding| &mut binding.value),
            ) {
                match self.ty(ty, module) {
                    Err(error) if error.starts_with("unsupported:") => {
                        boundary.get_or_insert(error);
                    }
                    result => result?,
                }
            }
        }
        let original = source.path(self.names())?;
        let resolved = self.path(&original, module, true, false)?;
        let _target = match resolved {
            Some(item) if item.kind != ResolvedKind::Trait => {
                return Err(format!("{} is not a trait", self.display(&original)?));
            }
            Some(ResolvedItem {
                path: ResolvedPath::Builtin(kind),
                ..
            }) => Some(kind),
            Some(_) => None,
            None => {
                let kind = original
                    .single()
                    .map(|ident| self.builtin_trait(ident))
                    .transpose()?
                    .flatten();
                if kind.is_none() {
                    return Err(format!("unresolved trait {}", self.display(&original)?));
                }
                kind
            }
        };
        if let Some(error) = boundary {
            return Err(error);
        }
        self.certify_generic(source, &original)?;
        source_clone::clone_path(source, &self.alias_budget)?;
        let mut canonical = TraitPath::Source(source.clone());
        self.trait_application(&mut canonical, module, true)?;
        Ok(canonical)
    }
    fn trait_application(
        &mut self,
        path: &mut TraitPath,
        module: ModuleId,
        allow_equalities: bool,
    ) -> Result<(), String> {
        let TraitPath::Source(source) = path else {
            return Ok(());
        };
        // Resolve all supplied children before deciding capability. The global
        // source-name admission has already checked every occurrence handle.
        let mut boundary = None;
        for segment in &mut source.segments {
            for argument in segment.args.iter_mut().chain(
                segment
                    .associated_types
                    .iter_mut()
                    .map(|binding| &mut binding.value),
            ) {
                match self.ty(argument, module) {
                    Err(error) if error.starts_with("unsupported:") => {
                        boundary.get_or_insert(error);
                    }
                    result => result?,
                }
            }
        }
        if source.segments.is_empty() {
            return Err("empty trait application path".into());
        }
        let original = source.path(self.names())?;
        let resolved = self.path(&original, module, true, false)?;
        let misplaced = source.segments[..source.segments.len() - 1]
            .iter()
            .any(|segment| !segment.args.is_empty() || !segment.associated_types.is_empty());
        let has_equalities = source
            .segments
            .iter()
            .any(|segment| !segment.associated_types.is_empty());
        if let Some(item) = resolved {
            if item.kind != ResolvedKind::Trait {
                return Err(format!("{} is not a trait", self.display(&original)?));
            }
            match item.path {
                ResolvedPath::Source(target) => {
                    let target_key = self.key(&target)?;
                    let mut trait_index = None;
                    for (index, declaration) in self.program.traits.iter().enumerate() {
                        self.alias_budget.charge_work(1)?;
                        if self.key(&declaration.name)? == target_key {
                            trait_index = Some(index);
                        }
                    }
                    let trait_index =
                        trait_index.ok_or("trait is not a retained source declaration")?;
                    // Every row is checked before any duplicate handling or boundary.
                    for segment in &source.segments {
                        for binding in &segment.associated_types {
                            self.check_associated_member(trait_index, binding.name)?;
                        }
                    }
                    let mut canonical = self.alias_source_path(&target)?;
                    let destination = canonical
                        .segments
                        .last_mut()
                        .ok_or("empty canonical trait path")?;
                    let final_segment = source.segments.last_mut().ok_or("empty trait path")?;
                    destination.region_args = std::mem::take(&mut final_segment.region_args);
                    destination.args = std::mem::take(&mut final_segment.args);
                    destination.associated_types =
                        std::mem::take(&mut final_segment.associated_types);
                    self.certify_generic(&mut canonical, &original)?;
                    *path = TraitPath::Source(canonical);
                }
                ResolvedPath::Builtin(id) => {
                    if has_equalities {
                        return Err("builtin trait has no associated type declaration".into());
                    }
                    if source
                        .segments
                        .iter()
                        .any(|segment| !segment.args.is_empty())
                    {
                        return Err("builtin traits take no type arguments".into());
                    }
                    *path = TraitPath::Builtin(id);
                }
                ResolvedPath::CatalogModule(_) => {
                    return Err("resolved declaration is not a trait".into());
                }
            }
        } else if let Some(ident) = original.single()
            && let Some(id) = self.builtin_trait(ident)?
        {
            if has_equalities {
                return Err("builtin trait has no associated type declaration".into());
            }
            if source
                .segments
                .iter()
                .any(|segment| !segment.args.is_empty())
            {
                return Err("builtin traits take no type arguments".into());
            }
            *path = TraitPath::Builtin(id);
        } else {
            return Err(format!("unresolved trait {}", self.display(&original)?));
        }
        if let Some(error) = boundary {
            return Err(error);
        }
        if misplaced {
            return Err("unsupported: trait arguments and equalities must belong to the final trait segment".into());
        }
        if has_equalities && !allow_equalities {
            return Err(
                "unsupported: associated equalities are supported only in function and impl bounds"
                    .into(),
            );
        }
        Ok(())
    }
    fn bounds(&mut self, bounds: &mut [TypeBound], module: ModuleId) -> Result<(), String> {
        let allow_equalities = !matches!(self.source_owner, SourceOwnerContext::Trait { .. });
        self.bounds_with_equality_policy(bounds, module, allow_equalities)
    }

    fn trait_method_bounds(
        &mut self,
        bounds: &mut [TypeBound],
        module: ModuleId,
        trait_index: usize,
        item_index: usize,
    ) -> Result<(), String> {
        if self.source_owner
            != (SourceOwnerContext::Trait {
                source_index: trait_index,
            })
            || self
                .program
                .traits
                .get(trait_index)
                .is_none_or(|owner| owner.module != module || owner.items.get(item_index).is_none())
        {
            return Err("trait method bounds have no exact source owner".into());
        }
        self.bounds_with_equality_policy(bounds, module, true)
    }

    fn bounds_with_equality_policy(
        &mut self,
        bounds: &mut [TypeBound],
        module: ModuleId,
        allow_equalities: bool,
    ) -> Result<(), String> {
        let mut boundary = None;
        for bound in bounds {
            self.ty(&mut bound.subject, module)?;
            for path in &mut bound.traits {
                match self.trait_application(path, module, allow_equalities) {
                    Err(error) if error.starts_with("unsupported:") => {
                        boundary.get_or_insert(error);
                    }
                    result => result?,
                }
            }
        }
        if let Some(error) = boundary {
            return Err(error);
        }
        Ok(())
    }
    fn item(
        &mut self,
        path: &AstPath,
        requester: ModuleId,
        namespace: bool,
        visible_only: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        self.program
            .modules
            .check(requester)
            .map_err(module_diagnostic)?;
        let key = self.key(path)?;
        let Some(bindings) = self.items.get(&key) else {
            return self.glob_item(path, requester, namespace, visible_only);
        };
        let mut result = None;
        for (index, item) in bindings.iter().enumerate() {
            item.visibility
                .validate_for(&self.program.modules, item.module)
                .map_err(module_diagnostic)?;
            let kind = match &item.kind {
                ItemKind::TypeAlias(_) if namespace => Some(ResolvedKind::TypeAlias),
                ItemKind::Module(_) if namespace => Some(ResolvedKind::Module),
                ItemKind::Function if !namespace => Some(ResolvedKind::Function),
                ItemKind::Const if !namespace => Some(ResolvedKind::Const),
                ItemKind::Trait if namespace => Some(ResolvedKind::Trait),
                ItemKind::Struct
                    if namespace || self.source_struct(&item.path)?.shape == StructShape::Tuple =>
                {
                    Some(ResolvedKind::Struct)
                }
                ItemKind::Enum if namespace => Some(ResolvedKind::Enum),
                ItemKind::Variant(shape) if namespace || *shape != ir::VariantShape::Struct => {
                    Some(ResolvedKind::Variant)
                }
                _ => None,
            };
            let mut resolved = if matches!(item.kind, ItemKind::Import(_, _)) {
                self.import(path, index, namespace)?
            } else {
                kind.map(|kind| ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: ResolvedPath::Source(item.path.clone()),
                    module: namespace,
                    kind,
                })
            };
            if let Some(resolved) = &mut resolved {
                if resolved.kind == ResolvedKind::Struct && !namespace {
                    let declaration = self.tuple_struct(resolved.source()?)?;
                    for field in &declaration.fields {
                        self.alias_budget.charge_work(1)?;
                        field
                            .visibility
                            .validate_for(&self.program.modules, declaration.module)
                            .map_err(module_diagnostic)?;
                        if !field
                            .visibility
                            .allows_reexport(&self.program.modules, &resolved.visibility)
                            .map_err(module_diagnostic)?
                        {
                            resolved.visibility = field.visibility.clone();
                        }
                    }
                }
                if matches!(item.kind, ItemKind::Variant(_)) {
                    let owner = item
                        .path
                        .semantic_parent(self.names())
                        .map_err(native_adapter::name_diagnostic)?;
                    resolved.visibility = self
                        .path_items(&owner)?
                        .and_then(|items| {
                            items
                                .iter()
                                .find(|item| matches!(item.kind, ItemKind::Enum))
                        })
                        .ok_or("variant has no checked enum owner")?
                        .visibility
                        .clone();
                }
                if resolved
                    .visibility
                    .allows_reexport(&self.program.modules, &item.visibility)
                    .map_err(module_diagnostic)?
                {
                    resolved.visibility = item.visibility.clone();
                }
                if !resolved
                    .visibility
                    .accessible_from(&self.program.modules, requester)
                    .map_err(module_diagnostic)?
                {
                    if visible_only {
                        continue;
                    }
                    return Err(format!(
                        "private item {} is inaccessible from requesting module",
                        self.display(path)?
                    ));
                }
            }
            if let Some(resolved) = resolved
                && result.replace(resolved).is_some()
            {
                return Err(format!(
                    "duplicate item {} in {} namespace",
                    self.display(path)?,
                    if namespace { "type" } else { "value" }
                ));
            }
        }
        if result.is_none() {
            return self.glob_item(path, requester, namespace, visible_only);
        }
        Ok(result)
    }
    fn same_resolved(&self, a: &ResolvedItem, b: &ResolvedItem) -> Result<bool, String> {
        Ok(match (&a.path, &b.path) {
            (ResolvedPath::Source(x), ResolvedPath::Source(y)) => self.key(x)? == self.key(y)?,
            (x, y) => format!("{x:?}") == format!("{y:?}"),
        })
    }
    /// Glob-imported name: `path` is `module::ident` with no explicit
    /// binding; each `use target::*` in `module` contributes `target::ident`
    /// when that item is visible to the importing module. Distinct results
    /// from different globs are ambiguous (E0659).
    fn glob_item(
        &mut self,
        path: &AstPath,
        requester: ModuleId,
        namespace: bool,
        visible_only: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        if self.globs.is_empty() {
            return Ok(None);
        }
        let Some(ident) = path.segments().last().copied() else {
            return Ok(None);
        };
        let parent = path
            .semantic_parent(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        let parent_key = self.key(&parent)?;
        let ident_key = self.key(path)?;
        let mut found: Option<ResolvedItem> = None;
        for index in 0..self.globs.len() {
            let glob = &self.globs[index];
            if self.key(&glob.module_path)? != parent_key {
                continue;
            }
            let guard = (index, ident_key.clone(), namespace);
            if !self.active_globs.insert(guard.clone()) {
                continue;
            }
            let (module, target, visibility) =
                (glob.module, glob.target.clone(), glob.visibility.clone());
            let result = (|| -> Result<Option<ResolvedItem>, String> {
                let Some(base) = self.path(&target, module, true, true)? else {
                    return Err(format!("unresolved glob import {}", self.display(&target)?));
                };
                if !matches!(base.kind, ResolvedKind::Module | ResolvedKind::Enum) {
                    return Err(format!(
                        "glob import target {} is not a module or enum",
                        self.display(&target)?
                    ));
                }
                let ResolvedPath::Source(source) = &base.path else {
                    return Ok(None);
                };
                let full = self.semantic_append(source, ident)?;
                let Some(mut item) = self.item(&full, module, namespace, true)? else {
                    return Ok(None);
                };
                if item
                    .visibility
                    .allows_reexport(&self.program.modules, &visibility)
                    .map_err(module_diagnostic)?
                {
                    item.visibility = visibility.clone();
                }
                Ok(Some(item))
            })();
            self.active_globs.remove(&guard);
            let Some(item) = result? else {
                continue;
            };
            if !item
                .visibility
                .accessible_from(&self.program.modules, requester)
                .map_err(module_diagnostic)?
            {
                if visible_only {
                    continue;
                }
                return Err(format!(
                    "private item {} is inaccessible from requesting module",
                    self.display(path)?
                ));
            }
            match &found {
                Some(previous) if !self.same_resolved(previous, &item)? => {
                    return Err(format!(
                        "E0659: {} is ambiguous (glob imports)",
                        self.display(path)?
                    ));
                }
                _ => found = Some(item),
            }
        }
        Ok(found)
    }
    fn import(
        &mut self,
        path: &AstPath,
        index: usize,
        namespace: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        let path_key = self.key(path)?;
        let key = (path_key.clone(), index, namespace);
        if let Some(resolved) = self.imports.get(&key) {
            return Ok(resolved.clone());
        }
        if self.active.len() >= 128 {
            return Err("import resolution nesting exceeds limit 128".into());
        }
        self.charge_entries::<(PathKey, usize, bool)>(1)?;
        self.active
            .try_reserve(1)
            .map_err(|_| "import active allocation failed")?;
        if !self.active.insert(key.clone()) {
            return Ok(None);
        }
        let item = self
            .items
            .get(&path_key)
            .and_then(|items| items.get(index))
            .ok_or("missing checked import binding")?;
        let ItemKind::Import(target, type_only) = &item.kind else {
            return Err("checked import binding has wrong kind".into());
        };
        let target = target.clone();
        let type_only = *type_only;
        let requester = item.module;
        let resolved = if type_only && !namespace {
            None
        } else {
            self.path(&target, requester, namespace, true)?
        };
        if type_only
            && resolved.as_ref().is_some_and(|item| {
                !matches!(
                    item.kind,
                    ResolvedKind::Module | ResolvedKind::Enum | ResolvedKind::Trait
                )
            })
        {
            return Err(format!(
                "{} is not a module in grouped self import",
                self.display(&target)?
            ));
        }
        self.active.remove(&key);
        self.charge_entries::<((PathKey, usize, bool), Option<ResolvedItem>)>(1)?;
        self.imports
            .try_reserve(1)
            .map_err(|_| "import cache allocation failed")?;
        self.imports.insert(key, resolved.clone());
        Ok(resolved)
    }
    fn path(
        &mut self,
        path: &AstPath,
        from: ModuleId,
        namespace: bool,
        importing: bool,
    ) -> Result<Option<ResolvedItem>, String> {
        path.check(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        self.program
            .modules
            .check(from)
            .map_err(module_diagnostic)?;
        let parts = path.segments();
        let mut at = 0;
        let initial_module = match path.anchor() {
            // Inside the native prelude library `crate` names the library root.
            PathAnchor::Crate => Some(match self.prelude_root_of(from)? {
                Some(prelude) if !self.user_crate_anchor => prelude,
                _ if !self.user_crate_anchor && self.extern_root_of(from)?.is_some() => {
                    self.extern_root_of(from)?.unwrap()
                }
                _ => self
                    .program
                    .modules
                    .source_root()
                    .map_err(module_diagnostic)?,
            }),
            PathAnchor::SelfModule => Some(from),
            PathAnchor::Super(levels) => {
                let mut current = from;
                for _ in 0..levels {
                    current = self
                        .program
                        .modules
                        .parent(current)
                        .map_err(module_diagnostic)?
                        .ok_or("super path escapes crate root")?;
                }
                Some(current)
            }
            PathAnchor::SelfType => {
                let Some(nominal) = self.self_nominal.clone() else {
                    return if self.self_allowed {
                        Ok(None)
                    } else {
                        Err("Self outside impl".into())
                    };
                };
                let mut result = self.item(&nominal, from, true, false)?;
                for (index, ident) in parts.iter().enumerate() {
                    let Some(current) = result else {
                        return Ok(None);
                    };
                    let full = self.semantic_append(current.source()?, *ident)?;
                    result = self.item(
                        &full,
                        from,
                        if index + 1 == parts.len() {
                            namespace
                        } else {
                            true
                        },
                        importing && index + 1 == parts.len(),
                    )?;
                }
                return Ok(result.filter(|item| item.module == namespace));
            }
            PathAnchor::Relative => None,
        };
        let mut current = if let Some(module) = initial_module {
            let canonical = self.module_path(module)?;
            if module
                == self
                    .program
                    .modules
                    .source_root()
                    .map_err(module_diagnostic)?
            {
                ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: ResolvedPath::Source(canonical),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            } else {
                // `self` and `super` select an existing declaration, not a new
                // public module. In particular, a bare anchor reexport must
                // retain that module's inherited/restricted audience.
                let resolved = self
                    .item(&canonical, from, true, false)?
                    .ok_or("module anchor has no canonical source declaration")?;
                if resolved.kind != ResolvedKind::Module {
                    return Err("module anchor does not name a module declaration".into());
                }
                resolved
            }
        } else {
            let Some(ident) = parts.first() else {
                return Ok(None);
            };
            at = 1;
            let full = self.semantic_append(&self.module_path(from)?, *ident)?;
            let type_namespace = if parts.len() == 1 { namespace } else { true };
            if let Some(item) =
                self.item(&full, from, type_namespace, importing && parts.len() == 1)?
            {
                item
            } else if let Some(found) = self.extern_path(path, from, namespace)? {
                return Ok(found);
            } else if let Some(found) = self.prelude_path(path, from, namespace, type_namespace)? {
                return Ok(found);
            } else if type_namespace
                && (self.atom_is(*ident, BuiltinAtom::Core, Namespace::Type)?
                    || self.atom_is(*ident, BuiltinAtom::Std, Namespace::Type)?)
            {
                ResolvedItem {
                    visibility: ir::Visibility::Public,
                    path: ResolvedPath::CatalogModule(CatalogModule::Root),
                    module: true,
                    kind: ResolvedKind::Module,
                }
            } else {
                return Ok(None);
            }
        };
        while let Some(ident) = parts.get(at) {
            if !matches!(current.kind, ResolvedKind::Module | ResolvedKind::Enum) {
                return Err(format!(
                    "non-module declaration in path {}",
                    self.display(path)?
                ));
            }
            if current.kind == ResolvedKind::Enum && at + 1 != parts.len() {
                return Ok(None);
            }
            let type_namespace = if at + 1 == parts.len() {
                namespace
            } else {
                true
            };
            let next = match &current.path {
                ResolvedPath::CatalogModule(module) => {
                    self.catalog_child(*module, *ident, type_namespace)?
                }
                ResolvedPath::Source(source) => {
                    let full = self.semantic_append(source, *ident)?;
                    self.item(
                        &full,
                        from,
                        type_namespace,
                        importing && at + 1 == parts.len(),
                    )?
                }
                ResolvedPath::Builtin(_) => None,
            };
            let Some(item) = next else {
                return Ok(None);
            };
            current = item;
            at += 1;
        }
        Ok((current.module == namespace).then_some(current))
    }
    /// The mounted dependency crate root containing `module`, if any.
    fn extern_root_of(&self, module: ModuleId) -> Result<Option<ModuleId>, String> {
        for mount in &self.program.extern_mounts {
            if self
                .program
                .modules
                .is_descendant(module, *mount)
                .map_err(module_diagnostic)?
            {
                return Ok(Some(*mount));
            }
        }
        Ok(None)
    }
    /// Extern prelude: an unresolved first segment naming a dependency crate
    /// resolves into its hidden mount `crate::__justrust_extern_<name>`.
    fn extern_path(
        &mut self,
        path: &AstPath,
        from: ModuleId,
        namespace: bool,
    ) -> Result<Option<Option<ResolvedItem>>, String> {
        if self.program.extern_mounts.is_empty() {
            return Ok(None);
        }
        let parts = path.segments();
        let Some(first) = parts.first().copied() else {
            return Ok(None);
        };
        let wanted = format!(
            "{}{}",
            crate::compiler::source::EXTERN_PREFIX,
            self.ident_display(first)?
        );
        let mut mount = None;
        for candidate in &self.program.extern_mounts {
            let name = self
                .program
                .modules
                .name(*candidate)
                .map_err(module_diagnostic)?;
            if let Some(name) = name
                && self.ident_display(name)? == wanted
            {
                mount = Some(*candidate);
            }
        }
        let Some(mount) = mount else {
            return Ok(None);
        };
        let base_path = self.module_path(mount)?;
        let mut segments = base_path.segments().to_vec();
        segments.extend_from_slice(&parts[1..]);
        let full = AstPath::new(self.names(), PathAnchor::Crate, &segments)
            .map_err(native_adapter::name_diagnostic)?;
        let previous = std::mem::replace(&mut self.user_crate_anchor, true);
        let found = self.path(&full, from, namespace, false);
        self.user_crate_anchor = previous;
        Ok(Some(found?))
    }
    /// An `extern crate core|std as <first>;` item declared at the crate root
    /// of `from` (the user crate or a mounted dependency). Only those items
    /// enter the extern prelude; module-level aliases stay ordinary imports.
    fn extern_crate_alias(
        &mut self,
        path: &AstPath,
        from: ModuleId,
        namespace: bool,
    ) -> Result<Option<Option<ResolvedItem>>, String> {
        let parts = path.segments();
        let Some(first) = parts.first().copied() else {
            return Ok(None);
        };
        if self.prelude_root_of(from)?.is_some() {
            return Ok(None);
        }
        let root = match self.extern_root_of(from)? {
            Some(mount) => mount,
            None => self
                .program
                .modules
                .source_root()
                .map_err(module_diagnostic)?,
        };
        if root == from {
            // The crate root's own items were already tried by the caller.
            return Ok(None);
        }
        let full = self.semantic_append(&self.module_path(root)?, first)?;
        let wanted = self.key(&full)?;
        let mut extern_crate = false;
        for (module, alias) in self.program.extern_crate_aliases.iter() {
            self.alias_budget.charge_work(1)?;
            if *module == root && self.key(alias)? == wanted {
                extern_crate = true;
            }
        }
        if !extern_crate {
            return Ok(None);
        }
        let mut segments = self.module_path(root)?.segments().to_vec();
        segments.extend_from_slice(parts);
        let rooted = AstPath::new(self.names(), PathAnchor::Crate, &segments)
            .map_err(native_adapter::name_diagnostic)?;
        let previous = std::mem::replace(&mut self.user_crate_anchor, true);
        let found = self.path(&rooted, root, namespace, false);
        self.user_crate_anchor = previous;
        Ok(Some(found?))
    }
    /// The native prelude library root containing `module`, if any.
    fn prelude_root_of(&self, module: ModuleId) -> Result<Option<ModuleId>, String> {
        let Some(prelude) = self.program.prelude_module else {
            return Ok(None);
        };
        Ok(self
            .program
            .modules
            .is_descendant(module, prelude)
            .map_err(module_diagnostic)?
            .then_some(prelude))
    }
    /// Fallback for an unresolved relative first segment: `std`/`core` name
    /// the native prelude library, and any other name is looked up in the
    /// library's `prelude::v1` module. User items and imports were tried first,
    /// so they shadow both. `Ok(None)` means not found (the caller continues
    /// with closed catalog paths such as `core::marker::Copy`).
    fn prelude_path(
        &mut self,
        path: &AstPath,
        from: ModuleId,
        namespace: bool,
        type_namespace: bool,
    ) -> Result<Option<Option<ResolvedItem>>, String> {
        if self.program.prelude_module.is_none() {
            return Ok(None);
        }
        let memoizable = self.active.is_empty() && self.active_globs.is_empty();
        let memo_key = (
            self.key(path)?,
            from,
            namespace,
            type_namespace,
            self.user_crate_anchor,
        );
        if memoizable && let Some(found) = self.prelude_memo.get(&memo_key) {
            return Ok(found.clone());
        }
        let found = self.prelude_path_uncached(path, from, namespace, type_namespace)?;
        if memoizable {
            self.charge_entries::<(PathKey, ModuleId, Option<ResolvedItem>)>(1)?;
            self.prelude_memo
                .try_reserve(1)
                .map_err(|_| "prelude memo allocation failed")?;
            self.prelude_memo.insert(memo_key, found.clone());
        }
        Ok(found)
    }
    fn prelude_path_uncached(
        &mut self,
        path: &AstPath,
        from: ModuleId,
        namespace: bool,
        type_namespace: bool,
    ) -> Result<Option<Option<ResolvedItem>>, String> {
        let Some(prelude) = self.program.prelude_module else {
            return Ok(None);
        };
        let parts = path.segments();
        let Some(first) = parts.first().copied() else {
            return Ok(None);
        };
        let rest = &parts[1..];
        // `extern crate core as std;` at a crate root adds `std` to that
        // crate's extern prelude, so every module of the crate sees it before
        // the implicit library names (and before the no_std `std` boundary).
        if let Some(found) = self.extern_crate_alias(path, from, namespace)? {
            return Ok(Some(found));
        }
        let no_std = self.program.no_std_module(from)?;
        if no_std && self.atom_is(first, BuiltinAtom::Std, Namespace::Type)? {
            return Err("unresolved std crate in no_std source; explicit extern crate std opt-in is unsupported".into());
        }
        let library = type_namespace
            && !rest.is_empty()
            && (self.atom_is(first, BuiltinAtom::Core, Namespace::Type)?
                || self.atom_is(first, BuiltinAtom::Std, Namespace::Type)?);
        // A bare crate name (`use core as c;`, `extern crate core as std;`)
        // denotes the library root module itself, so aliases keep resolving
        // through the real library rather than the closed catalog root.
        if type_namespace
            && rest.is_empty()
            && (self.atom_is(first, BuiltinAtom::Core, Namespace::Type)?
                || self.atom_is(first, BuiltinAtom::Std, Namespace::Type)?)
        {
            let base = if no_std {
                self.program.core_library_module
            } else {
                Some(prelude)
            };
            if let Some(base) = base {
                let canonical = self.module_path(base)?;
                if let Some(item) = self.item(&canonical, from, true, false)?
                    && item.kind == ResolvedKind::Module
                {
                    return Ok(Some(Some(item)));
                }
            }
        }
        let (base, suffix) = if library {
            let base = if no_std {
                self.program
                    .core_library_module
                    .ok_or("unsupported: native core library surface is unavailable")?
            } else {
                prelude
            };
            (base, rest)
        } else if let Some(scope) = self.program.implicit_prelude(from)? {
            (scope, parts)
        } else {
            return Ok(None);
        };
        let base_path = self.module_path(base)?;
        let mut segments = base_path.segments().to_vec();
        segments.extend_from_slice(suffix);
        let full = AstPath::new(self.names(), PathAnchor::Crate, &segments)
            .map_err(native_adapter::name_diagnostic)?;
        // The rebuilt canonical path is anchored at the user crate root even
        // when the requester is inside the prelude library.
        let previous = std::mem::replace(&mut self.user_crate_anchor, true);
        let found = self.path(&full, from, namespace, false);
        self.user_crate_anchor = previous;
        let found = found?;
        if found.is_none() {
            return Ok(None);
        }
        Ok(Some(found))
    }
    fn reject_nonbound_equalities(
        &mut self,
        path: &GenericPath,
        module: ModuleId,
    ) -> Result<(), String> {
        if !path
            .segments
            .iter()
            .any(|segment| !segment.associated_types.is_empty())
        {
            return Ok(());
        }
        let mut boundary = None;
        for segment in &path.segments {
            for ty in segment.args.iter().chain(
                segment
                    .associated_types
                    .iter()
                    .map(|binding| &binding.value),
            ) {
                walk_depth(DepthNode::Type(ty), Some(&self.alias_budget))?;
                let mut ty = ty.clone();
                match self.ty(&mut ty, module) {
                    Err(error) if error.starts_with("unsupported:") => {
                        boundary.get_or_insert(error);
                    }
                    result => result?,
                }
            }
        }
        let original = path.path(self.names())?;
        if let Some(item) = self.path(&original, module, true, false)?
            && item.kind == ResolvedKind::Trait
        {
            let target = self.key(item.source()?)?;
            let mut index = None;
            for (candidate, declaration) in self.program.traits.iter().enumerate() {
                self.alias_budget.charge_work(1)?;
                if self.key(&declaration.name)? == target {
                    index = Some(candidate);
                }
            }
            let index = index.ok_or("trait is not a retained source declaration")?;
            for segment in &path.segments {
                for binding in &segment.associated_types {
                    self.check_associated_member(index, binding.name)?;
                }
            }
        }
        if let Some(error) = boundary {
            return Err(error);
        }
        Err(
            "unsupported: associated equalities are supported only in function and impl bounds"
                .into(),
        )
    }
    fn generic_path(&mut self, path: &mut GenericPath, module: ModuleId) -> Result<(), String> {
        self.reject_nonbound_equalities(path, module)?;
        if path.anchor == PathAnchor::SelfType {
            if !self.self_allowed {
                return Err("Self outside impl".into());
            }
            for segment in &mut path.segments {
                for arg in segment.args.iter_mut().chain(
                    segment
                        .associated_types
                        .iter_mut()
                        .map(|binding| &mut binding.value),
                ) {
                    self.ty(arg, module)?;
                }
            }
            return Ok(());
        }
        let at = path
            .segments
            .iter()
            .position(|segment| !segment.region_args.is_empty() || !segment.args.is_empty())
            .ok_or("missing explicit type arguments")?;
        let original_use = path
            .path(self.names())?
            .original_use(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        clone_vector(&path.segments[..=at], &self.alias_budget)?;
        let prefix = GenericPath {
            anchor: path.anchor,
            segments: path.segments[..=at].to_vec(),
            resolved_use: None,
        }
        .path(self.names())?;
        // An already-canonical path (resolved once before, e.g. a prelude
        // item reached from inside the library) is anchored at the user root.
        let canonical_crate = path.resolved_use.is_some() && path.anchor == PathAnchor::Crate;
        let anchored = self.user_crate_anchor || canonical_crate;
        let previous = std::mem::replace(&mut self.user_crate_anchor, anchored);
        let found = match self.path(&prefix, module, true, false) {
            Ok(Some(item)) => Ok(Some(item)),
            Ok(None) => self.path(&prefix, module, false, false),
            Err(error) => Err(error),
        };
        self.user_crate_anchor = previous;
        let item = found?.ok_or_else(|| {
            format!(
                "unresolved generic declaration {}",
                self.display(&prefix)
                    .map(|d| d.to_string())
                    .unwrap_or_default()
            )
        })?;
        if item.kind == ResolvedKind::Module {
            return Err("generic arguments are not allowed on modules".into());
        }
        let mut args = std::mem::take(&mut path.segments[at].args);
        let region_args = std::mem::take(&mut path.segments[at].region_args);
        for arg in &mut args {
            self.ty(arg, module)?;
        }
        clone_vector(&path.segments[at + 1..], &self.alias_budget)?;
        let suffix = path.segments[at + 1..].to_vec();
        let mut canonical = self.alias_source_path(item.source()?)?;
        canonical
            .segments
            .last_mut()
            .ok_or("generic declaration has no final segment")?
            .region_args = region_args;
        canonical
            .segments
            .last_mut()
            .ok_or("generic declaration has no final segment")?
            .args = args;
        canonical
            .segments
            .try_reserve(suffix.len())
            .map_err(|_| "generic suffix allocation failed")?;
        canonical.segments.extend(suffix);
        self.certify_generic(&mut canonical, &original_use)?;
        *path = canonical;
        Ok(())
    }
    fn associated_path(
        &mut self,
        path: &GenericPath,
        module: ModuleId,
    ) -> Result<Option<(Type, AstIdent, Vec<Type>)>, String> {
        self.reject_nonbound_equalities(path, module)?;
        if path.segments.is_empty()
            || (path.segments.len() < 2 && path.anchor != PathAnchor::SelfType)
        {
            return Ok(None);
        }
        clone_vector(&path.segments, &self.alias_budget)?;
        let mut prefix = GenericPath {
            anchor: path.anchor,
            segments: path.segments[..path.segments.len() - 1].to_vec(),
            resolved_use: None,
        };
        self.certify_generic(&mut prefix, &path.path(self.names())?)?;
        let item = path.segments.last().ok_or("missing associated item")?;
        let owner_path = prefix.path(self.names())?;
        let mut owner = if let Some(ident) = owner_path.single()
            && self.param_contains(ident)?
        {
            Type::Param(ident)
        } else if prefix.anchor == PathAnchor::SelfType && prefix.segments.is_empty() {
            if !self.self_allowed {
                return Err("Self outside impl".into());
            }
            if let Some(nominal) = &self.self_nominal {
                let candidate = self.semantic_append(nominal, item.name)?;
                if self.path_items(&candidate)?.is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| matches!(item.kind, ItemKind::Variant(_)))
                }) {
                    return Ok(None);
                }
            }
            Type::SelfType
        } else {
            let Some(resolved) = self.path(&owner_path, module, true, false)? else {
                return self.primitive_associated_path(&prefix, item, module);
            };
            if resolved.kind == ResolvedKind::TypeAlias {
                let owner = if prefix
                    .segments
                    .iter()
                    .any(|segment| !segment.args.is_empty())
                {
                    Type::Apply(prefix)
                } else {
                    Type::Named(owner_path)
                };
                return self.resolved_associated_owner(owner, item, module);
            }
            if resolved.kind == ResolvedKind::Module {
                // Rust retries primitive type syntax only when the source
                // module has no value at the complete call path.
                if self
                    .path(&path.path(self.names())?, module, false, false)?
                    .is_none()
                {
                    return self.primitive_associated_path(&prefix, item, module);
                }
                return Ok(None);
            }
            if !matches!(resolved.kind, ResolvedKind::Struct | ResolvedKind::Enum) {
                return Ok(None);
            }
            if resolved.kind == ResolvedKind::Enum {
                let candidate = self.semantic_append(resolved.source()?, item.name)?;
                if self
                    .item(&candidate, module, false, false)?
                    .is_some_and(|item| item.kind == ResolvedKind::Variant)
                {
                    return Ok(None);
                }
            }
            if prefix
                .segments
                .iter()
                .any(|segment| !segment.args.is_empty())
            {
                self.generic_path(&mut prefix, module)?;
                Type::Apply(prefix)
            } else {
                let named = self.resolved_use(resolved.source()?, &owner_path)?;
                let arity = self
                    .nominal_arity
                    .get(&self.key(&named)?)
                    .copied()
                    .unwrap_or(0);
                if arity > 0 {
                    // `Vec::new()`: impl arguments are inferred by the checker.
                    let mut generic = self.alias_source_path(&named)?;
                    generic.segments.last_mut().ok_or("empty owner path")?.args =
                        vec![Type::Infer; arity];
                    return Ok(Some((Type::Apply(generic), item.name, item.args.clone())))
                        .and_then(|found| {
                            let mut found = found;
                            if let Some((_, _, args)) = &mut found {
                                for arg in args {
                                    self.ty(arg, module)?;
                                }
                            }
                            Ok(found)
                        });
                }
                Type::Named(named)
            }
        };
        self.ty(&mut owner, module)?;
        clone_vector(&item.args, &self.alias_budget)?;
        let mut args = item.args.clone();
        for arg in &mut args {
            self.ty(arg, module)?;
        }
        Ok(Some((owner, item.name, args)))
    }
    fn ty(&mut self, ty: &mut Type, module: ModuleId) -> Result<(), String> {
        if self.type_depth >= 128 {
            return Err("type resolution nesting exceeds resource limit 128".into());
        }
        self.type_depth += 1;
        // Required signatures retain their source projections as nonexecuting
        // schemas. Resolution still checks their real source owner and every
        // child, without issuing normalization or executable-shape evidence.
        let result = self.ty_inner(ty, module);
        self.type_depth -= 1;
        result
    }
    fn ty_inner(&mut self, ty: &mut Type, module: ModuleId) -> Result<(), String> {
        match ty {
            Type::Opaque(opaque) => {
                for bound in &mut opaque.bounds {
                    // Opaque result bounds are retained declaration bounds, not
                    // ordinary trait paths. Preserve and validate their exact
                    // associated bindings, including Fn's Output shorthand.
                    self.trait_application(bound, module, true)?;
                }
            }
            Type::Projection(projection) => self.projection(projection, module)?,
            Type::Apply(path) => {
                self.generic_path(path, module)?;
                let name = path.path(self.names())?;
                let key = self.key(&name)?;
                let (_, args) = path.application()?;
                if self.items.get(&key).is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| matches!(item.kind, ItemKind::TypeAlias(_)))
                }) {
                    *ty = self.expand_alias(&name, args, &name)?;
                } else if let Some(expected) = self.nominal_arity.get(&key) {
                    if *expected != args.len() {
                        return Err(format!(
                            "E0107: {} expects {expected} type arguments, got {}",
                            self.display(&name)?,
                            args.len()
                        ));
                    }
                } else if path.anchor != PathAnchor::SelfType {
                    return Err(format!("{} is not a nominal type", self.display(&name)?));
                }
            }
            Type::Param(name) => {
                if !self.param_contains(*name)? {
                    return Err(format!(
                        "unknown type parameter {}",
                        self.ident_display(*name)?
                    ));
                }
            }
            Type::Ref { referent, .. } => self.ty(referent, module)?,
            Type::Dyn(trait_path) => self.trait_application(trait_path, module, true)?,
            Type::Array(element, _)
            | Type::Slice(element)
            | Type::RawPtr {
                pointee: element, ..
            } => self.ty(element, module)?,
            Type::Named(name) => {
                let mut found = self.path(name, module, true, false)?;
                // Canonical `crate::__justrust_std::..` paths re-resolved from
                // inside the prelude library are anchored at the user root.
                if found.is_none() && name.anchor() == PathAnchor::Crate && !self.user_crate_anchor
                {
                    let previous = std::mem::replace(&mut self.user_crate_anchor, true);
                    let retry = self.path(name, module, true, false);
                    self.user_crate_anchor = previous;
                    found = retry?;
                }
                let item = found.ok_or_else(|| {
                    let shown = self
                        .display(name)
                        .map(|d| d.to_string())
                        .unwrap_or_default();
                    format!("unresolved type {shown}")
                })?;
                if item.kind == ResolvedKind::TypeAlias {
                    *ty = self.expand_alias(item.source()?, &[], name)?;
                    return Ok(());
                }
                if item.kind == ResolvedKind::Trait {
                    return Err(format!(
                        "unsupported: trait object type `{}` without `dyn` (accepted before edition 2021) is not implemented; write `dyn {}`",
                        self.display(name)?,
                        self.display(name)?
                    ));
                }
                if !matches!(item.kind, ResolvedKind::Struct | ResolvedKind::Enum) {
                    return Err(format!(
                        "{} is a module, not a nominal type",
                        self.display(name)?
                    ));
                }
                *name = self.resolved_use(item.source()?, name)?;
                if self
                    .nominal_arity
                    .get(&self.key(name)?)
                    .is_some_and(|arity| *arity != 0)
                {
                    return Err(format!(
                        "E0107: missing type arguments for {}",
                        self.display(name)?
                    ));
                }
            }
            Type::Tuple(fields) => {
                for ty in fields {
                    self.ty(ty, module)?;
                }
            }
            Type::SelfType if !self.self_allowed => return Err("Self outside impl".into()),
            _ => {}
        }
        Ok(())
    }
    fn variant_shape(&self, path: &AstPath) -> Result<Option<ir::VariantShape>, String> {
        Ok(self.path_items(path)?.and_then(|items| {
            items.iter().find_map(|item| {
                if let ItemKind::Variant(shape) = item.kind {
                    Some(shape)
                } else {
                    None
                }
            })
        }))
    }
    fn unsupported(message: &str) -> String {
        super::super::source::SourceError {
            kind: super::super::source::SourceErrorKind::Unsupported,
            primary: None,
            message: message.into(),
            _diagnostic_lease: None,
            location: None,
        }
        .to_string()
    }
    fn is_unit_struct(&self, item: &ResolvedItem) -> Result<bool, String> {
        if item.kind != ResolvedKind::Struct {
            return Ok(false);
        }
        let key = self.key(item.source()?)?;
        for declaration in &self.program.structs {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? == key {
                if declaration.shape != StructShape::Unit {
                    return Ok(false);
                }
                if !declaration.fields.is_empty() {
                    return Err("unit struct source schema has fields".into());
                }
                if !declaration.type_params.is_empty() {
                    return Err(Self::unsupported("generic unit structs are unsupported"));
                }
                return Ok(true);
            }
        }
        Err("resolved struct has no checked source schema".into())
    }
    fn is_static_mut(&self, item: &ResolvedItem) -> Result<bool, String> {
        let ResolvedPath::Source(path) = &item.path else {
            return Ok(false);
        };
        if self.static_muts.is_empty() {
            return Ok(false);
        }
        Ok(self.static_muts.contains(&self.key(path)?))
    }
    /// `S = v`, `S OP= v`, `S.f = v` on a `static mut S` become assignments
    /// through the accessor's raw pointer (`*S() ... = v`).
    fn static_mut_assignment(
        &mut self,
        stmt: &mut Stmt,
        module: ModuleId,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<(), String> {
        let (local, mut fields, op) = match stmt {
            Stmt::Assign { name, .. } => (*name, Vec::new(), None),
            Stmt::AssignOp { place, op, .. } => (place.local, place.fields.clone(), Some(*op)),
            Stmt::AssignPlace { place, .. } => (place.local, place.fields.clone(), None),
            _ => return Ok(()),
        };
        let LocalName::Named(ident) = local else {
            return Ok(());
        };
        let path = self.single_path(ident)?;
        if self.local_contains(&path, locals)? {
            return Ok(());
        }
        let Some(item) = self.path(&path, module, false, false)? else {
            return Ok(());
        };
        if item.kind != ResolvedKind::Function || !self.is_static_mut(&item)? {
            return Ok(());
        }
        let value = match stmt {
            Stmt::Assign { value, .. }
            | Stmt::AssignOp { value, .. }
            | Stmt::AssignPlace { value, .. } => std::mem::replace(value, Expr::Tuple(Vec::new())),
            _ => unreachable!(),
        };
        fields.insert(0, FieldSelector::Deref);
        *stmt = Stmt::AssignDeref {
            site: None,
            carrier: Expr::Call {
                name: path,
                args: Vec::new(),
            },
            fields,
            index: None,
            op,
            value,
        };
        Ok(())
    }
    fn source_struct(&self, path: &AstPath) -> Result<&StructDecl, String> {
        let key = self.key(path)?;
        for declaration in &self.program.structs {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? == key {
                return Ok(declaration);
            }
        }
        Err("resolved struct has no checked source schema".into())
    }
    fn tuple_struct(&self, path: &AstPath) -> Result<&StructDecl, String> {
        let declaration = self.source_struct(path)?;
        if declaration.shape != StructShape::Tuple {
            return Err("tuple constructor requires a tuple struct source schema".into());
        }
        if declaration.fields.is_empty() {
            return Err(Self::unsupported(
                "zero-field tuple structs are unsupported",
            ));
        }
        // Generic actuals belong to the surrounding checked GenericPath. Shape
        // admission must not erase that application or reject its declaration;
        // nominal planning validates parameter arity, bounds and substitutions.
        if declaration.fields.len() > 4096 {
            return Err("nominal field resource limit exceeded".into());
        }
        for (index, field) in declaration.fields.iter().enumerate() {
            self.alias_budget.charge_work(1)?;
            if !matches!(field.name, FieldName::Tuple(actual) if actual == index) {
                return Err("tuple struct fields must have contiguous numeric identities".into());
            }
        }
        Ok(declaration)
    }
    fn zero_constructor(
        &mut self,
        path: &AstPath,
        module: ModuleId,
    ) -> Result<Option<ResolvedItem>, String> {
        if let Some(item) = self.path(path, module, false, false)? {
            if item.kind == ResolvedKind::Const {
                return Err("unsupported: constant patterns require source-authenticated graph pattern values".into());
            }
            if item.kind == ResolvedKind::Variant {
                return Ok(Some(item));
            }
            if item.kind == ResolvedKind::Struct {
                self.tuple_struct(item.source()?)?;
                return Err("E0530: bindings cannot shadow tuple struct constructors".into());
            }
        }
        if let Some(item) = self.path(path, module, true, false)?
            && self.is_unit_struct(&item)?
        {
            return Ok(Some(item));
        }
        Ok(None)
    }
    fn constructor_pattern(
        &self,
        item: &ResolvedItem,
        source: &AstPath,
        bare_binding: bool,
    ) -> Result<Pattern, String> {
        let name = self.resolved_use(item.source()?, source)?;
        if self.is_unit_struct(item)? {
            if bare_binding && self.lexical_function.is_some() {
                return Err(Self::unsupported(
                    "bare unit constructor patterns in lexical-scope functions require deferred binder classification",
                ));
            }
            Ok(Pattern::Struct {
                name,
                fields: Vec::new(),
                rest: false,
            })
        } else {
            Ok(Pattern::Path(name))
        }
    }
    fn local_insert(
        &self,
        scope: &mut std::collections::HashSet<LocalKey>,
        name: AstIdent,
    ) -> Result<bool, String> {
        self.charge_entries::<LocalKey>(1)?;
        scope
            .try_reserve(1)
            .map_err(|_| "local scope allocation failed")?;
        let key = if let Some(function) = self.lexical_function {
            let arena = self
                .program
                .scopes
                .as_ref()
                .ok_or("macro function has no lexical scope arena")?;
            let binding = arena
                .declaration_for(self.names(), name)
                .map_err(scope_diagnostic)?;
            if arena.binding_function(binding).map_err(scope_diagnostic)? != function {
                return Err("binding declaration belongs to another function".into());
            }
            LocalKey::Anchored(binding)
        } else {
            LocalKey::Named(self.ident_key(name, LookupClass::Local)?)
        };
        Ok(scope.insert(key))
    }
    fn local_path(&self, path: &AstPath) -> Result<Option<LocalKey>, String> {
        path.check(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        if path.anchor() == PathAnchor::SelfModule && path.is_empty() {
            if let Some(function) = self.lexical_function {
                let arena = self
                    .program
                    .scopes
                    .as_ref()
                    .ok_or("receiver has no lexical scope arena")?;
                let occurrence = path.receiver_occurrence().ok_or_else(|| {
                    scope_diagnostic(super::super::semantic_scope::ScopeError::UnrecordedOccurrence)
                })?;
                arena
                    .check_receiver_reference(self.names(), function, occurrence.ident())
                    .map_err(scope_diagnostic)?;
            }
            return Ok(Some(LocalKey::Receiver));
        }
        if let Some(function) = self.lexical_function
            && let Some(ident) = path.single()
        {
            let arena = self
                .program
                .scopes
                .as_ref()
                .ok_or("macro function has no lexical scope arena")?;
            if arena
                .reference_function(self.names(), ident)
                .map_err(scope_diagnostic)?
                != function
            {
                return Err("binding reference belongs to another function".into());
            }
            let binding = arena
                .reference_for(self.names(), ident)
                .map_err(scope_diagnostic)?;
            if let Some(binding) = binding {
                if arena.binding_function(binding).map_err(scope_diagnostic)? != function {
                    return Err("resolved local belongs to another function".into());
                }
                return Ok(Some(LocalKey::Anchored(binding)));
            }
            // Recorded unresolved is a genuine item candidate. Missing or
            // foreign associations are errors above, never a name-key retry.
            return Ok(None);
        }
        path.single()
            .map(|ident| {
                self.ident_key(ident, LookupClass::Local)
                    .map(LocalKey::Named)
            })
            .transpose()
    }
    fn local_contains(
        &self,
        path: &AstPath,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<bool, String> {
        let Some(key) = self.local_path(path)? else {
            return Ok(false);
        };
        for scope in locals.iter().rev() {
            self.alias_budget.charge_work(1)?;
            if scope.contains(&key) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn tuple_variant(&self, path: &AstPath) -> Result<&VariantDecl, String> {
        let key = self.key(path)?;
        let owner = path
            .semantic_parent(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        let owner_key = self.key(&owner)?;
        for declaration in &self.program.enums {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? != owner_key {
                continue;
            }
            for variant in &declaration.variants {
                self.alias_budget.charge_work(1)?;
                let candidate = self.semantic_append(&declaration.name, variant.name)?;
                if self.key(&candidate)? != key {
                    continue;
                }
                if variant.shape != ir::VariantShape::Tuple {
                    return Err("positional rest requires an enum tuple variant".into());
                }
                if variant.fields.len() > 4096 {
                    return Err("nominal field resource limit exceeded".into());
                }
                for (index, field) in variant.fields.iter().enumerate() {
                    self.alias_budget.charge_work(1)?;
                    if !matches!(field.name, FieldName::Tuple(actual) if actual == index) {
                        return Err(
                            "enum tuple variant fields must have contiguous numeric identities"
                                .into(),
                        );
                    }
                }
                return Ok(variant);
            }
            return Err("resolved variant has no checked source variant schema".into());
        }
        Err("resolved variant has no checked source enum schema".into())
    }
    fn positional_constructor_pattern(
        &mut self,
        name: &mut AstPath,
        fields: &mut PositionalRest,
        module: ModuleId,
        names: &mut std::collections::HashSet<LocalKey>,
    ) -> Result<(), String> {
        // The value lookup enforces the complete constructor visibility,
        // including restrictions on fields omitted by positional rest.
        let item = self
            .path(name, module, false, false)?
            .ok_or("unresolved positional rest constructor")?;
        let arity = match item.kind {
            ResolvedKind::Variant => self.tuple_variant(item.source()?)?.fields.len(),
            ResolvedKind::Struct => self.tuple_struct(item.source()?)?.fields.len(),
            _ => return Err("positional rest requires a tuple constructor".into()),
        };
        let selected = fields
            .prefix
            .len()
            .checked_add(fields.suffix.len())
            .ok_or("positional rest pattern field count overflow")?;
        self.alias_budget.charge_work(selected)?;
        // Validate physical prefix/suffix positions without manufacturing
        // omitted wildcard nodes or changing source-order child identities.
        for (index, _) in fields.project(arity)?.iter() {
            if index >= arity {
                return Err("positional rest field index out of bounds".into());
            }
        }
        *name = self.resolved_use(item.source()?, name)?;
        for field in fields.prefix.iter_mut().chain(fields.suffix.iter_mut()) {
            self.pattern(field, module, names)?;
        }
        Ok(())
    }
    fn pattern(
        &mut self,
        pattern: &mut Pattern,
        module: ModuleId,
        names: &mut std::collections::HashSet<LocalKey>,
    ) -> Result<(), String> {
        let source_path = match pattern {
            Pattern::Apply { path, .. } => {
                self.reject_nonbound_equalities(path, module)?;
                clone_vector(&path.segments, &self.alias_budget)?;
                Some(path.clone())
            }
            Pattern::Struct { name, .. }
            | Pattern::Variant { name, .. }
            | Pattern::VariantRest { name, .. }
            | Pattern::Path(name) => Some(self.alias_source_path(name)?),
            _ => None,
        };
        let named_constructor = matches!(pattern, Pattern::Struct { .. })
            || matches!(pattern, Pattern::Apply { pattern, .. } if matches!(pattern.as_ref(), Pattern::Struct { .. }));
        if let Some(source_path) = source_path
            && let Some(expanded) =
                self.alias_constructor_path(&source_path, module, named_constructor)?
        {
            let canonical = expanded.path(self.names())?;
            let shape = self.variant_shape(&canonical)?;
            let inner = if let Pattern::Apply { pattern, .. } = pattern {
                pattern.as_mut()
            } else {
                &mut *pattern
            };
            match inner {
                Pattern::Struct { fields, rest, .. } if shape.is_some() => {
                    let fields = std::mem::take(fields)
                        .into_iter()
                        .map(|(name, pattern)| (FieldSelector::Named(name), pattern))
                        .collect();
                    *inner = Pattern::Variant {
                        name: canonical,
                        shape: shape.ok_or("missing variant shape")?,
                        fields,
                        rest: *rest,
                    };
                }
                Pattern::VariantRest { name, fields } => {
                    *name = canonical;
                    return self.positional_constructor_pattern(name, fields, module, names);
                }
                Pattern::Struct { name, .. }
                | Pattern::Variant { name, .. }
                | Pattern::Path(name) => *name = canonical,
                _ => return Err("unsupported alias constructor pattern".into()),
            }
            match inner {
                Pattern::Struct { fields, .. } => {
                    for (_, field) in fields {
                        self.pattern(field, module, names)?;
                    }
                }
                Pattern::Variant { fields, .. } => {
                    for (_, field) in fields {
                        self.pattern(field, module, names)?;
                    }
                }
                _ => {}
            }
            if expanded
                .segments
                .iter()
                .any(|segment| !segment.args.is_empty())
            {
                if let Pattern::Apply { path, .. } = pattern {
                    *path = expanded;
                } else {
                    let inner = std::mem::replace(pattern, Pattern::Wildcard);
                    *pattern = Pattern::Apply {
                        path: expanded,
                        pattern: Box::new(inner),
                    };
                }
            } else if let Pattern::Apply { pattern: inner, .. } = pattern {
                *pattern = *std::mem::replace(inner, Box::new(Pattern::Wildcard));
            }
            return Ok(());
        }
        match pattern {
            Pattern::Apply { path, pattern } => {
                self.generic_path(path, module)?;
                self.pattern(pattern, module, names)?;
            }
            Pattern::Ref { pattern, .. } => self.pattern(pattern, module, names)?,
            Pattern::At { name, pattern, .. } => {
                if !self.local_insert(names, *name)? {
                    return Err(format!(
                        "E0416: identifier {} is bound more than once in the same pattern",
                        self.ident_display(*name)?
                    ));
                }
                self.pattern(pattern, module, names)?;
            }
            Pattern::Variant { name, fields, .. } if name.anchor() == PathAnchor::SelfType => {
                if !self.self_allowed {
                    return Err("Self outside impl".into());
                }
                for (_, field) in fields {
                    self.pattern(field, module, names)?;
                }
            }
            Pattern::Variant {
                name,
                shape,
                fields,
                rest,
            } => {
                let item = self
                    .path(name, module, false, false)?
                    .ok_or("unresolved enum variant")?;
                if item.kind == ResolvedKind::Struct {
                    let declaration = self.tuple_struct(item.source()?)?;
                    if *shape != ir::VariantShape::Tuple
                        || *rest
                        || fields.len() != declaration.fields.len()
                    {
                        return Err("tuple struct pattern arity or shape mismatch".into());
                    }
                    for (index, (selector, _)) in fields.iter().enumerate() {
                        self.alias_budget.charge_work(1)?;
                        if !matches!(selector, FieldSelector::Tuple(actual) if *actual == index) {
                            return Err("tuple struct pattern fields must be contiguous".into());
                        }
                    }
                } else if item.kind != ResolvedKind::Variant {
                    return Err("resolved declaration is not a variant".into());
                }
                *name = self.resolved_use(item.source()?, name)?;
                for (_, pattern) in fields {
                    self.pattern(pattern, module, names)?;
                }
            }
            Pattern::Wildcard
            | Pattern::Bool(_)
            | Pattern::Int { .. }
            | Pattern::Char(_)
            | Pattern::Range { .. } => {}
            Pattern::Path(name) if name.anchor() == PathAnchor::SelfType && self.self_allowed => {}
            Pattern::Path(name) => {
                if let Some(literal) = self.const_pattern(name, module)? {
                    *pattern = literal;
                    return Ok(());
                }
                let item = self
                    .zero_constructor(name, module)?
                    .ok_or("unresolved zero-field constructor pattern")?;
                *pattern = self.constructor_pattern(&item, name, false)?;
            }
            Pattern::Binding {
                name,
                mutable,
                by_ref,
            } => {
                let path = self.single_path(*name)?;
                if !*mutable
                    && by_ref.is_none()
                    && let Some(literal) = self.const_pattern(&path, module)?
                {
                    *pattern = literal;
                    return Ok(());
                }
                if let Some(item) = self.zero_constructor(&path, module)? {
                    if *mutable || by_ref.is_some() {
                        return Err(format!(
                            "binding {} cannot shadow an enum variant",
                            self.ident_display(*name)?
                        ));
                    }
                    *pattern = self.constructor_pattern(&item, &path, true)?;
                    return Ok(());
                }
                if !self.local_insert(names, *name)? {
                    return Err(format!(
                        "duplicate pattern binding {}",
                        self.ident_display(*name)?
                    ));
                }
            }
            Pattern::VariantRest { name, fields } => {
                self.positional_constructor_pattern(name, fields, module, names)?;
            }
            Pattern::TupleRest(fields) | Pattern::ArrayRest(fields) => {
                for field in fields.prefix.iter_mut().chain(fields.suffix.iter_mut()) {
                    self.alias_budget.charge_work(1)?;
                    self.pattern(field, module, names)?;
                }
            }
            Pattern::Array(fields) => {
                for field in fields {
                    self.alias_budget.charge_work(1)?;
                    self.pattern(field, module, names)?;
                }
            }
            Pattern::Alternatives(fields) => {
                // Every alternative must bind exactly the same names (E0408).
                let mut first: Option<std::collections::HashSet<_>> = None;
                for field in fields {
                    self.alias_budget.charge_work(1)?;
                    let mut row_names = std::collections::HashSet::new();
                    self.pattern(field, module, &mut row_names)?;
                    match &first {
                        None => first = Some(row_names),
                        Some(expected)
                            if self.lexical_function.is_none() && *expected != row_names =>
                        {
                            return Err(
                                "E0408: variable is not bound in all patterns of an or-pattern"
                                    .into(),
                            );
                        }
                        Some(_) => {}
                    }
                }
                for name in first.unwrap_or_default() {
                    if !names.insert(name) {
                        return Err(
                            "E0416: identifier bound more than once in the same pattern".into()
                        );
                    }
                }
            }
            Pattern::Tuple(fields) => {
                for field in fields {
                    self.alias_budget.charge_work(1)?;
                    self.pattern(field, module, names)?;
                }
            }
            Pattern::Struct { name, fields, rest } => {
                let item = self
                    .path(name, module, true, false)?
                    .ok_or("unresolved struct pattern")?;
                for (_, pattern) in fields.iter_mut() {
                    self.pattern(pattern, module, names)?;
                }
                if item.kind == ResolvedKind::Variant {
                    *pattern = Pattern::Variant {
                        name: self.resolved_use(item.source()?, name)?,
                        shape: ir::VariantShape::Struct,
                        fields: std::mem::take(fields)
                            .into_iter()
                            .map(|(name, pattern)| (FieldSelector::Named(name), pattern))
                            .collect(),
                        rest: *rest,
                    };
                } else if item.kind == ResolvedKind::Struct {
                    *name = self.resolved_use(item.source()?, name)?;
                } else {
                    return Err("resolved declaration is not a struct pattern".into());
                }
            }
        }
        Ok(())
    }
    fn binding_constructor(
        &mut self,
        name: BindingName,
        module: ModuleId,
    ) -> Result<Option<ResolvedItem>, String> {
        let BindingName::Named(name) = name else {
            return Ok(None);
        };
        let path = self.single_path(name)?;
        let mut resolved = self.zero_constructor(&path, module)?;
        if let Some(item) = &mut resolved {
            item.path = ResolvedPath::Source(self.resolved_use(item.source()?, &path)?);
        }
        Ok(resolved)
    }
    fn block(
        &mut self,
        block: &mut Block,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        self.charge_entries::<std::collections::HashSet<LocalKey>>(1)?;
        locals
            .try_reserve(1)
            .map_err(|_| "scope stack allocation failed")?;
        locals.push(std::collections::HashSet::new());
        for stmt in &mut block.statements {
            if let Stmt::Let {
                name,
                mutable,
                ty,
                value,
            } = stmt
                && let Some(item) = self.binding_constructor(*name, module)?
            {
                if *mutable {
                    return Err("binding cannot shadow an enum variant".into());
                }
                walk_depth(DepthNode::Type(ty), Some(&self.alias_budget))?;
                *stmt = Stmt::LetPattern {
                    pattern: self.constructor_pattern(&item, item.source()?, true)?,
                    ty: ty.clone(),
                    value: value.clone(),
                };
            }
            self.static_mut_assignment(stmt, module, locals)?;
            match stmt {
                Stmt::Assign { name, .. } if self.anonymous_closure_body => {
                    let mut place = Place {
                        local: *name,
                        fields: Vec::new(),
                    };
                    self.mutable_capture_place(&mut place, locals)?;
                    if let Stmt::Assign { value, .. } = stmt {
                        self.expr(value, module, locals)?;
                        if !place.fields.is_empty() {
                            *stmt = Stmt::AssignPlace {
                                site: None,
                                place,
                                value: std::mem::replace(value, Expr::Tuple(Vec::new())),
                            };
                        }
                    }
                }
                Stmt::AssignOp { place, .. }
                | Stmt::AssignPlace { place, .. }
                | Stmt::AssignIndex { array: place, .. }
                | Stmt::AssignIndexOp { array: place, .. }
                    if self.anonymous_closure_body =>
                {
                    self.mutable_capture_place(place, locals)?;
                    match stmt {
                        Stmt::AssignIndex { index, value, .. }
                        | Stmt::AssignIndexOp { index, value, .. } => {
                            self.expr(value, module, locals)?;
                            self.expr(index, module, locals)?;
                        }
                        Stmt::AssignOp { value, .. } | Stmt::AssignPlace { value, .. } => {
                            self.expr(value, module, locals)?
                        }
                        _ => unreachable!(),
                    }
                }
                Stmt::Declare { name, ty, .. } => {
                    self.ty(ty, module)?;
                    if self.binding_constructor(*name, module)?.is_some() {
                        return Err(
                            "enum variant patterns in deferred declarations are unsupported".into(),
                        );
                    }
                    if let BindingName::Named(name) = name {
                        self.local_insert(locals.last_mut().ok_or("missing local scope")?, *name)?;
                    }
                }
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    self.ty(ty, module)?;
                    self.expr(value, module, locals)?;
                    if let BindingName::Named(name) = name {
                        self.local_insert(locals.last_mut().ok_or("missing local scope")?, *name)?;
                    }
                }
                Stmt::LetElse { .. } => self.let_else(stmt, module, locals)?,
                Stmt::LetPattern { pattern, ty, value } => {
                    self.ty(ty, module)?;
                    self.expr(value, module, locals)?;
                    let mut names = std::collections::HashSet::new();
                    self.pattern(pattern, module, &mut names)?;
                    self.charge_entries::<LocalKey>(names.len())?;
                    let scope = locals.last_mut().ok_or("missing local scope")?;
                    scope
                        .try_reserve(names.len())
                        .map_err(|_| "pattern scope allocation failed")?;
                    scope.extend(names);
                }
                Stmt::AssignNestedIndex { target, value, .. } => {
                    self.expr(value, module, locals)?;
                    self.expr(target, module, locals)?;
                }
                Stmt::AssignDeref {
                    carrier,
                    index,
                    value,
                    ..
                } => {
                    self.expr(value, module, locals)?;
                    self.expr(carrier, module, locals)?;
                    if let Some(index) = index {
                        self.expr(index, module, locals)?;
                    }
                }
                Stmt::AssignIndex { index, value, .. }
                | Stmt::AssignIndexOp { index, value, .. } => {
                    self.expr(value, module, locals)?;
                    self.expr(index, module, locals)?;
                }
                Stmt::Assign { value, .. }
                | Stmt::AssignOp { value, .. }
                | Stmt::AssignPlace { value, .. }
                | Stmt::Expr(value)
                | Stmt::ImplicitExpr(value)
                | Stmt::Return(Some(value)) => self.expr(value, module, locals)?,
                Stmt::WhileLet { .. } => self.while_let(stmt, module, locals)?,
                Stmt::For { .. } => self.for_loop(stmt, module, locals)?,
                Stmt::While {
                    condition, body, ..
                } => {
                    self.expr(condition, module, locals)?;
                    self.block(body, module, locals)?;
                }
                Stmt::Return(None) => {}
            }
        }
        if let Some(tail) = &mut block.tail {
            self.expr(tail, module, locals)?;
        }
        locals.pop();
        Ok(())
    }
    #[inline(never)]
    fn let_else(
        &mut self,
        statement: &mut Stmt,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        let Stmt::LetElse {
            pattern,
            ty,
            value,
            else_block,
        } = statement
        else {
            return Err("let-else bypassed resolver statement dispatcher".into());
        };
        self.ty(ty, module)?;
        let previous_depth = locals.len();
        let result = self.expr(value, module, locals);
        locals.truncate(previous_depth);
        result?;
        // Resolve actual pattern occurrences and certificates without making
        // the pending success bindings visible to the failure branch.
        let mut names = std::collections::HashSet::new();
        self.pattern(pattern, module, &mut names)?;
        let result = self.block(else_block, module, locals);
        locals.truncate(previous_depth);
        result?;
        // These are enclosing-scope declarations, not temporary match-arm
        // bindings. Divergence and dynamic initialization remain checker/IR
        // responsibilities, never inferred from a resolver scope insertion.
        self.charge_entries::<LocalKey>(names.len())?;
        let scope = locals
            .last_mut()
            .ok_or("missing let-else enclosing scope")?;
        scope
            .try_reserve(names.len())
            .map_err(|_| "let-else pattern scope allocation failed")?;
        scope.extend(names);
        Ok(())
    }

    #[inline(never)]
    fn while_let(
        &mut self,
        statement: &mut Stmt,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        let Stmt::WhileLet {
            pattern,
            scrutinee,
            body,
            ..
        } = statement
        else {
            return Err("while-let bypassed resolver statement dispatcher".into());
        };
        // The source binding occurrence is fixed, but it belongs only to the
        // successful iteration's body, never the next scrutinee or exit scope.
        let previous_depth = locals.len();
        let result = self.expr(scrutinee, module, locals);
        locals.truncate(previous_depth);
        result?;
        let mut names = std::collections::HashSet::new();
        self.pattern(pattern, module, &mut names)?;
        self.charge_entries::<std::collections::HashSet<LocalKey>>(1)?;
        locals
            .try_reserve(1)
            .map_err(|_| "while-let scope stack allocation failed")?;
        locals.push(names);
        let result = self.block(body, module, locals);
        // Nested failures can retain more than one scope, so restore depth
        // rather than popping only the most recently entered block.
        locals.truncate(previous_depth);
        result
    }

    #[inline(never)]
    fn for_loop(
        &mut self,
        statement: &mut Stmt,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        let Stmt::For {
            pattern,
            source,
            body,
            ..
        } = statement
        else {
            return Err("for loop bypassed resolver statement dispatcher".into());
        };
        let previous_depth = locals.len();
        let result = match &mut source.iterable {
            ForIterable::Range { start, end, .. } => self
                .expr(start, module, locals)
                .and_then(|()| self.expr(end, module, locals)),
            ForIterable::Value(value) => self.expr(value, module, locals),
        };
        locals.truncate(previous_depth);
        result?;
        let mut names = std::collections::HashSet::new();
        self.pattern(pattern, module, &mut names)?;
        self.charge_entries::<std::collections::HashSet<LocalKey>>(1)?;
        locals
            .try_reserve(1)
            .map_err(|_| "for scope stack allocation failed")?;
        locals.push(names);
        let result = self.block(body, module, locals);
        locals.truncate(previous_depth);
        result
    }

    fn expr(
        &mut self,
        expr: &mut Expr,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        // Keep recursive cast chains out of the constructor dispatch frame.
        // Resolve the operand before its target, preserving source validation order.
        if let Expr::TypeAlignment(target) = expr {
            return self.ty(target, module);
        }
        if let Expr::Cast { value, target } = expr {
            self.expr(value, module, locals)?;
            return self.ty(target, module);
        }
        // Break operands also recurse without retaining constructor scratch.
        if let Expr::Break {
            value: Some(value), ..
        } = expr
        {
            return self.expr(value, module, locals);
        }
        self.ordinary_expr(expr, module, locals)
    }

    #[inline(never)]
    fn if_let(
        &mut self,
        expression: &mut Expr,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        let Expr::IfLet {
            pattern,
            scrutinee,
            then_block,
            else_branch,
        } = expression
        else {
            return Err("if-let bypassed resolver expression dispatcher".into());
        };
        // The scrutinee sees only the preceding lexical environment.
        let previous_depth = locals.len();
        let result = self.expr(scrutinee, module, locals);
        locals.truncate(previous_depth);
        result?;
        let mut names = std::collections::HashSet::new();
        self.pattern(pattern, module, &mut names)?;
        self.charge_entries::<std::collections::HashSet<LocalKey>>(1)?;
        locals
            .try_reserve(1)
            .map_err(|_| "if-let scope stack allocation failed")?;
        locals.push(names);
        let result = self.block(then_block, module, locals);
        // A failed nested block may itself leave scopes on the stack. Restore
        // the entire preceding environment, not merely its topmost scope.
        locals.truncate(previous_depth);
        result?;
        let result = if let Some(branch) = else_branch {
            self.expr(branch, module, locals)
        } else {
            Ok(())
        };
        locals.truncate(previous_depth);
        result
    }

    #[inline(never)]
    fn ordinary_expr(
        &mut self,
        expr: &mut Expr,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        if let Expr::Var(name) = expr
            && self.stored_closure_value(name, module, locals)?
        {
            return Ok(());
        }
        // Anonymous closure paths can only be minted by the parser's private
        // declaration issuer. They do not exist in module/import lookup.
        if let Expr::Call { name, args } = expr
            && closures::anonymous_path(name, self.names())?
        {
            for arg in args.iter_mut() {
                self.expr(arg, module, locals)?;
            }
            self.closure_arguments(name, args, locals)?;
            return Ok(());
        }
        // Do not accidentally resolve a capture to a same-spelled module
        // constructor. Only explicit paths may name nonlocal values until
        // environment capture resolution is implemented.
        if self.anonymous_closure_body
            && let Expr::Var(path) = expr
            && path.anchor() == PathAnchor::Relative
            && path.single().is_some()
            && !self.local_contains(path, locals)?
        {
            *expr = self.capture_read(path)?;
            return Ok(());
        }
        if let Expr::Invoke { receiver, args } = expr {
            let receiver = std::mem::replace(
                receiver,
                Box::new(Expr::Block(Block {
                    statements: Vec::new(),
                    tail: None,
                })),
            );
            let args = std::mem::take(args);
            *expr = Expr::MethodCall {
                receiver,
                name: self.callable_call_ident()?,
                type_args: Vec::new(),
                args,
            };
            return self.expr(expr, module, locals);
        }
        if self.resolve_lexical_call(expr, module, locals)? {
            return Ok(());
        }
        if let Expr::Var(path) = expr
            && self.local_contains(path, locals)?
        {
            return Ok(());
        }
        let source_path = match expr {
            Expr::Apply { path, .. } => {
                self.reject_nonbound_equalities(path, module)?;
                clone_vector(&path.segments, &self.alias_budget)?;
                Some(path.clone())
            }
            Expr::Struct { name, .. } | Expr::Call { name, .. } | Expr::Var(name) => {
                Some(self.alias_source_path(name)?)
            }
            _ => None,
        };
        let named_constructor = matches!(expr, Expr::Struct { .. })
            || matches!(expr, Expr::Apply { value, .. } if matches!(value.as_ref(), Expr::Struct { .. }));
        if let Some(source_path) = source_path
            && let Some(expanded) =
                self.alias_constructor_path(&source_path, module, named_constructor)?
        {
            let canonical = expanded.path(self.names())?;
            let variant = self.variant_shape(&canonical)?;
            let value = if let Expr::Apply { value, .. } = expr {
                value.as_mut()
            } else {
                &mut *expr
            };
            if let Some(shape) = variant {
                let fields = match value {
                    Expr::Var(_) if shape == ir::VariantShape::Unit => Vec::new(),
                    Expr::Call { args, .. } if shape == ir::VariantShape::Tuple => {
                        std::mem::take(args)
                            .into_iter()
                            .enumerate()
                            .map(|(index, value)| (FieldSelector::Tuple(index), value))
                            .collect()
                    }
                    Expr::Struct { fields, .. } if shape == ir::VariantShape::Struct => {
                        std::mem::take(fields)
                            .into_iter()
                            .map(|(name, value)| (FieldSelector::Named(name), value))
                            .collect()
                    }
                    _ => return Err("enum alias variant constructor shape mismatch".into()),
                };
                *value = Expr::EnumVariant {
                    name: canonical,
                    shape,
                    fields,
                };
                if let Expr::EnumVariant { fields, .. } = value {
                    for (_, field) in fields {
                        self.expr(field, module, locals)?;
                    }
                }
            } else if self.path_items(&canonical)?.is_some_and(|items| {
                items
                    .iter()
                    .any(|item| matches!(item.kind, ItemKind::Struct))
            }) {
                let Expr::Struct { name, fields } = value else {
                    return Err("type alias is not a value constructor".into());
                };
                *name = canonical;
                for (_, field) in fields {
                    self.expr(field, module, locals)?;
                }
            } else {
                let Expr::Call { args, .. } = value else {
                    return Err("unresolved alias associated item".into());
                };
                let original_use = expanded.path(self.names())?;
                let mut owner_path = expanded;
                let method = owner_path.segments.pop().ok_or("missing associated item")?;
                self.certify_generic(&mut owner_path, &original_use)?;
                let owner = if owner_path
                    .segments
                    .iter()
                    .any(|segment| !segment.args.is_empty())
                {
                    Type::Apply(owner_path)
                } else {
                    Type::Named(owner_path.path(self.names())?)
                };
                let mut type_args = method.args;
                for ty in &mut type_args {
                    self.ty(ty, module)?;
                }
                for arg in args.iter_mut() {
                    self.expr(arg, module, locals)?;
                }
                *expr = Expr::AssociatedCall {
                    owner,
                    name: method.name,
                    type_args,
                    args: std::mem::take(args),
                };
                return Ok(());
            }
            if expanded
                .segments
                .iter()
                .any(|segment| !segment.args.is_empty())
            {
                if let Expr::Apply { path, .. } = expr {
                    *path = expanded;
                } else {
                    let value = std::mem::replace(expr, Expr::Tuple(Vec::new()));
                    *expr = Expr::Apply {
                        path: expanded,
                        value: Box::new(value),
                    };
                }
            } else if let Expr::Apply { value, .. } = expr {
                *expr = *std::mem::replace(value, Box::new(Expr::Tuple(Vec::new())));
            }
            return Ok(());
        }
        match expr {
            Expr::Apply { path, value } => {
                if matches!(value.as_ref(), Expr::Call { .. })
                    && let Some((owner, name, type_args)) = self.associated_path(path, module)?
                {
                    let Expr::Call { args, .. } = value.as_mut() else {
                        return Err("associated call lost source call".into());
                    };
                    *expr = Expr::AssociatedCall {
                        owner,
                        name,
                        type_args,
                        args: std::mem::take(args),
                    };
                    return self.expr(expr, module, locals);
                }
                if matches!(value.as_ref(), Expr::Var(_))
                    && let Some((owner, name, actuals)) = self.associated_path(path, module)?
                {
                    if !actuals.is_empty() {
                        return Err(
                            "associated constants cannot take method generic arguments".into()
                        );
                    }
                    *expr = Expr::AssociatedConst(Box::new(AssociatedConstUse {
                        owner,
                        trait_path: None,
                        name,
                    }));
                    return self.expr(expr, module, locals);
                }
                self.generic_path(path, module)?;
                self.expr(value, module, locals)?;
            }
            Expr::MethodCall {
                receiver,
                type_args,
                args,
                ..
            } => {
                self.expr(receiver, module, locals)?;
                for ty in type_args {
                    self.ty(ty, module)?;
                }
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::AssociatedCall {
                owner,
                type_args,
                args,
                ..
            } => {
                self.ty(owner, module)?;
                for ty in type_args {
                    self.ty(ty, module)?;
                }
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::TypeAlignment(_) => {
                return Err("alignment query bypassed resolver expression dispatcher".into());
            }
            Expr::NativeSyscall(elements)
            | Expr::RawSlice {
                arguments: elements,
                ..
            }
            | Expr::Array(elements)
            | Expr::Tuple(elements)
            | Expr::Format {
                arguments: elements,
                ..
            } => {
                for value in elements {
                    self.expr(value, module, locals)?;
                }
            }
            Expr::FormatInto(source) => {
                let source = &mut **source;
                for value in source
                    .arguments
                    .iter_mut()
                    .chain(source.rest.iter_mut().flatten())
                {
                    self.expr(value, module, locals)?;
                }
            }
            Expr::Cast { .. } => {
                return Err("cast bypassed resolver expression dispatcher".into());
            }
            Expr::SliceDataPointer(value)
            | Expr::Deref { value }
            | Expr::BorrowValue { value, .. }
            | Expr::Repeat { value, .. }
            | Expr::Index { base: value, .. }
            | Expr::Unary { value, .. } => self.expr(value, module, locals)?,
            Expr::Borrow { kind, place } => {
                if self.anonymous_closure_body
                    && let LocalName::Named(ident) = place.local
                {
                    let path = self.single_path(ident)?;
                    if !self.local_contains(&path, locals)? {
                        if !place.fields.is_empty() {
                            return Err(
                                "unsupported: projected closure captures are not implemented"
                                    .into(),
                            );
                        }
                        *place = self.capture_place_kind(&path, *kind)?;
                        return Ok(());
                    }
                }
                // A bare non-local root such as `&Marker` or an imported unit
                // variant names a value constructor, not a place.
                if let LocalName::Named(ident) = place.local
                    && place.fields.is_empty()
                {
                    let path = AstPath::new(self.names(), PathAnchor::Relative, &[ident])
                        .map_err(native_adapter::name_diagnostic)?;
                    let constructor = !self.local_contains(&path, locals)?
                        && (self
                            .path(&path, module, false, false)?
                            .is_some_and(|item| item.kind == ResolvedKind::Variant)
                            || match self.path(&path, module, true, false)? {
                                Some(item) => self.is_unit_struct(&item)?,
                                None => false,
                            });
                    if constructor {
                        *expr = Expr::BorrowValue {
                            kind: *kind,
                            value: Box::new(Expr::Var(path)),
                        };
                        return self.expr(expr, module, locals);
                    }
                }
            }
            Expr::DynamicIndex { base, index, .. } => {
                self.expr(base, module, locals)?;
                self.expr(index, module, locals)?;
            }
            Expr::RangeIndex {
                base, start, end, ..
            } => {
                self.expr(base, module, locals)?;
                for bound in [start, end].into_iter().flatten() {
                    self.expr(bound, module, locals)?;
                }
            }
            Expr::AssociatedConst(value) | Expr::AssociatedFn(value) => {
                self.ty(&mut value.owner, module)?;
                if let Some(path) = &mut value.trait_path {
                    self.trait_path(path, module)?;
                }
            }
            Expr::Var(name) => {
                if !self.local_contains(name, locals)? {
                    let path = self.alias_source_path(name)?;
                    if let Some((owner, name, actuals)) = self.associated_path(&path, module)? {
                        // The value namespace has one meaning per member name:
                        // an associated function is a function item value,
                        // never an associated constant lookup.
                        if self.names_associated_function(name)? {
                            if !actuals.is_empty() {
                                return Err(
                                    "unsupported: explicit generic arguments on function item values"
                                        .into(),
                                );
                            }
                            *expr = Expr::AssociatedFn(Box::new(AssociatedConstUse {
                                owner,
                                trait_path: None,
                                name,
                            }));
                            return Ok(());
                        }
                        if !actuals.is_empty() {
                            return Err(
                                "associated constants cannot take method generic arguments".into(),
                            );
                        }
                        *expr = Expr::AssociatedConst(Box::new(AssociatedConstUse {
                            owner,
                            trait_path: None,
                            name,
                        }));
                        return Ok(());
                    }
                }
                let value_item = self.path(name, module, false, false)?;
                if let Some(item) = &value_item
                    && item.kind == ResolvedKind::Const
                {
                    let target = item.source()?.clone();
                    let resolved = self.resolved_use(&target, name)?;
                    if self.inline_constant(expr, &target, locals)? {
                        return Ok(());
                    }
                    *expr = Expr::ConstItem(resolved);
                    return Ok(());
                }
                if let Some(item) = &value_item
                    && item.kind == ResolvedKind::Function
                    && self.is_static_mut(item)?
                {
                    // `static mut` use: `*NAME()` (raw-pointer place, E0133).
                    let mut call = Expr::Call {
                        name: name.clone(),
                        args: Vec::new(),
                    };
                    self.expr(&mut call, module, locals)?;
                    *expr = Expr::Deref {
                        value: Box::new(call),
                    };
                    return Ok(());
                }
                if let Some(item) = &value_item
                    && item.kind == ResolvedKind::Function
                    && !self.local_contains(name, locals)?
                    && !closures::anonymous_path(name, self.names())?
                {
                    // Function item value: canonical declaration path, typed
                    // later as the zero-sized `Ty::FnItem` of that declaration.
                    *name = self.resolved_use(item.source()?, name)?;
                    return Ok(());
                }
                if value_item
                    .as_ref()
                    .is_some_and(|item| item.kind == ResolvedKind::Struct)
                {
                    return Err(Self::unsupported(
                        "first-class tuple struct constructors are unsupported",
                    ));
                }
                if let Some(item) = value_item
                    && item.kind == ResolvedKind::Variant
                {
                    *expr = Expr::EnumVariant {
                        name: self.resolved_use(item.source()?, name)?,
                        shape: ir::VariantShape::Unit,
                        fields: Vec::new(),
                    };
                } else if let Some(item) = self.path(name, module, true, false)? {
                    if item.kind == ResolvedKind::TypeAlias {
                        // Diagnose the invalid alias constructor before the
                        // later declaration-wide unsupported-unit fence.
                        return Err("type alias is not a value constructor".into());
                    }
                    if self.is_unit_struct(&item)? {
                        *expr = Expr::Struct {
                            name: self.resolved_use(item.source()?, name)?,
                            fields: Vec::new(),
                        };
                    }
                }
            }
            Expr::Match { scrutinee, arms } => {
                self.expr(scrutinee, module, locals)?;
                for arm in arms {
                    let mut names = std::collections::HashSet::new();
                    self.pattern(&mut arm.pattern, module, &mut names)?;
                    locals.push(names);
                    if let Some(guard) = &mut arm.guard {
                        self.expr(guard, module, locals)?;
                    }
                    self.expr(&mut arm.body, module, locals)?;
                    locals.pop();
                }
            }
            Expr::Call { name, args } => {
                let path = self.alias_source_path(name)?;
                if let Some((owner, item, type_args)) = self.associated_path(&path, module)? {
                    *expr = Expr::AssociatedCall {
                        owner,
                        name: item,
                        type_args,
                        args: std::mem::take(args),
                    };
                    return self.expr(expr, module, locals);
                }
                if let Some(rewritten) = self.trait_path_call(name, args, module)? {
                    *expr = rewritten;
                    return self.expr(expr, module, locals);
                }
                if name.anchor() == PathAnchor::SelfType && self.self_allowed {
                    for arg in args.iter_mut() {
                        self.expr(arg, module, locals)?;
                    }
                    *expr = Expr::EnumVariant {
                        name: name.clone(),
                        shape: ir::VariantShape::Tuple,
                        fields: std::mem::take(args)
                            .into_iter()
                            .enumerate()
                            .map(|(index, value)| (FieldSelector::Tuple(index), value))
                            .collect(),
                    };
                    return Ok(());
                }
                let resolved = self.path(name, module, false, false)?.ok_or_else(|| {
                    let shown = self
                        .display(name)
                        .map(|d| d.to_string())
                        .unwrap_or_default();
                    format!("unresolved function {shown}")
                })?;
                for arg in args.iter_mut() {
                    self.expr(arg, module, locals)?;
                }
                if resolved.kind == ResolvedKind::Variant {
                    *expr = Expr::EnumVariant {
                        name: self.resolved_use(resolved.source()?, name)?,
                        shape: ir::VariantShape::Tuple,
                        fields: std::mem::take(args)
                            .into_iter()
                            .enumerate()
                            .map(|(index, value)| (FieldSelector::Tuple(index), value))
                            .collect(),
                    };
                } else if resolved.kind == ResolvedKind::Function {
                    *name = self.resolved_use(resolved.source()?, name)?;
                } else if resolved.kind == ResolvedKind::Struct {
                    let expected = self.tuple_struct(resolved.source()?)?.fields.len();
                    if args.len() != expected {
                        return Err("tuple struct constructor arity mismatch".into());
                    }
                    *expr = Expr::TupleStruct {
                        name: self.resolved_use(resolved.source()?, name)?,
                        fields: std::mem::take(args),
                    };
                } else {
                    return Err("resolved declaration is not callable".into());
                }
            }
            Expr::TupleStruct { name, fields } => {
                let item = self
                    .path(name, module, false, false)?
                    .ok_or("unresolved tuple struct constructor")?;
                if item.kind != ResolvedKind::Struct
                    || fields.len() != self.tuple_struct(item.source()?)?.fields.len()
                {
                    return Err("tuple struct constructor arity or shape mismatch".into());
                }
                for field in fields {
                    self.expr(field, module, locals)?;
                }
                *name = self.resolved_use(item.source()?, name)?;
            }
            Expr::Struct { name, fields }
                if name.anchor() == PathAnchor::SelfType && name.is_empty() =>
            {
                if !self.self_allowed {
                    return Err("Self outside impl".into());
                }
                for (_, value) in fields {
                    self.expr(value, module, locals)?;
                }
            }
            Expr::Struct { name, fields } => {
                let item = self
                    .path(name, module, true, false)?
                    .ok_or("unresolved struct")?;
                for (_, field) in fields.iter_mut() {
                    self.expr(field, module, locals)?;
                }
                if item.kind == ResolvedKind::Variant {
                    *expr = Expr::EnumVariant {
                        name: self.resolved_use(item.source()?, name)?,
                        shape: ir::VariantShape::Struct,
                        fields: std::mem::take(fields)
                            .into_iter()
                            .map(|(name, value)| (FieldSelector::Named(name), value))
                            .collect(),
                    };
                } else if item.kind == ResolvedKind::Struct {
                    *name = self.resolved_use(item.source()?, name)?;
                } else {
                    return Err("resolved declaration is not a struct".into());
                }
            }
            Expr::TraitCall {
                owner,
                trait_path,
                type_args,
                args,
                ..
            } => {
                if !matches!(owner, Type::Infer) {
                    self.ty(owner, module)?;
                }
                self.trait_path(trait_path, module)?;
                for actual in type_args {
                    self.ty(actual, module)?;
                }
                for arg in args {
                    self.expr(arg, module, locals)?;
                }
            }
            Expr::Field { base, .. } => self.expr(base, module, locals)?,
            Expr::Binary { left, right, .. } => {
                self.expr(left, module, locals)?;
                self.expr(right, module, locals)?;
            }
            Expr::IfLet { .. } => self.if_let(expr, module, locals)?,
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expr(condition, module, locals)?;
                self.block(then_block, module, locals)?;
                self.block(else_block, module, locals)?;
            }
            Expr::Block(block)
            | Expr::Unsafe(block)
            | Expr::Loop { body: block, .. }
            | Expr::LabeledBlock { body: block, .. } => self.block(block, module, locals)?,
            Expr::Try(value) => self.expr(value, module, locals)?,
            _ => {}
        }
        Ok(())
    }
    fn function_locals(
        &mut self,
        function: &Function,
        receiver: ReceiverKind,
    ) -> Result<std::collections::HashSet<LocalKey>, String> {
        self.lexical_function = function.lexical_scope;
        if let Some(function) = self.lexical_function {
            let arena = self
                .program
                .scopes
                .as_ref()
                .ok_or("macro function has no lexical scope arena")?;
            arena.check_names(self.names()).map_err(scope_diagnostic)?;
            arena.check_function(function).map_err(scope_diagnostic)?;
            if arena.has_receiver(function).map_err(scope_diagnostic)?
                != (receiver != ReceiverKind::None)
            {
                return Err("receiver binder differs from callable receiver kind".into());
            }
            if receiver != ReceiverKind::None {
                arena
                    .receiver_binding(self.names(), function)
                    .map_err(scope_diagnostic)?;
            }
        }
        let mut locals = std::collections::HashSet::new();
        self.charge_entries::<LocalKey>(
            function.params.len() + usize::from(receiver != ReceiverKind::None),
        )?;
        locals
            .try_reserve(function.params.len() + 1)
            .map_err(|_| "parameter scope allocation failed")?;
        for (name, _) in &function.params {
            if let BindingName::Named(name) = name {
                self.local_insert(&mut locals, *name)?;
            }
        }
        if receiver != ReceiverKind::None {
            locals.insert(LocalKey::Receiver);
        }
        Ok(locals)
    }
    fn source_order(&self, path: &AstPath) -> Result<(u32, u32, u32), String> {
        let last = *path
            .segments()
            .last()
            .ok_or("declaration path has no source name")?;
        let span = self
            .names()
            .span(last)
            .map_err(native_adapter::name_diagnostic)?;
        let original = self
            .names()
            .session()
            .original(span, self.names().budget().limits().backtrace_frames)
            .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        let raw = self
            .names()
            .session()
            .sources()
            .render_span(original)
            .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        Ok((raw.file.0, raw.lo, raw.hi))
    }
    fn ordered_paths(&self) -> Result<Vec<AstPath>, String> {
        self.charge_entries::<((u32, u32, u32), AstPath)>(self.items.len())?;
        let mut paths = Vec::new();
        paths
            .try_reserve_exact(self.items.len())
            .map_err(|_| "resolver ordering allocation failed")?;
        for items in self.items.values() {
            if let Some(item) = items.first() {
                paths.push((self.source_order(&item.path)?, item.path.clone()));
            }
        }
        let levels = usize::BITS as usize - paths.len().max(1).leading_zeros() as usize;
        self.alias_budget.charge_work(
            paths
                .len()
                .checked_mul(levels + 1)
                .ok_or("resolver ordering work overflow")?,
        )?;
        paths.sort_unstable_by_key(|(order, _)| *order);
        self.charge_entries::<AstPath>(paths.len())?;
        let mut ordered = Vec::new();
        ordered
            .try_reserve_exact(paths.len())
            .map_err(|_| "resolver ordered paths allocation failed")?;
        for (_, path) in paths {
            ordered.push(path);
        }
        Ok(ordered)
    }
    fn is_entry_path(&self, path: &AstPath) -> Result<bool, String> {
        path.check(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        if path.anchor() != PathAnchor::Crate || path.len() != 1 {
            return Ok(false);
        }
        self.atom_is(path.segments()[0], BuiltinAtom::EntryMain, Namespace::Value)
    }
}

/// Resolve one immutable parsed program using checked semantic identities.
/// Source paths remain structural. Catalog traits are explicit typed targets.
fn defer_equality_boundary(
    result: Result<(), String>,
    pending: &mut Option<String>,
) -> Result<(), String> {
    match result {
        Err(error) if error.starts_with("unsupported: associated equalities") => {
            pending.get_or_insert(error);
            Ok(())
        }
        result => result,
    }
}

#[derive(Clone, Debug)]
struct ResolutionState {
    alias_cache: HashMap<PathKey, Type>,
    nominal_arity: HashMap<PathKey, usize>,
    imports: HashMap<(PathKey, usize, bool), Option<ResolvedItem>>,
    prelude_memo: HashMap<PreludeMemoKey, Option<Option<ResolvedItem>>>,
    static_muts: std::collections::HashSet<PathKey>,
    resolved_closures: HashMap<PathKey, closures::ClosureSource>,
    resolved_closure_owners: HashMap<PathKey, Vec<PathKey>>,
}
impl ResolutionState {
    fn capture(resolver: &Resolver<'_>) -> Self {
        Self {
            alias_cache: resolver.alias_cache.clone(),
            nominal_arity: resolver.nominal_arity.clone(),
            imports: resolver.imports.clone(),
            prelude_memo: resolver.prelude_memo.clone(),
            static_muts: resolver.static_muts.clone(),
            resolved_closures: resolver.resolved_closures.clone(),
            resolved_closure_owners: resolver.resolved_closure_owners.clone(),
        }
    }
    fn resolver<'a>(&self, parsed: &'a ParsedProgram) -> Resolver<'a> {
        Resolver {
            static_muts: self.static_muts.clone(),
            globs: &parsed.glob_imports,
            active_globs: std::collections::HashSet::new(),
            program: &parsed.program,
            type_depth: 0,
            alias_cache: self.alias_cache.clone(),
            alias_active: std::collections::HashSet::new(),
            alias_budget: ir::PlanBudget::default(),
            nominal_arity: self.nominal_arity.clone(),
            self_nominal: None,
            self_allowed: false,
            source_owner: SourceOwnerContext::None,
            items: &parsed.items,
            imports: self.imports.clone(),
            prelude_memo: self.prelude_memo.clone(),
            active: std::collections::HashSet::new(),
            type_params: Vec::new(),
            lexical_function: None,
            user_crate_anchor: false,
            anonymous_closure_body: false,
            active_closure: None,
            resolved_closures: self.resolved_closures.clone(),
            resolved_closure_owners: self.resolved_closure_owners.clone(),
            current_function_name: None,
            current_callable: None,
        }
    }
}

/// Immutable namespace/header prerequisite. Imports, aliases, nominal/trait
/// headers, constants, trait-default bodies and anonymous adapters remain global
/// debt here. Implementation bodies and generated metadata remain finish debt.
/// Ordinary free-function bounds, types and bodies run in module item facts.
#[derive(Clone, Debug)]
pub struct ResolutionNamespace {
    parsed: Arc<ParsedProgram>,
    token: Arc<ResolutionToken>,
    program: Program,
    state: ResolutionState,
    modules: Vec<ModuleId>,
    functions: Vec<Vec<usize>>,
    /// Implementation blocks per module ordinal, in source order.
    impls: Vec<Vec<usize>>,
    associated_boundary: Option<String>,
    equality_boundary: Option<String>,
    associated_bound_sources: Vec<AssociatedBoundSource>,
    derived_debug_source: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct ResolvedModuleItems {
    // Opaque publication capability, not a declaration identity or graph key.
    // Keeping it alive prevents equal-shaped foreign snapshots from mixing.
    namespace_token: Arc<ResolutionToken>,
    // Source errors are values so completion order cannot select diagnostics.
    error: Option<(usize, String)>,
    /// First failing implementation block of this module. Ranked after every
    /// free-function error, then by implementation index.
    impl_error: Option<(usize, String)>,
    ordinal: usize,
    functions: Vec<(usize, Function)>,
    impls: Vec<(usize, ImplDecl)>,
    closures: HashMap<PathKey, closures::ClosureSource>,
    owners: HashMap<PathKey, Vec<PathKey>>,
}

// Only immutable AST values, canonical identities and lookup cache values are
// retained. Resolver scratch and its per-task budget never escape into facts.
impl crate::engine::Fact for ResolutionNamespace {}
impl crate::engine::Fact for ResolvedModuleItems {}

#[derive(Debug)]
struct ResolutionToken {}

pub fn resolution_modules(namespace: &ResolutionNamespace) -> &[ModuleId] {
    &namespace.modules
}
pub fn resolution_module_indices(
    namespace: &ResolutionNamespace,
    ordinal: usize,
) -> Result<&[usize], String> {
    namespace
        .functions
        .get(ordinal)
        .map(Vec::as_slice)
        .ok_or_else(|| "foreign resolution module ordinal".into())
}

pub fn resolve_module_items(
    namespace: &ResolutionNamespace,
    ordinal: usize,
) -> Result<ResolvedModuleItems, String> {
    let parsed = namespace.parsed.as_ref();
    let indices = resolution_module_indices(namespace, ordinal)?;
    let mut resolver = namespace.state.resolver(parsed);
    let mut functions = Vec::with_capacity(indices.len());
    let mut error = None;
    for &index in indices {
        let mut function = namespace.program.functions[index].clone();
        if let Err(diagnostic) = resolve_free_function(&mut resolver, index, &mut function) {
            error = Some((index, diagnostic));
            break;
        }
        functions.push((index, function));
    }
    let mut impls = Vec::new();
    let mut impl_error = None;
    if error.is_none() {
        let issuers = namespace.program.embedded_issuers.clone();
        let blocks = namespace
            .impls
            .get(ordinal)
            .ok_or("foreign resolution module ordinal")?;
        impls.reserve_exact(blocks.len());
        for &index in blocks {
            let mut block = namespace.program.impls[index].clone();
            if let Err(diagnostic) = resolve_impl_block(&mut resolver, index, &mut block, &issuers)
            {
                impl_error = Some((index, diagnostic));
                break;
            }
            impls.push((index, block));
        }
    }
    resolver
        .resolved_closures
        .retain(|key, _| !namespace.state.resolved_closures.contains_key(key));
    for (key, children) in &namespace.state.resolved_closure_owners {
        if let Some(output) = resolver.resolved_closure_owners.get_mut(key) {
            output.drain(..children.len());
        }
    }
    resolver
        .resolved_closure_owners
        .retain(|_, children| !children.is_empty());
    Ok(ResolvedModuleItems {
        namespace_token: namespace.token.clone(),
        error,
        impl_error,
        ordinal,
        functions,
        impls,
        closures: resolver.resolved_closures,
        owners: resolver.resolved_closure_owners,
    })
}

/// Resolve one implementation block in place: header, associated items and
/// bodies. Runs inside its module's RESOLVE_MODULE fact, never crate-wide.
fn resolve_impl_block(
    resolver: &mut Resolver<'_>,
    source_index: usize,
    block: &mut ImplDecl,
    primitive_issuers: &[crate::compiler::source::EmbeddedOriginalFile],
) -> Result<(), String> {
    let owner = if block.trait_path.is_some() {
        SourceOwnerContext::TraitImpl { source_index }
    } else {
        SourceOwnerContext::None
    };
    resolver.with_source_owner(owner, |resolver| {
        resolver.type_params = block.type_params.clone();
        resolver.bounds(&mut block.bounds, block.module)?;
        // `impl Trait<Self> for T` means `impl Trait<T> for T`: `Self` in
        // the header's trait arguments denotes the written self type.
        if let Some(TraitPath::Source(path)) = &mut block.trait_path {
            for segment in &mut path.segments {
                for argument in segment.args.iter_mut().chain(
                    segment
                        .associated_types
                        .iter_mut()
                        .map(|binding| &mut binding.value),
                ) {
                    replace_self_type(argument, &block.self_ty, &resolver.alias_budget, 0)?;
                }
            }
        }
        if let Some(path) = &mut block.trait_path {
            resolver.trait_path(path, block.module)?;
        }
        resolver.ty(&mut block.self_ty, block.module)?;
        if resolver.contains_projection(&block.self_ty, 0)? {
            return Err("unsupported: projection-bearing impl heads are unsupported".into());
        }
        resolver.self_nominal = match &block.self_ty {
            Type::Named(path) => Some(path.clone()),
            Type::Apply(path) => Some(path.path(resolver.names())?),
            // Trait impls may target any type (primitives, references,
            // tuples, slices); `Self` then has no nominal namespace.
            _ if block.trait_path.is_some() => None,
            _ => {
                let receipt = block.primitive_source.as_ref()
                    .ok_or("E0390: cannot define inherent impl for primitive types outside their authenticated source issuer")?;
                receipt.replay_source(block, source_index, resolver.names(), &primitive_issuers)?;
                None
            },
        };
        resolver.self_allowed = true;
        for associated in &mut block.associated_types {
            validate_depth(DepthNode::Type(&associated.value))?;
            resolver.ty(&mut associated.value, block.module)?;
        }
        for constant in &mut block.associated_constants {
            clone_vector(&block.type_params, &resolver.alias_budget)?;
            resolver.type_params = block.type_params.clone();
            resolver.ty(&mut constant.ty, block.module)?;
            if let Some(value) = &mut constant.initializer {
                resolver.expr(
                    value,
                    block.module,
                    &mut vec![std::collections::HashSet::new()],
                )?;
            }
        }
        for (item_index, item) in block.items.iter_mut().enumerate() {
            resolver.current_callable = Some(CallableSource::Associated {
                implementation: source_index,
                item: item_index,
            });
            let function = &mut item.function;
            resolver.type_params = block.type_params.clone();
            for name in &function.type_params {
                if resolver.param_contains(*name)? {
                    return Err("duplicate visible generic parameter".into());
                }
                resolver.type_params.push(*name);
            }
            resolver.bounds(&mut function.bounds, block.module)?;
            resolver.ty(&mut function.result, block.module)?;
            for (_, ty) in &mut function.params {
                resolver.ty(ty, block.module)?;
            }
            let names = resolver.function_locals(function, item.receiver)?;
            validate_depth(DepthNode::Block(&function.body))?;
            resolver.block(&mut function.body, block.module, &mut vec![names])?;
        }
        Ok(())
    })
}

fn resolve_free_function(
    resolver: &mut Resolver<'_>,
    index: usize,
    function: &mut Function,
) -> Result<(), String> {
    resolver.current_callable = Some(CallableSource::Free(index));
    let embedded = function.embedded_source.clone();
    if let Some(receipt) = &embedded {
        receipt.original()?.replay_source(
            function,
            resolver.names(),
            &resolver.program.embedded_issuers,
        )?;
    }
    // Local/free declarations are collected separately by the frontend.
    // None may inherit a preceding trait/default/impl Self owner.
    resolver.with_source_owner(SourceOwnerContext::None, |resolver| {
        resolver.anonymous_closure_body =
            closures::anonymous_path(&function.name, resolver.names())?;
        resolver.begin_closure_function(function)?;
        let embedded_resolved = if let Some(receipt) = &embedded {
            let names = resolver.program.names.clone();
            receipt.resolve_calls(&names, |path| {
                let Some(target) = resolver.path(path, function.module, false, false)? else {
                    return Ok(None);
                };
                if target.kind != ResolvedKind::Function {
                    return Ok(None);
                }
                resolver.resolved_use(target.source()?, path).map(Some)
            })?
        } else {
            None
        };
        resolver.type_params = function.type_params.clone();
        resolver.bounds(&mut function.bounds, function.module)?;
        resolver.ty(&mut function.result, function.module)?;
        for (name, ty) in &mut function.params {
            resolver.ty(ty, function.module)?;
            if resolver
                .binding_constructor(*name, function.module)?
                .is_some()
            {
                return Err("enum variant patterns in function parameters are unsupported".into());
            }
        }
        validate_depth(DepthNode::Block(&function.body))?;
        let locals = resolver.function_locals(function, ReceiverKind::None)?;
        if let Err(error) = resolver.block(&mut function.body, function.module, &mut vec![locals]) {
            return Err(format!(
                "in function {}: {error}",
                resolver.display(&function.name)?
            ));
        }
        resolver.finish_closure_function(function)?;
        function.embedded_source = None;
        if let Some(receipt) = embedded_resolved
            && receipt.matches_resolved(function, resolver.names())?
        {
            function.embedded_source = Some(receipt);
        }
        Ok(())
    })?;
    Ok(())
}

pub fn resolve(parsed: &ParsedProgram) -> Result<Program, String> {
    let namespace = prepare_resolution(parsed)?;
    let items = (0..resolution_modules(&namespace).len())
        .map(|ordinal| resolve_module_items(&namespace, ordinal))
        .collect::<Result<Vec<_>, _>>()?;
    finish_resolution(&namespace, items)
}

pub fn prepare_resolution(parsed: &ParsedProgram) -> Result<ResolutionNamespace, String> {
    prepare_resolution_owned(Arc::new(parsed.clone()))
}

/// Retain the graph's exact immutable parse fact, without cloning its Program.
/// Child work receives only this namespace, never a second caller-supplied AST.
pub fn prepare_resolution_owned(parsed: Arc<ParsedProgram>) -> Result<ResolutionNamespace, String> {
    let source = parsed;
    let parsed = source.as_ref();
    parsed.program.validate_c_source()?;
    parsed.program.validate_name_handles()?;
    parsed
        .program
        .modules
        .check_names(parsed.program.names.as_ref())
        .map_err(module_diagnostic)?;
    parsed
        .program
        .symbols
        .check(parsed.program.names.as_ref())?;
    #[expect(
        clippy::mutable_key_type,
        reason = "PathKey hashes immutable owner/anchor/namespace/name keys only; its shared resource ledger is not semantic identity"
    )]
    let mut static_muts = std::collections::HashSet::new();
    for function in parsed.program.functions.iter().filter(|f| f.static_mut) {
        static_muts.insert(name_scope::path_key(
            parsed.program.names.as_ref(),
            &function.name,
            Namespace::Type,
        )?);
    }
    let mut resolver = Resolver {
        static_muts,
        globs: &parsed.glob_imports,
        active_globs: std::collections::HashSet::new(),
        program: &parsed.program,
        type_depth: 0,
        alias_cache: HashMap::new(),
        alias_active: std::collections::HashSet::new(),
        alias_budget: ir::PlanBudget::default(),
        nominal_arity: HashMap::new(),
        self_nominal: None,
        self_allowed: false,
        source_owner: SourceOwnerContext::None,
        items: &parsed.items,
        imports: HashMap::new(),
        prelude_memo: HashMap::new(),
        active: std::collections::HashSet::new(),
        type_params: Vec::new(),
        lexical_function: None,
        user_crate_anchor: false,
        anonymous_closure_body: false,
        active_closure: None,
        resolved_closures: HashMap::new(),
        resolved_closure_owners: HashMap::new(),
        current_function_name: None,
        current_callable: None,
    };
    for (bucket, items) in &parsed.items {
        for item in items {
            if resolver.key(&item.path)? != *bucket {
                return Err("source item belongs to a different semantic path bucket".into());
            }
            item.visibility
                .validate_for(&parsed.program.modules, item.module)
                .map_err(module_diagnostic)?;
            if let ItemKind::Module(module) = item.kind {
                parsed
                    .program
                    .modules
                    .check(module)
                    .map_err(module_diagnostic)?;
            }
        }
    }
    resolver.charge_entries::<(PathKey, usize)>(
        parsed.program.structs.len() + parsed.program.enums.len(),
    )?;
    resolver
        .nominal_arity
        .try_reserve(parsed.program.structs.len() + parsed.program.enums.len())
        .map_err(|_| "nominal arity allocation failed")?;
    for (path, arity) in parsed
        .program
        .structs
        .iter()
        .map(|item| (&item.name, item.type_params.len()))
        .chain(
            parsed
                .program
                .enums
                .iter()
                .map(|item| (&item.name, item.type_params.len())),
        )
    {
        resolver.nominal_arity.insert(resolver.key(path)?, arity);
    }
    for (module, path) in &parsed.empty_use_prefixes {
        let target = resolver
            .path(path, *module, true, false)?
            .ok_or_else(|| "unresolved empty use group prefix".to_owned())?;
        if !matches!(
            target.kind,
            ResolvedKind::Module | ResolvedKind::Enum | ResolvedKind::Trait
        ) {
            return Err("resolved declaration is not a module in empty use group".into());
        }
    }
    // A glob is a declaration even when none of its members are used. Resolve
    // its owner eagerly so unused self imports cannot disappear behind lazy
    // candidate lookup. Compare canonical module paths, not written aliases.
    for glob in &parsed.glob_imports {
        let Some(target) = resolver.path(&glob.target, glob.module, true, true)? else {
            return Err(format!(
                "unresolved glob import {}",
                resolver.display(&glob.target)?
            ));
        };
        if target.kind == ResolvedKind::Module
            && let ResolvedPath::Source(source) = &target.path
            && resolver.key(source)? == resolver.key(&resolver.module_path(glob.module)?)?
        {
            return Err("cannot glob-import a module into itself".into());
        }
    }
    let paths = resolver.ordered_paths()?;
    for path in &paths {
        let key = resolver.key(path)?;
        let items = parsed.items.get(&key).ok_or("missing source item bucket")?;
        for (index, item) in items.iter().enumerate() {
            if matches!(item.kind, ItemKind::Import(_, _)) {
                let type_target = resolver.import(path, index, true)?;
                let value_target = resolver.import(path, index, false)?;
                if type_target.is_none() && value_target.is_none() {
                    return Err(format!(
                        "unresolved or cyclic import {}",
                        resolver.display(path)?
                    ));
                }
                let mut permits = false;
                for target in [type_target.as_ref(), value_target.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    permits |= target
                        .visibility
                        .allows_reexport(&parsed.program.modules, &item.visibility)
                        .map_err(module_diagnostic)?;
                }
                if !permits {
                    return Err(format!(
                        "private item or import cannot be publicly reexported as {}",
                        resolver.display(path)?
                    ));
                }
            }
        }
        let requester = items.first().ok_or("empty source item bucket")?.module;
        resolver.item(path, requester, true, false)?;
        resolver.item(path, requester, false, false)?;
    }
    resolver.resolve_aliases()?;
    let root = parsed
        .program
        .modules
        .source_root()
        .map_err(module_diagnostic)?;
    let mut declared_entry = false;
    for function in &parsed.program.functions {
        if function.module == root && resolver.is_entry_path(&function.name)? {
            declared_entry = true;
        }
    }
    if !declared_entry {
        for path in &paths {
            if !resolver.is_entry_path(path)? {
                continue;
            }
            if let Some(target) = resolver.item(path, root, false, false)?
                && target.kind == ResolvedKind::Function
            {
                let key = resolver.key(target.source()?)?;
                for function in &parsed.program.functions {
                    if resolver.key(&function.name)? == key
                        && function.type_params.is_empty()
                        && function.params.is_empty()
                        && matches!(function.result, Type::Unit)
                    {
                        return Err("imported main entry points are unsupported; declare a root main function".into());
                    }
                }
            }
        }
    }
    let mut program = parsed.program.clone();
    // Inlined constants have no CONST_VALUE fact; their uses carry the value.
    // Each is still resolved and qualified once so unused bad items reject.
    for declaration in &parsed.program.constants {
        if const_inline::needs_inline(declaration) {
            let mut probe = Expr::Var(declaration.name.clone());
            resolver.inline_constant(&mut probe, &declaration.name, &mut vec![])?;
        }
    }
    program
        .constants
        .retain(|declaration| !const_inline::needs_inline(declaration));
    // The value-namespace slot freed above holds a checked zero-argument
    // `const fn` computing the initializer, so unused constants still type check.
    for declaration in &parsed.program.constants {
        if const_inline::needs_inline(declaration) {
            program
                .functions
                .push(const_inline::checked_body(declaration));
        }
    }
    for declaration in &mut program.constants {
        resolver.type_params.clear();
        resolver.ty(&mut declaration.ty, declaration.module)?;
        resolver.expr(
            &mut declaration.initializer,
            declaration.module,
            &mut vec![std::collections::HashSet::new()],
        )?;
    }
    for path in &paths {
        let key = resolver.key(path)?;
        for (index, item) in parsed
            .items
            .get(&key)
            .ok_or("missing source import bucket")?
            .iter()
            .enumerate()
        {
            if matches!(item.kind, ItemKind::Import(_, _))
                && let Some(target) = resolver.import(path, index, true)?
                && target.kind == ResolvedKind::Trait
            {
                resolver.charge_entries::<(ModuleId, TraitPath)>(1)?;
                program
                    .trait_imports
                    .try_reserve(1)
                    .map_err(|_| "trait import allocation failed")?;
                let path = match &target.path {
                    ResolvedPath::Source(path) => {
                        TraitPath::Source(resolver.alias_source_path(path)?)
                    }
                    ResolvedPath::Builtin(id) => TraitPath::Builtin(*id),
                    ResolvedPath::CatalogModule(_) => {
                        return Err("trait import names a module".into());
                    }
                };
                program.trait_imports.push((item.module, path));
            }
        }
    }
    let mut equality_boundary = None;
    let mut associated_boundary = None;
    let mut associated_bound_sources = Vec::new();
    let mut derived_debug_source = None;
    for (source_index, declaration) in program.traits.iter_mut().enumerate() {
        if generated_debug::canonical_source(&parsed.program, declaration)?
            && derived_debug_source.replace(source_index).is_some()
        {
            return Err("duplicate canonical native Debug trait".into());
        }
        resolver.with_source_owner(SourceOwnerContext::Trait { source_index }, |resolver| {
            clone_vector(&declaration.type_params, &resolver.alias_budget)?;
            resolver.type_params = declaration.type_params.clone();
            resolver.self_nominal = None;
            resolver.self_allowed = true;
            defer_equality_boundary(
                resolver.bounds(&mut declaration.bounds, declaration.module),
                &mut equality_boundary,
            )?;
            for path in &mut declaration.supertraits {
                defer_equality_boundary(
                    resolver.trait_path(path, declaration.module),
                    &mut equality_boundary,
                )?;
            }
            for (member_index, associated) in declaration.associated_types.iter_mut().enumerate() {
                resolver.charge_entries::<(GenericPath, TraitPath)>(associated.bounds.len())?;
                let mut bounds = Vec::new();
                bounds
                    .try_reserve_exact(associated.bounds.len())
                    .map_err(|_| "associated bound source allocation failed")?;
                for path in &mut associated.bounds {
                    match resolver.associated_bound(path, declaration.module) {
                        Ok(target) => {
                            let TraitPath::Source(source) = path else {
                                unreachable!()
                            };
                            source_clone::clone_path(source, &resolver.alias_budget)?;
                            bounds.push((source.clone(), target));
                        }
                        Err(error) if error.starts_with("unsupported:") => {
                            associated_boundary.get_or_insert(error);
                        }
                        Err(error) => return Err(error),
                    }
                }
                resolver.charge_entries::<AssociatedBoundSource>(1)?;
                associated_bound_sources
                    .try_reserve_exact(1)
                    .map_err(|_| "associated source record allocation failed")?;
                associated_bound_sources.push(AssociatedBoundSource {
                    trait_index: source_index,
                    member_index,
                    module: declaration.module,
                    trait_name: declaration.name.clone(),
                    member_name: associated.name,
                    bounds: bounds.into(),
                });
            }
            for constant in &mut declaration.associated_constants {
                clone_vector(&declaration.type_params, &resolver.alias_budget)?;
                resolver.type_params = declaration.type_params.clone();
                resolver.ty(&mut constant.ty, declaration.module)?;
                if let Some(value) = &mut constant.initializer {
                    resolver.expr(
                        value,
                        declaration.module,
                        &mut vec![std::collections::HashSet::new()],
                    )?;
                }
            }
            for (item_index, item) in declaration.items.iter_mut().enumerate() {
                clone_vector(&declaration.type_params, &resolver.alias_budget)?;
                resolver.type_params = declaration.type_params.clone();
                resolver.charge_entries::<AstIdent>(item.signature.type_params.len())?;
                resolver
                    .type_params
                    .try_reserve(item.signature.type_params.len())
                    .map_err(|_| "trait method parameter allocation failed")?;
                resolver
                    .type_params
                    .extend_from_slice(&item.signature.type_params);
                defer_equality_boundary(
                    resolver.trait_method_bounds(
                        &mut item.signature.bounds,
                        declaration.module,
                        source_index,
                        item_index,
                    ),
                    &mut equality_boundary,
                )?;
                resolver.ty(&mut item.signature.result, declaration.module)?;
                for (_, ty) in &mut item.signature.params {
                    resolver.ty(ty, declaration.module)?;
                }
                if let Some(function) = &mut item.default {
                    resolver.current_callable = Some(CallableSource::TraitDefault {
                        trait_index: source_index,
                        item: item_index,
                    });
                    clone_vector(&declaration.type_params, &resolver.alias_budget)?;
                    resolver.type_params = declaration.type_params.clone();
                    resolver.charge_entries::<AstIdent>(function.type_params.len())?;
                    resolver
                        .type_params
                        .try_reserve(function.type_params.len())
                        .map_err(|_| "trait default parameter allocation failed")?;
                    resolver
                        .type_params
                        .extend_from_slice(&function.type_params);
                    defer_equality_boundary(
                        resolver.trait_method_bounds(
                            &mut function.bounds,
                            declaration.module,
                            source_index,
                            item_index,
                        ),
                        &mut equality_boundary,
                    )?;
                    resolver.ty(&mut function.result, declaration.module)?;
                    for (_, ty) in &mut function.params {
                        resolver.ty(ty, declaration.module)?;
                    }
                    let names = resolver.function_locals(function, item.receiver)?;
                    validate_depth(DepthNode::Block(&function.body))?;
                    resolver.block(&mut function.body, declaration.module, &mut vec![names])?;
                }
            }
            Ok(())
        })?;
    }
    for enumeration in &mut program.enums {
        resolver.type_params = enumeration.type_params.clone();
        resolver.bounds(&mut enumeration.bounds, enumeration.module)?;
        for variant in &mut enumeration.variants {
            for field in &mut variant.fields {
                validate_depth(DepthNode::Type(&field.ty))?;
                resolver.ty(&mut field.ty, enumeration.module)?;
            }
        }
    }
    for structure in &mut program.structs {
        resolver.type_params = structure.type_params.clone();
        resolver.bounds(&mut structure.bounds, structure.module)?;
        for field in &mut structure.fields {
            validate_depth(DepthNode::Type(&field.ty))?;
            resolver.ty(&mut field.ty, structure.module)?;
        }
    }
    let mut modules = Vec::new();
    let mut functions: Vec<Vec<usize>> = Vec::new();
    let mut impls: Vec<Vec<usize>> = Vec::new();
    // Anonymous adapters establish capture descriptors before parent bodies.
    for index in 0..program.functions.len() {
        if closures::anonymous_path(&program.functions[index].name, resolver.names())? {
            resolve_free_function(&mut resolver, index, &mut program.functions[index])?;
        } else {
            let module = program.functions[index].module;
            let ordinal =
                if let Some(ordinal) = modules.iter().position(|candidate| *candidate == module) {
                    ordinal
                } else {
                    modules.push(module);
                    functions.push(Vec::new());
                    impls.push(Vec::new());
                    modules.len() - 1
                };
            functions[ordinal].push(index);
        }
    }
    for (index, block) in program.impls.iter().enumerate() {
        let ordinal = match modules
            .iter()
            .position(|candidate| *candidate == block.module)
        {
            Some(ordinal) => ordinal,
            None => {
                modules.push(block.module);
                functions.push(Vec::new());
                impls.push(Vec::new());
                modules.len() - 1
            }
        };
        impls[ordinal].push(index);
    }
    let state = ResolutionState::capture(&resolver);
    Ok(ResolutionNamespace {
        parsed: source,
        token: Arc::new(ResolutionToken {}),
        program,
        state,
        modules,
        functions,
        impls,
        associated_boundary,
        equality_boundary,
        associated_bound_sources,
        derived_debug_source,
    })
}

pub fn finish_resolution(
    namespace: &ResolutionNamespace,
    items: Vec<ResolvedModuleItems>,
) -> Result<Program, String> {
    finish_resolution_with(namespace, items, |program| {
        program.validate_name_handles()?;
        Ok(program)
    })
}

/// Seal the exact final allocation that the graph publishes. The Arc returned
/// alongside the capability is only another immutable reference to that same
/// allocation, never a caller-provided AST paired with an unrelated token.
pub(in crate::compiler) fn finish_resolution_sealed(
    namespace: &ResolutionNamespace,
    items: Vec<ResolvedModuleItems>,
) -> Result<(Arc<Program>, ValidatedProgram), String> {
    finish_resolution_with(namespace, items, |program| {
        let program = Arc::new(program);
        let validated = ValidatedProgram::new(program.clone())?;
        Ok((program, validated))
    })
}

// Mutable construction scratch reaches either validating finalizer only after
// all canonical rewrites finish. Input validation cannot authorize this output.
fn finish_resolution_with<T>(
    namespace: &ResolutionNamespace,
    mut items: Vec<ResolvedModuleItems>,
    seal: impl FnOnce(Program) -> Result<T, String>,
) -> Result<T, String> {
    let parsed = namespace.parsed.as_ref();
    for item in &items {
        if !Arc::ptr_eq(&item.namespace_token, &namespace.token) {
            return Err("resolved module items belong to a different namespace snapshot".into());
        }
    }
    items.sort_by_key(|item| item.ordinal);
    if items.len() != namespace.modules.len() {
        return Err("incomplete resolved module inventory".into());
    }
    for (ordinal, item) in items.iter().enumerate() {
        if item.ordinal != ordinal {
            return Err("foreign resolved module inventory".into());
        }
        let expected = &namespace.functions[ordinal];
        let prefix = match &item.error {
            Some((index, _)) if expected.get(item.functions.len()) == Some(index) => {
                &expected[..item.functions.len()]
            }
            Some(_) => return Err("foreign resolved module error inventory".into()),
            None => expected.as_slice(),
        };
        if !item
            .functions
            .iter()
            .map(|(index, _)| *index)
            .eq(prefix.iter().copied())
        {
            return Err("foreign resolved module inventory".into());
        }
        let expected = &namespace.impls[ordinal];
        let prefix = match (&item.error, &item.impl_error) {
            (Some(_), None) => &expected[..0],
            (Some(_), Some(_)) => return Err("foreign resolved module error inventory".into()),
            (None, Some((index, _))) if expected.get(item.impls.len()) == Some(index) => {
                &expected[..item.impls.len()]
            }
            (None, Some(_)) => return Err("foreign resolved module error inventory".into()),
            (None, None) => expected.as_slice(),
        };
        if !item
            .impls
            .iter()
            .map(|(index, _)| *index)
            .eq(prefix.iter().copied())
        {
            return Err("foreign resolved module inventory".into());
        }
    }
    if let Some((_, diagnostic)) = items
        .iter()
        .filter_map(|item| item.error.as_ref())
        .min_by_key(|(index, _)| *index)
    {
        return Err(diagnostic.clone());
    }
    if let Some((_, diagnostic)) = items
        .iter()
        .filter_map(|item| item.impl_error.as_ref())
        .min_by_key(|(index, _)| *index)
    {
        return Err(diagnostic.clone());
    }
    let mut program = namespace.program.clone();
    let mut resolver = namespace.state.resolver(parsed);
    let associated_boundary = namespace.associated_boundary.clone();
    let equality_boundary = namespace.equality_boundary.clone();
    let associated_bound_sources = namespace.associated_bound_sources.clone();
    let derived_debug_source = namespace.derived_debug_source;
    for item in items {
        for (index, function) in item.functions {
            program.functions[index] = function;
        }
        for (index, block) in item.impls {
            program.impls[index] = block;
        }
        for (key, source) in item.closures {
            if resolver.resolved_closures.insert(key, source).is_some() {
                return Err("duplicate resolved closure source".into());
            }
        }
        for (key, children) in item.owners {
            resolver
                .resolved_closure_owners
                .entry(key)
                .or_default()
                .extend(children);
        }
    }
    let derived_debug = generated_metadata::materialize_debug(&mut program, derived_debug_source)?;
    for index in derived_debug {
        let implementation = &mut program.impls[index];
        resolver.self_nominal = match &implementation.self_ty {
            Type::Named(path) => Some(path.clone()),
            _ => None,
        };
        resolver.self_allowed = true;
        resolver.source_owner = SourceOwnerContext::None;
        resolver.type_params.clear();
        for item in &mut implementation.items {
            let locals = resolver.function_locals(&item.function, item.receiver)?;
            resolver.block(
                &mut item.function.body,
                implementation.module,
                &mut vec![locals],
            )?;
        }
    }
    // Closure syntax is final only after free, default and implementation bodies
    // have all been resolved by this producer. No child can observe a partial map.
    program.closure_sources = Arc::new(resolver.resolved_closures.clone());
    program.closure_owners = Arc::new(
        resolver
            .resolved_closure_owners
            .iter()
            .map(|(key, children)| (key.clone(), children.clone().into()))
            .collect(),
    );
    // The input program was checked against parser-issued source certificates
    // before resolution. Only this canonical rewrite can publish resolved heads.
    program.generated_owners = generated_owners::capture(&program)?;
    program.associated_bound_sources = associated_bound_sources.into();
    // Finalize before borrowing the immutable snapshot, but defer this error:
    // existing name-contract and source-boundary diagnostics take precedence.
    let c_source_seal = program.seal_resolved_c_source();
    let sealed = seal(program)?;
    if let Some(error) = associated_boundary {
        return Err(error);
    }
    if let Some(error) = equality_boundary {
        return Err(error);
    }
    c_source_seal?;
    Ok(sealed)
}

#[cfg(test)]
fn assert_noncallable_local_shadow(source: &str) {
    let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
    // The raw resolver intentionally lacks the prepared Fn prelude. It must
    // select the local binding rather than capture the same-named outer item.
    assert!(
        error.contains("calling a local value requires the prelude Fn trait"),
        "{error}"
    );
    let stock = super::parser_tests::stock_compile(source, std::path::Path::new("unused"), true);
    assert!(!stock.status.success());
    assert!(String::from_utf8_lossy(&stock.stderr).contains("E0618"));
    for workers in [1, 8] {
        for check in [true, false] {
            assert!(
                crate::compiler::compile(source.into(), workers, check).is_err(),
                "noncallable shadow accepted at workers={workers}, check={check}"
            );
        }
    }
}

#[cfg(test)]
mod item_resolution_tests {
    use super::*;

    #[test]
    fn module_items_resolve_bodies_without_mutating_namespace() {
        let parsed = parse_unresolved("mod a{pub type Number=i32;pub fn run()->Number{let f=|x:i32|x+1;3}}mod b{pub fn run()->i32{crate::a::run()}}fn main(){b::run();}").unwrap();
        let namespace = prepare_resolution(&parsed).unwrap();
        let before = format!("{:?}", namespace.program);
        assert_eq!(resolution_modules(&namespace).len(), 3);
        let mut items = (0..resolution_modules(&namespace).len())
            .map(|ordinal| resolve_module_items(&namespace, ordinal).unwrap())
            .collect::<Vec<_>>();
        assert!(items.iter().all(|item| item.error.is_none()));
        assert!(items.iter().any(|item| !item.closures.is_empty()));
        items.reverse();
        let program = finish_resolution(&namespace, items).unwrap();
        assert!(!program.closure_sources.is_empty());
        assert_eq!(before, format!("{:?}", namespace.program));
        assert_eq!(namespace.program.functions.len(), program.functions.len());
    }

    #[test]
    fn module_diagnostics_follow_function_order_not_module_completion_order() {
        let parsed = parse_unresolved("mod a{pub fn good(){}}mod b{pub fn first_bad(){missing_b();}}mod a_tail{pub fn later_bad(){missing_a();}}fn main(){}").unwrap();
        let namespace = prepare_resolution(&parsed).unwrap();
        let mut items = (0..resolution_modules(&namespace).len())
            .map(|ordinal| resolve_module_items(&namespace, ordinal).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(items.iter().filter(|item| item.error.is_some()).count(), 2);
        let expected = items
            .iter()
            .filter_map(|item| item.error.as_ref())
            .min_by_key(|(index, _)| *index)
            .unwrap()
            .1
            .clone();
        assert!(expected.contains("first_bad"), "{expected}");
        items.reverse();
        assert_eq!(finish_resolution(&namespace, items).unwrap_err(), expected);
    }

    #[test]
    fn module_inventory_rejects_missing_duplicate_and_foreign_outputs() {
        let parsed = parse_unresolved("mod a{pub fn f(){}}fn main(){}").unwrap();
        let namespace = prepare_resolution(&parsed).unwrap();
        assert!(resolve_module_items(&namespace, usize::MAX).is_err());
        assert!(finish_resolution(&namespace, Vec::new()).is_err());
        let item = resolve_module_items(&namespace, 0).unwrap();
        assert!(finish_resolution(&namespace, vec![item.clone(), item]).is_err());
        let complete = (0..namespace.modules.len())
            .map(|ordinal| resolve_module_items(&namespace, ordinal).unwrap())
            .collect::<Vec<_>>();
        let mut forged = complete.clone();
        forged[0].error = Some((usize::MAX, "forged source diagnostic".into()));
        assert!(
            finish_resolution(&namespace, forged)
                .unwrap_err()
                .contains("inventory")
        );
        let mut missing = complete.clone();
        missing[0].functions.clear();
        missing[0].error = Some((namespace.functions[0][0], "forged source diagnostic".into()));
        missing.pop();
        assert!(
            finish_resolution(&namespace, missing)
                .unwrap_err()
                .contains("inventory")
        );
        let mut duplicate = complete;
        duplicate[0].functions.clear();
        duplicate[0].error = Some((namespace.functions[0][0], "forged source diagnostic".into()));
        duplicate[1] = duplicate[0].clone();
        assert!(
            finish_resolution(&namespace, duplicate)
                .unwrap_err()
                .contains("inventory")
        );
    }

    #[test]
    fn namespace_prerequisite_still_rejects_unused_alias_cycles() {
        let parsed = parse_unresolved("type A=B;type B=A;fn main(){}").unwrap();
        assert!(prepare_resolution(&parsed).unwrap_err().contains("E0391"));
    }

    #[test]
    fn equal_shaped_foreign_parsed_and_module_facts_are_rejected() {
        let source = "mod a{pub fn f()->bool{false}}fn main(){}";
        let parsed = Arc::new(parse_unresolved(source).unwrap());
        let namespace = prepare_resolution_owned(parsed.clone()).unwrap();
        assert!(Arc::ptr_eq(&namespace.parsed, &parsed));
        // A caller cannot mutate the snapshot retained by the namespace.
        assert!(Arc::get_mut(&mut parsed.clone()).is_none());
        let mut changed = parsed.as_ref().clone();
        changed.program.functions[0].body.tail = Some(Box::new(Expr::Bool(true)));
        assert_eq!(
            changed.program.names.owner_path(),
            parsed.program.names.owner_path()
        );
        let independent = parse_unresolved(source).unwrap();
        for foreign in [independent, changed] {
            let foreign_namespace = prepare_resolution_owned(Arc::new(foreign)).unwrap();
            assert_eq!(namespace.functions, foreign_namespace.functions);
            let mut items = (0..namespace.modules.len())
                .map(|ordinal| resolve_module_items(&namespace, ordinal).unwrap())
                .collect::<Vec<_>>();
            items[0] = resolve_module_items(&foreign_namespace, 0).unwrap();
            assert!(
                finish_resolution(&namespace, items)
                    .unwrap_err()
                    .contains("different namespace snapshot")
            );
            // Foreign diagnostics must not mask the immutable snapshot check.
            let mut item = resolve_module_items(&foreign_namespace, 0).unwrap();
            item.error = Some((0, "foreign diagnostic".into()));
            assert!(
                finish_resolution(&namespace, vec![item])
                    .unwrap_err()
                    .contains("different namespace snapshot")
            );
        }
        assert!(matches!(
            namespace.parsed.program.functions[0].body.tail.as_deref(),
            Some(Expr::Bool(false))
        ));
    }
}

#[cfg(test)]
mod associated_projection_tests {
    use super::*;

    #[test]
    fn nested_closed_projection_stock_sources_keep_real_boxed_children() {
        for source in [
            "trait Has{type Out;} struct S; struct U; impl Has for S{type Out=U;} impl Has for U{type Out=i32;} fn take(x:<<S as Has>::Out as Has>::Out)->i32{x} fn main(){if take(42)==42{println!(\"ok\");}}",
            "trait Has{type Out;} struct A; struct B; struct C; impl Has for A{type Out=B;} impl Has for B{type Out=C;} impl Has for C{type Out=i32;} fn pass(x:<<<A as Has>::Out as Has>::Out as Has>::Out)->i32{x} fn main(){if pass(42)==42{println!(\"ok\");}}",
        ] {
            let program = parse(source).unwrap();
            assert_eq!(program.traits[0].associated_types.len(), 1);
            assert!(program.traits[0].items.is_empty());
            let Type::Projection(outer) = &program.functions[0].params[0].1 else {
                panic!("qualified source projection was erased");
            };
            let SourceProjection::Qualified {
                self_ty,
                trait_path,
                ..
            } = outer.as_ref()
            else {
                panic!("qualified projection became owning Self syntax");
            };
            assert!(matches!(self_ty, Type::Projection(_)));
            assert!(matches!(trait_path, TraitPath::Source(_)));
        }
    }

    #[test]
    fn generic_impl_rhs_and_owning_self_keep_exact_source_shape() {
        let program = parse("trait Has<T>{type Out;type Other;}struct S;impl<U> Has<U> for S{type Out=U;type Other=Self::Out;}fn main(){}").unwrap();
        let implementation = &program.impls[0];
        assert!(matches!(
            implementation.associated_types[0].value,
            Type::Param(_)
        ));
        assert!(matches!(&implementation.associated_types[1].value,
            Type::Projection(p) if matches!(p.as_ref(), SourceProjection::SelfItem { .. })));
    }

    #[test]
    fn trait_method_equalities_retain_exact_callable_sources() {
        let program = parse("trait Call<A>{type Out;}trait Apply{fn required<F:Call<i32,Out=i32>>(&self,f:F)->i32;fn provided<F:Call<i32,Out=i32>>(&self,f:F)->i32{3}}fn main(){}").unwrap();
        let owner = &program.traits[1];
        assert_eq!(owner.items.len(), 2);
        for item in &owner.items {
            assert_eq!(item.signature.bounds.len(), 1);
            let TraitPath::Source(path) = &item.signature.bounds[0].traits[0] else {
                panic!("method bound lost its canonical source path");
            };
            assert_eq!(path.segments.last().unwrap().associated_types.len(), 1);
            if let Some(default) = &item.default {
                assert_eq!(default.bounds.len(), 1);
            }
        }
        for source in [
            "trait Call<A>{type Out;}trait Apply<F:Call<i32,Out=i32>>{}fn main(){}",
            "trait Call<A>{type Out;}trait Apply:Call<i32,Out=i32>{}fn main(){}",
        ] {
            let error = parse(source).expect_err("trait header equality was admitted");
            assert!(error.contains("associated equalities"), "{error}");
        }
    }

    #[test]
    fn trait_method_equalities_reject_wrong_source_owner_item_and_module() {
        let program = parse(
            "trait Apply{fn call(&self);}mod foreign{pub trait Other{fn call(&self);}}fn main(){}",
        )
        .unwrap();
        #[expect(
            clippy::mutable_key_type,
            reason = "PathKey equality and hashing use immutable path identity, never the retained allocation budget"
        )]
        let items = HashMap::new();
        let mut resolver = Resolver {
            program: &program,
            type_depth: 0,
            alias_cache: HashMap::new(),
            alias_active: Default::default(),
            alias_budget: ir::PlanBudget::default(),
            nominal_arity: HashMap::new(),
            self_nominal: None,
            self_allowed: false,
            source_owner: SourceOwnerContext::Trait { source_index: 0 },
            items: &items,
            imports: HashMap::new(),
            prelude_memo: HashMap::new(),
            active: Default::default(),
            type_params: Vec::new(),
            lexical_function: None,
            user_crate_anchor: false,
            anonymous_closure_body: false,
            active_closure: None,
            resolved_closures: HashMap::new(),
            resolved_closure_owners: HashMap::new(),
            current_function_name: None,
            current_callable: None,
            static_muts: Default::default(),
            globs: &[],
            active_globs: Default::default(),
        };
        let module = program.traits[0].module;
        assert!(resolver.trait_method_bounds(&mut [], module, 0, 0).is_ok());
        assert!(resolver.trait_method_bounds(&mut [], module, 0, 1).is_err());
        assert!(resolver.trait_method_bounds(&mut [], module, 1, 0).is_err());
        assert!(
            resolver
                .trait_method_bounds(&mut [], program.traits[1].module, 0, 0)
                .is_err()
        );
        resolver.source_owner = SourceOwnerContext::None;
        assert!(resolver.trait_method_bounds(&mut [], module, 0, 0).is_err());
    }

    #[test]
    fn aliases_keep_definition_projection_occurrences_and_substitute_actual_children() {
        let program = parse("mod m{pub trait Has<T>{type Out;}}use m::Has as H;struct S;type Alias<T>=<S as H<T>>::Out;fn take(x:Alias<i32>)->Alias<i32>{x}fn main(){}").unwrap();
        let function = &program.functions[0];
        let Type::Projection(projection) = &function.result else {
            panic!("alias erased projection");
        };
        let SourceProjection::Qualified {
            trait_path: TraitPath::Source(path),
            ..
        } = projection.as_ref()
        else {
            panic!("alias lost source trait");
        };
        assert!(matches!(
            path.segments.last().unwrap().args[0],
            Type::Int(ir::IntTy::I32)
        ));
        let original = path
            .path(program.names.as_ref())
            .unwrap()
            .original_use(program.names.as_ref())
            .unwrap();
        assert_eq!(original.len(), 1);
        assert_eq!(program.names.spelling(original.segments()[0]).unwrap(), "H");
        assert!(
            retained_function_projection(
                program.names.as_ref(),
                function,
                projection,
                &ir::PlanBudget::default()
            )
            .unwrap()
        );
    }

    #[test]
    fn required_projected_get_preserves_exact_source_occurrences() {
        let parsed = parse_unresolved("trait Has{type Out;fn get()->Self::Out;fn pass(x:Self::Out)->Self::Out;}struct S;impl Has for S{type Out=i32;fn get()->Self::Out{42}fn pass(x:Self::Out)->Self::Out{x}}fn main(){}").unwrap();
        let program = resolve(&parsed).unwrap();
        let budget = ir::PlanBudget::default();
        for (source, resolved) in parsed.program.traits[0]
            .items
            .iter()
            .zip(&program.traits[0].items)
        {
            assert!(resolved.default.is_none());
            assert!(matches!(&resolved.signature.result,
                Type::Projection(p) if matches!(p.as_ref(), SourceProjection::SelfItem { .. })));
            assert!(
                exact_source_type(
                    program.names.as_ref(),
                    &source.signature.result,
                    &resolved.signature.result,
                    &budget,
                    0,
                )
                .unwrap()
            );
            for ((_, source), (_, resolved)) in source
                .signature
                .params
                .iter()
                .zip(&resolved.signature.params)
            {
                assert!(
                    exact_source_type(program.names.as_ref(), source, resolved, &budget, 0)
                        .unwrap()
                );
            }
        }
        let Type::Projection(get) = &program.traits[0].items[0].signature.result else {
            panic!("required get lost its projection");
        };
        assert!(
            !retained_type_projection(
                program.names.as_ref(),
                &program.traits[0].items[1].signature.result,
                get,
                &budget,
            )
            .unwrap(),
            "a different method's same-spelled projection is not the source occurrence"
        );
        assert!(matches!(&program.impls[0].items[0].function.result,
            Type::Projection(p) if matches!(p.as_ref(), SourceProjection::SelfItem { .. })));
    }

    #[test]
    fn required_projected_signatures_admit_nested_and_alias_revealed_types() {
        for source in [
            "trait Has{type Out;fn pass(x:(Self::Out,i128))->(Self::Out,i128);}fn main(){}",
            "struct Wrap<T>{value:T}trait Has{type Out;fn pass(x:Wrap<Self::Out>)->Wrap<Self::Out>;}fn main(){}",
            "trait Has<T>{type Out;fn pass(x:<Self as Has<T>>::Out)->Self::Out;}fn main(){}",
            "type Alias<T>=<T as Has>::Out;trait Has{type Out;fn pass(x:Alias<Self>)->Alias<Self>;}fn main(){}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            let program = resolve(&parsed).unwrap_or_else(|error| panic!("{source}: {error}"));
            assert!(program.traits[0].items[0].default.is_none());
            assert!(!matches!(
                program.traits[0].items[0].signature.result,
                Type::Unit | Type::Infer | Type::Param(_)
            ));
        }
    }

    #[test]
    fn projected_default_types_are_retained_for_canonical_universal_checking() {
        for source in [
            "trait Has{type Out;fn get()->Self::Out{1}}fn main(){}",
            "trait Has{type Out;fn pass(x:Self::Out){}}fn main(){}",
            "trait Has{type Out;fn get(){let x:Self::Out=1;}}fn main(){}",
            "trait Has{type Out;fn get(){if true{let x:(Self::Out,i128)=(1,2);}}}fn main(){}",
            "type Alias<T>=<T as Has>::Out;trait Has{type Out;fn get()->Alias<Self>{1}}fn main(){}",
            "type Alias<T>=<T as Has>::Out;trait Has{type Out;fn get(){let x:Alias<Self>=1;}}fn main(){}",
            "trait Has{type Out;fn required()->Self::Out;fn default(){let x:Self::Out=1;}}fn main(){}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            let resolved = resolve(&parsed).unwrap();
            assert!(!resolved.traits.is_empty(), "{source}");
        }
        parse("trait Has{type Out;fn required()->Self::Out;fn default()->i32{1}}fn main(){}")
            .unwrap();
    }

    #[test]
    fn projection_capability_rejection_does_not_hide_invalid_source_children() {
        for source in [
            "trait Has{type Out;fn get(x:(Self::Out,Missing))->Self::Out;}fn main(){}",
            "trait Pair<A,B>{type Out;}trait Has{type Out;fn get()-><Self as Pair<Self::Out,Missing>>::Out;}fn main(){}",
            "trait Has{type Out;fn get(x:(Self::Out,Missing))->Self::Out{1}}fn main(){}",
            "trait Has{type Out;fn get(){let x:Self::Out=1;let y:Missing=2;}}fn main(){}",
            "trait Has{type Out;fn get()->Self::Out{1}fn later(x:Missing);}fn main(){}",
            "trait Has{type Out;fn get()->Self::Out{1}}struct Broken{value:Missing}fn main(){}",
            "trait Has{type Out;fn get()->Self::Out{1}}struct S;impl Has for S{type Out=Missing;}fn main(){}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            let error = resolve(&parsed).unwrap_err();
            assert!(error.contains("unresolved type"), "{source}: {error}");
            assert!(!error.starts_with("unsupported:"), "{source}: {error}");
        }
    }

    #[test]
    fn required_projection_retains_projected_trait_actuals_for_world_validation() {
        let parsed = parse_unresolved("trait Pair<A,B>{type Out;}trait Has{type Out;fn get()-><Self as Pair<Self::Out,i32>>::Out;}fn main(){}").unwrap();
        // Resolution authenticates source identities. The canonical world must
        // still reject this declaration's missing Pair membership.
        resolve(&parsed).unwrap();
    }

    #[test]
    fn required_projection_rejects_foreign_source_occurrences() {
        let source = "trait Has{type Out;fn get()->Self::Out;}fn main(){}";
        let mut parsed = parse_unresolved(source).unwrap();
        let foreign = parse_unresolved(source).unwrap();
        parsed.program.traits[0].items[0].signature.result =
            foreign.program.traits[0].items[0].signature.result.clone();
        let error = resolve(&parsed).unwrap_err();
        assert!(!error.starts_with("unsupported:"), "{error}");
    }

    #[test]
    fn ambiguous_or_foreign_self_stay_closed() {
        let invalid = parse("trait Base{type Out;}trait Child:Base{}struct S;impl Child for S{type X=Self::Out;}fn main(){}").unwrap();
        assert!(crate::compiler::frontend::check(&invalid).is_err());
        assert!(
            parse("trait Base{type Out;}trait Child:Base{fn get()->Self::Out;}fn main(){}").is_ok()
        );
        for source in [
            "trait Has{type Out;}struct S;impl S{fn get()->Self::Out{1}}fn main(){}",
            "trait Has{type Out;}fn take<T>(x:T::Out){}fn main(){}",
            "trait Has{type Out;}struct S;fn take(x:Self::Out){}fn main(){}",
            "trait Has{type Out;fn get()->Self::Out;}fn take(x:Self::Out){}fn main(){}",
            "trait Has{type Out;fn get()->Self::Out;}trait Other{fn get()->Self::Out;}fn main(){}",
            "trait Has{type Out;fn get()->Self::Out;}trait Other{type Different;fn get()->Self::Out;}fn main(){}",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }

    #[test]
    fn member_transplant_within_one_owner_does_not_reuse_another_source_site() {
        let program = parse("trait Has{type Out;}struct S;fn take(a:<S as Has>::Out,b:<S as Has>::Out){}fn main(){}").unwrap();
        let function = &program.functions[0];
        let Type::Projection(first) = &function.params[0].1 else {
            panic!()
        };
        let Type::Projection(second) = &function.params[1].1 else {
            panic!()
        };
        let mut changed = first.as_ref().clone();
        let SourceProjection::Qualified { item, .. } = &mut changed else {
            panic!()
        };
        let SourceProjection::Qualified {
            item: replacement, ..
        } = second.as_ref()
        else {
            panic!()
        };
        *item = *replacement;
        assert!(
            !retained_function_projection(
                program.names.as_ref(),
                function,
                &changed,
                &ir::PlanBudget::default()
            )
            .unwrap()
        );
        assert!(
            retained_function_projection(
                program.names.as_ref(),
                function,
                first,
                &ir::PlanBudget::default()
            )
            .unwrap()
        );
    }

    #[test]
    fn projection_parsing_does_not_split_ordinary_shift_expressions() {
        let program = parse("fn main(){let x=3<<2;let y=x>>1;}").unwrap();
        let Stmt::Let {
            value: Expr::Binary { op, .. },
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!()
        };
        assert!(matches!(op, BinaryOp::Shl));
        let Stmt::Let {
            value: Expr::Binary { op, .. },
            ..
        } = &program.functions[0].body.statements[1]
        else {
            panic!()
        };
        assert!(matches!(op, BinaryOp::Shr));
    }
}

#[cfg(test)]
mod tuple_shape_tests {
    use super::*;

    #[test]
    fn tuple_constructor_and_let_pattern_retain_nominal_syntax_and_numeric_selectors() {
        let parsed =
            parse_unresolved("struct Pair(i32,i32);fn main(){let p=Pair(1,2);let Pair(x,y)=p;}")
                .unwrap();
        let program = resolve(&parsed).unwrap();
        let statements = &program.functions[0].body.statements;
        let Stmt::Let {
            value: Expr::TupleStruct { name, fields },
            ..
        } = &statements[0]
        else {
            panic!("tuple constructor became a callable or ordinary record");
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(
                program.names.as_ref(),
                &program.structs[0].name,
                Namespace::Type
            )
            .unwrap()
        );
        let Stmt::LetPattern {
            pattern:
                Pattern::Variant {
                    shape,
                    fields,
                    rest,
                    ..
                },
            ..
        } = &statements[1]
        else {
            panic!("tuple let pattern lost explicit source constructor shape");
        };
        assert_eq!(*shape, ir::VariantShape::Tuple);
        assert!(!rest);
        assert!(matches!(fields[0].0, FieldSelector::Tuple(0)));
        assert!(matches!(fields[1].0, FieldSelector::Tuple(1)));
    }

    #[test]
    fn imported_constructor_and_scoped_explicit_child_bindings_resolve() {
        for source in [
            "mod a{pub struct Pair(pub i32,pub i32);}use a::Pair as P;fn main(){let p=P(1,2);let P(x,y)=p;}",
            "struct Pair(i32,i32);fn main(){macro_rules! n{()=>{1};}let p=Pair(n!(),2);let Pair(x,y)=p;let z=x+y;}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn stock_confirmed_tuple_constructor_shadow_is_rejected_but_type_names_can_bind() {
        for source in [
            "struct Pair(i32);fn main(){let Pair=7;}",
            "struct Pair(i32);fn main(){let mut Pair=7;}",
            "struct Pair(i32);fn f(Pair:i32){}fn main(){}",
            "struct Pair(i32);use Pair as P;fn main(){let P=7;}",
        ] {
            let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
            assert!(
                error.contains("E0530: bindings cannot shadow tuple struct constructors"),
                "{source}: {error}"
            );
        }
        for source in [
            "struct Pair{x:i32}fn main(){let Pair=7;}",
            "struct Pair(i32);type A=Pair;fn main(){let A=7;}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn alias_call_wrong_arity_and_private_constructor_remain_invalid() {
        for (source, diagnostic) in [
            (
                "struct Pair(i32);type A=Pair;fn main(){let p=A(1);}",
                "unresolved function",
            ),
            (
                "struct Pair(i32,i32);fn main(){let p=Pair(1);}",
                "tuple struct constructor arity mismatch",
            ),
            (
                "mod a{pub struct Pair(pub i32,i32);}fn main(){let p=a::Pair(1,2);}",
                "private item",
            ),
        ] {
            let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
            assert!(error.contains(diagnostic), "{source}: {error}");
            assert!(!error.contains("unsupported:"), "{source}: {error}");
        }
    }

    #[test]
    fn first_class_tuple_constructor_is_explicitly_unsupported() {
        let parsed = parse_unresolved("struct Pair(i32);fn main(){let p=Pair;}").unwrap();
        assert_eq!(
            resolve(&parsed).unwrap_err(),
            "in function crate::main: unsupported: first-class tuple struct constructors are unsupported"
        );
    }
}

#[cfg(test)]
mod generic_tuple_resolution_tests {
    use super::*;

    #[test]
    fn imported_generic_tuple_constructor_keeps_actuals_and_original_use() {
        let source = "mod m{pub struct Pair<T>(pub T,pub i32);}use m::Pair as P;fn main(){let p=P::<i32>(1,2);let P(x,y)=p;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::Let {
            value: Expr::Apply { path, value },
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("generic tuple constructor lost its application");
        };
        let Expr::TupleStruct { name, fields } = value.as_ref() else {
            panic!("generic tuple constructor became a callable");
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(path.segments.last().unwrap().args.len(), 1);
        let key = name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap();
        assert_eq!(
            key,
            name_scope::path_key(
                program.names.as_ref(),
                &program.structs[0].name,
                Namespace::Type,
            )
            .unwrap()
        );
        let application = path.path(program.names.as_ref()).unwrap();
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), &application, Namespace::Type).unwrap(),
            key
        );
        assert!(
            !application
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn generic_tuple_constructor_keeps_arity_privacy_and_alias_boundaries() {
        for source in [
            "struct Pair<T>(T,i32);fn main(){let p=Pair::<i32>(1);}",
            "mod m{pub struct Pair<T>(pub T,i32);}fn main(){let p=m::Pair::<i32>(1,2);}",
            "struct Pair<T>(T);type A<T>=Pair<T>;fn main(){let p=A::<i32>(1);}",
            "struct Pair<T>(T);fn main(){let Pair=1;}",
        ] {
            assert!(
                resolve(&parse_unresolved(source).unwrap()).is_err(),
                "{source}"
            );
        }
    }
}

#[cfg(test)]
mod let_else_tests {
    use super::*;

    #[test]
    fn let_else_retains_actual_source_statement_and_full_constructor_certificate() {
        let source =
            "enum E{V(i32),Other}use E::V as P;fn main(){let P(x)=P(1)else{return;};let after=x;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::LetElse {
            pattern,
            value,
            else_block,
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("let-else became fabricated match or let-pattern source syntax");
        };
        let Pattern::Variant { name, fields, .. } = pattern.as_ref() else {
            panic!("let-else lost actual constructor pattern syntax");
        };
        assert_eq!(fields.len(), 1);
        assert!(matches!(value.as_ref(), Expr::EnumVariant { .. }));
        assert!(matches!(else_block.statements[0], Stmt::Return(None)));
        let target = program.enums[0]
            .name
            .push(program.names.as_ref(), program.enums[0].variants[0].name)
            .unwrap();
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(program.names.as_ref(), &target, Namespace::Type).unwrap()
        );
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn let_else_rhs_and_failure_see_old_bindings_before_success_publication() {
        for source in [
            "enum E{V(i32),Other}fn x()->i32{7}fn y()->i32{8}fn main(){let E::V(x)=E::V(x())else{let y=2;let z=x()+y;return;};let after=x+y();}",
            "enum E{V(i32),Other}fn x()->i32{7}fn main(){macro_rules! n{()=>{1};}let E::V(x)=E::V(x()+n!())else{let z=x();return;};let after=x+n!();}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        let source = "enum E{V(i32),Other}fn x()->i32{7}fn main(){let E::V(x)=E::V(x())else{let z=x();return;};let after=x();}";
        assert_noncallable_local_shadow(source);
    }

    #[test]
    fn let_else_failure_cannot_see_pending_success_binding() {
        let source = "enum E{V(i32),Other}fn main(){let E::V(x)=E::V(1)else{let y=x();return;};let after=x;}";
        let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
        assert!(error.contains("unresolved function"), "{error}");
        assert!(!error.contains("local binding is not callable"), "{error}");
    }
}

#[cfg(test)]
mod while_let_tests {
    use super::*;

    #[test]
    fn while_let_retains_actual_source_pattern_scrutinee_and_constructor_certificate() {
        let source = "mod a{pub struct Pair(pub i32);}use a::Pair as P;fn main(){while let P(x)=P(1){let z=x;break;}let after=1;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::WhileLet {
            pattern,
            scrutinee,
            body,
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("while-let became a fabricated source loop or match");
        };
        let Pattern::Variant { name, fields, .. } = pattern.as_ref() else {
            panic!("while-let lost actual constructor pattern syntax");
        };
        assert_eq!(fields.len(), 1);
        assert!(matches!(scrutinee.as_ref(), Expr::TupleStruct { .. }));
        assert_eq!(body.statements.len(), 2);
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(
                program.names.as_ref(),
                &program.structs[0].name,
                Namespace::Type
            )
            .unwrap()
        );
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn while_let_scrutinee_and_exit_see_outer_callable_not_iteration_binder() {
        for source in [
            "struct P(i32);fn x()->i32{7}fn main(){while let P(x)=P(x()){let z=x;break;}let after=x();}",
            "struct P(i32);fn x()->i32{7}fn main(){macro_rules! n{()=>{1};}while let P(x)=P(x()){let z=x+n!();break;}let after=x();}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        let source =
            "struct P(i32);fn x()->i32{7}fn main(){while let P(x)=P(x()){let z=x();break;}}";
        assert_noncallable_local_shadow(source);
    }

    #[test]
    fn labeled_while_let_keeps_real_enum_rest_children_and_label() {
        let source = "enum E{V(i32,i32)}fn main(){'again:while let E::V(x,..)=E::V(1,2){let z=x;continue 'again;}}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::WhileLet { label, pattern, .. } = &program.functions[0].body.statements[0] else {
            panic!("labeled while-let lost source statement identity");
        };
        assert!(label.is_some());
        let Pattern::VariantRest { name, fields } = pattern.as_ref() else {
            panic!("while-let payload rest was expanded into omitted syntax");
        };
        assert_eq!(fields.prefix.len(), 1);
        assert!(fields.suffix.is_empty());
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod if_let_tests {
    use super::*;

    #[test]
    fn if_let_keeps_actual_constructor_certificate_and_absent_else() {
        let source = "mod a{pub struct Pair(pub i32);}use a::Pair as P;fn main(){if let P(x)=P(1){let z=x;}let after=1;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::ImplicitExpr(Expr::IfLet {
            pattern,
            scrutinee,
            else_branch,
            ..
        }) = &program.functions[0].body.statements[0]
        else {
            panic!("if-let was replaced by fabricated source match syntax");
        };
        assert!(else_branch.is_none());
        let Pattern::Variant { name, fields, .. } = pattern.as_ref() else {
            panic!("if-let lost its actual constructor pattern");
        };
        assert_eq!(fields.len(), 1);
        assert!(matches!(scrutinee.as_ref(), Expr::TupleStruct { .. }));
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(
                program.names.as_ref(),
                &program.structs[0].name,
                Namespace::Type
            )
            .unwrap()
        );
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn if_let_scrutinee_else_and_following_code_see_outer_callable_not_pattern_binder() {
        for source in [
            "struct P(i32);fn x()->i32{7}fn main(){if let P(x)=P(x()){let z=x;}else{let z=x();}let after=x();}",
            "struct P(i32);fn x()->i32{7}fn main(){macro_rules! n{()=>{1};}if let P(x)=P(x()){let z=x+n!();}else{let z=x();}let after=x();}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        let source =
            "struct P(i32);fn x()->i32{7}fn main(){if let P(x)=P(x()){let z=x();}else{let z=x();}}";
        assert_noncallable_local_shadow(source);
    }

    #[test]
    fn if_let_else_if_retains_genuine_expression_without_source_wrapper() {
        let source = "fn main(){if let true=false{}else if let false=false{}else{}let after=1;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::ImplicitExpr(Expr::IfLet {
            else_branch: Some(branch),
            ..
        }) = &program.functions[0].body.statements[0]
        else {
            panic!("if-let did not retain its actual else expression");
        };
        let Expr::IfLet {
            else_branch: Some(last),
            ..
        } = branch.as_ref()
        else {
            panic!("else-if-let acquired a fabricated source block");
        };
        assert!(matches!(last.as_ref(), Expr::Block(_)));
    }
}

#[cfg(test)]
mod array_pattern_tests {
    use super::*;

    #[test]
    fn array_rest_retains_only_written_children_and_binder_scope() {
        let source = "fn main(){let [x,..,y]=[1,2,3];let z=x+y;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::LetPattern {
            pattern: Pattern::ArrayRest(fields),
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("array rest lost explicit source syntax");
        };
        assert_eq!(fields.prefix.len(), 1);
        assert_eq!(fields.suffix.len(), 1);
        assert!(matches!(fields.prefix[0], Pattern::Binding { .. }));
        assert!(matches!(fields.suffix[0], Pattern::Binding { .. }));
        assert_eq!(
            fields
                .project(3)
                .unwrap()
                .iter()
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        for source in [
            "fn main(){let [x,y]=[1,2];let z=x+y;}",
            "fn main(){macro_rules! n{()=>{1};}let [x,..,y]=[n!(),2,3];let z=x+y;}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn nested_array_constructor_patterns_keep_canonical_use_certificates() {
        // Resolver-only control. The checker separately owns selected-Copy
        // admission and must not infer it merely from these nominal children.
        let source = "mod a{pub struct Pair(pub i32);}use a::Pair as P;fn main(){let [P(x),..,P(y)]=[P(1),P(2),P(3)];let z=x+y;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::LetPattern {
            pattern: Pattern::ArrayRest(fields),
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("nested array pattern lost source syntax");
        };
        for child in fields.prefix.iter().chain(fields.suffix.iter()) {
            let Pattern::Variant { name, fields, .. } = child else {
                panic!("nested tuple constructor lost exact source shape");
            };
            assert_eq!(fields.len(), 1);
            assert_eq!(
                name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
                name_scope::path_key(
                    program.names.as_ref(),
                    &program.structs[0].name,
                    Namespace::Type
                )
                .unwrap()
            );
            assert!(
                !name
                    .source_occurrences(program.names.as_ref())
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn array_patterns_preserve_real_binding_duplicates_and_constructor_shadow_errors() {
        for (source, diagnostic) in [
            (
                "fn main(){let [x,..,x]=[1,2,3];}",
                "duplicate pattern binding",
            ),
            (
                "struct Pair(i32);fn main(){let [Pair,..]=[1,2];}",
                "E0530: bindings cannot shadow tuple struct constructors",
            ),
        ] {
            let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
            assert!(error.contains(diagnostic), "{source}: {error}");
            assert!(!error.contains("unsupported:"), "{source}: {error}");
        }
    }
}

#[cfg(test)]
mod positional_rest_tests {
    use super::*;

    #[test]
    fn imported_tuple_rest_retains_only_real_children_and_certified_constructor() {
        let parsed = parse_unresolved(
            "mod a{pub struct Triple(pub i32,pub i32,pub i32);}use a::Triple as T;fn main(){let T(x,..,y)=T(1,2,3);let z=x+y;}",
        )
        .unwrap();
        let program = resolve(&parsed).unwrap();
        let Stmt::LetPattern {
            pattern: Pattern::VariantRest { name, fields },
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("positional rest lost its explicit source shape");
        };
        assert_eq!(fields.prefix.len(), 1);
        assert_eq!(fields.suffix.len(), 1);
        assert_eq!(
            fields
                .project(3)
                .unwrap()
                .iter()
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(
                program.names.as_ref(),
                &program.structs[0].name,
                Namespace::Type
            )
            .unwrap()
        );
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn tuple_rest_traverses_nested_real_bindings_without_omitted_nodes() {
        for source in [
            "fn main(){let (x,..,y)=(1,2,3);let z=x+y;}",
            "struct T(i32,i32,i32);fn main(){macro_rules! n{()=>{1};}let T(x,..,y)=T(n!(),2,3);let z=x+y;}",
            "struct T(i32,i32);fn main(){let T(..)=T(1,2);}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn tuple_rest_omitted_private_fields_still_restrict_constructor() {
        let source = "mod a{pub struct T(pub i32,i32,pub i32);pub fn make()->T{T(1,2,3)}}fn main(){let a::T(x,..,y)=a::make();}";
        let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
        assert!(error.contains("private item"), "{error}");
        assert!(!error.contains("unsupported:"), "{error}");
    }

    #[test]
    fn tuple_rest_excess_fields_remain_invalid() {
        let source = "struct T(i32);fn main(){let T(x,..,y)=T(1);}";
        let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
        assert!(
            error.contains("positional rest pattern has too many fields"),
            "{error}"
        );
    }

    #[test]
    fn enum_tuple_rest_certifies_full_constructor_and_preserves_suffix_indices() {
        let source =
            "enum E{V(i32,i32,i32),Other}fn main(){let E::V(x,..,y)=E::V(1,2,3);let z=x+y;}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::LetPattern {
            pattern: Pattern::VariantRest { name, fields },
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("enum rest lost actual variant constructor syntax");
        };
        assert_eq!(
            fields
                .project(3)
                .unwrap()
                .iter()
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        let target = program.enums[0]
            .name
            .push(program.names.as_ref(), program.enums[0].variants[0].name)
            .unwrap();
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(program.names.as_ref(), &target, Namespace::Type).unwrap()
        );
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn original_enum_rest_boundary_now_retains_variant_certificate_and_index() {
        let source = "enum E{V(i32,i32)}fn main(){let E::V(x,..)=E::V(1,2);}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let Stmt::LetPattern {
            pattern: Pattern::VariantRest { name, fields },
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("enum rest did not retain actual variant syntax");
        };
        assert_eq!(fields.prefix.len(), 1);
        assert!(fields.suffix.is_empty());
        assert_eq!(
            fields
                .project(2)
                .unwrap()
                .iter()
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            vec![0]
        );
        let target = program.enums[0]
            .name
            .push(program.names.as_ref(), program.enums[0].variants[0].name)
            .unwrap();
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(program.names.as_ref(), &target, Namespace::Type).unwrap()
        );
        assert!(
            !name
                .source_occurrences(program.names.as_ref())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn enum_tuple_rest_imports_aliases_and_generic_schema_keep_existing_resolution() {
        for source in [
            "enum E{V(i32,i32,i32)}use E::V as P;fn main(){let P(x,..,y)=P(1,2,3);let z=x+y;}",
            "enum E<T>{V(T,i32,T)}type A=E<i32>;fn main(){let A::V(x,..,y)=A::V(1,2,3);let z=x+y;}",
            "enum E{V(i32,i32)}fn main(){let E::V(..)=E::V(1,2);}",
        ] {
            resolve(&parse_unresolved(source).unwrap())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn enum_tuple_rest_wrong_source_shape_and_excess_fields_are_invalid() {
        for (source, diagnostic) in [
            (
                "enum E{V}fn main(){let E::V(..)=E::V;}",
                "positional rest requires an enum tuple variant",
            ),
            (
                "enum E{V{x:i32}}fn main(){let E::V(..)=E::V{x:1};}",
                // Record variants are not positional value constructors, so
                // namespace resolution rejects this before tuple-shape checks.
                "unresolved positional rest constructor",
            ),
            (
                "enum E{V(i32)}fn main(){let E::V(x,..,y)=E::V(1);}",
                "positional rest pattern has too many fields",
            ),
        ] {
            let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
            assert!(error.contains(diagnostic), "{source}: {error}");
            assert!(!error.contains("unsupported:"), "{source}: {error}");
        }
    }
}

#[cfg(test)]
mod unit_shape_tests {
    use super::*;

    #[test]
    fn direct_unit_value_normalizes_to_exact_nominal_struct_expression() {
        let parsed = parse_unresolved("struct S;fn main(){let x=S;}").unwrap();
        let program = resolve(&parsed).unwrap();
        let Stmt::Let {
            value: Expr::Struct { name, fields },
            ..
        } = &program.functions[0].body.statements[0]
        else {
            panic!("unit constructor did not retain nominal struct expression");
        };
        assert!(fields.is_empty());
        assert_eq!(
            name_scope::path_key(program.names.as_ref(), name, Namespace::Type).unwrap(),
            name_scope::path_key(
                program.names.as_ref(),
                &program.structs[0].name,
                Namespace::Type
            )
            .unwrap()
        );
    }

    #[test]
    fn bare_alias_to_unit_remains_invalid_before_planning_unit_fence() {
        let parsed = parse_unresolved("struct S;type A=S;fn main(){let x=A;}").unwrap();
        let error = resolve(&parsed).unwrap_err();
        assert_eq!(
            error,
            "in function crate::main: type alias is not a value constructor"
        );
    }

    #[test]
    fn unused_unit_declaration_preserves_real_shape() {
        let parsed = parse_unresolved("struct S;fn main(){}").unwrap();
        let program = resolve(&parsed).unwrap();
        assert_eq!(program.structs[0].shape, StructShape::Unit);
    }

    #[test]
    fn imported_values_braced_aliases_and_bindingless_unit_patterns_resolve() {
        for source in [
            "mod a{pub struct S;}use a::S as U;fn main(){let x=U;}",
            "struct S;type A=S;fn main(){let x=S{};let y=A{};}",
            "struct S;fn main(){let S=S;match S{S=>{}}}",
            "mod a{pub struct S;}fn main(){let a::S=a::S;match a::S{a::S=>{}}}",
            "struct S;fn main(){macro_rules! n{()=>{1};}let x=n!();let crate::S=S;match S{crate::S=>{}}}",
            "struct S;fn main(){macro_rules! n{()=>{1};}let x=n!();let S{}=S;}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            resolve(&parsed).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn scoped_bare_unit_pattern_reclassification_is_unsupported_before_ghost_use() {
        for source in [
            "struct S;fn main(){macro_rules! n{()=>{1};}let x=n!();let S=S;}",
            "struct S;fn main(){macro_rules! n{()=>{1};}let x=n!();match S{S=>{}}}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            let error = resolve(&parsed).unwrap_err();
            assert_eq!(
                error,
                "in function crate::main: unsupported: bare unit constructor patterns in lexical-scope functions require deferred binder classification",
                "{source}"
            );
        }
    }

    #[test]
    fn unit_tuple_call_is_not_callable() {
        let parsed = parse_unresolved("struct S;fn main(){let x=S();}").unwrap();
        let error = resolve(&parsed).unwrap_err();
        assert_eq!(error, "in function crate::main: unresolved function S");
        let stock = super::super::parser_tests::stock_compile(
            "struct S;fn main(){let x=S();}",
            std::path::Path::new("unused"),
            true,
        );
        assert!(!stock.status.success());
        assert!(String::from_utf8_lossy(&stock.stderr).contains("E0618"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_trait_applications_preserve_import_certificates_and_all_actuals() {
        let parsed = parse_unresolved("mod defs{pub trait Convert<A>{fn convert(self)->A;}}use defs::Convert as C;struct S;impl C<i32> for S{fn convert(self)->i32{7}}fn relay<T,U:C<T>>(x:U)->T{x.convert()}fn main(){let n=<S as C<i32>>::convert(S);}").unwrap();
        let program = resolve(&parsed).unwrap();
        let TraitPath::Source(path) = program.impls[0].trait_path.as_ref().unwrap() else {
            panic!("source trait became a builtin");
        };
        assert_eq!(path.segments.last().unwrap().args.len(), 1);
        assert!(matches!(
            path.segments.last().unwrap().args[0],
            Type::Int(_)
        ));
        let canonical = path.path(program.names.as_ref()).unwrap();
        let original = canonical.original_use(program.names.as_ref()).unwrap();
        assert_ne!(canonical.segments().len(), original.segments().len());
        assert_eq!(program.trait_imports.len(), 1);
        let TraitPath::Source(import) = &program.trait_imports[0].1 else {
            panic!("source trait import became a builtin");
        };
        assert!(
            import
                .segments
                .iter()
                .all(|segment| segment.args.is_empty())
        );
        let TraitPath::Source(bound) = &program.functions[0].bounds[0].traits[0] else {
            panic!("source bound became a builtin");
        };
        assert!(matches!(
            bound.segments.last().unwrap().args[0],
            Type::Param(_)
        ));
    }

    #[test]
    fn trait_parameters_remain_in_scope_for_supertraits_required_and_default_items() {
        let parsed = parse_unresolved("trait Convert<A>{fn convert(self)->A;}trait Relay<T:Copy>:Convert<T>{fn relay(self,x:T)->T;fn echo(self,x:T)->T{x}}fn main(){}").unwrap();
        let program = resolve(&parsed).unwrap();
        assert_eq!(program.traits[1].type_params.len(), 1);
        let TraitPath::Source(parent) = &program.traits[1].supertraits[0] else {
            panic!("source supertrait became a builtin");
        };
        assert!(matches!(
            parent.segments.last().unwrap().args[0],
            Type::Param(_)
        ));
        for item in &program.traits[1].items {
            assert!(matches!(item.signature.result, Type::Param(_)));
        }
        for source in [
            "trait Bad<T:Copy<i32>>{fn f(self);}fn main(){}",
            "trait Bad<T:core::marker::Copy<i32>>{fn f(self);}fn main(){}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            assert!(
                resolve(&parsed)
                    .unwrap_err()
                    .contains("builtin traits take no type arguments")
            );
        }
    }

    #[test]
    fn anchored_module_reexports_preserve_original_visibility() {
        for source in [
            "mod a{pub use self as Alias;}fn main(){}",
            "mod a{pub mod b{pub use super as Alias;}}fn main(){}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            let error = resolve(&parsed).unwrap_err();
            assert!(
                error.contains("cannot be publicly reexported"),
                "{source}: {error}"
            );
        }
        for source in [
            "pub mod a{pub use self as Alias;pub fn f()->i32{7}}fn main(){if a::Alias::f()==7{println!(\"reexports ok\");}}",
            "pub mod a{pub fn f()->i32{7}pub mod b{pub use super as Alias;}}fn main(){if a::b::Alias::f()==7{println!(\"reexports ok\");}}",
        ] {
            let parsed = parse_unresolved(source).unwrap();
            resolve(&parsed).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }
}

impl Resolver<'_> {
    /// The `call` member occurrence of the prelude `std::ops::Fn` trait,
    /// used to lower `f(args)` on a local callable value to `f.call((args,))`.
    fn callable_call_ident(&self) -> Result<AstIdent, String> {
        let program = self.program;
        let names = program.names.as_ref();
        for decl in &program.traits {
            if !program.in_prelude(decl.module) {
                continue;
            }
            let Some(last) = decl.name.segments().last() else {
                continue;
            };
            if names
                .spelling(*last)
                .map_err(native_adapter::name_diagnostic)?
                != "Fn"
            {
                continue;
            }
            for item in &decl.items {
                if names
                    .spelling(item.signature.name)
                    .map_err(native_adapter::name_diagnostic)?
                    == "call"
                {
                    return Ok(item.signature.name);
                }
            }
        }
        Err("unsupported: calling a local value requires the prelude Fn trait".into())
    }
}

/// Replace written `Self` with the impl's written self type inside a trait
/// header argument. Only structural positions are rewritten; projections,
/// opaque types and trait objects keep `Self` and stay with ordinary checks.
fn replace_self_type(
    ty: &mut Type,
    self_ty: &Type,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 {
        return Err("impl header Self substitution depth limit exceeded".into());
    }
    budget.charge_work(1)?;
    match ty {
        Type::SelfType => *ty = self_ty.clone(),
        Type::Ref { referent, .. } => replace_self_type(referent, self_ty, budget, depth + 1)?,
        Type::Array(element, _) | Type::Slice(element) => {
            replace_self_type(element, self_ty, budget, depth + 1)?
        }
        Type::RawPtr { pointee, .. } => replace_self_type(pointee, self_ty, budget, depth + 1)?,
        Type::Tuple(fields) => {
            for field in fields {
                replace_self_type(field, self_ty, budget, depth + 1)?;
            }
        }
        Type::Apply(path) => {
            for segment in &mut path.segments {
                for argument in &mut segment.args {
                    replace_self_type(argument, self_ty, budget, depth + 1)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
