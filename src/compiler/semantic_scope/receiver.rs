//! Canonical receiver binders, retained in immutable function-root scope rows.
//! Ordinary lexical lookup still decides hygiene. A receiver use must resolve
//! to this exact binder, never merely to a token spelled `self`.
use super::*;

impl ScopeArenaBuilder {
    /// A module definition has no function-local bindings. Import selection has
    /// already authenticated its module frontier. Keep expansion ownership in
    /// the calling function, but never substitute its current lexical bindings
    /// for the module definition's empty local environment.
    pub(in crate::compiler) fn module_definition_frontier(
        &mut self,
        origin: super::super::source_item_macro::CheckedDef,
        call: FrontierId,
    ) -> Result<FrontierId, ScopeError> {
        if !self.module_visible_from(origin.definition(), origin.call())? {
            return Err(ScopeError::ConflictingAssociation);
        }
        let function = self.function(call)?;
        self.arena.transition()?;
        self.arena.push_frontier(FrontierRecord {
            scope: function,
            head: None,
        })
    }

    /// An exported module definition has an empty function-local environment.
    /// Only a selection bound to the actual invocation and canonical source
    /// crate may authenticate this otherwise invisible definition frontier.
    pub(in crate::compiler) fn exported_definition_frontier(
        &mut self,
        selection: &super::super::source_item_macro::exports::ExportSelection<'_>,
        names: &super::super::semantic_name::NameStoreBuilder,
        span: super::super::expansion_span::SpanId,
        call: FrontierId,
    ) -> Result<FrontierId, ScopeError> {
        self.check_names(names)?;
        selection
            .check_call(names, span)
            .map_err(|error| match error {
                super::super::source_macro::DriverError::Token(error) => ScopeError::Token(error),
                super::super::source_macro::DriverError::Name(error) => ScopeError::Name(error),
                super::super::source_macro::DriverError::Scope(error) => error,
                _ => ScopeError::ConflictingAssociation,
            })?;
        let function = self.function(call)?;
        self.arena.transition()?;
        self.arena.push_frontier(FrontierRecord {
            scope: function,
            head: None,
        })
    }

    pub(crate) fn receiver_function_root(
        &mut self,
        names: &impl NameAccess,
        receiver: Option<AstIdent>,
    ) -> Result<FrontierId, ScopeError> {
        self.check_names(names)?;
        if let Some(receiver) = receiver {
            names.check(receiver)?;
            if names.spelling(receiver)? != "self" {
                return Err(ScopeError::InvalidHandle);
            }
        }
        self.arena.transition()?;
        let scope = ScopeId {
            owner: self.arena.owner,
            index: self.arena.index(self.arena.scopes.len())?,
        };
        self.arena.scopes.push(ScopeRecord {
            parent: None,
            function: scope,
            receiver,
        })?;
        self.arena
            .push_frontier(FrontierRecord { scope, head: None })
    }
}

impl ScopeArena {
    pub(crate) fn has_receiver(&self, function: ScopeId) -> Result<bool, ScopeError> {
        self.check_function(function)?;
        Ok(self.scope(function)?.receiver.is_some())
    }

    pub(crate) fn receiver_binding(
        &self,
        names: &impl NameAccess,
        function: ScopeId,
    ) -> Result<BindingId, ScopeError> {
        self.check_names(names)?;
        self.check_function(function)?;
        let receiver = self
            .scope(function)?
            .receiver
            .ok_or(ScopeError::InvalidHandle)?;
        let binding = self.declaration_for(names, receiver)?;
        if self.binding_function(binding)? != function {
            return Err(ScopeError::FunctionMismatch);
        }
        Ok(binding)
    }

    pub(crate) fn check_receiver_reference(
        &self,
        names: &impl NameAccess,
        function: ScopeId,
        occurrence: AstIdent,
    ) -> Result<(), ScopeError> {
        names.check(occurrence)?;
        if names.spelling(occurrence)? != "self"
            || self.reference_function(names, occurrence)? != function
            || self.reference_for(names, occurrence)?
                != Some(self.receiver_binding(names, function)?)
        {
            return Err(ScopeError::ConflictingAssociation);
        }
        Ok(())
    }
}
