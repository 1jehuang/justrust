//! `unused_extern_crates` at deny/forbid, following stock rustc
//! (compiler/rustc_resolve/src/check_unused.rs, MIT OR Apache-2.0, Rust
//! Project Contributors). An `extern crate` item is reported when the crate
//! is never used through it (`std` is exempt: it provides the panic handler),
//! or, when it carries no attributes and is not renamed, when the crate is
//! already in the extern prelude (`core`, `std` in a std crate, and
//! dependencies) so a `use` would do. Uses are decided on the configured
//! tokens of the crate's files: a path starting with the item's name outside
//! every macro body. Shapes this model cannot decide (items outside the crate
//! root module, attributes other than `cfg`, local declarations of the same
//! name, uses only inside macro bodies) are explicitly unsupported.
use std::ops::Range;

use super::{AttributeError, Filter};

/// One configured `extern crate` item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternEntry {
    pub crate_name: String,
    pub local: String,
    pub aliased: bool,
    /// Has an attribute other than an active `cfg`.
    pub attributed: bool,
    pub public: bool,
    /// Declared directly in the file's top-level module.
    pub top_level: bool,
    /// `unused_extern_crates` is at deny/forbid at the item.
    pub denied: bool,
    pub range: Range<usize>,
}

/// Per-file facts of one crate under a denied `unused_extern_crates`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternFacts {
    pub entries: Vec<ExternEntry>,
    /// Identifiers starting a path outside macro bodies.
    pub heads: Vec<String>,
    /// Identifiers starting a path inside macro bodies.
    pub macro_heads: Vec<String>,
    /// Identifiers declared in the type namespace by local items or imports.
    pub declared: Vec<String>,
}

fn ident(text: &str) -> &str {
    text.strip_prefix("r#").unwrap_or(text)
}

fn is_ident(text: &str) -> bool {
    text.starts_with(|c: char| c == '_' || c.is_alphabetic())
}

impl Filter<'_> {
    fn ex_tok(&self, i: usize) -> &str {
        self.tokens.get(i).map_or("", |t| t.text(self.text))
    }

    /// Record a configured `extern crate` item whose keyword is `keyword`.
    pub(super) fn record_extern_crate(
        &mut self,
        item_start: usize,
        keyword: usize,
        stop: usize,
        attributed: bool,
    ) {
        if self.ex_tok(keyword + 1) != "crate" {
            return;
        }
        let denied = self.levels.denied("unused_extern_crates");
        self.unused_extern_denied |= denied;
        let crate_name = ident(self.ex_tok(keyword + 2)).to_string();
        let aliased = self.ex_tok(keyword + 3) == "as";
        let local = if aliased {
            ident(self.ex_tok(keyword + 4)).to_string()
        } else {
            crate_name.clone()
        };
        self.extern_crates.push(ExternEntry {
            crate_name,
            local,
            aliased,
            attributed,
            public: self.ex_tok(item_start) == "pub" && self.ex_tok(item_start + 1) != "(",
            top_level: self.module_path.is_empty(),
            denied,
            range: self.tokens[item_start].range.start..self.tokens[stop - 1].range.end,
        });
    }

    /// Path heads and local declarations of this file, when any lint level
    /// in it denies `unused_extern_crates`.
    pub(super) fn extern_facts(&self) -> Result<Option<ExternFacts>, AttributeError> {
        if self.cap_lints_allow || !self.unused_extern_denied {
            return Ok(None);
        }
        let n = self.tokens.len();
        let mut bodies = Vec::new();
        for i in 0..n {
            if self.tokens[i].doc.is_none()
                && self.ex_tok(i) == "macro_rules"
                && self.ex_tok(i + 1) == "!"
                && let Some(close) = self.tokens.get(i + 3).and_then(|t| t.close)
            {
                bodies.push((i + 3, close));
            }
        }
        let inside_body = |i: usize| bodies.iter().any(|(open, close)| *open < i && i < *close);
        let own_alias = |i: usize| {
            let at = self.tokens[i].range.start;
            self.extern_crates.iter().any(|e| e.range.contains(&at))
        };
        // `extern crate` items this walker did not record (in function
        // bodies or macro bodies) may be linted by stock too.
        for i in 0..n {
            if self.tokens[i].doc.is_none()
                && self.ex_tok(i) == "extern"
                && self.ex_tok(i + 1) == "crate"
                && !self.token_masked(i)
                && !own_alias(i)
            {
                return Err(super::error(
                    super::AttributeErrorKind::Unsupported,
                    self.tokens[i].range.clone(),
                    "`deny(unused_extern_crates)`: an extern crate outside a module scope is unsupported",
                ));
            }
        }
        let mut facts = ExternFacts {
            entries: self.extern_crates.clone(),
            ..ExternFacts::default()
        };
        for i in 0..n {
            let t = self.ex_tok(i);
            if self.tokens[i].doc.is_some() || !is_ident(t) || self.token_masked(i) {
                continue;
            }
            let name = ident(t).to_string();
            let path_sep = |at: usize| self.ex_tok(at) == ":" && self.ex_tok(at + 1) == ":";
            let after_sep = i >= 2 && path_sep(i - 2);
            let prev = if after_sep {
                "::"
            } else if i > 0 {
                self.ex_tok(i - 1)
            } else {
                ""
            };
            let before_sep = if i >= 3 { self.ex_tok(i - 3) } else { "" };
            let next = if path_sep(i + 1) {
                "::"
            } else {
                self.ex_tok(i + 1)
            };
            // Declarations in the type namespace that could shadow the name.
            if matches!(
                prev,
                "mod" | "struct" | "enum" | "union" | "trait" | "type" | "as"
            ) && !own_alias(i)
                || prev == "::" && matches!(next, ";" | "," | "}")
            {
                facts.declared.push(name);
                continue;
            }
            // A path head: not preceded by `seg::` (only `crate::`/`self::`
            // or a leading `::`).
            let head = next == "::"
                && (prev != "::"
                    || !is_ident(before_sep)
                    || matches!(before_sep, "crate" | "self"))
                || prev == "use" && matches!(next, ";" | "as");
            if head {
                if inside_body(i) {
                    facts.macro_heads.push(name);
                } else {
                    facts.heads.push(name);
                }
            }
        }
        for list in [
            &mut facts.heads,
            &mut facts.macro_heads,
            &mut facts.declared,
        ] {
            list.sort();
            list.dedup();
        }
        Ok(Some(facts))
    }
}

/// A crate-wide `unused_extern_crates` violation.
pub struct Violation {
    /// (file index, entry index) of the reported item.
    pub at: (usize, usize),
    pub unsupported: bool,
    pub message: String,
}

/// Decide the lint over every file of one crate (`true` marks the crate
/// root file); `no_std` is the crate's `#![no_std]`.
pub fn check(files: &[(bool, &ExternFacts)], no_std: bool) -> Result<(), Violation> {
    let any = |pick: fn(&ExternFacts) -> &Vec<String>, name: &str| {
        files
            .iter()
            .any(|(_, f)| pick(f).binary_search_by(|n| n.as_str().cmp(name)).is_ok())
    };
    for (file, (root, facts)) in files.iter().enumerate() {
        for (index, entry) in facts.entries.iter().enumerate() {
            if !entry.denied {
                continue;
            }
            let fail = |unsupported: bool, message: String| Violation {
                at: (file, index),
                unsupported,
                message,
            };
            if !root || !entry.top_level {
                return Err(fail(
                    true,
                    "`deny(unused_extern_crates)`: an extern crate outside the crate root module is unsupported".into(),
                ));
            }
            if entry.public {
                continue;
            }
            if entry.crate_name == "std" {
                // Exempt from `unused`; idiomatic unless std is already in
                // the extern prelude and the item is plain.
                if no_std || entry.aliased || entry.attributed {
                    continue;
                }
                return Err(fail(
                    false,
                    "`extern crate` is not idiomatic in the new edition (`unused_extern_crates` is denied)".into(),
                ));
            }
            if entry.attributed {
                return Err(fail(
                    true,
                    "`deny(unused_extern_crates)`: an attributed extern crate is unsupported"
                        .into(),
                ));
            }
            let in_prelude = entry.crate_name != "alloc";
            if in_prelude && !entry.aliased {
                // Unused, or used but not idiomatic: rejected either way.
                return Err(fail(
                    false,
                    "unused or non-idiomatic extern crate (`unused_extern_crates` is denied)"
                        .into(),
                ));
            }
            if entry.aliased && entry.local.starts_with('_') {
                continue;
            }
            let local = entry.local.as_str();
            if any(|f| &f.declared, local) {
                return Err(fail(
                    true,
                    format!(
                        "`deny(unused_extern_crates)`: a local declaration named `{local}` is unsupported"
                    ),
                ));
            }
            if any(|f| &f.heads, local) {
                continue;
            }
            if any(|f| &f.macro_heads, local) {
                return Err(fail(
                    true,
                    "`deny(unused_extern_crates)`: a use only inside a macro body is unsupported"
                        .into(),
                ));
            }
            return Err(fail(
                false,
                "unused extern crate (`unused_extern_crates` is denied)".into(),
            ));
        }
    }
    Ok(())
}
