//! `cfg!(predicate)`: a boolean literal decided by the target configuration.
//! The parser only sees target facts and `debug_assertions`, so predicates on
//! user cfg names or features get an explicit diagnostic.

use super::{Expr, ParseError, Parser};
use crate::compiler::source::CfgSet;
use crate::compiler::source::source_cfg::{Meta, MetaValue, evaluate, parse_meta};

impl Parser<'_, '_> {
    pub(super) fn cfg_macro_expression(&mut self) -> Result<Expr, ParseError> {
        let site = self.token().span;
        self.cursor.bump();
        self.expect("!")?;
        let close = match self.token().text {
            "(" => ")",
            "[" => "]",
            "{" => "}",
            _ => return Err(self.error("expected a cfg! delimiter")),
        };
        let open = self.token().text;
        self.cursor.bump();
        let mut text = String::new();
        let mut depth = 0usize;
        loop {
            let token = self.token();
            if token.matches("") {
                return Err(self.error("unterminated cfg! invocation"));
            }
            if depth == 0 && token.matches(close) {
                break;
            }
            if token.matches(open) {
                depth += 1;
            } else if token.matches(close) {
                depth -= 1;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            if token.is_string() {
                if token.text.contains(['"', '\\']) {
                    return Err(self.unsupported("escaped cfg! string values are unsupported"));
                }
                text.push('"');
                text.push_str(token.text);
                text.push('"');
            } else if token.is_ident()
                || token.matches("(")
                || token.matches(")")
                || token.matches(",")
                || token.matches("=")
            {
                text.push_str(token.text);
            } else {
                return Err(self.error("invalid cfg! predicate token"));
            }
            self.cursor.bump();
        }
        let trimmed = text.trim_end_matches(|c| c == ',' || c == ' ');
        let meta = parse_meta(trimmed, 0)
            .map_err(|error| self.error(&format!("invalid cfg! predicate: {}", error.message)))?;
        let root =
            crate::compiler::source_macro_binding::invocation_root(self.names.session(), site)
                .map_err(super::native_adapter::transport_error)?;
        let configured = self
            .cfg_roots
            .iter()
            .find(|(handle, _)| *handle == root)
            .map(|(_, cfg)| cfg.clone());
        let cfg = match configured {
            Some(cfg) => cfg,
            None => {
                // No crate configuration reached this parse: decide only
                // target facts and debug_assertions.
                if let Some(name) = unknown_name(&meta) {
                    return Err(self.unsupported(&format!(
                        "cfg! on `{name}` needs the crate configuration, which is unavailable here"
                    )));
                }
                let mut cfg = CfgSet::native_x86_64_linux();
                cfg.set_debug_assertions(self.debug_assertions);
                std::sync::Arc::new(cfg)
            }
        };
        let value = evaluate(&meta, &cfg)
            .map_err(|error| self.error(&format!("invalid cfg! predicate: {}", error.message)))?;
        self.expect(close)?;
        Ok(Expr::Bool(value))
    }
}

/// The first predicate name the parser cannot decide.
fn unknown_name(meta: &Meta) -> Option<&str> {
    match (&*meta.name, &meta.value) {
        ("all" | "any" | "not", MetaValue::List(args)) => args.iter().find_map(unknown_name),
        (name, _)
            if name.starts_with("target_")
                || matches!(name, "unix" | "windows" | "panic" | "debug_assertions") =>
        {
            None
        }
        (name, _) => Some(name),
    }
}
