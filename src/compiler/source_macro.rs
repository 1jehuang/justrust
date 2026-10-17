//! Bounded parser-driven local macro orchestration over retained native trees.
//! No source printing, relexing, binder authority, or canonical body proof lives here.
use super::expansion_budget::{ByteLease, Counter, Storage};
use super::expansion_span::SpanId;
use super::macro_cursor::TokenSlice;
use super::macro_rules::{CallSite, DefinitionSite, ExpandedTokens, MacroError, RulesDef};
use super::semantic_name::{
    AstIdent, LookupClass, NameError, NameKey, NameStoreBuilder, Namespace,
};
use super::semantic_scope::{FrontierId, ScopeId};
use super::token_session::{BudgetVec, SessionId, TokenError};
use super::token_tree::{Node, TokenStream};

#[derive(Debug)]
pub(super) enum DriverError {
    Token(TokenError),
    Name(NameError),
    Macro(MacroError),
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
pub(super) struct LocalMacroDriver {
    source: TokenStream,
    owner: ScopeId,
    session: SessionId,
    rules: Option<LocalRules>,
    _lease: ByteLease,
}
impl LocalMacroDriver {
    pub(super) fn new(
        source: TokenStream,
        owner: ScopeId,
        names: &NameStoreBuilder,
    ) -> Result<Self, DriverError> {
        source.roots(names.session())?;
        let lease = names
            .budget()
            .reserve(Storage::Retained, std::mem::size_of::<Self>() as u64)?;
        Ok(Self {
            source,
            owner,
            session: names.session().id(),
            rules: None,
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
    pub(super) fn owner(&self) -> ScopeId {
        self.owner
    }
    pub(super) fn definition_frontier(&self) -> Option<FrontierId> {
        self.rules.as_ref().map(|rules| rules.definition)
    }
    /// Find an actual checked group by its opening span, never by printable text
    /// or prepared offsets. The flat temporary walk is charged before expansion.
    pub(super) fn group_body(
        &self,
        open: SpanId,
        names: &NameStoreBuilder,
    ) -> Result<TokenSlice, DriverError> {
        self.check(names)?;
        let session = names.session();
        session.span(open)?;
        let mut pending = BudgetVec::new(names.budget(), Storage::Temporary)?;
        pending.push(self.source.roots(session)?)?;
        while let Some(range) = pending.pop() {
            for id in self.source.children(session, &range)? {
                names.budget().charge(Counter::Work, 1)?;
                if let Node::Delimited {
                    open: actual,
                    children,
                    ..
                } = self.source.node(session, *id)?
                {
                    if *actual == open {
                        return Ok(TokenSlice::new(
                            session,
                            self.source.clone(),
                            children.clone(),
                        )?);
                    }
                    pending.push(children.clone())?;
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
        if self.rules.is_some() {
            return Err(DriverError::Unsupported {
                span: site.span,
                message: "only one function-local macro definition is supported",
            });
        }
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        let rules = names.compile_local_rules(body, site)?;
        if !rules.supports_local_fragments(names.session())? {
            return Err(DriverError::Unsupported {
                span: site.span,
                message: "literal fragments are not yet admitted by source macro client",
            });
        }
        self.rules = Some(LocalRules {
            name: key,
            definition: frontier,
            rules,
        });
        Ok(())
    }
    /// Lexical visibility and function ownership are checked by the parser's
    /// ScopeArena before this call and again when issuing ExpansionAdmission.
    pub(super) fn expand(
        &self,
        name: AstIdent,
        args: TokenSlice,
        site: CallSite,
        _frontier: FrontierId,
        names: &mut NameStoreBuilder,
    ) -> Result<ExpandedTokens, DriverError> {
        self.check(names)?;
        let key = names.key(name, LookupClass::Item(Namespace::Macro))?;
        let Some(rules) = &self.rules else {
            return Err(MacroError::Syntax {
                span: Some(site.span),
                message: "unknown function-local macro",
            }
            .into());
        };
        if rules.name != key {
            return Err(MacroError::Syntax {
                span: Some(site.span),
                message: "unknown function-local macro",
            }
            .into());
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
