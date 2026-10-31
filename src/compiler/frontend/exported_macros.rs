//! Qualified exported calls use declaration completeness, never textual scope.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn qualified_macro_ahead(&self) -> Result<bool, ParseError> {
        self.qualified_macro_at(&self.cursor)
    }

    /// A namespace being available does not make builtin macros or unary `!`
    /// require a local macro driver. Only a genuine qualified invocation does.
    pub(super) fn qualified_macro_at(&self, cursor: &TokenCursor<'_>) -> Result<bool, ParseError> {
        let mut probe = TokenCursor::new(self.input);
        if !probe.restore(cursor.checkpoint()) {
            return Err(native_adapter::transport_error(
                super::super::token_session::TokenError::InvalidHandle,
            ));
        }
        if !probe.current().is_ident() && probe.current().dollar_crate().is_none() {
            return Ok(false);
        }
        probe.bump();
        let mut qualified = false;
        for _ in 0..128 {
            self.names
                .budget()
                .charge(super::super::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            if !probe.current().matches("::") {
                return Ok(qualified && probe.current().matches("!"));
            }
            qualified = true;
            probe.bump();
            if !probe.current().is_ident() {
                return Ok(false);
            }
            probe.bump();
        }
        Err(self.limit("qualified macro path depth resource limit 128"))
    }

    pub(super) fn prepare_exported_macro(
        &mut self,
    ) -> Result<
        (
            NativeTokenBuffer,
            super::super::semantic_scope::ExpansionAdmission,
            u32,
        ),
        ParseError,
    > {
        let path = self.path()?;
        let name = path
            .segments()
            .last()
            .copied()
            .ok_or_else(|| self.error("exported macro name missing"))?;
        let span = self.names.span(name).map_err(native_adapter::name_error)?;
        self.expect("!")?;
        let args = self.macro_group()?;
        let depth = self
            .expansion_depth
            .checked_add(1)
            .ok_or_else(|| self.limit("macro expansion depth overflow"))?;
        self.names
            .budget()
            .check_depth(super::super::expansion_budget::Depth::Expansion, depth)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let parent = match &self.expansion {
            Some(proof) => {
                proof
                    .validate_span(self.names.session(), span)
                    .map_err(scope_parse_error)?;
                Some(proof.expansion(self.names).map_err(scope_parse_error)?)
            }
            None => None,
        };
        let namespace = self
            .lexical
            .exports
            .clone()
            .ok_or_else(|| self.unsupported("exported macro namespace unavailable"))?;
        let selected = namespace
            .select(&path, self.names)
            .map_err(|error| self.driver_error(error))?
            .ok_or_else(|| self.error("no exported macro at this crate-root path"))?;
        let call = self
            .lexical
            .function
            .as_ref()
            .ok_or_else(|| self.unsupported("exported macro requires a function context"))?
            .frontier;
        let definition = self
            .lexical
            .scopes
            .as_mut()
            .ok_or_else(|| {
                native_adapter::transport_error(
                    super::super::token_session::TokenError::InvalidHandle,
                )
            })?
            .exported_definition_frontier(&selected, self.names, span, call)
            .map_err(scope_parse_error)?;
        let expanded = selected
            .expand(
                self.names,
                args,
                super::super::macro_rules::CallSite { span, parent },
            )
            .map_err(|error| self.driver_error(error))?;
        let expansion = expanded.expansion(self.names.session()).map_err(|error| {
            self.driver_error(super::super::source_macro::DriverError::Macro(error))
        })?;
        let slice = expanded.tokens(self.names.session()).map_err(|error| {
            self.driver_error(super::super::source_macro::DriverError::Macro(error))
        })?;
        let scopes = self.lexical.scopes.as_mut().ok_or_else(|| {
            native_adapter::transport_error(super::super::token_session::TokenError::InvalidHandle)
        })?;
        let admission = match &self.expansion {
            Some(parent) => {
                scopes.anchor_nested_expansion(self.names, expansion, definition, call, parent)
            }
            None => scopes.anchor_expansion(self.names, expansion, definition, call),
        }
        .map_err(scope_parse_error)?;
        let eof = self
            .names
            .expansion_eof(&admission)
            .map_err(scope_parse_error)?;
        let buffer =
            expanded_adapter::lower(slice, self.names, self.names.edition(), eof, &admission)?;
        Ok((buffer, admission, depth))
    }
}
