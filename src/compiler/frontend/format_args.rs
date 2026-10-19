//! Output-macro argument lists: positional, `name = value` and inline
//! `{name}` captures, resolved against parsed placeholders (format_spec.rs).
//! The result is one source expression per placeholder in order. Only plain
//! local reads may be referenced by more than one placeholder, because the
//! checker borrows each placeholder operand once and a duplicated arbitrary
//! expression would be evaluated twice.
use super::format_spec::{ArgRef, FormatSpec, Placeholder};
use super::*;

type FormatArguments = (Vec<Expr>, Vec<ir::FormatMode>, Vec<FormatSpec>);

impl Parser<'_, '_> {
    pub(super) fn format_arguments(
        &mut self,
        close: &str,
        holders: &[Placeholder],
    ) -> Result<FormatArguments, ParseError> {
        let site = self.token().span;
        let mut positional: Vec<Expr> = Vec::new();
        let mut named: Vec<(String, Expr)> = Vec::new();
        while self.eat(",") && !self.is(close) {
            if self.is_identifier()
                && self.cursor.peek(1).is_some_and(|t| t.text == "=")
                && !self.cursor.peek(2).is_some_and(|t| t.text == "=")
            {
                let name = self.token().text.to_owned();
                self.cursor.bump();
                self.expect("=")?;
                let value = self.nested_expr()?;
                if named.iter().any(|(n, _)| *n == name) {
                    return Err(self.error(&format!("duplicate argument named `{name}`")));
                }
                named.push((name, value));
            } else {
                if !named.is_empty() {
                    return Err(self.error("positional arguments cannot follow named arguments"));
                }
                positional.push(self.nested_expr()?);
            }
        }
        self.expect(close)?;
        let total = positional.len() + named.len();
        let mut used = vec![0usize; total];
        let mut next = 0usize;
        let mut slots: Vec<Option<usize>> = Vec::with_capacity(holders.len());
        let mut captures: Vec<Option<String>> = Vec::with_capacity(holders.len());
        for holder in holders {
            let (slot, capture) = match &holder.arg {
                ArgRef::Next => {
                    let index = next;
                    next += 1;
                    (Some(index), None)
                }
                ArgRef::Index(index) => (Some(*index), None),
                ArgRef::Name(name) => match named.iter().position(|(n, _)| n == name) {
                    Some(at) => (Some(positional.len() + at), None),
                    None => (None, Some(name.clone())),
                },
            };
            if let Some(slot) = slot {
                if slot >= total {
                    return Err(self.error(&format!(
                        "invalid reference to positional argument {slot} ({} arguments were given)",
                        positional.len()
                    )));
                }
                used[slot] += 1;
            }
            slots.push(slot);
            captures.push(capture);
        }
        if let Some(unused) = used.iter().position(|count| *count == 0) {
            return Err(if unused < positional.len() {
                self.error("argument never used")
            } else {
                self.error("named argument never used")
            });
        }
        let mut values: Vec<Option<Expr>> = positional
            .into_iter()
            .chain(named.into_iter().map(|(_, value)| value))
            .map(Some)
            .collect();
        let mut arguments = Vec::with_capacity(holders.len());
        let mut modes = Vec::with_capacity(holders.len());
        let mut specs = Vec::with_capacity(holders.len());
        for ((holder, slot), capture) in holders.iter().zip(&slots).zip(captures) {
            let expr = match (slot, capture) {
                (Some(slot), _) if used[*slot] == 1 => values[*slot]
                    .take()
                    .ok_or_else(|| self.error("format argument consumed twice"))?,
                (Some(slot), _) => {
                    let value = values[*slot]
                        .as_ref()
                        .ok_or_else(|| self.error("format argument consumed twice"))?;
                    if !matches!(value, Expr::Var(path) if path.single().is_some()) {
                        return Err(self.unsupported(
                            "format arguments referenced by several placeholders must be plain local variables",
                        ));
                    }
                    value.clone()
                }
                (None, Some(name)) => self.inline_capture(&name, site)?,
                (None, None) => return Err(self.error("unresolved format argument")),
            };
            arguments.push(expr);
            modes.push(holder.spec.native_mode().unwrap_or(ir::FormatMode::Display));
            specs.push(holder.spec);
        }
        Ok((arguments, modes, specs))
    }

    /// `{name}` with no explicit `name = ...` argument reads local `name`.
    fn inline_capture(
        &mut self,
        name: &str,
        site: crate::compiler::expansion_span::SpanId,
    ) -> Result<Expr, ParseError> {
        let origin =
            panic_callsite(self.names.session(), site).map_err(native_adapter::transport_error)?;
        let ident = self
            .names
            .intern_root(name, origin)
            .map_err(native_adapter::name_error)?;
        self.reference_lexical(ident)?;
        Ok(Expr::Var(
            AstPath::new(self.names, PathAnchor::Relative, &[ident])
                .map_err(native_adapter::name_error)?,
        ))
    }
}
