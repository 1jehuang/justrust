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
                && token.matches("const")
                && self.cursor.peek(1).is_some_and(|t| t.is_ident())
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
        for (name, _, _) in tree {
            let Some(name) = name else {
                return Err(self.unsupported("block-local glob imports are unsupported"));
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
    pub(super) fn local_use_item(&mut self) -> Result<(), ParseError> {
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
            let name =
                name.ok_or_else(|| self.unsupported("block-local glob imports are unsupported"))?;
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
        self.expect("const")?;
        let written = self.ident()?;
        let key = self.name_key(written, LookupClass::Item(Namespace::Value))?;
        let hidden = self
            .local_consts
            .scopes
            .last()
            .and_then(|scope| scope.iter().find(|(k, _)| *k == key).map(|(_, h)| *h))
            .ok_or_else(|| self.error("block-local const was not pre-scanned"))?;
        self.expect(":")?;
        let ty = self.ty()?;
        self.expect("=")?;
        // An item context: the initializer sees items, never local bindings.
        let frontier = self.lexical.function.as_ref().map(|f| f.frontier);
        let scopes_taken = self.lexical.function.take();
        let initializer = self.nested_expr();
        self.lexical.function = scopes_taken;
        if let (Some(function), Some(frontier)) = (self.lexical.function.as_mut(), frontier) {
            function.frontier = frontier;
        }
        let initializer = initializer?;
        self.expect(";")?;
        // Defined in the enclosing function's module with that module's
        // private visibility; publish_local_consts gives it the full path.
        let name = AstPath::new(self.names, PathAnchor::Relative, &[hidden])
            .map_err(native_adapter::name_error)?;
        self.local_consts.pending.push(ConstDecl {
            module: self.module,
            visibility: ir::Visibility::Restricted(self.module),
            name,
            ty,
            initializer,
        });
        Ok(())
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
