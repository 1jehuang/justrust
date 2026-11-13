//! Macro invocations in disabled item positions: module items with a path
//! (`a::b! { .. }`) and associated items (`impl`/`trait` bodies). Disabled
//! code only records them as unexpanded syntax. Active associated-item macros
//! are not expanded yet and stay explicit.
use super::*;

impl Parser<'_, '_> {
    /// Length of `name` / `a::b` / `::a::b` followed by `!`, if present.
    fn macro_path_ahead(&self) -> Option<usize> {
        let mut at = 0;
        if self.cursor.peek(0).is_some_and(|t| t.matches("::")) {
            at = 1;
        }
        loop {
            if !self.cursor.peek(at).is_some_and(|t| t.is_ident()) {
                return None;
            }
            at += 1;
            match self.cursor.peek(at) {
                Some(t) if t.matches("!") => return Some(at),
                Some(t) if t.matches("::") && at < 256 => at += 1,
                _ => return None,
            }
        }
    }

    /// Consumes `name! (...)` / `name! [...];` / `name! { ... }` when the
    /// current tokens start an associated-item macro invocation.
    pub(super) fn associated_macro_item(&mut self) -> Result<bool, ParseError> {
        if self.macro_path_ahead().is_none() {
            return Ok(false);
        }
        if self.purpose != ParserPurpose::InactiveSyntax {
            return Err(self.unsupported("associated item macro invocations are unsupported"));
        }
        self.inactive_path_macro_item()?;
        Ok(true)
    }

    /// A disabled module item `a::b! ...` with a qualified path.
    pub(super) fn inactive_qualified_macro_item(&mut self) -> Result<bool, ParseError> {
        if self.purpose != ParserPurpose::InactiveSyntax {
            return Ok(false);
        }
        match self.macro_path_ahead() {
            Some(bang) if bang > 1 => {
                self.inactive_path_macro_item()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn inactive_path_macro_item(&mut self) -> Result<(), ParseError> {
        let span = self.token().span;
        let path = self.path()?;
        self.expect("!")?;
        let braced = self.is("{");
        let syntax = self.inactive_invocation(path, span)?;
        if braced {
            self.eat(";");
        } else if !self.eat(";") {
            return Err(self.error(
                "macros that expand to items must be delimited with braces or followed by a semicolon",
            ));
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
        Ok(())
    }
}
