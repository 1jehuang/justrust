//! Statement macro output belongs to the invoking block, not a new scope.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn try_macro_statements(&mut self, end: &str) -> Result<Option<Block>, ParseError> {
        if self.purpose != ParserPurpose::Active || self.item_expansion.is_some() {
            return Ok(None);
        }
        let qualified = self.qualified_macro_ahead()?;
        if !qualified {
            if !self.is_identifier() || !self.cursor.peek(1).is_some_and(|token| token.matches("!"))
            {
                return Ok(None);
            }
            // Builtins keep their own checked expression consumers. A user
            // binding shadowing a builtin still takes the ordinary driver.
            if self.builtin_macro_ahead(self.token().text)? {
                return Ok(None);
            }
        }
        let mut probe = TokenCursor::new(self.input);
        if !probe.restore(self.cursor.checkpoint()) {
            return Err(self.error("invalid statement macro cursor"));
        }
        while !probe.current().matches("!") {
            self.statement_macro_probe_work()?;
            if probe.current().matches("") {
                return Ok(None);
            }
            probe.bump();
        }
        probe.bump();
        let braced = probe.current().matches("{");
        let mut depth = 0usize;
        loop {
            self.statement_macro_probe_work()?;
            let token = probe.bump();
            if token.matches("") {
                return Err(self.error("unterminated statement macro invocation"));
            }
            if token.matches("(") || token.matches("[") || token.matches("{") {
                depth += 1;
                if depth > 128 {
                    return Err(self.limit("statement macro group nesting exceeds limit 128"));
                }
            } else if token.matches(")") || token.matches("]") || token.matches("}") {
                if depth == 0 {
                    return Err(self.error("invalid statement macro invocation group"));
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
        let after = probe.current();
        let continued = [
            ".", "?", "(", "[", "::", "+", "-", "*", "/", "%", "&", "|", "^", "&&", "||", "==",
            "!=", "<", ">", "<=", ">=", "<<", ">>", "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=",
            "^=", "<<=", ">>=", "as", "..", "..=",
        ]
        .iter()
        .any(|spelling| after.matches(spelling));
        if !after.matches(";") && !after.matches(end) && (!braced || continued) {
            return Ok(None);
        }
        let (buffer, admission, depth) = if qualified {
            self.prepare_exported_macro()?
        } else {
            self.prepare_local_macro()?
        };
        // Borrow the actual lexical state just like an expression expansion.
        // In particular, do not enter/leave a block around generated lets:
        // their destructors must run at the enclosing source block's exit.
        self.isolated_parse_at(&buffer, Some(admission), depth, |child| {
            child.block_contents("")
        })
        .map(Some)
    }

    fn statement_macro_probe_work(&self) -> Result<(), ParseError> {
        self.names
            .budget()
            .charge(super::super::expansion_budget::Counter::Work, 1)
            .map_err(|error| native_adapter::transport_error(error.into()))
    }
}
