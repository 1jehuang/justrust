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
use super::*;

/// Reserved hidden-name prefix, like `__justrust_extern_`. Names are
/// `__justrust_local_const_<prepared offset>_<written>`, unique per crate.
const HIDDEN_PREFIX: &str = "__justrust_local_const_";

#[derive(Default)]
pub(super) struct LocalConsts {
    /// Enclosing blocks, innermost last: (written name key, hidden ident).
    scopes: Vec<Vec<(NameKey, AstIdent)>>,
    /// Hoisted declarations not yet published by the item parser.
    pending: Vec<ConstDecl>,
}

impl Parser<'_, '_> {
    /// Enter a block: pre-scan its direct statements for `const NAME:` items
    /// so later and earlier statements alike see them.
    pub(super) fn enter_local_const_scope(&mut self) -> Result<(), ParseError> {
        let mut scope = Vec::new();
        let start = self.cursor.checkpoint();
        if !self.is("{") {
            // Not a braced block; the block parser reports the syntax error.
            self.local_consts.scopes.push(scope);
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
        Ok(())
    }

    pub(super) fn leave_local_const_scope(&mut self) {
        self.local_consts.scopes.pop();
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
        Ok(())
    }
}
