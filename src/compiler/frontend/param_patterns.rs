//! Destructured function and method parameters.
//!
//! `fn f((a, b): (A, B)) { body }` is the callable `fn f($param$0: (A, B))`
//! whose body starts with `let (a, b) = $param$0;`. The whole argument keeps
//! one ordinary parameter local, so the signature, ABI and call sites are
//! unchanged, while typing, irrefutability, moves and borrows of the pattern
//! are exactly those of the ordinary `let` pattern path.
//!
//! Stock drops parameters in reverse order, each one's bindings before the
//! part its pattern left unbound. Once a destructured parameter may own a
//! destructor, it and every later owning parameter are therefore moved into a
//! fresh body local (`let p = p;`) in parameter order, so scope exit replays
//! that order. Parameters that cannot need drop are never rebound.
use super::*;

/// One body-entry statement owed to a parameter.
pub(super) struct Prelude {
    local: AstIdent,
    /// `let local = local;` before any pattern, fixing the drop position.
    rebind: bool,
    mutable: bool,
    pattern: Option<Pattern>,
}

impl Prelude {
    pub(super) fn is_pattern(&self) -> bool {
        self.pattern.is_some()
    }
}

/// Syntactic over-approximation of "values of this type may run a destructor".
fn may_need_drop(ty: &Type) -> bool {
    match ty {
        Type::Int(_)
        | Type::Float(_)
        | Type::Char
        | Type::Bool
        | Type::Unit
        | Type::Str
        | Type::Never
        | Type::Ref { .. }
        | Type::RawPtr { .. }
        | Type::FnPointer { .. } => false,
        Type::Tuple(fields) => fields.iter().any(may_need_drop),
        Type::Array(element, _) => may_need_drop(element),
        _ => true,
    }
}

/// A scalar literal, range or unit path test anywhere in the pattern. Enum
/// constructors need resolution, so the body checker decides those.
fn literal_test(pattern: &Pattern) -> bool {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        match pattern {
            Pattern::Bool(_)
            | Pattern::Int { .. }
            | Pattern::Char(_)
            | Pattern::Str(_)
            | Pattern::Range { .. } => {
                return true;
            }
            Pattern::At { pattern, .. }
            | Pattern::Apply { pattern, .. }
            | Pattern::Ref { pattern, .. } => pending.push(pattern),
            Pattern::Alternatives(fields) | Pattern::Tuple(fields) | Pattern::Array(fields) => {
                pending.extend(fields);
            }
            Pattern::TupleRest(rest)
            | Pattern::ArrayRest(rest)
            | Pattern::VariantRest { fields: rest, .. } => {
                pending.extend(rest.prefix.iter().chain(&rest.suffix));
            }
            Pattern::Variant { fields, .. } => {
                pending.extend(fields.iter().map(|(_, pattern)| pattern));
            }
            Pattern::Struct { fields, .. } => {
                pending.extend(fields.iter().map(|(_, pattern)| pattern));
            }
            // Paths, bindings, wildcards and a subslice `..` test nothing here.
            _ => {}
        }
    }
    false
}

/// Bindings one pattern introduces. Alternatives bind the same names, so the
/// first row stands for all of them.
fn bindings<'a>(pattern: &'a Pattern, out: &mut Vec<AstIdent>, pending: &mut Vec<&'a Pattern>) {
    pending.push(pattern);
    while let Some(pattern) = pending.pop() {
        match pattern {
            Pattern::Binding { name, .. } => out.push(*name),
            Pattern::At { name, pattern, .. } => {
                out.push(*name);
                pending.push(pattern);
            }
            Pattern::Apply { pattern, .. } | Pattern::Ref { pattern, .. } => pending.push(pattern),
            Pattern::Alternatives(rows) => pending.extend(rows.first()),
            Pattern::Tuple(fields) | Pattern::Array(fields) => pending.extend(fields),
            Pattern::TupleRest(rest)
            | Pattern::ArrayRest(rest)
            | Pattern::VariantRest { fields: rest, .. } => {
                pending.extend(rest.prefix.iter().chain(&rest.suffix));
            }
            Pattern::Variant { fields, .. } => {
                pending.extend(fields.iter().map(|(_, pattern)| pattern));
            }
            Pattern::Struct { fields, .. } => {
                pending.extend(fields.iter().map(|(_, pattern)| pattern));
            }
            // No bindings: literals, paths, wildcards and a subslice `..`.
            _ => {}
        }
    }
}

impl Parser<'_, '_> {
    /// Parses one parameter pattern when the parameter is not a plain
    /// `mut? name` or `_` binding.
    pub(super) fn destructured_parameter(&mut self) -> Result<Option<Pattern>, ParseError> {
        let next = self.cursor.peek(1);
        let starts = ["(", "&", "&&", "[", "ref"]
            .iter()
            .any(|text| self.is(text));
        let constructor = !self.is("mut")
            && !self.is("_")
            && next
                .is_some_and(|token| ["(", "{", "::", "@"].iter().any(|text| token.matches(text)));
        // Anything that cannot start a binding (a literal, `-1`, `true`) is a
        // pattern too, so it is diagnosed as one.
        let binding = self.is_identifier() || self.is("_") || self.is("mut") || self.is("self");
        if binding && !(starts || constructor) {
            return Ok(None);
        }
        let pattern = self.pattern_no_alternatives()?;
        if self.is("|") {
            return Err(self.error(
                "top-level or-patterns are not allowed in function parameters; wrap the pattern in parentheses",
            ));
        }
        if literal_test(&pattern) {
            return Err(self.error("E0005: refutable pattern in function argument"));
        }
        Ok(Some(pattern))
    }

    /// Records what the body owes this parameter and returns the name of its
    /// whole-argument local.
    pub(super) fn parameter_prelude(
        &mut self,
        name: BindingName,
        mutable: bool,
        pattern: Option<Pattern>,
        ty: &Type,
        ordinal: usize,
    ) -> Result<BindingName, ParseError> {
        let ordered = self
            .lexical
            .pending_param_patterns
            .iter()
            .any(|prior| prior.rebind);
        let rebind = may_need_drop(ty) && (ordered || pattern.is_some());
        if pattern.is_none() && !rebind {
            return Ok(name);
        }
        let local = match name {
            BindingName::Named(local) if pattern.is_none() => local,
            _ => {
                let site = panic_callsite(self.names.session(), self.token().span)
                    .map_err(native_adapter::transport_error)?;
                self.names
                    .anonymous_parameter(site, ordinal)
                    .map_err(native_adapter::name_error)?
            }
        };
        let mut pending = std::mem::take(&mut self.lexical.pending_param_patterns);
        self.reserve_parser_items(&mut pending, 1)?;
        pending.push(Prelude {
            local,
            rebind,
            mutable,
            pattern,
        });
        self.lexical.pending_param_patterns = pending;
        Ok(BindingName::Named(local))
    }

    /// E0415: one parameter list binds each identifier at most once.
    pub(super) fn distinct_param_bindings(
        &mut self,
        params: &[(BindingName, Type)],
        destructured: &[Prelude],
    ) -> Result<(), ParseError> {
        if !destructured.iter().any(Prelude::is_pattern) {
            return Ok(());
        }
        let mut seen = std::collections::HashSet::new();
        for (name, _) in params {
            if let BindingName::Named(name) = name {
                seen.insert(self.name_key(*name, LookupClass::Local)?);
            }
        }
        let mut names = Vec::new();
        let mut pending = Vec::new();
        for pattern in destructured.iter().filter_map(|p| p.pattern.as_ref()) {
            bindings(pattern, &mut names, &mut pending);
        }
        for name in names {
            if !seen.insert(self.name_key(name, LookupClass::Local)?) {
                return Err(
                    self.error("E0415: identifier is bound more than once in this parameter list")
                );
            }
        }
        Ok(())
    }

    /// Pattern names enter the body scope after every whole parameter.
    pub(super) fn bind_param_patterns(
        &mut self,
        destructured: &[Prelude],
    ) -> Result<(), ParseError> {
        for prelude in destructured {
            self.reference_lexical(prelude.local)?;
            if prelude.rebind {
                self.bind_lexical(prelude.local)?;
            }
            if let Some(pattern) = &prelude.pattern {
                if prelude.rebind {
                    self.reference_lexical(prelude.local)?;
                }
                self.bind_pattern_lexical(pattern)?;
            }
        }
        Ok(())
    }
}

/// Destructuring runs in parameter order before the first written statement.
pub(super) fn prepend(
    parser: &mut Parser<'_, '_>,
    mut body: Block,
    destructured: Vec<Prelude>,
) -> Result<Block, ParseError> {
    if destructured.is_empty() {
        return Ok(body);
    }
    let mut statements = Vec::new();
    parser.reserve_parser_items(
        &mut statements,
        2 * destructured.len() + body.statements.len(),
    )?;
    for prelude in destructured {
        let whole = |parser: &mut Parser<'_, '_>| {
            AstPath::new(parser.names, PathAnchor::Relative, &[prelude.local])
                .map(Expr::Var)
                .map_err(native_adapter::name_error)
        };
        if prelude.rebind {
            statements.push(Stmt::Let {
                name: BindingName::Named(prelude.local),
                mutable: prelude.mutable,
                ty: Type::Infer,
                value: whole(parser)?,
            });
        }
        if let Some(pattern) = prelude.pattern {
            statements.push(Stmt::LetPattern {
                pattern,
                ty: Type::Infer,
                value: whole(parser)?,
            });
        }
    }
    statements.append(&mut body.statements);
    body.statements = statements;
    Ok(body)
}
