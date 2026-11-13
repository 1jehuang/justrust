//! Macro invocations in associated-item position (`impl`/`trait` bodies).
//! Disabled code only records them as unexpanded syntax, like module items.
//! Active associated-item macros are not expanded yet and stay explicit.
use super::*;

impl Parser<'_, '_> {
    /// Consumes `name! (...)` / `name! [...];` / `name! { ... }` when the
    /// current tokens start an associated-item macro invocation.
    pub(super) fn associated_macro_item(&mut self) -> Result<bool, ParseError> {
        if !self.token().is_ident() || !self.cursor.peek(1).is_some_and(|t| t.matches("!")) {
            return Ok(false);
        }
        if self.purpose != ParserPurpose::InactiveSyntax {
            return Err(self.unsupported("associated item macro invocations are unsupported"));
        }
        let span = self.token().span;
        let name = self.ident()?;
        self.expect("!")?;
        let path = AstPath::new(self.names, PathAnchor::Relative, &[name])
            .map_err(native_adapter::name_error)?;
        let braced = self.is("{");
        let syntax = self.inactive_invocation(path, span)?;
        if braced {
            self.eat(";");
        } else {
            self.expect(";")?;
        }
        self.lexical
            .inactive_items
            .as_mut()
            .ok_or_else(|| {
                native_adapter::transport_error(
                    super::super::token_session::TokenError::InvalidHandle,
                )
            })?
            .push(syntax)
            .map_err(native_adapter::transport_error)?;
        Ok(true)
    }
}
