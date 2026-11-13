//! `unused_import_braces` at deny/forbid, following stock rustc's early lint
//! (compiler/rustc_lint/src/unused.rs `UnusedImportBraces`, MIT OR
//! Apache-2.0, Rust Project Contributors). Every brace group of a `use` tree
//! is checked; one holding exactly one entry is reported unless that entry
//! is itself a brace group or is `self` (optionally renamed). A glob or a
//! simple path (`{a::b}`, `{a as b}`) is reported. Decided on the configured
//! tokens of each module-scope `use` item at its own lint level. `use` items
//! this walker does not visit (function bodies, macro bodies) are decided
//! only when they cannot trigger the lint; otherwise they are unsupported.
use super::{AttributeError, AttributeErrorKind, Filter, error};

impl Filter<'_> {
    fn ib_tok(&self, i: usize) -> &str {
        self.tokens.get(i).map_or("", |t| t.text(self.text))
    }

    /// The first offending single-entry brace group in `start..stop`, as
    /// (open brace token, displayed entry name).
    fn redundant_import_brace(&self, start: usize, stop: usize) -> Option<(usize, String)> {
        for open in start..stop {
            if self.ib_tok(open) != "{" || self.tokens[open].doc.is_some() {
                continue;
            }
            let Some(close) = self.tokens[open].close else {
                continue;
            };
            // Top-level entries of this group.
            let mut entries: Vec<(usize, usize)> = Vec::new();
            let mut at = open + 1;
            let mut entry_start = at;
            while at < close {
                if let Some(inner) = self.tokens[at].close.filter(|c| *c > at) {
                    at = inner + 1;
                    continue;
                }
                if self.ib_tok(at) == "," {
                    entries.push((entry_start, at));
                    entry_start = at + 1;
                }
                at += 1;
            }
            if entry_start < close {
                entries.push((entry_start, close));
            }
            let [(first, end)] = entries.as_slice() else {
                continue;
            };
            let (first, end) = (*first, *end);
            if (first..end).any(|t| self.ib_tok(t) == "{") {
                continue;
            }
            // `{name as alias}` displays the alias; `{*}` displays `*`.
            let name = if end >= first + 3 && self.ib_tok(end - 2) == "as" {
                if self.ib_tok(end - 3) == "self" {
                    continue;
                }
                self.ib_tok(end - 1)
            } else {
                let last = self.ib_tok(end - 1);
                if last == "self" {
                    continue;
                }
                last
            };
            let name = name.strip_prefix("r#").unwrap_or(name).to_string();
            return Some((open, name));
        }
        None
    }

    /// Check one configured module-scope `use` item at its lint level.
    pub(super) fn check_import_braces(
        &mut self,
        keyword: usize,
        stop: usize,
    ) -> Result<(), AttributeError> {
        self.checked_uses.push(keyword);
        if self.cap_lints_allow || !self.levels.denied("unused_import_braces") {
            return Ok(());
        }
        if let Some((open, name)) = self.redundant_import_brace(keyword + 1, stop) {
            return Err(error(
                AttributeErrorKind::Syntax,
                self.tokens[open].range.clone(),
                &format!("braces around {name} is unnecessary (`unused_import_braces` is denied)"),
            ));
        }
        Ok(())
    }

    /// `use` items the item walker did not visit, when the lint is denied
    /// somewhere in this file: unsupported only if they could trigger it.
    pub(super) fn check_unvisited_import_braces(&self) -> Result<(), AttributeError> {
        if self.cap_lints_allow || !self.import_braces_denied {
            return Ok(());
        }
        let n = self.tokens.len();
        for i in 0..n {
            if self.tokens[i].doc.is_some()
                || self.ib_tok(i) != "use"
                || self.token_masked(i)
                || self.checked_uses.contains(&i)
            {
                continue;
            }
            let mut stop = i + 1;
            while stop < n && !matches!(self.ib_tok(stop), ";" | "}" | ")" | "]") {
                if let Some(close) = self.tokens[stop].close.filter(|c| *c > stop) {
                    stop = close;
                }
                stop += 1;
            }
            if self.redundant_import_brace(i + 1, stop).is_some() {
                return Err(error(
                    AttributeErrorKind::Unsupported,
                    self.tokens[i].range.clone(),
                    "`deny(unused_import_braces)`: a braced import outside a module scope is unsupported",
                ));
            }
        }
        Ok(())
    }
}
