//! `unused_macros` at deny/forbid, following stock rustc's resolver rule
//! (compiler/rustc_resolve/src/build_reduced_graph.rs, MIT OR Apache-2.0,
//! Rust Project Contributors): every local `macro_rules!` definition that is
//! not `#[macro_export]`ed and whose name does not start with `_` must be
//! used. Uses are decided on the configured tokens of one file: a definition
//! is used when its name is invoked (`name!`) after it, inside its textual
//! scope and outside every macro body. Positions this model cannot decide
//! (invocations inside other macro bodies or attributes, imports of the name,
//! file modules that inherit the textual scope, definitions outside module
//! scopes, repeated names) are explicitly unsupported.
use super::{AttributeError, AttributeErrorKind, Filter, error};

/// One configured module-scope `macro_rules!` definition.
#[derive(Clone, Debug)]
pub(super) struct MacroDef {
    /// Token index of the `macro_rules` keyword.
    keyword: usize,
    /// `unused_macros` is at deny/forbid at the definition.
    denied: bool,
    exported: bool,
}

impl MacroDef {
    pub(super) fn keyword(&self) -> usize {
        self.keyword
    }
}

fn ident(text: &str) -> &str {
    text.strip_prefix("r#").unwrap_or(text)
}

impl Filter<'_> {
    /// Record a configured module-scope `macro_rules!` definition.
    pub(super) fn record_macro_definition(&mut self, keyword: usize, exported: bool) {
        let denied = self.levels.denied("unused_macros");
        self.unused_macros_denied |= denied;
        self.macro_defs.push(MacroDef {
            keyword,
            denied,
            exported,
        });
    }

    fn tok(&self, i: usize) -> &str {
        self.tokens.get(i).map_or("", |t| t.text(self.text))
    }

    pub(super) fn token_masked(&self, i: usize) -> bool {
        let start = self.tokens[i].range.start;
        self.bytes[start] == b' ' && self.text.as_bytes()[start] != b' '
    }

    fn unsupported(&self, at: usize, message: &str) -> AttributeError {
        error(
            AttributeErrorKind::Unsupported,
            self.tokens[at].range.clone(),
            &format!("`deny(unused_macros)`: {message} is unsupported"),
        )
    }

    /// Decide `unused_macros` for this file once its items have been walked.
    pub(super) fn check_unused_macros(&self) -> Result<(), AttributeError> {
        if self.cap_lints_allow || !self.unused_macros_denied {
            return Ok(());
        }
        let n = self.tokens.len();
        // `#[macro_use]` extends textual scopes past module ends, across files.
        if let Some(i) =
            (0..n).find(|&i| self.tok(i) == "macro_use" && self.tokens[i].doc.is_none())
        {
            return Err(self.unsupported(i, "`macro_use`"));
        }
        // Every macro body in the file, configured or not.
        let mut bodies = Vec::new();
        for i in 0..n {
            if self.tokens[i].doc.is_some() || self.tok(i) != "macro_rules" {
                continue;
            }
            if self.tok(i + 1) != "!" {
                continue;
            }
            let Some(close) = self.tokens.get(i + 3).and_then(|t| t.close) else {
                continue;
            };
            bodies.push((i, i + 3, close));
        }
        // Definitions this walker did not see (function bodies, macro
        // bodies) may be linted by stock too.
        for &(keyword, _, _) in &bodies {
            if self.token_masked(keyword)
                || self.macro_defs.iter().any(|d| d.keyword == keyword)
                || ident(self.tok(keyword + 2)).starts_with('_')
            {
                continue;
            }
            return Err(self.unsupported(keyword, "a macro definition outside a module scope"));
        }
        let inside_body = |i: usize| {
            bodies
                .iter()
                .find(|(_, open, close)| *open < i && i < *close)
        };
        for def in &self.macro_defs {
            let name_at = def.keyword + 2;
            let name = ident(self.tok(name_at));
            if !def.denied || def.exported || name.starts_with('_') {
                continue;
            }
            if self
                .macro_defs
                .iter()
                .filter(|d| ident(self.tok(d.keyword + 2)) == name)
                .count()
                > 1
            {
                return Err(self.unsupported(name_at, "a repeated macro name"));
            }
            // Textual scope: the rest of the enclosing module body.
            let scope_end = (0..def.keyword)
                .rev()
                .find_map(|o| self.tokens[o].close.filter(|close| *close > def.keyword))
                .unwrap_or(n);
            let body_close = bodies
                .iter()
                .find(|(k, _, _)| *k == def.keyword)
                .map_or(name_at, |(_, _, close)| *close);
            let mut used = false;
            for i in body_close + 1..scope_end {
                let t = self.tok(i);
                if t == "mod"
                    && self.tok(i + 2) == ";"
                    && !self.token_masked(i)
                    && inside_body(i).is_none()
                {
                    return Err(self.unsupported(i, "a file module inside a macro's textual scope"));
                }
                if self.tokens[i].doc.is_some() || ident(t) != name {
                    continue;
                }
                let invoked = self.tok(i + 1) == "!";
                if self.token_masked(i) {
                    let at = self.tokens[i].range.start;
                    let configured_out = self
                        .inactive_items
                        .iter()
                        .any(|item| item.range.contains(&at));
                    if invoked && !configured_out {
                        return Err(self.unsupported(i, "an invocation inside an attribute"));
                    }
                    continue;
                }
                if inside_body(i).is_some() {
                    return Err(self.unsupported(i, "an invocation inside a macro body"));
                }
                if invoked {
                    used = true;
                    break;
                }
                // `use name;` or `use self::name as other;` can use the
                // macro; ordinary values and types of the same name cannot.
                let mut j = i;
                while j > 0 && !matches!(self.tok(j - 1), ";" | "{" | "}") {
                    j -= 1;
                    if self.tok(j) == "use" {
                        return Err(self.unsupported(i, "an import of a local macro"));
                    }
                }
            }
            if !used {
                return Err(error(
                    AttributeErrorKind::Syntax,
                    self.tokens[name_at].range.clone(),
                    &format!("unused macro definition: `{name}` (`unused_macros` is denied)"),
                ));
            }
        }
        Ok(())
    }
}
