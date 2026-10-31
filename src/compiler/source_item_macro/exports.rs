//! Configured export declarations, independent of parsing function bodies.
//! The configured source item walker, not a second module scanner, selects
//! exact original macro_rules ranges. Token/rule identities never use paths.
mod namespace;
pub(in crate::compiler) mod producer;
use super::*;
use crate::compiler::expansion_span::SpanOrigin;
use crate::compiler::semantic_name::NameStore;
use crate::compiler::source::{PreparedLexInput, PreparedSource, SourceError, SourceErrorKind};
use crate::compiler::token::{Delimiter, Token, TokenKind};
use crate::compiler::token_session::{CrateId, TokenSession};
pub(in crate::compiler) use namespace::Namespace as ExportNamespace;

struct Declaration {
    name: AstIdent,
    attribute: SpanId,
    site: DefinitionSite,
    rules: RulesDef,
}
/// One file's immutable declaration-only output. The original source snapshot
/// and rule owner remain linked when a caller imports this inventory.
pub(in crate::compiler) struct LocalExports {
    file: crate::compiler::keys::SourceFile,
    child_count: usize,
    has_item_macro_invocations: bool,
    has_unresolved_imports: bool,
    names: Arc<NameStore>,
    root: SourceFileHandle,
    crate_id: CrateId,
    declarations: BudgetVec<Declaration>,
}
impl std::fmt::Debug for LocalExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalExports")
            .field("root", &self.root)
            .field("declarations", &self.declarations.len())
            .finish()
    }
}
impl crate::engine::Fact for LocalExports {}

/// An immutable explicit source-subtree completeness fact. An active item
/// invocation keeps declaration completeness open and prevents export lookup.
/// Children are retained by
/// reference, never flattened/copied into each ancestor's declaration list.
pub(in crate::compiler) struct ExportSubtree {
    local: Arc<LocalExports>,
    children: BudgetVec<Arc<ExportSubtree>>,
}
impl std::fmt::Debug for ExportSubtree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportSubtree")
            .field("local", &self.local)
            .field("children", &self.children.len())
            .finish()
    }
}
impl crate::engine::Fact for ExportSubtree {}
impl ExportSubtree {
    pub(in crate::compiler) fn complete(
        local: Arc<LocalExports>,
        children: BudgetVec<Arc<Self>>,
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
    ) -> Result<Self, SourceError> {
        if children.len() != local.child_count {
            return Err(contract("incomplete macro export source subtree"));
        }
        for (index, child) in children.as_slice().iter().enumerate() {
            if child.local.file != local.file.child(index) {
                return Err(contract(
                    "macro export completeness changed canonical child ownership",
                ));
            }
        }
        let result = Self { local, children };
        let mut pending = BudgetVec::new(budget, Storage::Temporary).map_err(token_error)?;
        let mut seen = BudgetVec::<(&NameStore, AstIdent)>::new(budget, Storage::Temporary)
            .map_err(token_error)?;
        pending.push(&result).map_err(token_error)?;
        while let Some(tree) = pending.pop() {
            if tree.local.root != result.local.root || tree.local.crate_id != result.local.crate_id
            {
                return Err(contract("foreign crate in macro export completeness"));
            }
            for declaration in tree.local.declarations.as_slice() {
                let key = tree
                    .local
                    .names
                    .key(declaration.name, LookupClass::Item(Namespace::Macro))
                    .map_err(name_error)?;
                for (names, name) in seen.as_slice() {
                    budget
                        .charge(Counter::Work, 1)
                        .map_err(|error| token_error(error.into()))?;
                    let previous = names
                        .key(*name, LookupClass::Item(Namespace::Macro))
                        .map_err(name_error)?;
                    if tree
                        .local
                        .names
                        .same_key_across(key, names, previous)
                        .map_err(name_error)?
                    {
                        let span = tree
                            .local
                            .names
                            .session()
                            .original(declaration.site.span, 512)
                            .map_err(token_error)?;
                        let span = tree
                            .local
                            .names
                            .session()
                            .sources()
                            .render_span(span)
                            .map_err(token_error)?;
                        return Err(SourceError::syntax(
                            Some(span),
                            "duplicate exported macro in crate-root namespace",
                        ));
                    }
                }
                seen.push((&tree.local.names, declaration.name))
                    .map_err(token_error)?;
            }
            for child in tree.children.as_slice().iter().rev() {
                pending.push(child.as_ref()).map_err(token_error)?;
            }
        }
        Ok(result)
    }
    /// All declaration owners needed to serve selected rule/token handles.
    /// This gathers immutable stores, never parses any dependency body.
    pub(in crate::compiler) fn stores(
        &self,
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
    ) -> Result<BudgetVec<Arc<NameStore>>, SourceError> {
        let mut result = BudgetVec::new(budget, Storage::Temporary).map_err(token_error)?;
        let mut pending = BudgetVec::new(budget, Storage::Temporary).map_err(token_error)?;
        pending.push(self).map_err(token_error)?;
        while let Some(tree) = pending.pop() {
            result.push(tree.local.names.clone()).map_err(token_error)?;
            for child in tree.children.as_slice().iter().rev() {
                pending.push(child.as_ref()).map_err(token_error)?;
            }
        }
        Ok(result)
    }
    pub(in crate::compiler) fn select<'a>(
        &'a self,
        root: SourceFileHandle,
        name: AstIdent,
        names: &NameStoreBuilder,
    ) -> Result<Option<ExportSelection<'a>>, DriverError> {
        if self.local.root != root {
            return Err(TokenError::InvalidHandle.into());
        }
        let invocation = names.span(name)?;
        let caller =
            super::super::source_macro_binding::invocation_root(names.session(), invocation)?;
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        if self.local.has_unresolved_imports {
            return Err(DriverError::Unsupported {
                span: invocation,
                message: "exported macro lookup requires configured macro import/reexport completeness",
            });
        }
        // Source-tree completeness is not declaration completeness. Any active
        // unexpanded item invocation may add an export or a duplicate, even
        // when an original declaration already matches this lookup.
        let mut pending = BudgetVec::new(names.budget(), Storage::Temporary)?;
        pending.push(self)?;
        while let Some(tree) = pending.pop() {
            names.budget().charge(Counter::Work, 1)?;
            if tree.local.has_item_macro_invocations {
                return Err(DriverError::Unsupported {
                    span: invocation,
                    message: "exported macro lookup requires admitted item-macro declaration completeness",
                });
            }
            for child in tree.children.as_slice().iter().rev() {
                pending.push(child.as_ref())?;
            }
        }
        pending.push(self)?;
        while let Some(tree) = pending.pop() {
            if !names.session().serves(tree.local.names.session().id()) {
                return Err(TokenError::ForeignHandle.into());
            }
            for (index, declaration) in tree.local.declarations.as_slice().iter().enumerate() {
                names.budget().charge(Counter::Work, 1)?;
                if names.key(declaration.name, LookupClass::Item(Namespace::Macro))? == key {
                    let left = names.spelling(name)?;
                    let right = names.spelling(declaration.name)?;
                    names
                        .budget()
                        .charge(Counter::Work, left.len().max(right.len()) as u64)?;
                    if left != right {
                        return Err(TokenError::AtomCollision.into());
                    }
                    return Ok(Some(ExportSelection {
                        inventory: &tree.local,
                        index,
                        invocation,
                        caller,
                    }));
                }
            }
            for child in tree.children.as_slice().iter().rev() {
                pending.push(child.as_ref())?;
            }
        }
        Ok(None)
    }
}
impl ExportSelection<'_> {
    fn declaration(&self, names: &NameStoreBuilder) -> Result<&Declaration, DriverError> {
        if !names.session().serves(self.inventory.names.session().id()) {
            return Err(TokenError::ForeignHandle.into());
        }
        if super::super::source_macro_binding::invocation_root(names.session(), self.invocation)?
            != self.caller
        {
            return Err(TokenError::InvalidHandle.into());
        }
        let declaration = self
            .inventory
            .declarations
            .as_slice()
            .get(self.index)
            .ok_or(TokenError::InvalidHandle)?;
        if super::super::source_macro_binding::validate_definition(
            names.session(),
            declaration.site,
        )? != self.inventory.root
            || super::super::source_macro_binding::definition_root(
                names.session(),
                declaration.attribute,
            )? != self.inventory.root
        {
            return Err(TokenError::InvalidHandle.into());
        }
        Ok(declaration)
    }
    pub(in crate::compiler) fn check_call(
        &self,
        names: &NameStoreBuilder,
        call: SpanId,
    ) -> Result<(), DriverError> {
        self.declaration(names)?;
        if call != self.invocation {
            return Err(TokenError::InvalidHandle.into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn expand(
        &self,
        names: &mut NameStoreBuilder,
        args: TokenSlice,
        call: CallSite,
    ) -> Result<ExpandedTokens, DriverError> {
        self.check_call(names, call.span)?;
        let declaration = self.declaration(names)?;
        names
            .expand_local_rules(&declaration.rules, args, call)
            .map_err(DriverError::Macro)
    }
}

/// Selection is bound to the actual invocation occurrence and target crate.
/// A public AST edit, same-spelled name or another inventory cannot mint it.
pub(in crate::compiler) struct ExportSelection<'a> {
    inventory: &'a LocalExports,
    index: usize,
    invocation: SpanId,
    caller: SourceFileHandle,
}

fn token_error(error: TokenError) -> SourceError {
    SourceError::from_token(error)
}
fn contract(message: &str) -> SourceError {
    failure(SourceErrorKind::Contract, None, message)
}

fn failure(
    kind: SourceErrorKind,
    span: Option<crate::compiler::source::SourceSpan>,
    message: &str,
) -> SourceError {
    let mut error = SourceError::syntax(span, message);
    error.kind = kind;
    error
}
fn macro_error(error: crate::compiler::macro_rules::MacroError) -> SourceError {
    use crate::compiler::macro_rules::MacroError;
    match error {
        MacroError::Contract(error) => token_error(error),
        MacroError::Resource(error) => token_error(error.into()),
        MacroError::Unsupported { .. } => failure(
            SourceErrorKind::Unsupported,
            None,
            "unsupported exported macro fragment kind",
        ),
        other => SourceError::syntax(None, other.to_string()),
    }
}
fn name_error(error: crate::compiler::semantic_name::NameError) -> SourceError {
    use crate::compiler::semantic_name::NameError;
    match error {
        NameError::Token(error) => token_error(error),
        NameError::Unsupported(_) => {
            failure(SourceErrorKind::Unsupported, None, &error.to_string())
        }
        other => SourceError::syntax(None, other.to_string()),
    }
}
impl LocalExports {
    #[cfg(test)]
    pub(in crate::compiler) fn root(&self) -> SourceFileHandle {
        self.root
    }
    pub(in crate::compiler) fn names(&self) -> &Arc<NameStore> {
        &self.names
    }

    /// `source` is an actual configured file projection. Each source-issued
    /// record is checked against its retained original file before tokenization.
    /// No function/type body is parsed or admitted here.
    pub(in crate::compiler) fn collect(
        file_key: crate::compiler::keys::SourceFile,
        child_count: usize,
        source: &PreparedSource,
        projection: &PreparedLexInput<'_>,
        budget: super::super::expansion_budget::ExpansionBudget,
        owner: super::super::owner_path::Issued,
        parent: Option<&LocalExports>,
    ) -> Result<Self, SourceError> {
        let (mut session, cap) =
            TokenSession::with_owner(projection.snapshot().clone(), budget, owner)
                .map_err(token_error)?;
        let lexed = super::super::lexer::lex_prepared(projection, source.edition(), &mut session)
            .map_err(|error| match error {
            super::super::lexer::LexError::Projection(error) => error,
            super::super::lexer::LexError::Transport(error) => token_error(error),
            other => SourceError::syntax(None, other.to_string()),
        })?;
        // Group only source-validated original ranges while the token session
        // is exclusively owned here. No mutable builder-session escape exists.
        let mut originals =
            BudgetVec::new(session.budget(), Storage::Temporary).map_err(token_error)?;
        let mut next = 0usize;
        for export in &source.macro_exports {
            let (attribute, item) = export.spans(source)?;
            let file = session
                .sources()
                .file(item.file.0 as usize)
                .map_err(token_error)?;
            let mut events =
                BudgetVec::new(session.budget(), Storage::Temporary).map_err(token_error)?;
            let mut first = None;
            let mut last = None;
            while next < lexed.events().len() {
                session
                    .budget()
                    .charge(Counter::Work, 1)
                    .map_err(|error| token_error(error.into()))?;
                let event = &lexed.events()[next];
                let data = session.span(event.token.span()).map_err(token_error)?;
                let SpanOrigin::Original(original) = data.origin else {
                    next += 1;
                    continue;
                };
                let rendered = session
                    .sources()
                    .render_span(original)
                    .map_err(token_error)?;
                if original.file() == file && rendered.lo >= item.hi {
                    break;
                }
                next += 1;
                if original.file() != file || rendered.hi <= item.lo {
                    continue;
                }
                if rendered.lo < item.lo
                    || rendered.hi > item.hi
                    || data.expansion.is_some()
                    || data.context != session.root_context()
                {
                    return Err(contract("export declaration token escaped original item"));
                }
                let original_text = session.sources().text(file).map_err(token_error)?;
                if original_text.get(rendered.lo as usize..rendered.hi as usize)
                    != projection.text().get(event.prepared.clone())
                {
                    return Err(contract(
                        "export declaration token changed original spelling",
                    ));
                }
                first.get_or_insert(rendered.lo);
                last = Some(rendered.hi);
                events.push(event.token).map_err(token_error)?;
            }
            if first != Some(item.lo) || last != Some(item.hi) {
                return Err(contract("missing original macro export declaration tokens"));
            }
            for doc in lexed.docs() {
                let original = session.original(doc.span, 512).map_err(token_error)?;
                let rendered = session
                    .sources()
                    .render_span(original)
                    .map_err(token_error)?;
                if original.file() == file && item.lo <= rendered.lo && rendered.hi <= item.hi {
                    return Err(failure(
                        SourceErrorKind::Unsupported,
                        Some(rendered),
                        "documentation tokens inside exported macro rules are unsupported",
                    ));
                }
            }
            let stream = super::super::token_tree::group(
                &mut session,
                events.as_slice(),
                super::super::token_tree::GroupPolicy::OriginalSource,
            )
            .map_err(token_error)?;
            let slice = TokenSlice::roots(&session, stream).map_err(token_error)?;
            let attribute = session
                .sources()
                .span(file, attribute.lo, attribute.hi)
                .map_err(token_error)?;
            let attribute = session.original_span(attribute).map_err(token_error)?;
            originals.push((slice, attribute)).map_err(token_error)?;
        }
        let mut names =
            NameStoreBuilder::new(session, cap, source.edition()).map_err(name_error)?;
        if let Some(parent) = parent {
            if file_key.parent().as_ref() != Some(&parent.file) {
                return Err(contract(
                    "export declaration file has another canonical parent",
                ));
            }
            let mut stores = parent.names.session().linked_stores();
            stores.push(parent.names.clone());
            names.link_crate_files(stores).map_err(name_error)?;
            // A child links the canonical crate-root store, not an invented
            // crate identity derived from this file's spelling or ordinal.
            let root = parent
                .names
                .session()
                .linked_stores()
                .into_iter()
                .chain(std::iter::once(parent.names.clone()))
                .find(|store| store.session().id() == parent.names.session().crate_key())
                .ok_or_else(|| contract("missing export crate-root owner"))?;
            names.join_crate(&root).map_err(name_error)?;
        }
        let file = names.session().sources().file(0).map_err(token_error)?;
        let root = names
            .session()
            .sources()
            .source_crate_root(file)
            .map_err(token_error)?;
        let crate_id = match parent {
            Some(parent) if parent.root == root => parent.crate_id,
            Some(_) => return Err(contract("export child changed canonical source crate")),
            None => names.register_crate_root(root).map_err(token_error)?,
        };
        let mut declarations =
            BudgetVec::new(names.budget(), Storage::Retained).map_err(token_error)?;
        // Source records are issued in source order by the same walker that
        // controls active cfg, inline modules, path overrides and child IO.
        for (export, (slice, attribute)) in source.macro_exports.iter().zip(originals.as_slice()) {
            let (_, item) = export.spans(source)?;
            let attribute = *attribute;
            let keyword = leaf(slice, names.session(), 0)?;
            let bang = leaf(slice, names.session(), 1)?;
            let token = leaf(slice, names.session(), 2)?;
            if !word(names.session(), keyword, "macro_rules")?
                || !matches!(bang.kind, TokenKind::Punct { ch: '!', .. })
                || !matches!(token.kind, TokenKind::Ident(_))
            {
                return Err(SourceError::syntax(
                    Some(item),
                    "invalid exported macro_rules declaration".to_owned(),
                ));
            }
            let length = slice.len(names.session()).map_err(token_error)?;
            let Node::Delimited { kind, .. } =
                slice.node(names.session(), 3).map_err(token_error)?
            else {
                return Err(SourceError::syntax(
                    Some(item),
                    "exported macro requires rule body".to_owned(),
                ));
            };
            let braced = *kind == Delimiter::Brace;
            if length != 4
                && !(length == 5
                    && matches!(
                        leaf(slice, names.session(), 4)?.kind,
                        TokenKind::Punct { ch: ';', .. }
                    ))
                || (!braced && length != 5)
            {
                return Err(SourceError::syntax(
                    Some(item),
                    "invalid exported macro delimiter or trailing tokens".to_owned(),
                ));
            }
            let name = names
                .from_token(token, source.edition())
                .map_err(name_error)?;
            let body = slice
                .children(names.session(), 3)
                .map_err(token_error)?
                .ok_or_else(|| contract("missing macro export rule body"))?;
            let site = DefinitionSite {
                span: token.span,
                edition: source.edition(),
                defining_crate: crate_id,
            };
            super::super::source_macro_binding::validate_definition(names.session(), site)
                .map_err(token_error)?;
            let rules = names.compile_local_rules(body, site).map_err(macro_error)?;
            declarations
                .push(Declaration {
                    name,
                    attribute,
                    site,
                    rules,
                })
                .map_err(token_error)?;
        }
        let names = names.freeze().map_err(name_error)?;
        let has_unresolved_imports = file_key.parent().is_none() && source.has_root_use_items;
        Ok(Self {
            file: file_key,
            child_count,
            has_item_macro_invocations: source.has_item_macro_invocations,
            has_unresolved_imports,
            names,
            root,
            crate_id,
            declarations,
        })
    }
}

fn leaf(slice: &TokenSlice, session: &TokenSession, index: usize) -> Result<Token, SourceError> {
    match slice.node(session, index).map_err(token_error)? {
        Node::Leaf(token) => Ok(*token),
        _ => Err(contract(
            "macro export header requires original token leaves",
        )),
    }
}
fn word(session: &TokenSession, token: Token, expected: &str) -> Result<bool, SourceError> {
    match token.kind {
        TokenKind::Ident(ident) if !ident.raw => {
            Ok(session.symbol(ident.symbol).map_err(token_error)? == expected)
        }
        _ => Ok(false),
    }
}
