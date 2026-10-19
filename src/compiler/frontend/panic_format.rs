//! `panic!("fmt {}", args)` (and unreachable!/todo!/unimplemented! with
//! arguments): the diagnostic header and the formatted message are written to
//! stderr by an ordinary native format output, then a preformatted abort
//! (no further diagnostic text) terminates the process.
use super::*;

impl Parser<'_, '_> {
    /// True when the macro's first argument is a string literal carrying
    /// placeholders or followed by format arguments.
    pub(super) fn panic_has_format(&mut self) -> bool {
        let token = self.token();
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
        let mut header = format!("panicked at {path}:{line}:{column}:\n");
        if spelling != "panic" {
            header.push_str(default);
            header.push_str(": ");
        }
        let token = self.token();
        let text = self.string_literal_value(token.text)?;
        self.cursor.bump();
        let (mut pieces, holders) =
            format_spec::parse_format(&text).map_err(|message| self.error(&message))?;
        let (arguments, modes, specs) = self.format_arguments(close, &holders)?;
        if holders.is_empty() && !arguments.is_empty() {
            return Err(self.error("argument never used"));
        }
        header.push_str(&pieces[0]);
        pieces[0] = header;
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
                            stream: ir::OutputStream::Stderr,
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
                stream: ir::OutputStream::Stderr,
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
