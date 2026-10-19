use super::*;

impl<'input> Parser<'input, '_> {
    /// Keep the substantial declaration temporaries off the recursive block frame.
    #[inline(never)]
    pub(super) fn let_statement(&mut self) -> Result<Stmt, ParseError> {
        let pattern = self.pattern_no_alternatives()?;
        if self.is("|") {
            return Err(self.error("top-level or-patterns are not allowed in let bindings; wrap the pattern in parentheses"));
        }
        let annotation = if self.eat(":") {
            Some(self.ty()?)
        } else {
            None
        };
        if self.eat(";") {
            return self.deferred_let(pattern, annotation);
        }
        self.expect("=")?;
        let initializer = self.cursor.checkpoint();
        let value = self.expr(0)?;
        self.finish_let(pattern, annotation, initializer, value)
    }

    #[inline(never)]
    fn deferred_let(
        &mut self,
        pattern: Pattern,
        annotation: Option<Type>,
    ) -> Result<Stmt, ParseError> {
        if matches!(
            pattern,
            Pattern::Bool(_) | Pattern::Int { .. } | Pattern::Char(_) | Pattern::Range { .. }
        ) {
            return Err(self.error("refutable literal pattern in let binding"));
        }
        let Pattern::Binding {
            name,
            mutable,
            by_ref: None,
        } = pattern
        else {
            return Err(self.unsupported("deferred pattern declarations are unsupported"));
        };
        let ty = annotation.unwrap_or(Type::Infer);
        self.bind_lexical(name)?;
        Ok(Stmt::Declare {
            name: BindingName::Named(name),
            ty,
            mutable,
        })
    }

    #[inline(never)]
    fn finish_let(
        &mut self,
        pattern: Pattern,
        annotation: Option<Type>,
        initializer: token_cursor::CursorMark<'input>,
        value: Expr,
    ) -> Result<Stmt, ParseError> {
        if self.is("else") {
            self.validate_let_else_initializer(initializer)?;
            self.cursor.bump();
            // The pattern is not in scope in the initializer or diverging block.
            let else_block = self.block()?;
            self.expect(";")?;
            let pattern = self
                .names
                .try_parser_box(pattern)
                .map_err(native_adapter::name_error)?;
            let value = self
                .names
                .try_parser_box(value)
                .map_err(native_adapter::name_error)?;
            self.bind_pattern_lexical(&pattern)?;
            return Ok(Stmt::LetElse {
                pattern,
                ty: annotation.unwrap_or(Type::Infer),
                value,
                else_block,
            });
        }
        if matches!(
            pattern,
            Pattern::Bool(_) | Pattern::Int { .. } | Pattern::Char(_) | Pattern::Range { .. }
        ) {
            return Err(self.error("refutable literal pattern in let binding"));
        }
        self.expect(";")?;
        self.bind_pattern_lexical(&pattern)?;
        let ty = annotation.unwrap_or(Type::Infer);
        if let Pattern::Binding {
            name,
            mutable,
            by_ref: None,
        } = pattern
        {
            Ok(Stmt::Let {
                name: BindingName::Named(name),
                mutable,
                ty,
                value,
            })
        } else {
            Ok(Stmt::LetPattern { pattern, ty, value })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse;

    #[test]
    fn nested_declarations_preserve_scope_and_tail_values() {
        let source = format!(
            "fn main(){{let x={}1{};}}",
            "{let x=".repeat(12),
            ";x}".repeat(12),
        );
        parse(&source).expect("bounded nested let initializers parse");
    }

    #[test]
    fn nested_declaration_depth_is_rejected_on_an_ordinary_stack() {
        let source = format!(
            "fn main(){{let x={}1{};}}",
            "{let x=".repeat(300),
            ";x}".repeat(300),
        );
        let error = parse(&source).expect_err("depth must remain bounded");
        assert!(error.contains("nesting"), "{error}");
    }
}
