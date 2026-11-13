//! `missing_docs` at deny/forbid, following stock rustc's `MissingDoc` lint
//! pass (adapted from compiler/rustc_lint/src/builtin.rs, MIT OR Apache-2.0,
//! Rust Project Contributors): the crate, exported items (except impls and
//! imports), trait items, public inherent-impl items of exported types, named
//! fields and enum variants must carry documentation (`///`, `#[doc = ..]` or
//! `#[doc(hidden)]`). Trait-impl items are exempt.
//!
//! Each file records syntactic facts while its configured items are walked.
//! The crate-level decision needs every file's facts, so it runs once the
//! user crate has been assembled. Export status comes from a conservative
//! syntactic reachability model: an item is exported when it and its whole
//! module chain are `pub`, not exported when a private module hides it and no
//! `pub use` can name it, and undecided otherwise. An undocumented item whose
//! status is undecided is explicitly unsupported, never silently accepted.
use std::collections::BTreeMap;
use std::ops::Range;

/// Owner of an associated item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Container {
    /// Trait declaration name and whether the trait is `pub`.
    Trait(String, bool),
    /// Last path segment of an inherent impl's self type (empty: unknown).
    Inherent(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocKind {
    /// Inner attributes of a module body or file (the crate at the root).
    Inner,
    /// `mod name` declaration.
    Module,
    /// Ordinary item. `nominal`: a struct, enum, union, type alias or trait
    /// that an inherent impl may name.
    Item {
        desc: &'static str,
        nominal: bool,
        hidden: bool,
    },
    /// `#[macro_export] macro_rules!`: always exported.
    ExportedMacro,
    /// Named field of `owner` (declared in the same module).
    Field { owner: String, owner_public: bool },
    /// Enum variant of `owner`.
    Variant { owner: String, owner_public: bool },
    /// Associated item.
    Assoc(Container),
    /// `pub use`: every identifier it mentions, and whether it has a glob
    /// whose prefix is not a plain module name.
    Use {
        names: Vec<String>,
        keyword_glob: bool,
    },
    /// Item macro invocation; generated items are unknown.
    Invocation(Option<Container>),
    /// Item shape this model does not interpret (foreign blocks, unknown items).
    Opaque,
    /// Module-level `impl .. Debug for Name ..`; the last self-type path
    /// segment, empty when the self type is not a path.
    DebugImpl(String),
    /// An `impl .. Debug ..` this model does not place (function bodies,
    /// macro bodies).
    DebugUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocEntry {
    /// Inline module path inside the file.
    pub module: Vec<String>,
    pub name: String,
    /// Plain `pub` (restricted visibility is not exported).
    pub public: bool,
    pub documented: bool,
    /// `missing_docs` is at deny/forbid at the item.
    pub denied: bool,
    pub range: Range<usize>,
    pub kind: DocKind,
    /// `missing_debug_implementations` is at deny/forbid at the item.
    pub debug_denied: bool,
    /// The item carries an active `derive(Debug)`.
    pub debug_derived: bool,
}

/// Violation found by [`check`]: the entry index and its message.
#[derive(Debug)]
pub struct DocViolation {
    pub index: usize,
    pub unsupported: bool,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Not,
    Maybe,
    Yes,
}

fn and(a: Status, b: Status) -> Status {
    match (a, b) {
        (Status::Not, _) | (_, Status::Not) => Status::Not,
        (Status::Yes, Status::Yes) => Status::Yes,
        _ => Status::Maybe,
    }
}

struct Model<'a> {
    /// Module path -> (some declaration is pub, some declaration is not).
    modules: BTreeMap<&'a [String], (bool, bool)>,
    mentions: std::collections::BTreeSet<&'a str>,
    universal: bool,
}

impl<'a> Model<'a> {
    fn path(&self, module: &[String], name: &str, public: bool) -> Status {
        if !public {
            return Status::Not;
        }
        // Any path that reexports an item out of a private module names the
        // deepest private module on its chain (globs never import a private
        // module), so only a mention of it or a module below it can make the
        // item reachable.
        let mut private = None;
        for i in 1..=module.len() {
            match self.modules.get(&module[..i]) {
                Some((true, false)) => {}
                Some((false, true)) => private = Some(i - 1),
                _ if private.is_some() => {}
                _ => return Status::Maybe,
            }
        }
        let Some(k) = private else {
            return Status::Yes;
        };
        if self.universal
            || (!name.is_empty() && self.mentions.contains(name))
            || module[k..]
                .iter()
                .any(|m| self.mentions.contains(m.as_str()))
        {
            Status::Maybe
        } else {
            Status::Not
        }
    }
}

/// Decide `missing_docs` for the user crate. `entries` pairs every file's
/// facts with the file's absolute module prefix, in assembly order.
pub fn check(entries: &[(&[String], &DocEntry)]) -> Result<(), DocViolation> {
    if !entries.iter().any(|(_, e)| e.denied || e.debug_denied) {
        return Ok(());
    }
    let absolute: Vec<Vec<String>> = entries
        .iter()
        .map(|(prefix, e)| prefix.iter().chain(&e.module).cloned().collect())
        .collect();
    let mut module_paths: Vec<Vec<String>> = Vec::new();
    for (i, (_, e)) in entries.iter().enumerate() {
        if e.kind == DocKind::Module {
            let mut path = absolute[i].clone();
            path.push(e.name.clone());
            module_paths.push(path);
        }
    }
    let mut model = Model {
        modules: BTreeMap::new(),
        mentions: Default::default(),
        universal: false,
    };
    let mut m = 0;
    for (_, e) in entries {
        match &e.kind {
            DocKind::Module => {
                let slot = model
                    .modules
                    .entry(module_paths[m].as_slice())
                    .or_insert((false, false));
                if e.public {
                    slot.0 = true;
                } else {
                    slot.1 = true;
                }
                m += 1;
            }
            DocKind::Use {
                names,
                keyword_glob,
            } => {
                model.universal |= *keyword_glob;
                model.mentions.extend(names.iter().map(String::as_str));
            }
            _ => {}
        }
    }
    // Generated module-level items may include `pub use`.
    for (i, (_, e)) in entries.iter().enumerate() {
        if e.kind == DocKind::Invocation(None) && model.path(&absolute[i], "", true) != Status::Not
        {
            model.universal = true;
        }
    }
    check_debug(entries, &absolute, &model)?;
    let mut nominals: BTreeMap<&str, Vec<(Status, bool)>> = BTreeMap::new();
    for (i, (_, e)) in entries.iter().enumerate() {
        if let DocKind::Item {
            nominal: true,
            hidden,
            ..
        } = e.kind
        {
            nominals
                .entry(e.name.as_str())
                .or_default()
                .push((model.path(&absolute[i], &e.name, e.public), hidden));
        }
    }
    let nominal = |name: &str| -> Status {
        let Some(found) = nominals.get(name) else {
            return Status::Maybe;
        };
        if found.iter().all(|(_, hidden)| *hidden) {
            return Status::Not;
        }
        if found.iter().any(|(_, hidden)| *hidden) {
            return Status::Maybe;
        }
        if found.iter().all(|(s, _)| *s == Status::Yes) {
            Status::Yes
        } else if found.iter().all(|(s, _)| *s == Status::Not) {
            Status::Not
        } else {
            Status::Maybe
        }
    };
    // Module documentation may sit on the declaration or inside the body.
    let mut inner_docs: BTreeMap<&[String], (bool, bool)> = BTreeMap::new();
    for (i, (_, e)) in entries.iter().enumerate() {
        if e.kind == DocKind::Inner {
            let slot = inner_docs
                .entry(absolute[i].as_slice())
                .or_insert((false, false));
            slot.0 |= e.documented;
            slot.1 |= e.denied;
        }
    }
    let mut m = 0;
    for (i, (_, e)) in entries.iter().enumerate() {
        let module = absolute[i].as_slice();
        let (status, denied, documented, desc) = match &e.kind {
            DocKind::Inner => {
                if !module.is_empty() {
                    continue;
                }
                (Status::Yes, e.denied, e.documented, "the crate")
            }
            DocKind::Module => {
                let path = module_paths[m].as_slice();
                m += 1;
                let inner = inner_docs.get(path).copied();
                (
                    model.path(module, &e.name, e.public),
                    inner.map_or(e.denied, |(_, denied)| denied),
                    e.documented || inner.is_some_and(|(doc, _)| doc),
                    "a module",
                )
            }
            DocKind::Item { desc, .. } => (
                model.path(module, &e.name, e.public),
                e.denied,
                e.documented,
                *desc,
            ),
            DocKind::ExportedMacro => (Status::Yes, e.denied, e.documented, "a macro"),
            DocKind::Field {
                owner,
                owner_public,
            } => (
                and(
                    model.path(module, owner, *owner_public),
                    if e.public { Status::Yes } else { Status::Not },
                ),
                e.denied,
                e.documented,
                "a struct field",
            ),
            DocKind::Variant {
                owner,
                owner_public,
            } => (
                model.path(module, owner, *owner_public),
                e.denied,
                e.documented,
                "a variant",
            ),
            DocKind::Assoc(Container::Trait(name, public)) => (
                model.path(module, name, *public),
                e.denied,
                e.documented,
                "an associated item",
            ),
            DocKind::Assoc(Container::Inherent(self_ty)) => (
                if e.public {
                    nominal(self_ty)
                } else {
                    Status::Not
                },
                e.denied,
                e.documented,
                "an associated item",
            ),
            DocKind::Use { .. } | DocKind::DebugImpl(_) | DocKind::DebugUnknown => continue,
            DocKind::Invocation(container) => {
                let status = match container {
                    None => model.path(module, "", true),
                    Some(Container::Trait(name, public)) => model.path(module, name, *public),
                    Some(Container::Inherent(self_ty)) => nominal(self_ty),
                };
                if e.denied && status != Status::Not {
                    return Err(DocViolation {
                        index: i,
                        unsupported: true,
                        message: "`deny(missing_docs)` over items generated by a macro invocation is unsupported".into(),
                    });
                }
                continue;
            }
            DocKind::Opaque => {
                if e.denied && model.path(module, &e.name, e.public) != Status::Not {
                    return Err(DocViolation {
                        index: i,
                        unsupported: true,
                        message: "`deny(missing_docs)` over this item form is unsupported".into(),
                    });
                }
                continue;
            }
        };
        if !denied || documented {
            continue;
        }
        match status {
            Status::Not => {}
            Status::Yes => {
                return Err(DocViolation {
                    index: i,
                    unsupported: false,
                    message: format!("missing documentation for {desc} (`missing_docs` is denied)"),
                });
            }
            Status::Maybe => {
                return Err(DocViolation {
                    index: i,
                    unsupported: true,
                    message: format!(
                        "`deny(missing_docs)`: whether this undocumented item ({desc}) is exported through a reexport is undecided"
                    ),
                });
            }
        }
    }
    Ok(())
}

/// `missing_debug_implementations` at deny/forbid, following stock rustc's
/// `MissingDebugImplementations` pass (compiler/rustc_lint/src/builtin.rs,
/// MIT OR Apache-2.0, Rust Project Contributors): every reachable struct,
/// enum and union needs a `Debug` impl whose self type is that type (any
/// arguments, any bounds). Reachability through public signatures of a
/// `pub` item in a private module, impls this model cannot place and items
/// generated by macros are explicitly unsupported.
fn check_debug(
    entries: &[(&[String], &DocEntry)],
    absolute: &[Vec<String>],
    model: &Model<'_>,
) -> Result<(), DocViolation> {
    if !entries.iter().any(|(_, e)| e.debug_denied) {
        return Ok(());
    }
    let mut nominals: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, e) in entries {
        if let DocKind::Item { nominal: true, .. } = e.kind {
            *nominals.entry(e.name.as_str()).or_default() += 1;
        }
    }
    let mut implemented: std::collections::BTreeSet<&str> = Default::default();
    let mut unplaced = false;
    for (_, e) in entries {
        match &e.kind {
            DocKind::DebugImpl(target) => {
                if nominals.get(target.as_str()) == Some(&1) {
                    implemented.insert(target);
                } else if !target.is_empty() {
                    // Aliases, repeated or foreign names.
                    unplaced = true;
                }
            }
            DocKind::DebugUnknown | DocKind::Invocation(None) | DocKind::Opaque => unplaced = true,
            _ => {}
        }
    }
    for (i, (_, e)) in entries.iter().enumerate() {
        let DocKind::Item { desc, .. } = e.kind else {
            continue;
        };
        if !e.debug_denied
            || e.debug_derived
            || e.name.is_empty()
            || !matches!(desc, "a struct" | "an enum" | "a union")
            || implemented.contains(e.name.as_str())
        {
            continue;
        }
        let status = match model.path(&absolute[i], &e.name, e.public) {
            // A `pub` item of a private module is reachable through public
            // signatures that name it.
            Status::Not if e.public => Status::Maybe,
            status => status,
        };
        match status {
            Status::Not => {}
            Status::Yes if !unplaced => {
                return Err(DocViolation {
                    index: i,
                    unsupported: false,
                    message: "type does not implement `Debug`; consider adding `#[derive(Debug)]` or a manual implementation (`missing_debug_implementations` is denied)".into(),
                });
            }
            _ => {
                return Err(DocViolation {
                    index: i,
                    unsupported: true,
                    message: "`deny(missing_debug_implementations)`: whether this type is reachable without a `Debug` impl is undecided".into(),
                });
            }
        }
    }
    Ok(())
}

use super::{Filter, Token};

/// Documentation state of the attributes on the item being admitted.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DocState {
    pub documented: bool,
    pub hidden: bool,
    pub debug_derived: bool,
}

fn is_ident(text: &str) -> bool {
    let name = text.strip_prefix("r#").unwrap_or(text);
    name.chars()
        .next()
        .is_some_and(|c| c == '_' || c.is_alphabetic())
        && name.chars().all(|c| c == '_' || c.is_alphanumeric())
}

impl Filter<'_> {
    fn t(&self, i: usize) -> &str {
        self.tokens.get(i).map_or("", |t: &Token| t.text(self.text))
    }
    fn plain_pub(&self, i: usize) -> bool {
        self.t(i) == "pub" && self.t(i + 1) != "("
    }
    fn doc_entry(&mut self, name: &str, public: bool, range: Range<usize>, kind: DocKind) {
        self.docs.push(DocEntry {
            module: self.module_path.clone(),
            name: name.trim_start_matches("r#").to_string(),
            public,
            documented: self.doc_state.documented,
            denied: self.levels.denied("missing_docs"),
            range,
            kind,
            debug_denied: self.levels.denied("missing_debug_implementations"),
            debug_derived: self.doc_state.debug_derived,
        });
    }
    /// Inner attributes of the crate root or a module body.
    pub(super) fn record_inner_docs(&mut self, at: Range<usize>) {
        self.doc_entry("", true, at, DocKind::Inner);
    }
    /// One configured, active item of a module scope.
    pub(super) fn record_item_docs(
        &mut self,
        kind: &str,
        item_start: usize,
        keyword: usize,
        body: Option<(usize, usize)>,
        exported_macro: bool,
    ) {
        let public = self.plain_pub(item_start);
        let range = self.tokens[keyword].range.clone();
        let mut name = self.t(keyword + 1).to_string();
        let item = |desc, nominal| DocKind::Item {
            desc,
            nominal,
            hidden: false,
        };
        let entry = match kind {
            "mod" => DocKind::Module,
            "fn" => item("a function", false),
            "struct" => item("a struct", true),
            "enum" => item("an enum", true),
            "union" => item("a union", true),
            "trait" => item("a trait", true),
            "type" => item("a type alias", true),
            "const" => item("a constant", false),
            "static" => {
                if name == "mut" {
                    name = self.t(keyword + 2).to_string();
                }
                item("a static", false)
            }
            "macro_rules" => {
                if !exported_macro {
                    return;
                }
                name = self.t(keyword + 2).to_string();
                DocKind::ExportedMacro
            }
            "use" => {
                if !public {
                    // A renaming private import can be named by a public glob.
                    let stop = body.map_or(self.tokens.len(), |(_, close)| close);
                    if !(keyword..stop.min(self.tokens.len())).any(|i| self.t(i) == "as") {
                        return;
                    }
                }
                let mut names = Vec::new();
                let mut keyword_glob = false;
                let mut i = keyword + 1;
                while i < self.tokens.len() && self.t(i) != ";" {
                    let t = self.t(i);
                    if t == "*" {
                        let prefix = if i >= 2 && self.t(i - 1) == ":" {
                            self.t(i.saturating_sub(3))
                        } else {
                            ""
                        };
                        keyword_glob |= !is_ident(prefix)
                            || matches!(prefix, "self" | "super" | "crate" | "Self");
                    } else if is_ident(t) {
                        names.push(t.trim_start_matches("r#").to_string());
                    }
                    i += 1;
                }
                DocKind::Use {
                    names,
                    keyword_glob,
                }
            }
            "extern" if self.t(keyword + 1) == "crate" => return,
            "impl" => {
                let open = body.map_or(self.tokens.len(), |(open, _)| open);
                let Some(target) = self.debug_impl_target(keyword, open) else {
                    return;
                };
                self.debug_impls.push(keyword);
                DocKind::DebugImpl(target)
            }
            "unknown" if body.is_none() && self.trait_impl_invocation(item_start).is_some() => {
                // Only trait impls: nothing generated needs documentation.
                if self.trait_impl_invocation(item_start) == Some(true) {
                    let range = self.tokens[item_start].range.clone();
                    self.doc_entry("", false, range, DocKind::DebugUnknown);
                }
                return;
            }
            "unknown"
                if self.tokens[item_start
                    ..body
                        .map_or(item_start + 3, |(open, _)| open)
                        .min(self.tokens.len())]
                    .iter()
                    .any(|t| t.text(self.text) == "!") =>
            {
                DocKind::Invocation(None)
            }
            _ => DocKind::Opaque,
        };
        let entry = match entry {
            DocKind::Item { desc, nominal, .. } => DocKind::Item {
                desc,
                nominal,
                hidden: self.doc_state.hidden,
            },
            other => other,
        };
        // Generated items and members of uninterpreted forms (foreign
        // blocks) may be public whatever the outer spelling.
        let public = public || matches!(entry, DocKind::Invocation(None) | DocKind::Opaque);
        if !is_ident(&name) || name == "_" {
            name.clear();
        }
        let named_public = public && (!name.is_empty() || !matches!(entry, DocKind::Item { .. }));
        self.doc_entry(&name, named_public, range, entry);
        if let Some((open, close)) = body {
            match kind {
                "struct" | "union" => self.record_fields(open, close, &name, public, false),
                "enum" => self.record_variants(open, close, &name, public),
                _ => {}
            }
        }
    }
    /// Named fields of a braced struct, union or enum variant body.
    fn record_fields(
        &mut self,
        open: usize,
        close: usize,
        owner: &str,
        owner_public: bool,
        inherit: bool,
    ) {
        let mut i = open + 1;
        while i < close {
            let mut documented = false;
            let mut uncertain = false;
            while i < close {
                if self.tokens[i].doc.is_some() {
                    documented = true;
                    i += 1;
                } else if self.t(i) == "#" && self.t(i + 1) == "[" {
                    match self.t(i + 2) {
                        "doc" => documented = true,
                        // Inert tool attributes.
                        "rustfmt" | "clippy" | "default" => {}
                        // Lint levels and configuration are not modeled here.
                        _ => uncertain = true,
                    }
                    i = self.tokens[i + 1].close.map_or(i + 2, |c| c + 1);
                } else {
                    break;
                }
            }
            if i >= close {
                break;
            }
            let public = inherit || self.plain_pub(i);
            if self.t(i) == "pub" {
                i += 1;
                if self.t(i) == "(" {
                    i = self.tokens[i].close.map_or(i + 1, |c| c + 1);
                }
            }
            let name = self.t(i).to_string();
            let range = self.tokens[i.min(close)].range.clone();
            let saved = self.doc_state;
            self.doc_state = DocState {
                documented,
                ..Default::default()
            };
            let kind = if uncertain && !documented {
                DocKind::Opaque
            } else {
                DocKind::Field {
                    owner: owner.to_string(),
                    owner_public,
                }
            };
            self.doc_entry(&name, public || uncertain, range, kind);
            self.doc_state = saved;
            i = self.skip_element(i, close);
        }
    }
    fn record_variants(&mut self, open: usize, close: usize, owner: &str, owner_public: bool) {
        let mut i = open + 1;
        while i < close {
            let mut documented = false;
            let mut uncertain = false;
            while i < close {
                if self.tokens[i].doc.is_some() {
                    documented = true;
                    i += 1;
                } else if self.t(i) == "#" && self.t(i + 1) == "[" {
                    match self.t(i + 2) {
                        "doc" => documented = true,
                        // Inert tool attributes.
                        "rustfmt" | "clippy" | "default" => {}
                        // Lint levels and configuration are not modeled here.
                        _ => uncertain = true,
                    }
                    i = self.tokens[i + 1].close.map_or(i + 2, |c| c + 1);
                } else {
                    break;
                }
            }
            if i >= close {
                break;
            }
            let name = self.t(i).to_string();
            let saved = self.doc_state;
            self.doc_state = DocState {
                documented,
                ..Default::default()
            };
            let kind = if uncertain && !documented {
                DocKind::Opaque
            } else {
                DocKind::Variant {
                    owner: owner.to_string(),
                    owner_public,
                }
            };
            self.doc_entry(&name, true, self.tokens[i].range.clone(), kind);
            self.doc_state = saved;
            if self.t(i + 1) == "{"
                && let Some(c) = self.tokens[i + 1].close
            {
                self.record_fields(i + 1, c, owner, owner_public, true);
            }
            i = self.skip_element(i, close);
        }
    }
    /// Index after the comma ending the element at `i` (angle-aware).
    fn skip_element(&self, mut i: usize, close: usize) -> usize {
        let mut angle = 0usize;
        while i < close {
            match self.t(i) {
                "," if angle == 0 => return i + 1,
                "<" => angle += 1,
                ">" if i == 0 || self.t(i - 1) != "-" => angle = angle.saturating_sub(1),
                _ => {}
            }
            i = self.tokens[i].close.map_or(i + 1, |c| c + 1);
        }
        close
    }
    /// The documentation container of an impl or trait body, or `None` for
    /// trait impls (their items are exempt).
    pub(super) fn doc_container(
        &self,
        kind: &str,
        item_start: usize,
        keyword: usize,
        open: usize,
    ) -> Option<Container> {
        if kind == "trait" {
            return Some(Container::Trait(
                self.t(keyword + 1).trim_start_matches("r#").to_string(),
                self.plain_pub(item_start),
            ));
        }
        let mut angle = 0usize;
        let mut last = String::new();
        let mut i = keyword + 1;
        while i < open {
            let t = self.t(i);
            match t {
                "for" if angle == 0 => return None,
                "where" if angle == 0 => break,
                "<" => angle += 1,
                ">" if self.t(i - 1) != "-" => angle = angle.saturating_sub(1),
                _ if angle == 0 && is_ident(t) && !matches!(t, "dyn" | "mut" | "const") => {
                    last = t.trim_start_matches("r#").to_string();
                }
                _ => {}
            }
            i = self.tokens[i].close.map_or(i + 1, |c| c + 1);
        }
        Some(Container::Inherent(last))
    }
    /// One active member of an impl or trait body.
    pub(super) fn record_assoc_docs(&mut self, kind: &str, member: usize, keyword: usize) {
        let Some(container) = self.assoc_container.clone() else {
            return;
        };
        let public = matches!(container, Container::Trait(..)) || self.plain_pub(member);
        let range = self.tokens[keyword.min(self.tokens.len() - 1)]
            .range
            .clone();
        let name = self.t(keyword + 1).to_string();
        let entry = match kind {
            "fn" | "const" | "type" => DocKind::Assoc(container),
            _ => DocKind::Invocation(Some(container)),
        };
        self.doc_entry(&name, public, range, entry);
    }
}

impl Filter<'_> {
    /// The self type of `impl .. Debug for Target ..` whose header spans
    /// `keyword..open`: its last path segment, empty for non-path types.
    pub(super) fn debug_impl_target(&self, keyword: usize, open: usize) -> Option<String> {
        let mut angle = 0usize;
        let mut last = "";
        let mut i = keyword + 1;
        let mut for_at = None;
        while i < open {
            let t = self.t(i);
            match t {
                "for" if angle == 0 && self.t(i + 1) != "<" => {
                    for_at = Some(i);
                    break;
                }
                "{" | ";" if angle == 0 => break,
                "<" => angle += 1,
                ">" if self.t(i - 1) != "-" => angle = angle.saturating_sub(1),
                _ if angle == 0 && is_ident(t) => last = t,
                _ => {}
            }
            i = self.tokens[i].close.map_or(i + 1, |c| c + 1);
        }
        let for_at = for_at?;
        if last.trim_start_matches("r#") != "Debug" {
            return None;
        }
        let mut target = String::new();
        let mut i = for_at + 1;
        while i < open {
            let t = self.t(i);
            if t == ":" {
                i += 1;
                continue;
            }
            if !is_ident(t) || matches!(t, "where" | "dyn" | "fn" | "impl" | "for") {
                break;
            }
            target = t.trim_start_matches("r#").to_string();
            i += 1;
        }
        if i == for_at + 1 {
            target.clear();
        }
        Some(target)
    }

    /// `impl .. Debug ..` headers outside the module scopes walked here.
    pub(super) fn record_unplaced_debug_impls(&mut self) {
        let n = self.tokens.len();
        for k in 0..n {
            if self.tokens[k].doc.is_some()
                || self.t(k) != "impl"
                // `-> impl Debug`, `x: impl Debug` are types, not items.
                || (k > 0 && !matches!(self.t(k - 1), ";" | "{" | "}" | "]" | "unsafe"))
                || self.debug_impls.contains(&k)
                || self.token_masked(k)
            {
                continue;
            }
            let open = (k + 1..n)
                .find(|&j| matches!(self.t(j), "{" | ";"))
                .unwrap_or(n);
            if (k + 1..open).any(|j| self.t(j) == "Debug") {
                let range = self.tokens[k].range.clone();
                self.doc_entry("", false, range, DocKind::DebugUnknown);
            }
        }
    }
}

impl Filter<'_> {
    /// `name!(..);` at `start` invoking the file-local `macro_rules!` in
    /// textual scope whose every transcriber expands to trait impls only.
    /// `Some(mentions_debug)` when so, `None` when undecided.
    fn trait_impl_invocation(&self, start: usize) -> Option<bool> {
        let name = self.t(start).trim_start_matches("r#");
        if !is_ident(name) || self.t(start + 1) != "!" {
            return None;
        }
        self.tokens.get(start + 2)?.close?;
        if self
            .tokens
            .iter()
            .any(|t| t.doc.is_none() && t.text(self.text) == "macro_use")
        {
            return None;
        }
        // The latest earlier definition whose module body encloses the call.
        let def = self.macro_defs.iter().rev().find(|d| {
            let k = d.keyword();
            k < start
                && self.t(k + 2).trim_start_matches("r#") == name
                && (0..k)
                    .rev()
                    .find(|&o| self.tokens[o].close.is_some_and(|c| c > k))
                    .map_or(true, |o| self.tokens[o].close.is_some_and(|c| c > start))
        })?;
        let open = def.keyword() + 3;
        let close = self.tokens.get(open)?.close?;
        let mut debug = false;
        let mut i = open + 1;
        let mut rules = 0;
        while i < close {
            // (matcher) => {transcriber} ;
            let matcher_close = self.tokens[i].close?;
            if self.t(matcher_close + 1) != "=" || self.t(matcher_close + 2) != ">" {
                return None;
            }
            let t_open = matcher_close + 3;
            let t_close = self.tokens.get(t_open)?.close?;
            if !self.trait_impls_only(t_open + 1, t_close, &mut debug) {
                return None;
            }
            rules += 1;
            i = t_close + 1;
            if self.t(i) == ";" {
                i += 1;
            }
        }
        (rules > 0).then_some(debug)
    }

    fn trait_impls_only(&self, mut i: usize, end: usize, debug: &mut bool) -> bool {
        while i < end {
            let t = self.t(i);
            if t == "$" && self.t(i + 1) == "(" {
                let Some(close) = self.tokens[i + 1].close else {
                    return false;
                };
                if !self.trait_impls_only(i + 2, close, debug) {
                    return false;
                }
                i = close + 1;
                // Optional separator, then the repetition operator.
                if !matches!(self.t(i), "*" | "+" | "?") {
                    i += 1;
                }
                if !matches!(self.t(i), "*" | "+" | "?") {
                    return false;
                }
                i += 1;
                continue;
            }
            if t == "#" && self.t(i + 1) == "[" {
                let Some(close) = self.tokens[i + 1].close else {
                    return false;
                };
                i = close + 1;
                continue;
            }
            if t == ";" {
                i += 1;
                continue;
            }
            let k = if t == "unsafe" { i + 1 } else { i };
            if self.t(k) != "impl" {
                return false;
            }
            let mut j = k + 1;
            let mut trait_impl = false;
            while j < end && self.t(j) != "{" {
                let tj = self.t(j);
                if tj == "for" && self.t(j + 1) != "<" {
                    trait_impl = true;
                }
                *debug |= tj == "Debug";
                j = self.tokens[j].close.map_or(j + 1, |c| c + 1);
            }
            if !trait_impl || j >= end {
                return false;
            }
            let Some(close) = self.tokens[j].close else {
                return false;
            };
            *debug |= (j..close).any(|x| self.t(x) == "Debug");
            i = close + 1;
        }
        true
    }
}
