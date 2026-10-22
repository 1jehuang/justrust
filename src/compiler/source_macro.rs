//! Bounded parser-driven local macro orchestration over retained native trees.
//! No source printing, relexing, binder authority, or canonical body proof lives here.
use super::expansion_budget::{ByteLease, Counter, Storage};
use super::expansion_span::SpanId;
use super::macro_cursor::TokenSlice;
use super::macro_rules::{CallSite, DefinitionSite, ExpandedTokens, MacroError, RulesDef};
use super::semantic_name::{
    AstIdent, LookupClass, NameError, NameKey, NameStoreBuilder, Namespace,
};
use super::semantic_scope::{FrontierId, ScopeArenaBuilder, ScopeId};
use super::token_session::{BudgetVec, SessionId, TokenError};
use super::token_tree::{Node, TokenStream};
use std::num::NonZeroU64;

#[derive(Debug)]
pub(super) enum DriverError {
    Token(TokenError),
    Name(NameError),
    Macro(MacroError),
    Scope(super::semantic_scope::ScopeError),
    Unsupported { span: SpanId, message: &'static str },
}
impl From<TokenError> for DriverError {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}
impl From<NameError> for DriverError {
    fn from(error: NameError) -> Self {
        Self::Name(error)
    }
}
impl From<MacroError> for DriverError {
    fn from(error: MacroError) -> Self {
        Self::Macro(error)
    }
}
impl From<super::expansion_budget::BudgetFailure> for DriverError {
    fn from(error: super::expansion_budget::BudgetFailure) -> Self {
        Self::Token(TokenError::Budget(error))
    }
}

struct LocalRules {
    name: NameKey,
    definition: FrontierId,
    rules: RulesDef,
}
/// Issued by one function's append-only driver. It fixes the selected rule and
/// lexical points before expansion, rather than redoing textual lookup later.
#[derive(Clone, Copy)]
pub(super) struct CheckedDef {
    driver: NonZeroU64,
    owner: ScopeId,
    index: u32,
    definition: FrontierId,
    call: FrontierId,
}
impl CheckedDef {
    pub(super) fn definition(self) -> FrontierId {
        self.definition
    }
    pub(super) fn call(self) -> FrontierId {
        self.call
    }
}
pub(super) struct LocalMacroDriver {
    id: NonZeroU64,
    source: TokenStream,
    owner: ScopeId,
    session: SessionId,
    rules: BudgetVec<LocalRules>,
    _lease: ByteLease,
}
impl LocalMacroDriver {
    pub(super) fn new(
        source: TokenStream,
        owner: ScopeId,
        names: &mut NameStoreBuilder,
    ) -> Result<Self, DriverError> {
        source.roots(names.session())?;
        let lease = names
            .budget()
            .reserve(Storage::Retained, std::mem::size_of::<Self>() as u64)?;
        Ok(Self {
            id: names.issue_driver_id()?,
            source,
            owner,
            session: names.session().id(),
            rules: BudgetVec::new(names.budget(), Storage::Retained)?,
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
    /// Find an actual checked group by its opening span, never by printable text
    /// or prepared offsets. The flat temporary walk is charged before expansion.
    pub(super) fn group_body(
        &self,
        open: SpanId,
        names: &NameStoreBuilder,
    ) -> Result<TokenSlice, DriverError> {
        let source = TokenSlice::roots(names.session(), self.source.clone())?;
        self.group_body_in(&source, open, names)
    }
    pub(super) fn group_body_in(
        &self,
        source: &TokenSlice,
        open: SpanId,
        names: &NameStoreBuilder,
    ) -> Result<TokenSlice, DriverError> {
        self.check(names)?;
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
                    if *actual == open {
                        return Ok(range.group_children(session, children.clone())?);
                    }
                    pending.push(range.group_children(session, children.clone())?)?;
                }
            }
        }
        Err(TokenError::InvalidHandle.into())
    }
    pub(super) fn define(
        &mut self,
        name: AstIdent,
        body: TokenSlice,
        site: DefinitionSite,
        frontier: FrontierId,
        names: &mut NameStoreBuilder,
    ) -> Result<(), DriverError> {
        self.check(names)?;
        if names.session().span(site.span)?.expansion.is_some() {
            return Err(DriverError::Unsupported {
                span: site.span,
                message: "generated macro definitions are unsupported",
            });
        }
        super::source_macro_binding::validate_definition(names.session(), site)?;
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        let rules = names.compile_local_rules(body, site)?;
        u32::try_from(self.rules.len()).map_err(|_| TokenError::IdExhausted)?;
        self.rules.push(LocalRules {
            name: key,
            definition: frontier,
            rules,
        })?;
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn select(
        &self,
        name: AstIdent,
        call: FrontierId,
        scopes: &ScopeArenaBuilder,
        names: &NameStoreBuilder,
    ) -> Result<CheckedDef, DriverError> {
        let span = names.span(name)?;
        self.lookup(name, call, scopes, names)?
            .ok_or(DriverError::Macro(MacroError::Syntax {
                span: Some(span),
                message: "unknown function-local macro at this lexical point",
            }))
    }
    /// Absence is distinct from invalid provenance, scope, or expansion errors.
    pub(super) fn lookup(
        &self,
        name: AstIdent,
        call: FrontierId,
        scopes: &ScopeArenaBuilder,
        names: &NameStoreBuilder,
    ) -> Result<Option<CheckedDef>, DriverError> {
        self.check(names)?;
        scopes.check_names(names).map_err(DriverError::Scope)?;
        if scopes.function(call).map_err(DriverError::Scope)? != self.owner {
            return Err(TokenError::ForeignHandle.into());
        }
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        for (index, rules) in self.rules.as_slice().iter().enumerate().rev() {
            names.budget().charge(Counter::Work, 1)?;
            if rules.name == key
                && scopes
                    .visible_from(rules.definition, call)
                    .map_err(DriverError::Scope)?
            {
                return Ok(Some(CheckedDef {
                    driver: self.id,
                    owner: self.owner,
                    index: u32::try_from(index).map_err(|_| TokenError::IdExhausted)?,
                    definition: rules.definition,
                    call,
                }));
            }
        }
        Ok(None)
    }
    /// Lexical visibility and function ownership are checked by the parser's
    /// ScopeArena before this call and again when issuing ExpansionAdmission.
    pub(super) fn expand(
        &self,
        selected: CheckedDef,
        args: TokenSlice,
        site: CallSite,
        names: &mut NameStoreBuilder,
    ) -> Result<ExpandedTokens, DriverError> {
        self.check(names)?;
        if selected.owner != self.owner || selected.driver != self.id {
            return Err(TokenError::ForeignHandle.into());
        }
        let rules = self
            .rules
            .as_slice()
            .get(selected.index as usize)
            .ok_or(TokenError::InvalidHandle)?;
        if selected.definition != rules.definition {
            return Err(TokenError::InvalidHandle.into());
        }
        names
            .expand_local_rules(&rules.rules, args, site)
            .map_err(|error| {
                DriverError::Macro(match error {
                    MacroError::NoMatch => MacroError::Syntax {
                        span: Some(site.span),
                        message: "no macro rule matched",
                    },
                    MacroError::Ambiguous { span: None } => MacroError::Ambiguous {
                        span: Some(site.span),
                    },
                    other => other,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::{BudgetResource, ExpansionBudget};
    use crate::compiler::source::PreparedLexInput;
    use crate::compiler::token::Edition;
    use crate::compiler::token_tree::{GroupPolicy, group};
    use std::path::Path;

    struct Fixture {
        names: NameStoreBuilder,
        scopes: ScopeArenaBuilder,
        driver: LocalMacroDriver,
        first: TokenSlice,
        second: TokenSlice,
        site: DefinitionSite,
        name: AstIdent,
        at: FrontierId,
    }
    fn fixture() -> Fixture {
        let budget = ExpansionBudget::default();
        let projection =
            PreparedLexInput::identity(Path::new("rules.rs"), "()=>{1};()=>{2};", &budget).unwrap();
        let (mut session, session_cap) = super::super::token_session::TokenSession::standalone(
            projection.snapshot().clone(),
            budget,
        )
        .unwrap();
        let lexed =
            super::super::lexer::lex_prepared(&projection, Edition::E2024, &mut session).unwrap();
        let events: Vec<_> = lexed.events().iter().map(|event| event.token).collect();
        let stream = group(&mut session, &events, GroupPolicy::OriginalSource).unwrap();
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
        let name = names.intern_root("value", span).unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
        let at = scopes.function_root(&names).unwrap();
        let owner = scopes.function(at).unwrap();
        let driver = LocalMacroDriver::new(stream, owner, &mut names).unwrap();
        Fixture {
            names,
            scopes,
            driver,
            first,
            second,
            site: DefinitionSite {
                span,
                edition: Edition::E2024,
                defining_crate,
            },
            name,
            at,
        }
    }
    #[test]
    fn selected_definition_is_committed_and_foreign_driver_handles_reject() {
        let mut f = fixture();
        f.driver
            .define(f.name, f.first.clone(), f.site, f.at, &mut f.names)
            .unwrap();
        let selected = f.driver.select(f.name, f.at, &f.scopes, &f.names).unwrap();
        f.driver
            .define(f.name, f.second.clone(), f.site, f.at, &mut f.names)
            .unwrap();
        let latest = f.driver.select(f.name, f.at, &f.scopes, &f.names).unwrap();
        assert_eq!(selected.index, 0);
        assert_eq!(latest.index, 1);
        let args = f.first.subslice(f.names.session(), 0..0).unwrap();
        let site = CallSite {
            span: f.site.span,
            parent: None,
        };
        let old_output = f
            .driver
            .expand(selected, args.clone(), site, &mut f.names)
            .unwrap();
        let new_output = f
            .driver
            .expand(latest, args.clone(), site, &mut f.names)
            .unwrap();
        for (output, expected) in [(old_output, "1"), (new_output, "2")] {
            let output = output.tokens(f.names.session()).unwrap();
            let Node::Leaf(super::super::token::Token {
                kind: super::super::token::TokenKind::Literal { spelling, .. },
                ..
            }) = output.node(f.names.session(), 0).unwrap()
            else {
                panic!("expected literal");
            };
            assert_eq!(f.names.session().symbol(*spelling).unwrap(), expected);
        }
        let other =
            LocalMacroDriver::new(f.driver.source.clone(), f.driver.owner, &mut f.names).unwrap();
        assert!(matches!(
            other.expand(selected, args.clone(), site, &mut f.names),
            Err(DriverError::Token(TokenError::ForeignHandle))
        ));
        let invalid = CheckedDef {
            index: u32::MAX,
            ..selected
        };
        assert!(matches!(
            f.driver.expand(invalid, args, site, &mut f.names),
            Err(DriverError::Token(TokenError::InvalidHandle))
        ));
    }
    #[test]
    fn reverse_search_exhaustion_is_resource_not_no_match() {
        let mut f = fixture();
        for _ in 0..64 {
            f.driver
                .define(f.name, f.first.clone(), f.site, f.at, &mut f.names)
                .unwrap();
        }
        let absent = f.names.intern_root("absent", f.site.span).unwrap();
        let budget = f.names.budget().clone();
        let remaining = budget.limits().counters[Counter::Work as usize]
            - budget.snapshot().counters[Counter::Work as usize];
        budget.charge(Counter::Work, remaining - 8).unwrap();
        assert!(matches!(
            f.driver.select(absent, f.at, &f.scopes, &f.names),
            Err(DriverError::Token(TokenError::Budget(_)))
                | Err(DriverError::Scope(_))
                | Err(DriverError::Name(NameError::Token(TokenError::Budget(_))))
        ));
        assert_eq!(
            budget.snapshot().failure.unwrap().resource,
            BudgetResource::Counter(Counter::Work)
        );
        drop(f);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
}
