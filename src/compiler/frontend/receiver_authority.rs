//! Bare `self` keeps its actual token occurrence. Its lexical binding must be
//! the immutable receiver issued for this exact function, including macro marks.
use super::*;

pub(super) fn local(path: &AstPath) -> LocalName {
    match path.receiver_occurrence() {
        Some(occurrence) => LocalName::ReceiverOccurrence(occurrence),
        None => LocalName::Receiver,
    }
}

pub(super) fn key(
    arena: &super::super::semantic_scope::ScopeArena,
    names: &NameStore,
    function: super::super::semantic_scope::ScopeId,
    occurrence: Option<AstIdent>,
    declaration: bool,
) -> Result<LocalKey, super::super::semantic_scope::ScopeError> {
    use super::super::semantic_scope::ScopeError;
    if declaration {
        let canonical = arena.receiver_binding(names, function)?;
        if let Some(occurrence) = occurrence
            && arena.declaration_for(names, occurrence)? != canonical
        {
            return Err(ScopeError::ConflictingAssociation);
        }
    } else {
        arena.check_receiver_reference(
            names,
            function,
            occurrence.ok_or(ScopeError::UnrecordedOccurrence)?,
        )?;
    }
    Ok(LocalKey::Receiver)
}

pub(super) fn validate_kind(
    program: &Program,
    function: &Function,
    receiver: ReceiverKind,
) -> Result<(), String> {
    if let Some(scope) = function.lexical_scope {
        let arena = program
            .scopes
            .as_ref()
            .ok_or_else(|| "receiver has no lexical arena".to_owned())?;
        if arena
            .has_receiver(scope)
            .map_err(type_check::scope_diagnostic)?
            != (receiver != ReceiverKind::None)
        {
            return Err("receiver binder differs from callable receiver kind".into());
        }
        if receiver != ReceiverKind::None {
            arena
                .receiver_binding(&*program.names, scope)
                .map_err(type_check::scope_diagnostic)?;
        }
    }
    Ok(())
}

impl Parser<'_, '_> {
    pub(super) fn receiver_binding_token(&mut self) -> Result<(), ParseError> {
        self.expect_receiver_syntax()?;
        let occurrence = self.input.mint_name(&self.token(), self.names)?;
        self.cursor.bump();
        self.lexical.pending_receiver = Some(occurrence);
        Ok(())
    }

    fn expect_receiver_syntax(&self) -> Result<(), ParseError> {
        if self.is("self") {
            Ok(())
        } else {
            Err(self.error("expected self receiver"))
        }
    }

    pub(super) fn bind_receiver_lexical(
        &mut self,
        receiver: Option<AstIdent>,
    ) -> Result<(), ParseError> {
        if let Some(receiver) = receiver {
            self.bind_lexical(receiver)?;
        }
        Ok(())
    }

    pub(super) fn receiver_expression(&mut self) -> Result<Expr, ParseError> {
        self.expect_receiver_syntax()?;
        let occurrence = self.input.mint_name(&self.token(), self.names)?;
        self.cursor.bump();
        self.reference_lexical(occurrence)?;
        Ok(Expr::Var(
            AstPath::receiver(self.names, occurrence).map_err(native_adapter::name_error)?,
        ))
    }
}
