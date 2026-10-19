//! Original module textual macros. Item expansion is not function expansion.
use super::expansion_budget::{ByteLease, Counter, Storage};
use super::expansion_span::SpanId;
use super::macro_cursor::TokenSlice;
use super::macro_rules::{CallSite, DefinitionSite, ExpandedTokens, RulesDef};
use super::semantic_name::{AstIdent, LookupClass, NameKey, NameStoreBuilder, Namespace};
use super::semantic_scope::{ModuleFrontierId, ScopeArenaBuilder};
use super::source_macro::DriverError;
use super::token_session::{BudgetVec, SessionId, TokenError, allocate_identity};
use super::token_tree::{Node, TokenStream};
use std::num::NonZeroU64;
use std::sync::atomic::AtomicU64;

pub(super) fn group_body(
    source: &TokenSlice,
    open: SpanId,
    names: &NameStoreBuilder,
) -> Result<TokenSlice, TokenError> {
    let session = names.session();
    session.span(open)?;
    let mut pending = BudgetVec::new(names.budget(), Storage::Temporary)?;
    pending.push(source.clone())?;
    while let Some(range) = pending.pop() {
        for index in 0..range.len(session)? {
            names.budget().charge(Counter::Work, 1)?;
            if let Node::Delimited {
                open: actual,
                children,
                ..
            } = range.node(session, index)?
            {
                let child = range.group_children(session, children.clone())?;
                if *actual == open {
                    return Ok(child);
                }
                pending.push(child)?;
            }
        }
    }
    Err(TokenError::InvalidHandle)
}
struct Definition {
    name: NameKey,
    point: ModuleFrontierId,
    rules: RulesDef,
    /// Original rule body, kept so expression-position calls can register
    /// the same rules with a function-local driver.
    body: TokenSlice,
    site: DefinitionSite,
}
#[derive(Clone, Copy)]
pub(super) struct CheckedDef {
    driver: NonZeroU64,
    index: u32,
    definition: ModuleFrontierId,
    call: ModuleFrontierId,
}
impl CheckedDef {
    pub(super) fn definition(self) -> ModuleFrontierId {
        self.definition
    }
    pub(super) fn call(self) -> ModuleFrontierId {
        self.call
    }
}
pub(super) struct SourceItemMacroDriver {
    id: NonZeroU64,
    session: SessionId,
    source: TokenStream,
    definitions: BudgetVec<Definition>,
    _lease: ByteLease,
}
impl SourceItemMacroDriver {
    pub(super) fn new(source: TokenStream, names: &NameStoreBuilder) -> Result<Self, DriverError> {
        source.roots(names.session())?;
        let lease = names
            .budget()
            .reserve(Storage::Retained, std::mem::size_of::<Self>() as u64)?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            id: allocate_identity(&NEXT)?,
            session: names.session().id(),
            source,
            definitions: BudgetVec::new(names.budget(), Storage::Retained)?,
            _lease: lease,
        })
    }
    fn check(&self, names: &NameStoreBuilder) -> Result<(), DriverError> {
        names.budget().charge(Counter::Work, 1)?;
        if self.session != names.session().id() {
            return Err(TokenError::ForeignHandle.into());
        }
        self.source.roots(names.session())?;
        Ok(())
    }
    pub(super) fn define(
        &mut self,
        name: AstIdent,
        body: TokenSlice,
        site: DefinitionSite,
        point: ModuleFrontierId,
        names: &mut NameStoreBuilder,
    ) -> Result<(), DriverError> {
        self.check(names)?;
        if names.session().span(site.span)?.expansion.is_some() {
            return Err(DriverError::Unsupported {
                span: site.span,
                message: "generated macro definitions are unsupported",
            });
        }
        let name = names.key(name, LookupClass::Item(Namespace::Macro))?;
        let rules = names.compile_local_rules(body.clone(), site)?;
        u32::try_from(self.definitions.len()).map_err(|_| TokenError::IdExhausted)?;
        self.definitions.push(Definition {
            name,
            point,
            rules,
            body,
            site,
        })?;
        Ok(())
    }
    pub(super) fn select(
        &self,
        name: AstIdent,
        call: ModuleFrontierId,
        scopes: &ScopeArenaBuilder,
        names: &NameStoreBuilder,
    ) -> Result<CheckedDef, DriverError> {
        self.check(names)?;
        scopes.check_names(names).map_err(DriverError::Scope)?;
        scopes.module_of(call).map_err(DriverError::Scope)?;
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        for (index, definition) in self.definitions.as_slice().iter().enumerate().rev() {
            names.budget().charge(Counter::Work, 1)?;
            if definition.name == key
                && scopes
                    .module_visible_from(definition.point, call)
                    .map_err(DriverError::Scope)?
            {
                return Ok(CheckedDef {
                    driver: self.id,
                    index: u32::try_from(index).map_err(|_| TokenError::IdExhausted)?,
                    definition: definition.point,
                    call,
                });
            }
        }
        Err(super::macro_rules::MacroError::Syntax {
            span: Some(names.span(name)?),
            message: "unknown module macro at this lexical point",
        }
        .into())
    }
    /// The latest module definition of `name` visible at `call`, as its
    /// original rule body and site (for function-local re-registration).
    pub(super) fn visible_body(
        &self,
        name: AstIdent,
        call: ModuleFrontierId,
        scopes: &ScopeArenaBuilder,
        names: &NameStoreBuilder,
    ) -> Result<Option<(TokenSlice, DefinitionSite)>, DriverError> {
        self.check(names)?;
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        for definition in self.definitions.as_slice().iter().rev() {
            names.budget().charge(Counter::Work, 1)?;
            if definition.name == key
                && scopes
                    .module_visible_from(definition.point, call)
                    .map_err(DriverError::Scope)?
            {
                return Ok(Some((definition.body.clone(), definition.site)));
            }
        }
        Ok(None)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.definitions.len() == 0
    }
    pub(super) fn known(
        &self,
        name: AstIdent,
        call: ModuleFrontierId,
        scopes: &ScopeArenaBuilder,
        names: &NameStoreBuilder,
    ) -> Result<bool, DriverError> {
        self.check(names)?;
        scopes.check_names(names).map_err(DriverError::Scope)?;
        scopes.module_of(call).map_err(DriverError::Scope)?;
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        for definition in self.definitions.as_slice() {
            names.budget().charge(Counter::Work, 1)?;
            if definition.name == key
                && scopes
                    .module_visible_from(definition.point, call)
                    .map_err(DriverError::Scope)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn expand(
        &self,
        selected: CheckedDef,
        args: TokenSlice,
        site: CallSite,
        names: &mut NameStoreBuilder,
    ) -> Result<ExpandedTokens, DriverError> {
        self.check(names)?;
        if selected.driver != self.id {
            return Err(TokenError::ForeignHandle.into());
        }
        let definition = self
            .definitions
            .as_slice()
            .get(selected.index as usize)
            .ok_or(TokenError::InvalidHandle)?;
        if definition.point != selected.definition
            || site.parent.is_some()
            || names.session().span(site.span)?.expansion.is_some()
        {
            return Err(TokenError::InvalidHandle.into());
        }
        names
            .expand_local_rules(&definition.rules, args, site)
            .map_err(|error| {
                DriverError::Macro(match error {
                    super::macro_rules::MacroError::NoMatch => {
                        super::macro_rules::MacroError::Syntax {
                            span: Some(site.span),
                            message: "no macro_rules arm matches this invocation",
                        }
                    }
                    other => other,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;
    use crate::compiler::module_identity::ModuleArenaBuilder;
    use crate::compiler::source::PreparedLexInput;
    use crate::compiler::token::{Edition, TokenKind};
    use crate::compiler::token_tree::{GroupPolicy, group};
    use std::path::Path;

    #[test]
    fn selected_rule_remains_committed_and_other_driver_rejects_handle() {
        let budget = ExpansionBudget::default();
        let projection =
            PreparedLexInput::identity(Path::new("items.rs"), "()=>{1};()=>{2};", &budget).unwrap();
        let (mut session, session_cap) = super::super::token_session::TokenSession::standalone(
            projection.snapshot().clone(),
            budget,
        )
        .unwrap();
        let lexed =
            super::super::lexer::lex_prepared(&projection, Edition::E2024, &mut session).unwrap();
        let events: Vec<_> = lexed.events().iter().map(|event| event.token).collect();
        let stream = group(&session, &events, GroupPolicy::OriginalSource).unwrap();
        let span = events[0].span();
        let file = session.sources().file(0).unwrap();
        let defining_crate = session.register_crate_root(file).unwrap();
        let source = TokenSlice::roots(&session, stream.clone()).unwrap();
        let middle = source.len(&session).unwrap() / 2;
        let first = source.subslice(&session, 0..middle).unwrap();
        let second = source
            .subslice(&session, middle..source.len(&session).unwrap())
            .unwrap();
        let mut names = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let name = names.intern_root("make", span).unwrap();
        let modules = ModuleArenaBuilder::for_source(&names).unwrap();
        let module = modules.source_root().unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
        let point = scopes.module_root(&names, &modules, module, None).unwrap();
        let mut driver = SourceItemMacroDriver::new(stream.clone(), &names).unwrap();
        let site = DefinitionSite {
            span,
            edition: Edition::E2024,
            defining_crate,
        };
        driver
            .define(name, first.clone(), site, point, &mut names)
            .unwrap();
        let old = driver.select(name, point, &scopes, &names).unwrap();
        driver
            .define(name, second, site, point, &mut names)
            .unwrap();
        let latest = driver.select(name, point, &scopes, &names).unwrap();
        assert_eq!(old.index, 0);
        assert_eq!(latest.index, 1);
        let args = first.subslice(names.session(), 0..0).unwrap();
        let call = CallSite { span, parent: None };
        for (selected, expected) in [(old, "1"), (latest, "2")] {
            let result = driver
                .expand(selected, args.clone(), call, &mut names)
                .unwrap();
            let output = result.tokens(names.session()).unwrap();
            let Node::Leaf(super::super::token::Token {
                kind: TokenKind::Literal { spelling, .. },
                ..
            }) = output.node(names.session(), 0).unwrap()
            else {
                panic!("expected literal");
            };
            assert_eq!(names.session().symbol(*spelling).unwrap(), expected);
        }
        let other = SourceItemMacroDriver::new(stream, &names).unwrap();
        let nonmatching = first.subslice(names.session(), 0..1).unwrap();
        let error = driver
            .expand(latest, nonmatching, call, &mut names)
            .err()
            .unwrap();
        assert!(
            matches!(error, DriverError::Macro(super::super::macro_rules::MacroError::Syntax { span: Some(actual), .. }) if actual == call.span)
        );
        assert!(matches!(
            other.expand(old, args.clone(), call, &mut names),
            Err(DriverError::Token(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            driver.expand(
                CheckedDef {
                    index: u32::MAX,
                    ..old
                },
                args,
                call,
                &mut names
            ),
            Err(DriverError::Token(TokenError::InvalidHandle))
        ));
    }
}
