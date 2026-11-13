//! Block-local `const` items (`fn f() { const N: u32 = 4; ... }`).
//!
//! Stock semantics: the item is visible throughout its enclosing block,
//! including before its declaration; it shadows outer items of the same name;
//! its initializer is an item context, so it cannot read local variables
//! (E0435) but may name sibling items. Each local const is therefore hoisted to
//! an ordinary `ConstDecl` in the function's module with a unique hidden name, so constant
//! evaluation, privacy and the "no locals" rule are the existing item rules.
//! Single-segment value paths that do not name a local binding are rewritten to
//! the hidden path of the innermost enclosing block const of that name.
//!
//! Block-local `fn` items and `use` imports follow the same protocol: each is
//! an item hoisted into the enclosing module under a hidden name, visible
//! throughout its block, and a path whose first segment names it is rewritten
//! to that hidden item. Functions are parsed as items (no enclosing locals);
//! a block `use` becomes an ordinary private import of its module, so privacy,
//! resolution and unused-import validation stay the existing item rules.
use super::*;

/// Reserved hidden-name prefix, like `__justrust_extern_`. Names are
/// `__justrust_local_const_<prepared offset>_<written>`, unique per crate.
const HIDDEN_PREFIX: &str = "__justrust_local_const_";
/// Hidden names of block-local `fn` items and `use` bindings.
const HIDDEN_ITEM_PREFIX: &str = "__justrust_local_item_";

#[derive(Default)]
pub(super) struct LocalConsts {
    /// Enclosing blocks, innermost last: (written name key, hidden ident).
    scopes: Vec<Vec<(NameKey, AstIdent)>>,
    /// Same, for block-local `fn` items and `use` imports in the type and
    /// value namespaces (a `use` may name either).
    item_scopes: Vec<Vec<(NameKey, NameKey, AstIdent)>>,
    /// Hoisted functions not yet published by the item parser.
    pending_functions: Vec<Function>,
    /// Hoisted imports (hidden path segment, target) not yet published.
    pending_imports: Vec<(AstIdent, AstPath, bool)>,
    /// Hoisted declarations not yet published by the item parser.
    pending: Vec<ConstDecl>,
    /// Block-local nominal items and impls, published with their module item.
    pending_items: Option<Box<ParsedDeclarations>>,
    /// The module path of the module item being parsed.
    pub(super) module_path: Option<AstPath>,
    /// Hidden name for the block-local item whose header is being parsed.
    declared: Option<AstIdent>,
}

impl Parser<'_, '_> {
    /// Enter a block: pre-scan its direct statements for `const NAME:` items
    /// so later and earlier statements alike see them.
    #[inline(never)]
    pub(super) fn enter_local_const_scope(&mut self) -> Result<(), ParseError> {
        let mut scope = Vec::new();
        let mut items = Vec::new();
        let start = self.cursor.checkpoint();
        if !self.is("{") {
            // Not a braced block; the block parser reports the syntax error.
            self.local_consts.scopes.push(scope);
            self.local_consts.item_scopes.push(items);
            return Ok(());
        }
        self.cursor.bump();
        let mut depth = 0usize;
        let mut statement_start = true;
        loop {
            let token = self.token();
            if token.matches("") {
                break;
            }
            if depth == 0 && token.matches("}") {
                break;
            }
            if depth == 0
                && statement_start
                && (token.matches("const") || token.matches("static"))
                && self
                    .cursor
                    .peek(1)
                    .is_some_and(|t| t.is_ident() && !t.matches("_"))
                && self.cursor.peek(2).is_some_and(|t| t.matches(":"))
            {
                self.cursor.bump();
                let written = self.ident()?;
                let key = self.name_key(written, LookupClass::Item(Namespace::Value))?;
                if scope.iter().any(|(existing, _)| *existing == key) {
                    return Err(self.error("E0428: duplicate block-local const name"));
                }
                // The `const` token's prepared offset is unique within the
                // assembled crate, so per-file parses cannot collide.
                let spelling = format!(
                    "{HIDDEN_PREFIX}{}_{}",
                    token.bytes.start,
                    self.names
                        .spelling(written)
                        .map_err(native_adapter::name_error)?
                );
                let hidden = self
                    .names
                    .intern_root(&spelling, token.span)
                    .map_err(native_adapter::name_error)?;
                scope.push((key, hidden));
                statement_start = false;
                continue;
            }
            if depth == 0
                && statement_start
                && token.matches("fn")
                && self.cursor.peek(1).is_some_and(|t| t.is_ident())
            {
                self.cursor.bump();
                let written = self.ident()?;
                self.prescan_local_item(token.bytes.start, token.span, written, &mut items)?;
                statement_start = false;
                continue;
            }
            if depth == 0
                && statement_start
                && ["struct", "enum", "trait", "type"]
                    .iter()
                    .any(|keyword| token.matches(keyword))
                && self.cursor.peek(1).is_some_and(|t| t.is_ident())
            {
                self.cursor.bump();
                let written = self.ident()?;
                self.prescan_local_item(token.bytes.start, token.span, written, &mut items)?;
                statement_start = false;
                continue;
            }
            if depth == 0 && statement_start && token.matches("use") {
                self.prescan_local_use(token.bytes.start, token.span, &mut items)?;
                statement_start = false;
                continue;
            }
            if token.matches("{") || token.matches("(") || token.matches("[") {
                depth += 1;
            } else if token.matches("}") || token.matches(")") || token.matches("]") {
                depth = depth.saturating_sub(1);
            }
            statement_start = depth == 0 && (token.matches(";") || token.matches("}"));
            self.cursor.bump();
        }
        if !self.cursor.restore(start) {
            return Err(native_adapter::transport_error(
                super::super::token_session::TokenError::InvalidInput(
                    "block-local const scan lost its start",
                ),
            ));
        }
        self.local_consts.scopes.push(scope);
        self.local_consts.item_scopes.push(items);
        Ok(())
    }

    /// Each binding name a block `use` tree introduces is a block item.
    #[inline(never)]
    fn prescan_local_use(
        &mut self,
        offset: usize,
        span: super::super::expansion_span::SpanId,
        items: &mut Vec<(NameKey, NameKey, AstIdent)>,
    ) -> Result<(), ParseError> {
        self.cursor.bump();
        let mut tree = Vec::new();
        let (mut work, mut bytes) = (0, 0);
        let prefix = AstPath::new(self.names, PathAnchor::Relative, &[])
            .map_err(native_adapter::name_error)?;
        self.use_tree(&prefix, 0, &mut work, &mut bytes, &mut tree)?;
        for (name, _, type_only) in tree {
            let Some(name) = name else {
                if !type_only {
                    return Err(self.unsupported("block-local empty use prefixes are unsupported"));
                }
                // `use path::*;` binds no names here; the resolver scopes it.
                continue;
            };
            self.prescan_local_item(offset, span, name, items)?;
        }
        Ok(())
    }

    #[inline(never)]
    fn prescan_local_item(
        &mut self,
        offset: usize,
        span: super::super::expansion_span::SpanId,
        written: AstIdent,
        items: &mut Vec<(NameKey, NameKey, AstIdent)>,
    ) -> Result<(), ParseError> {
        let value = self.name_key(written, LookupClass::Item(Namespace::Value))?;
        let ty = self.name_key(written, LookupClass::Item(Namespace::Type))?;
        if items.iter().any(|(v, t, _)| *v == value || *t == ty) {
            return Err(self.error("E0428: duplicate block-local item name"));
        }
        let spelling = format!(
            "{HIDDEN_ITEM_PREFIX}{offset}_{}",
            self.names
                .spelling(written)
                .map_err(native_adapter::name_error)?
        );
        let hidden = self
            .names
            .intern_root(&spelling, span)
            .map_err(native_adapter::name_error)?;
        self.reserve_parser_items(items, 1)?;
        items.push((value, ty, hidden));
        Ok(())
    }

    pub(super) fn leave_local_const_scope(&mut self) {
        self.local_consts.scopes.pop();
        self.local_consts.item_scopes.pop();
    }

    fn hidden_local_item(&self, written: AstIdent) -> Result<Option<AstIdent>, ParseError> {
        if self.local_consts.item_scopes.iter().all(Vec::is_empty) {
            return Ok(None);
        }
        let value = self.name_key(written, LookupClass::Item(Namespace::Value))?;
        let ty = self.name_key(written, LookupClass::Item(Namespace::Type))?;
        for scope in self.local_consts.item_scopes.iter().rev() {
            if let Some((_, _, hidden)) = scope.iter().find(|(v, t, _)| *v == value || *t == ty) {
                return Ok(Some(*hidden));
            }
        }
        Ok(None)
    }

    /// Rewrite a relative path whose first segment names a block-local item.
    #[inline(never)]
    pub(super) fn local_item_path(&self, path: &AstPath) -> Result<Option<AstPath>, ParseError> {
        if path.anchor() != PathAnchor::Relative {
            return Ok(None);
        }
        let Some(first) = path.segments().first().copied() else {
            return Ok(None);
        };
        let Some(hidden) = self.hidden_local_item(first)? else {
            return Ok(None);
        };
        let mut segments = path.segments().to_vec();
        segments[0] = hidden;
        AstPath::new(self.names, PathAnchor::Relative, &segments)
            .map(Some)
            .map_err(native_adapter::name_error)
    }

    /// Same rewrite for the head segment of a generic path.
    #[inline(never)]
    pub(super) fn rewrite_local_item_generic(
        &self,
        path: &mut GenericPath,
    ) -> Result<(), ParseError> {
        if path.anchor != PathAnchor::Relative {
            return Ok(());
        }
        if let Some(first) = path.segments.first_mut()
            && let Some(hidden) = self.hidden_local_item(first.name)?
        {
            first.name = hidden;
        }
        Ok(())
    }

    /// Parse a block-local `fn` item and hoist it.
    #[inline(never)]
    pub(super) fn local_fn_item(&mut self) -> Result<(), ParseError> {
        if self.item_expansion.is_some() || self.expansion.is_some() {
            return Err(self.unsupported("generated block-local fn items are unsupported"));
        }
        let written = self
            .cursor
            .peek(1)
            .filter(|t| t.is_ident())
            .ok_or_else(|| self.error("expected block-local fn name"))?;
        let _ = written;
        // An item context: the nested function sees items, never the
        // enclosing function's locals or generic parameters.
        let lexical = self.lexical.function.take();
        let type_params = std::mem::take(&mut self.type_params);
        let bounds = std::mem::take(&mut self.visible_bounds);
        let function = self.function();
        self.lexical.function = lexical;
        self.type_params = type_params;
        self.visible_bounds = bounds;
        let mut function = function?;
        let name = function
            .name
            .single()
            .ok_or_else(|| self.error("block-local fn must have a single name"))?;
        let hidden = self
            .hidden_local_item(name)?
            .ok_or_else(|| self.error("block-local fn was not pre-scanned"))?;
        function.visibility = ir::Visibility::Restricted(self.module);
        function.name = AstPath::new(self.names, PathAnchor::Relative, &[hidden])
            .map_err(native_adapter::name_error)?;
        self.local_consts.pending_functions.push(function);
        Ok(())
    }

    /// Parse a block-local `use` item and hoist each binding.
    #[inline(never)]
    pub(super) fn local_use_item(&mut self, statements: &mut Vec<Stmt>) -> Result<(), ParseError> {
        if self.item_expansion.is_some() || self.expansion.is_some() {
            return Err(self.unsupported("generated block-local use items are unsupported"));
        }
        self.expect("use")?;
        let mut tree = Vec::new();
        let (mut work, mut bytes) = (0, 0);
        let prefix = AstPath::new(self.names, PathAnchor::Relative, &[])
            .map_err(native_adapter::name_error)?;
        self.use_tree(&prefix, 0, &mut work, &mut bytes, &mut tree)?;
        self.expect(";")?;
        for (name, target, type_only) in tree {
            let Some(name) = name else {
                if !type_only {
                    return Err(self.unsupported("block-local empty use prefixes are unsupported"));
                }
                // A block glob is scoped to this block: the resolver consults
                // it before the enclosing module's items (stock order).
                let target = self.local_item_path(&target)?.unwrap_or(target);
                statements.push(Stmt::UseGlob(target));
                continue;
            };
            let hidden = self
                .hidden_local_item(name)?
                .ok_or_else(|| self.error("block-local use was not pre-scanned"))?;
            // A block import's own path is resolved in the module; an outer
            // block item named by its first segment is rewritten too.
            let target = self.local_item_path(&target)?.unwrap_or(target);
            self.local_consts
                .pending_imports
                .push((hidden, target, type_only));
        }
        Ok(())
    }

    /// Parse `const NAME: TYPE = INIT;` at statement position and hoist it.
    pub(super) fn local_const_item(&mut self) -> Result<(), ParseError> {
        if self.item_expansion.is_some() || self.expansion.is_some() {
            return Err(self.unsupported("generated block-local const items are unsupported"));
        }
        let storage = if self.eat("static") {
            ConstantStorage::Static
        } else {
            self.expect("const")?;
            ConstantStorage::Const
        };
        let unnamed = storage == ConstantStorage::Const && self.is("_");
        let hidden = if unnamed {
            // `const _` names nothing; it is only evaluated and checked.
            self.unnamed_const_name()?
        } else {
            let written = self.ident()?;
            let key = self.name_key(written, LookupClass::Item(Namespace::Value))?;
            self.local_consts
                .scopes
                .last()
                .and_then(|scope| scope.iter().find(|(k, _)| *k == key).map(|(_, h)| *h))
                .ok_or_else(|| self.error("block-local const was not pre-scanned"))?
        };
        self.expect(":")?;
        let ty = self.ty()?;
        self.expect("=")?;
        // An item context: the initializer sees items, never local bindings.
        let frontier = self.lexical.function.as_ref().map(|f| f.frontier);
        let scopes_taken = self.lexical.function.take();
        let initializer_start = self.cursor.index();
        let initializer = self.nested_expr();
        self.lexical.function = scopes_taken;
        if let (Some(function), Some(frontier)) = (self.lexical.function.as_mut(), frontier) {
            function.frontier = frontier;
        }
        let initializer = initializer?;
        let initializer = if unnamed {
            initializer
        } else {
            self.fold_scalar_const_initializer(&ty, initializer_start, initializer)?
        };
        self.expect(";")?;
        let (ty, initializer) = if unnamed {
            self.unnamed_const_initializer(ty, initializer)?
        } else {
            (ty, initializer)
        };
        // Defined in the enclosing function's module with that module's
        // private visibility; publish_local_consts gives it the full path.
        let name = AstPath::new(self.names, PathAnchor::Relative, &[hidden])
            .map_err(native_adapter::name_error)?;
        self.local_consts.pending.push(ConstDecl {
            assertion: unnamed && matches!(ty, Type::Bool),
            storage,
            module: self.module,
            visibility: ir::Visibility::Restricted(self.module),
            name,
            ty,
            initializer,
        });
        Ok(())
    }

    /// `const _: T = e;` (module or block level): a fresh unique hidden
    /// name, unique by the `_` token's prepared offset, so no path can ever
    /// name it and several unnamed constants never collide.
    pub(super) fn unnamed_const_name(&mut self) -> Result<AstIdent, ParseError> {
        let token = self.token();
        if !token.matches("_") {
            return Err(self.error("expected `_`"));
        }
        let spelling = format!("{HIDDEN_PREFIX}{}__", token.bytes.start);
        let name = self
            .names
            .intern_root(&spelling, token.span)
            .map_err(native_adapter::name_error)?;
        self.cursor.bump();
        Ok(name)
    }

    /// `const _: () = assert!(cond);` is evaluated by stock rustc even though
    /// nothing names it. Lower it to the scalar boolean constant
    /// `cond || (1u8 / 0u8 == 0u8)`, which the scalar evaluator rejects
    /// (E0080) exactly when the assertion fails. Any other unit initializer
    /// that could panic has no evaluator here, so it fails closed.
    pub(super) fn unnamed_const_initializer(
        &mut self,
        ty: Type,
        initializer: Expr,
    ) -> Result<(Type, Expr), ParseError> {
        if !matches!(ty, Type::Unit) {
            return Ok((ty, initializer));
        }
        let condition = match initializer {
            Expr::If {
                condition,
                then_block,
                else_block,
            } if then_block.statements.is_empty()
                && then_block.tail.is_none()
                && else_block.statements.is_empty()
                && matches!(else_block.tail.as_deref(), Some(Expr::Panic(_))) =>
            {
                *condition
            }
            Expr::Block(Block {
                statements,
                tail: None,
            }) if statements.is_empty() => {
                return Ok((
                    Type::Unit,
                    Expr::Block(Block {
                        statements,
                        tail: None,
                    }),
                ));
            }
            _ => {
                return Err(self.unsupported(
                    "unnamed constants other than `()` or a single `assert!` are unsupported",
                ));
            }
        };
        if !super::root_resolution::scalar_const_assertion(&condition) {
            return Err(self.unsupported("constant assertions require a scalar constant condition"));
        }
        let byte = |magnitude| {
            Box::new(Expr::Integer(IntegerLiteral {
                magnitude,
                suffix: Some(ir::IntTy::U8),
            }))
        };
        let failure = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Binary {
                op: BinaryOp::Div,
                left: byte(1),
                right: byte(0),
                site: None,
            }),
            right: byte(0),
            site: None,
        };
        Ok((
            Type::Bool,
            Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(condition),
                right: Box::new(failure),
                site: None,
            },
        ))
    }

    /// The hidden path for a single-segment value use of a block-local const.
    pub(super) fn local_const_path(&self, ident: AstIdent) -> Result<Option<AstPath>, ParseError> {
        if self.local_consts.scopes.is_empty() {
            return Ok(None);
        }
        let key = self.name_key(ident, LookupClass::Item(Namespace::Value))?;
        for scope in self.local_consts.scopes.iter().rev() {
            if let Some((_, hidden)) = scope.iter().find(|(k, _)| *k == key) {
                // A same-module relative item path: the hidden const lives in
                // the module of the function whose block declares it.
                return AstPath::new(self.names, PathAnchor::Relative, &[*hidden])
                    .map(Some)
                    .map_err(native_adapter::name_error);
            }
        }
        Ok(None)
    }

    /// Publish hoisted local consts as ordinary crate-root items.
    pub(super) fn publish_local_consts(
        &mut self,
        module: ModuleId,
        module_path: &AstPath,
        parsed: &mut ParsedDeclarations,
    ) -> Result<(), ParseError> {
        let pending = std::mem::take(&mut self.local_consts.pending);
        for mut declaration in pending {
            if declaration.module != module {
                return Err(self.error("block-local const escaped its defining module"));
            }
            let hidden = declaration
                .name
                .single()
                .ok_or_else(|| self.error("block-local const has no hidden name"))?;
            let path = module_path
                .push(self.names, hidden)
                .map_err(native_adapter::name_error)?;
            declaration.name = path.clone();
            let visibility = declaration.visibility.clone();
            self.reserve_parser_items(&mut parsed.program.constants, 1)?;
            parsed.program.constants.push(declaration);
            self.insert_item(parsed, module, &path, visibility, ItemKind::Const)?;
        }
        let functions = std::mem::take(&mut self.local_consts.pending_functions);
        for mut function in functions {
            if function.module != module {
                return Err(self.error("block-local fn escaped its defining module"));
            }
            let hidden = function
                .name
                .single()
                .ok_or_else(|| self.error("block-local fn has no hidden name"))?;
            let path = module_path
                .push(self.names, hidden)
                .map_err(native_adapter::name_error)?;
            function.name = path.clone();
            let visibility = function.visibility.clone();
            self.reserve_parser_items(&mut parsed.program.functions, 1)?;
            parsed.program.functions.push(function);
            self.insert_item(parsed, module, &path, visibility, ItemKind::Function)?;
        }
        if let Some(items) = self.local_consts.pending_items.take() {
            self.merge_local_declarations(parsed, *items)?;
        }
        let imports = std::mem::take(&mut self.local_consts.pending_imports);
        for (hidden, target, type_only) in imports {
            let path = module_path
                .push(self.names, hidden)
                .map_err(native_adapter::name_error)?;
            self.insert_item(
                parsed,
                module,
                &path,
                ir::Visibility::Restricted(module),
                ItemKind::Import(target, type_only),
            )?;
        }
        Ok(())
    }
}

impl Parser<'_, '_> {
    /// `struct`, `enum`, `trait`, `type` or `impl` at statement position.
    pub(super) fn at_local_nominal_item(&self) -> bool {
        let named =
            |keyword: &str| self.is(keyword) && self.cursor.peek(1).is_some_and(|t| t.is_ident());
        named("struct") || named("enum") || named("trait") || named("type") || self.is("impl")
    }

    /// The name an item declaration publishes: the hidden name reserved by
    /// the enclosing block's pre-scan while a block-local item is parsed.
    pub(super) fn local_declared_name(
        &mut self,
        written: AstIdent,
    ) -> Result<AstIdent, ParseError> {
        let Some(hidden) = self.local_consts.declared.take() else {
            return Ok(written);
        };
        let written_key = self.name_key(written, LookupClass::Item(Namespace::Type))?;
        let found = self
            .local_consts
            .item_scopes
            .last()
            .and_then(|scope| scope.iter().find(|(_, ty, _)| *ty == written_key))
            .map(|(_, _, hidden)| *hidden);
        match found {
            Some(found)
                if self.name_key(found, LookupClass::Item(Namespace::Type))?
                    == self.name_key(hidden, LookupClass::Item(Namespace::Type))? =>
            {
                Ok(hidden)
            }
            _ => Err(self.error("block-local item was not pre-scanned")),
        }
    }

    /// Parse a block-local nominal item (or impl) as an ordinary item of the
    /// enclosing module under its hidden name. Its declarations are retained
    /// until the enclosing module item finishes, then published with it.
    #[inline(never)]
    pub(super) fn local_nominal_item(&mut self) -> Result<(), ParseError> {
        if self.item_expansion.is_some() || self.expansion.is_some() {
            return Err(self.unsupported("generated block-local items are unsupported"));
        }
        let module = self.module;
        let module_path = self.local_consts.module_path.clone().ok_or_else(|| {
            self.unsupported("block-local items outside a module item are unsupported")
        })?;
        if !self.is("impl") {
            let written = self
                .cursor
                .peek(1)
                .ok_or_else(|| self.error("expected block-local item name"))?;
            let written = self.input.mint_name(&written, self.names)?;
            let hidden = self
                .hidden_local_item(written)?
                .ok_or_else(|| self.error("block-local item was not pre-scanned"))?;
            self.local_consts.declared = Some(hidden);
        }
        // An item context: no enclosing locals, generic parameters, bounds
        // or impl lifetimes are visible (E0401/E0434 stay rejections).
        let lexical = self.lexical.function.take();
        let type_params = std::mem::take(&mut self.type_params);
        let const_params = std::mem::take(&mut self.const_params);
        let impl_const_params = std::mem::take(&mut self.impl_const_params);
        let bounds = std::mem::take(&mut self.visible_bounds);
        let impl_regions = std::mem::take(&mut self.impl_regions);
        let pending_regions = std::mem::take(&mut self.pending_regions);
        let mut parsed = match self.local_consts.pending_items.take() {
            Some(parsed) => parsed,
            None => Box::new(ParsedDeclarations {
                program: ProgramDeclarations::default(),
                items: HashMap::new(),
                empty_use_prefixes: Vec::new(),
                glob_imports: Vec::new(),
            }),
        };
        let start = self.cursor.index();
        let result = self.nonmodule_item(
            module,
            &module_path,
            &mut parsed,
            ir::Visibility::Restricted(module),
            false,
            start,
        );
        self.local_consts.declared = None;
        self.lexical.function = lexical;
        self.type_params = type_params;
        self.const_params = const_params;
        self.impl_const_params = impl_const_params;
        self.visible_bounds = bounds;
        self.impl_regions = impl_regions;
        self.pending_regions = pending_regions;
        // Items declared in blocks of this item's own bodies.
        let nested = self.local_consts.pending_items.take();
        let merged = match nested {
            Some(nested) => self.merge_local_declarations(&mut parsed, *nested),
            None => Ok(()),
        };
        self.local_consts.pending_items = Some(parsed);
        result?;
        merged
    }

    /// Append `source` declarations to `target`, rebasing declaration indices.
    pub(super) fn merge_local_declarations(
        &mut self,
        target: &mut ParsedDeclarations,
        mut source: ParsedDeclarations,
    ) -> Result<(), ParseError> {
        if source
            .program
            .impls
            .iter()
            .any(|block| block.primitive_source.is_some())
            || source
                .program
                .functions
                .iter()
                .any(|function| function.embedded_source.is_some())
        {
            return Err(self.unsupported("embedded original block-local items are unsupported"));
        }
        for key in source.items.keys() {
            if target.items.get(key).is_some_and(|items| {
                items
                    .iter()
                    .any(|item| !matches!(item.kind, ItemKind::Import(..)))
            }) && source.items[key]
                .iter()
                .any(|item| !matches!(item.kind, ItemKind::Import(..)))
            {
                return Err(self.error("E0428: duplicate block-local item name"));
            }
        }
        let base = DeclarationBase {
            structs: target.program.structs.len(),
            enums: target.program.enums.len(),
            traits: target.program.traits.len(),
        };
        for (key, mut items) in source.items.drain() {
            self.reserve_item_publication(target)?;
            base.rebase(&mut items)
                .map_err(|message| self.error(&message))?;
            target.items.entry(key).or_default().extend(items);
        }
        let mut generated = source.program;
        generated_metadata::rebase(
            &mut generated.generated_metadata,
            target.program.structs.len(),
            target.program.impls.len(),
        )?;
        target.program.functions.extend(generated.functions);
        target.program.constants.extend(generated.constants);
        target.program.structs.extend(generated.structs);
        target.program.enums.extend(generated.enums);
        target.program.impls.extend(generated.impls);
        target.program.traits.extend(generated.traits);
        target
            .program
            .generated_metadata
            .extend(generated.generated_metadata);
        target.program.trait_imports.extend(generated.trait_imports);
        target
            .program
            .closure_sources
            .extend(generated.closure_sources);
        target.empty_use_prefixes.extend(source.empty_use_prefixes);
        target.glob_imports.extend(source.glob_imports);
        Ok(())
    }
}
