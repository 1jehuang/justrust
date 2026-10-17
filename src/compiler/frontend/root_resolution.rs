//! Checked source namespace, import and alias resolution.
//! No display spelling is used as a lookup key or reconstructed into an AST.
use super::super::module_identity::{CatalogModule, ModuleSegment};
use super::super::semantic_name::BuiltinAtom;
use super::super::semantic_scope::{BindingId, ScopeId, scope_diagnostic};
use super::*;

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
    fn trait_path(&self) -> Result<TraitPath, String> {
        match &self.path {
            ResolvedPath::Source(path) if self.kind == ResolvedKind::Trait => {
                Ok(TraitPath::Source(path.clone()))
            }
            ResolvedPath::Builtin(trait_id) => Ok(TraitPath::Builtin(*trait_id)),
            _ => Err("resolved declaration is not a trait".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedKind {
    TypeAlias,
    Trait,
    Module,
    Function,
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

struct Resolver<'a> {
    program: &'a Program,
    type_depth: usize,
    alias_cache: HashMap<PathKey, Type>,
    alias_active: std::collections::HashSet<PathKey>,
    alias_budget: ir::PlanBudget,
    nominal_arity: HashMap<PathKey, usize>,
    self_nominal: Option<AstPath>,
    self_allowed: bool,
    items: &'a HashMap<PathKey, Vec<Item>>,
    imports: HashMap<(PathKey, usize, bool), Option<ResolvedItem>>,
    active: std::collections::HashSet<(PathKey, usize, bool)>,
    type_params: Vec<AstIdent>,
    lexical_function: Option<ScopeId>,
}
impl Resolver<'_> {
    fn names(&self) -> &NameStore {
        self.program.names.as_ref()
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
                args: Vec::new(),
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
                        Type::Named(path) => spellings.push(path.clone()),
                        Type::Apply(path) => {
                            spellings.push(path.path(self.names())?);
                            for segment in &path.segments {
                                pending.extend(&segment.args);
                            }
                        }
                        Type::Tuple(fields) => pending.extend(fields),
                        Type::Ref { referent, .. }
                        | Type::Array(referent, _)
                        | Type::Slice(referent) => pending.push(referent),
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
                Type::SelfType => return Err("Self outside impl in type alias".into()),
                Type::Ref { region: None, .. } => {
                    return Err("E0106: missing lifetime specifier in type alias".into());
                }
                Type::Ref { .. } => {
                    return Err("explicit lifetime type aliases are unsupported".into());
                }
                Type::Slice(_) => return Err("unsized type aliases are unsupported".into()),
                Type::Array(t, _) => check_target(t)?,
                Type::Tuple(fields) => {
                    for field in fields {
                        check_target(field)?;
                    }
                }
                Type::Apply(path) => {
                    for segment in &path.segments {
                        for arg in &segment.args {
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
        let result = self.ty(&mut target, module);
        let params = std::mem::replace(&mut self.type_params, saved_params);
        self.self_nominal = saved_self;
        self.self_allowed = saved_allowed;
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
            Type::Param(name) => {
                self.charge_entries::<NameKey>(1)?;
                used.try_reserve(1)
                    .map_err(|_| "type parameter set allocation failed")?;
                used.insert(self.ident_key(*name, LookupClass::TypeParameter)?);
            }
            Type::Apply(path) => {
                for segment in &path.segments {
                    for arg in &segment.args {
                        self.collect_params(arg, used, depth + 1)?;
                    }
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.collect_params(field, used, depth + 1)?;
                }
            }
            Type::Ref { referent, .. } | Type::Array(referent, _) | Type::Slice(referent) => {
                self.collect_params(referent, used, depth + 1)?
            }
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
            Type::Named(path) => *path = self.resolved_use(path, source)?,
            Type::Apply(path) => {
                self.certify_generic(path, source)?;
                for segment in &mut path.segments {
                    for arg in &mut segment.args {
                        self.alias_use(arg, source, depth + 1)?;
                    }
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.alias_use(field, source, depth + 1)?;
                }
            }
            Type::Ref { referent, .. } | Type::Array(referent, _) | Type::Slice(referent) => {
                self.alias_use(referent, source, depth + 1)?
            }
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
                    for arg in &mut segment.args {
                        self.substitute(arg, params, args, depth + 1)?;
                    }
                }
            }
            Type::Tuple(fields) => {
                for field in fields {
                    self.substitute(field, params, args, depth + 1)?;
                }
            }
            Type::Ref { referent, .. } | Type::Array(referent, _) | Type::Slice(referent) => {
                self.substitute(referent, params, args, depth + 1)?
            }
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
        let TraitPath::Source(source) = path else {
            return Ok(());
        };
        let resolved = self.path(source, module, true, false)?;
        if let Some(item) = resolved {
            if item.kind != ResolvedKind::Trait {
                return Err(format!("{} is not a trait", self.display(source)?));
            }
            *path = match item.trait_path()? {
                TraitPath::Source(target) => TraitPath::Source(self.resolved_use(&target, source)?),
                builtin => builtin,
            };
        } else if let Some(ident) = source.single()
            && let Some(id) = self.builtin_trait(ident)?
        {
            *path = TraitPath::Builtin(id);
        } else {
            return Err(format!("unresolved trait {}", self.display(source)?));
        }
        Ok(())
    }
    fn bounds(&mut self, bounds: &mut [TypeBound], module: ModuleId) -> Result<(), String> {
        for bound in bounds {
            if !self.param_contains(bound.parameter)? {
                return Err("bound parameter is not in scope".into());
            }
            for path in &mut bound.traits {
                self.trait_path(path, module)?;
            }
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
            return Ok(None);
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
        Ok(result)
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
            PathAnchor::Crate => Some(
                self.program
                    .modules
                    .source_root()
                    .map_err(module_diagnostic)?,
            ),
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
    fn generic_path(&mut self, path: &mut GenericPath, module: ModuleId) -> Result<(), String> {
        if path.anchor == PathAnchor::SelfType {
            if !self.self_allowed {
                return Err("Self outside impl".into());
            }
            for segment in &mut path.segments {
                for arg in &mut segment.args {
                    self.ty(arg, module)?;
                }
            }
            return Ok(());
        }
        let at = path
            .segments
            .iter()
            .position(|segment| !segment.args.is_empty())
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
        let item = match self.path(&prefix, module, true, false)? {
            Some(item) => Some(item),
            None => self.path(&prefix, module, false, false)?,
        }
        .ok_or_else(|| "unresolved generic declaration".to_owned())?;
        if item.kind == ResolvedKind::Module {
            return Err("generic arguments are not allowed on modules".into());
        }
        let mut args = std::mem::take(&mut path.segments[at].args);
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
                return Ok(None);
            };
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
                Type::Named(self.resolved_use(resolved.source()?, &owner_path)?)
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
        let result = self.ty_inner(ty, module);
        self.type_depth -= 1;
        result
    }
    fn ty_inner(&mut self, ty: &mut Type, module: ModuleId) -> Result<(), String> {
        match ty {
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
            Type::Array(element, _) | Type::Slice(element) => self.ty(element, module)?,
            Type::Named(name) => {
                let item = self
                    .path(name, module, true, false)?
                    .ok_or_else(|| "unresolved type".to_owned())?;
                if item.kind == ResolvedKind::TypeAlias {
                    *ty = self.expand_alias(item.source()?, &[], name)?;
                    return Ok(());
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
        if !declaration.type_params.is_empty() {
            return Err(Self::unsupported("generic tuple structs are unsupported"));
        }
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
            Pattern::Wildcard | Pattern::Bool(_) | Pattern::Int { .. } => {}
            Pattern::Path(name) if name.anchor() == PathAnchor::SelfType && self.self_allowed => {}
            Pattern::Path(name) => {
                let item = self
                    .zero_constructor(name, module)?
                    .ok_or("unresolved zero-field constructor pattern")?;
                *pattern = self.constructor_pattern(&item, name, false)?;
            }
            Pattern::Binding { name, mutable } => {
                let path = self.single_path(*name)?;
                if let Some(item) = self.zero_constructor(&path, module)? {
                    if *mutable {
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
            Pattern::Tuple(fields) => {
                for field in fields {
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
            match stmt {
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
                Stmt::AssignDeref { carrier, value } => {
                    self.expr(value, module, locals)?;
                    self.expr(carrier, module, locals)?;
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

    fn expr(
        &mut self,
        expr: &mut Expr,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        // Keep recursive cast chains out of the constructor dispatch frame.
        // Resolve the operand before its target, preserving source validation order.
        if let Expr::Cast { value, target } = expr {
            self.expr(value, module, locals)?;
            return self.ty(target, module);
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
        if let Expr::Var(path) | Expr::Call { name: path, .. } = expr
            && self.local_contains(path, locals)?
        {
            return if matches!(expr, Expr::Call { .. }) {
                Err("local binding is not callable".into())
            } else {
                Ok(())
            };
        }
        let source_path = match expr {
            Expr::Apply { path, .. } => {
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
            Expr::Array(elements) | Expr::Tuple(elements) => {
                for value in elements {
                    self.expr(value, module, locals)?;
                }
            }
            Expr::Cast { .. } => {
                return Err("cast bypassed resolver expression dispatcher".into());
            }
            Expr::Deref { value }
            | Expr::Repeat { value, .. }
            | Expr::Index { base: value, .. }
            | Expr::Unary { value, .. } => self.expr(value, module, locals)?,
            Expr::Borrow { .. } => {}
            Expr::DynamicIndex { base, index } => {
                self.expr(base, module, locals)?;
                self.expr(index, module, locals)?;
            }
            Expr::Var(name) => {
                let value_item = self.path(name, module, false, false)?;
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
                let resolved = self
                    .path(name, module, false, false)?
                    .ok_or_else(|| "unresolved function".to_owned())?;
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
                args,
                ..
            } => {
                self.ty(owner, module)?;
                self.trait_path(trait_path, module)?;
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
            | Expr::Loop { body: block, .. }
            | Expr::LabeledBlock { body: block, .. } => self.block(block, module, locals)?,
            Expr::Break {
                value: Some(value), ..
            } => self.expr(value, module, locals)?,
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
            if receiver != ReceiverKind::None {
                return Err("unsupported: lexical macro bindings for method receivers".into());
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
pub fn resolve(parsed: &ParsedProgram) -> Result<Program, String> {
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
    let mut resolver = Resolver {
        program: &parsed.program,
        type_depth: 0,
        alias_cache: HashMap::new(),
        alias_active: std::collections::HashSet::new(),
        alias_budget: ir::PlanBudget::default(),
        nominal_arity: HashMap::new(),
        self_nominal: None,
        self_allowed: false,
        items: &parsed.items,
        imports: HashMap::new(),
        active: std::collections::HashSet::new(),
        type_params: Vec::new(),
        lexical_function: None,
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
                program
                    .trait_imports
                    .push((item.module, target.trait_path()?));
            }
        }
    }
    for declaration in &mut program.traits {
        for path in &mut declaration.supertraits {
            resolver.trait_path(path, declaration.module)?;
        }
        resolver.self_nominal = None;
        resolver.self_allowed = true;
        for item in &mut declaration.items {
            resolver.type_params = item.signature.type_params.clone();
            resolver.bounds(&mut item.signature.bounds, declaration.module)?;
            resolver.ty(&mut item.signature.result, declaration.module)?;
            for (_, ty) in &mut item.signature.params {
                resolver.ty(ty, declaration.module)?;
            }
            if let Some(function) = &mut item.default {
                resolver.type_params = function.type_params.clone();
                resolver.bounds(&mut function.bounds, declaration.module)?;
                resolver.ty(&mut function.result, declaration.module)?;
                for (_, ty) in &mut function.params {
                    resolver.ty(ty, declaration.module)?;
                }
                let names = resolver.function_locals(function, item.receiver)?;
                validate_depth(DepthNode::Block(&function.body))?;
                resolver.block(&mut function.body, declaration.module, &mut vec![names])?;
            }
        }
        resolver.self_allowed = false;
    }
    for enumeration in &mut program.enums {
        resolver.type_params = enumeration.type_params.clone();
        for variant in &mut enumeration.variants {
            for field in &mut variant.fields {
                validate_depth(DepthNode::Type(&field.ty))?;
                resolver.ty(&mut field.ty, enumeration.module)?;
            }
        }
    }
    for structure in &mut program.structs {
        resolver.type_params = structure.type_params.clone();
        for field in &mut structure.fields {
            validate_depth(DepthNode::Type(&field.ty))?;
            resolver.ty(&mut field.ty, structure.module)?;
        }
    }
    for function in &mut program.functions {
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
    }
    for block in &mut program.impls {
        resolver.type_params = block.type_params.clone();
        resolver.bounds(&mut block.bounds, block.module)?;
        if let Some(path) = &mut block.trait_path {
            resolver.trait_path(path, block.module)?;
        }
        resolver.ty(&mut block.self_ty, block.module)?;
        resolver.self_nominal = Some(match &block.self_ty {
            Type::Named(path) => path.clone(),
            Type::Apply(path) => path.path(resolver.names())?,
            _ => return Err("impl requires a nominal self pattern".into()),
        });
        resolver.self_allowed = true;
        for item in &mut block.items {
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
        resolver.self_nominal = None;
        resolver.self_allowed = false;
    }
    program.validate_name_handles()?;
    Ok(program)
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
        let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
        assert!(error.contains("local binding is not callable"), "{error}");
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
        let error = resolve(&parse_unresolved(source).unwrap()).unwrap_err();
        assert!(error.contains("local binding is not callable"), "{error}");
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
        assert_eq!(error, "in function crate::main: unresolved function");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
