//! `panic!("fmt {}", args)` (and unreachable!/todo!/unimplemented! with
//! arguments): the diagnostic header and the formatted message are written to
//! stderr by an ordinary native format output, then a preformatted abort
//! (no further diagnostic text) terminates the process.
use super::*;

impl Parser<'_, '_> {
    /// True when the macro's first argument is a string literal carrying
    /// placeholders or followed by format arguments.
    /// Editions 2015/2018: a panic-family macro given exactly one string
    /// literal uses it verbatim as the payload (no format parsing).
    /// `skip` is 1 when a leading separator comma precedes the literal.
    pub(super) fn legacy_panic_literal(
        &mut self,
        close: &str,
        skip: usize,
    ) -> Result<Option<String>, ParseError> {
        if !self.names.edition().pre_2021() {
            return Ok(None);
        }
        let Some(token) = self.cursor.peek(skip) else {
            return Ok(None);
        };
        if !token.string || token.is_opaque_literal() {
            return Ok(None);
        }
        let single = match self.cursor.peek(skip + 1) {
            Some(t) if t.text == close => true,
            Some(t) if t.text == "," => self.cursor.peek(skip + 2).is_some_and(|t| t.text == close),
            _ => false,
        };
        if !single {
            return Ok(None);
        }
        for _ in 0..skip {
            self.cursor.bump();
        }
        let token = self.token();
        let text = self.string_literal_value(token.text)?;
        self.cursor.bump();
        self.eat(",");
        Ok(Some(text))
    }

    pub(super) fn panic_has_format(&mut self) -> bool {
        let token = self.token();
        // `concat!` in format position always goes through format parsing
        // (rustc expands it eagerly, then treats it as the format string).
        if token.is_ident()
            && token.text == "concat"
            && self.cursor.peek(1).is_some_and(|t| t.matches("!"))
            && self.builtin_macro_ahead("concat").unwrap_or(false)
        {
            return true;
        }
        if !token.string {
            return false;
        }
        if self.cursor.peek(1).is_some_and(|t| t.text == ",")
            && self
                .cursor
                .peek(2)
                .is_some_and(|t| !matches!(t.text, ")" | "]" | "}"))
        {
            return true;
        }
        token.text.contains('{') || token.text.contains('}')
    }

    pub(super) fn formatted_panic(
        &mut self,
        close: &str,
        spelling: &str,
        default: &str,
        site: crate::compiler::expansion_span::SpanId,
    ) -> Result<Expr, ParseError> {
        let (path, line, column) = {
            let session = self.names.session();
            let original = session
                .original(
                    panic_callsite(session, site).map_err(native_adapter::transport_error)?,
                    512,
                )
                .map_err(native_adapter::transport_error)?;
            let (path, line, column) = session
                .sources()
                .location(original)
                .map_err(native_adapter::transport_error)?;
            (path.display().to_string(), line, column)
        };
        let mut header = format!(") panicked at {path}:{line}:{column}:\n");
        if spelling != "panic" {
            header.push_str(default);
            header.push_str(": ");
        }
        let text = if self.at_builtin_concat()? {
            let text = self.eager_concat(0)?;
            let lone = self.is(close)
                || self.is(",") && self.cursor.peek(1).is_some_and(|t| t.text == close);
            if self.names.edition().pre_2021() && lone {
                // 2015/2018 panic! takes a lone non-literal payload verbatim
                // (no format parsing); keep that form explicit.
                return Err(self.unsupported(
                    "a lone concat! panic payload before edition 2021 is unsupported",
                ));
            }
            text
        } else {
            let token = self.token();
            let text = self.string_literal_value(token.text)?;
            self.cursor.bump();
            text
        };
        let (mut pieces, holders) =
            format_spec::parse_format(&text).map_err(|message| self.error(&message))?;
        let (mut arguments, mut modes, mut specs, counts) =
            self.format_arguments(close, &holders)?;
        if counts.iter().any(|(w, p)| w.is_some() || p.is_some()) {
            return Err(self.unsupported(
                "width and precision arguments (`$`, `.*`) in panic messages are unsupported",
            ));
        }
        if holders.is_empty() && !arguments.is_empty() {
            return Err(self.error("argument never used"));
        }
        header.push_str(&pieces[0]);
        pieces[0] = header;
        // The native program supplies its own process identity. This expression
        // is generated by the panic builtin, not an unsafe operation authorized
        // for the surrounding user's code.
        let syscall = Expr::NativeSyscall(
            [39u128, 0, 0, 0, 0, 0, 0]
                .into_iter()
                .map(|magnitude| {
                    Expr::Integer(IntegerLiteral {
                        magnitude,
                        suffix: Some(ir::IntTy::Usize),
                    })
                })
                .collect(),
        );
        let pid = Expr::Unsafe(Block {
            statements: Vec::new(),
            tail: Some(
                self.names
                    .try_parser_box(syscall)
                    .map_err(native_adapter::name_error)?,
            ),
        });
        pieces.insert(0, "\nthread 'main' (".to_owned());
        arguments.insert(0, pid);
        modes.insert(0, ir::FormatMode::Display);
        specs.insert(0, format_spec::FormatSpec::PLAIN);
        if let Some(last) = pieces.last_mut() {
            last.push_str(
                "\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
            );
        }
        self.names
            .retain_parser_text(
                pieces.iter().map(String::len).sum::<usize>()
                    + arguments.len() * std::mem::size_of::<Expr>()
                    + modes.len() * std::mem::size_of::<ir::FormatMode>(),
            )
            .map_err(native_adapter::name_error)?;
        let output = if arguments.is_empty() {
            let text = pieces.pop().unwrap_or_default();
            Expr::Output(
                self.names
                    .try_parser_box(SourceOutput {
                        output: ir::LiteralOutput {
                            text,
                            stream: ir::OutputStream::PanicStderr,
                            newline: true,
                        },
                        site,
                    })
                    .map_err(native_adapter::name_error)?,
            )
        } else {
            Expr::Format {
                pieces,
                arguments,
                modes,
                specs,
                stream: ir::OutputStream::PanicStderr,
                newline: true,
            }
        };
        let panic = self
            .names
            .try_parser_box(SourcePanic {
                message: String::new(),
                site,
                preformatted: true,
            })
            .map_err(native_adapter::name_error)?;
        let tail = self
            .names
            .try_parser_box(Expr::Panic(panic))
            .map_err(native_adapter::name_error)?;
        Ok(Expr::Block(Block {
            statements: vec![Stmt::Expr(output)],
            tail: Some(tail),
        }))
    }
}

impl Parser<'_, '_> {
    /// True when the format-string position holds a resolved builtin
    /// `concat!` invocation, which rustc expands eagerly into one literal.
    pub(super) fn at_builtin_concat(&mut self) -> Result<bool, ParseError> {
        Ok(self.token().is_ident()
            && self.cursor.peek(1).is_some_and(|t| t.matches("!"))
            && self.token().text == "concat"
            && self.builtin_macro_ahead("concat")?)
    }

    /// Eagerly expand builtin `concat!(lit, ...)` in format-string position.
    /// Operands are string, char, integer, and bool literals or nested
    /// `concat!`; anything else is an explicit unsupported boundary.
    pub(super) fn eager_concat(&mut self, depth: usize) -> Result<String, ParseError> {
        if depth > 32 {
            return Err(self.limit("concat! nesting exceeds limit 32"));
        }
        self.ident()?;
        self.expect("!")?;
        let close = if self.eat("(") {
            ")"
        } else if self.eat("[") {
            "]"
        } else if self.eat("{") {
            "}"
        } else {
            return Err(self.error("expected a concat! delimiter"));
        };
        let mut out = String::new();
        while !self.is(close) {
            let buffer;
            let token = if self.token().is_opaque_literal() {
                buffer = self
                    .input
                    .opaque_literal_buffer(&self.token(), self.names)?;
                TokenCursor::new(&buffer).current()
            } else {
                self.token()
            };
            if token.is_byte_string() || token.is_byte() {
                return Err(self.error("cannot concatenate a byte string literal"));
            }
            if token.string {
                out.push_str(&self.string_literal_value(token.text)?);
                self.cursor.bump();
            } else if token.is_char() {
                out.push(parse_char_literal(token.text).map_err(|e| self.error(e))?);
                self.cursor.bump();
            } else if token.matches("true") || token.matches("false") {
                out.push_str(token.text);
                self.cursor.bump();
            } else if token.kind_is_number()
                && token.text.bytes().all(|b| b.is_ascii_digit() || b == b'_')
            {
                out.extend(token.text.chars().filter(|c| *c != '_'));
                self.cursor.bump();
            } else if self.at_builtin_concat()? {
                out.push_str(&self.eager_concat(depth + 1)?);
            } else {
                return Err(self.unsupported(
                    "concat! operands other than unsuffixed decimal integer, string, char and bool literals are unsupported",
                ));
            }
            if self
                .names
                .budget()
                .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                .is_err()
            {
                return Err(self.limit("concat! work budget exhausted"));
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect(close)?;
        Ok(out)
    }
}
