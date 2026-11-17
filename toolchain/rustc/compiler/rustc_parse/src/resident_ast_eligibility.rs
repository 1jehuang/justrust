//! Bounded syntax-only admission for a deliberately tiny pre-expansion AST subset.
//!
//! This does not authorize reuse: the caller must independently prove parser-state
//! purity, source identity, source-map/session lifetime, and symbol-gallery replay.
//! Estimated bytes count visited structures and symbol spellings, not allocator
//! capacity or shared interner storage. Retained lazy token streams are rejected.

use std::mem::size_of;

use rustc_ast::{self as ast, token};
use rustc_span::{Symbol, kw, sym};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub max_items: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_estimated_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_items: 256,
            max_nodes: 4096,
            max_depth: 64,
            max_estimated_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Admission {
    pub items: usize,
    pub nodes: usize,
    pub estimated_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Unsupported,
    NonDummyId,
    BudgetExceeded,
}

/// Inspect the actual parsed tree without mutating it or any parser/session state.
/// Hard ceilings apply even when callers supply larger limits.
pub(crate) fn inspect(krate: &ast::Crate, limits: Limits) -> Result<Admission, Error> {
    let mut check = Check { limits, admission: Admission::default() };
    check.node::<ast::Crate>(1)?;
    if krate.id != ast::DUMMY_NODE_ID && krate.id != ast::CRATE_NODE_ID {
        return Err(Error::NonDummyId);
    }
    if krate.is_placeholder || !krate.attrs.is_empty() {
        return Err(Error::Unsupported);
    }
    if krate.items.len() > limits.max_items.min(256) {
        return Err(Error::BudgetExceeded);
    }
    for item in &krate.items {
        check.item(item)?;
    }
    Ok(check.admission)
}

struct Check {
    limits: Limits,
    admission: Admission,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Primitive {
    Integer,
    Bool,
}

impl Check {
    fn bytes(&mut self, bytes: usize) -> Result<(), Error> {
        self.admission.estimated_bytes =
            self.admission.estimated_bytes.checked_add(bytes).ok_or(Error::BudgetExceeded)?;
        if self.admission.estimated_bytes > self.limits.max_estimated_bytes.min(4 * 1024 * 1024) {
            return Err(Error::BudgetExceeded);
        }
        Ok(())
    }

    fn node<T>(&mut self, depth: usize) -> Result<(), Error> {
        if depth > self.limits.max_depth.min(64) {
            return Err(Error::BudgetExceeded);
        }
        self.admission.nodes = self.admission.nodes.checked_add(1).ok_or(Error::BudgetExceeded)?;
        if self.admission.nodes > self.limits.max_nodes.min(4096) {
            return Err(Error::BudgetExceeded);
        }
        self.bytes(size_of::<T>())
    }

    fn id(&self, id: ast::NodeId) -> Result<(), Error> {
        if id == ast::DUMMY_NODE_ID { Ok(()) } else { Err(Error::NonDummyId) }
    }

    fn symbol(&mut self, symbol: Symbol) -> Result<(), Error> {
        self.bytes(symbol.as_str().len())
    }

    fn path(&mut self, path: &ast::Path, depth: usize) -> Result<Symbol, Error> {
        self.node::<ast::Path>(depth)?;
        let [segment] = path.segments.as_slice() else { return Err(Error::Unsupported) };
        self.node::<ast::PathSegment>(depth + 1)?;
        self.id(segment.id)?;
        if segment.args.is_some() {
            return Err(Error::Unsupported);
        }
        self.symbol(segment.ident.name)?;
        Ok(segment.ident.name)
    }

    fn ty(&mut self, ty: &ast::Ty, depth: usize) -> Result<Primitive, Error> {
        self.node::<ast::Ty>(depth)?;
        self.id(ty.id)?;
        let ast::TyKind::Path(None, path) = &ty.kind else { return Err(Error::Unsupported) };
        match self.path(path, depth + 1)? {
            name if name == sym::u32 || name == sym::u64 => Ok(Primitive::Integer),
            name if name == sym::bool => Ok(Primitive::Bool),
            _ => Err(Error::Unsupported),
        }
    }

    fn item(&mut self, item: &ast::Item) -> Result<(), Error> {
        self.node::<ast::Item>(2)?;
        self.id(item.id)?;
        self.admission.items = self.admission.items.checked_add(1).ok_or(Error::BudgetExceeded)?;
        if !item.attrs.is_empty()
            || item.tokens.is_some()
            || !matches!(
                item.vis.kind,
                ast::VisibilityKind::Inherited | ast::VisibilityKind::Public
            )
        {
            return Err(Error::Unsupported);
        }
        let fun = match &item.kind {
            ast::ItemKind::Mod(ast::Safety::Default, ident, ast::ModKind::Unloaded) => {
                self.symbol(ident.name)?;
                return Ok(());
            }
            ast::ItemKind::Fn(fun) => fun,
            _ => return Err(Error::Unsupported),
        };
        self.node::<ast::Fn>(3)?;
        self.symbol(fun.ident.name)?;
        let header = &fun.sig.header;
        if !matches!(header.safety, ast::Safety::Default)
            || !matches!(header.constness, ast::Const::No)
            || !matches!(header.ext, ast::Extern::None)
            || header.coroutine_marker.is_some()
            || !matches!(fun.defaultness, ast::Defaultness::Implicit)
            || !fun.generics.params.is_empty()
            || !fun.generics.where_clause.is_empty()
            || fun.contract.is_some()
            || fun.define_opaque.is_some()
            || fun.eii_impl.is_some()
            || fun.sig.decl.has_self()
            || fun.sig.decl.c_variadic()
        {
            return Err(Error::Unsupported);
        }
        self.node::<ast::FnDecl>(4)?;
        // Bound iteration and temporary storage before inspecting any parameters.
        if fun.sig.decl.inputs.len() > self.limits.max_nodes.min(4096) {
            return Err(Error::BudgetExceeded);
        }
        let mut args = Vec::new();
        for param in &fun.sig.decl.inputs {
            self.node::<ast::Param>(5)?;
            self.id(param.id)?;
            if param.is_placeholder || !param.attrs.is_empty() {
                return Err(Error::Unsupported);
            }
            self.node::<ast::Pat>(6)?;
            self.id(param.pat.id)?;
            let ast::PatKind::Ident(ast::BindingMode::NONE, ident, None) = &param.pat.kind else {
                return Err(Error::Unsupported);
            };
            if ident.name == kw::SelfLower || args.iter().any(|(name, _)| *name == ident.name) {
                return Err(Error::Unsupported);
            }
            self.symbol(ident.name)?;
            let primitive = self.ty(&param.ty, 6)?;
            args.push((ident.name, primitive));
        }
        let ast::FnRetTy::Ty(output) = &fun.sig.decl.output else {
            return Err(Error::Unsupported);
        };
        self.ty(output, 5)?;
        let Some(body) = &fun.body else { return Err(Error::Unsupported) };
        self.node::<ast::Block>(4)?;
        self.id(body.id)?;
        if !matches!(body.rules, ast::BlockCheckMode::Default) {
            return Err(Error::Unsupported);
        }
        let [stmt] = body.stmts.as_slice() else { return Err(Error::Unsupported) };
        self.node::<ast::Stmt>(5)?;
        self.id(stmt.id)?;
        let ast::StmtKind::Expr(expr) = &stmt.kind else { return Err(Error::Unsupported) };
        self.expr(expr, &args, 6)?;
        Ok(())
    }

    fn expr(
        &mut self,
        expr: &ast::Expr,
        args: &[(Symbol, Primitive)],
        depth: usize,
    ) -> Result<Primitive, Error> {
        self.node::<ast::Expr>(depth)?;
        self.id(expr.id)?;
        if !expr.attrs.is_empty() || expr.tokens.is_some() {
            return Err(Error::Unsupported);
        }
        match &expr.kind {
            ast::ExprKind::Lit(lit) => {
                self.symbol(lit.symbol)?;
                match lit.kind {
                    token::LitKind::Bool
                        if lit.suffix.is_none()
                            && (lit.symbol == kw::True || lit.symbol == kw::False) =>
                    {
                        Ok(Primitive::Bool)
                    }
                    token::LitKind::Integer if lit.suffix.is_none() => {
                        // Decimal only, checked without allocating or trusting a mutated token.
                        let text = lit.symbol.as_str();
                        if !text.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                            return Err(Error::Unsupported);
                        }
                        let mut value = 0u64;
                        let mut digit = false;
                        for byte in text.bytes() {
                            if byte == b'_' {
                                continue;
                            }
                            if !byte.is_ascii_digit() {
                                return Err(Error::Unsupported);
                            }
                            digit = true;
                            value = value
                                .checked_mul(10)
                                .and_then(|v| v.checked_add(u64::from(byte - b'0')))
                                .ok_or(Error::Unsupported)?;
                        }
                        if !digit {
                            return Err(Error::Unsupported);
                        }
                        Ok(Primitive::Integer)
                    }
                    _ => Err(Error::Unsupported),
                }
            }
            ast::ExprKind::Path(None, path) => {
                let name = self.path(path, depth + 1)?;
                args.iter()
                    .find(|(arg, _)| *arg == name)
                    .map(|(_, ty)| *ty)
                    .ok_or(Error::Unsupported)
            }
            ast::ExprKind::Paren(inner) => self.expr(inner, args, depth + 1),
            ast::ExprKind::Unary(ast::UnOp::Neg | ast::UnOp::Not, inner) => {
                if self.expr(inner, args, depth + 1)? != Primitive::Integer {
                    return Err(Error::Unsupported);
                }
                Ok(Primitive::Integer)
            }
            ast::ExprKind::Binary(op, left, right)
                if matches!(
                    op.node,
                    ast::BinOpKind::Add
                        | ast::BinOpKind::Sub
                        | ast::BinOpKind::Mul
                        | ast::BinOpKind::Div
                        | ast::BinOpKind::Rem
                        | ast::BinOpKind::BitXor
                        | ast::BinOpKind::BitAnd
                        | ast::BinOpKind::BitOr
                        | ast::BinOpKind::Shl
                        | ast::BinOpKind::Shr
                ) =>
            {
                if self.expr(left, args, depth + 1)? != Primitive::Integer
                    || self.expr(right, args, depth + 1)? != Primitive::Integer
                {
                    return Err(Error::Unsupported);
                }
                Ok(Primitive::Integer)
            }
            _ => Err(Error::Unsupported),
        }
    }
}

#[cfg(test)]
mod tests {
    use rustc_session::parse::ParseSess;
    use rustc_span::{FileName, create_default_session_globals_then};

    use super::*;
    use crate::lexer::StripTokens;

    fn with_crate(source: &str, test: impl FnOnce(ast::Crate)) {
        create_default_session_globals_then(|| {
            let psess = ParseSess::new();
            let mut parser = crate::unwrap_or_emit_fatal(crate::new_parser_from_source_str(
                &psess,
                FileName::Custom("resident-ast-test".into()),
                source.into(),
                StripTokens::Nothing,
            ));
            let krate = parser.parse_crate_mod().unwrap();
            psess.dcx().abort_if_errors();
            test(krate);
        });
    }

    #[test]
    fn primitive_functions_and_fresh_modules() {
        for source in [
            "",
            "mod fresh;",
            "pub mod fresh;",
            "fn f(x: u32, y: u64) -> u64 { (x + y) * !1 }",
            "pub fn f(x: bool) -> bool { x }",
            "fn f() -> bool { true }",
        ] {
            with_crate(source, |krate| {
                let admission = inspect(&krate, Limits::default()).unwrap();
                assert_eq!(admission.items, krate.items.len());
                assert!(admission.nodes > 0);
                assert!(admission.estimated_bytes >= size_of::<ast::Crate>());
            });
        }
    }

    #[test]
    fn unsupported_syntax_fails_closed() {
        for source in [
            "#![allow(dead_code)] fn f() -> u32 { 1 }",
            "#[inline] fn f() -> u32 { 1 }",
            "fn f<T>() -> u32 { 1 }",
            "fn f() -> u32 where { 1 }",
            "const fn f() -> u32 { 1 }",
            "unsafe fn f() -> u32 { 1 }",
            "extern fn f() -> u32 { 1 }",
            "fn f(mut x: u32) -> u32 { x }",
            "fn f(x: i32) -> u32 { 1 }",
            "fn f() {}",
            "fn f() -> u32 { let x = 1; x }",
            "fn f() -> u32 { g() }",
            "fn f() -> u32 { m!() }",
            "fn f() -> u32 { { 1 } }",
            "fn f() -> u32 { 1; }",
            "fn f() -> u32 { unknown }",
            "mod loaded {}",
            "pub(crate) fn f() -> u32 { 1 }",
            "fn f() -> u32 { 1f32 }",
            "fn f() -> u32 { 1u32 }",
            "fn f() -> u64 { 1u64 }",
            "fn f(#[allow(unused)] x: u32) -> u32 { x }",
            "fn f(x: &u32) -> u32 { 1 }",
            "fn f((x, y): u32) -> u32 { 1 }",
            "fn f() -> u32 { if true { 1 } else { 2 } }",
            "fn f() -> u32 { unsafe { 1 } }",
            "fn f() -> u32 { 18446744073709551616 }",
            "fn f() -> bool { true && false }",
            "fn f(x: bool) -> bool { !x }",
        ] {
            with_crate(source, |krate| {
                assert!(inspect(&krate, Limits::default()).is_err(), "{source}")
            });
        }
    }

    #[test]
    fn nested_ids_and_recovery_nodes_fail_closed() {
        for mutation in 0..6 {
            with_crate("fn f(x: u32) -> u32 { x }", |mut krate| {
                let ast::ItemKind::Fn(fun) = &mut krate.items[0].kind else { panic!() };
                let param = &mut fun.sig.decl.inputs[0];
                match mutation {
                    0 => param.id = ast::CRATE_NODE_ID,
                    1 => param.pat.id = ast::CRATE_NODE_ID,
                    2 => param.ty.id = ast::CRATE_NODE_ID,
                    3 => {
                        let ast::TyKind::Path(_, path) = &mut param.ty.kind else { panic!() };
                        path.segments[0].id = ast::CRATE_NODE_ID;
                    }
                    4 => param.pat.kind = ast::PatKind::Missing,
                    _ => {
                        let body = fun.body.as_mut().unwrap();
                        let ast::StmtKind::Expr(expr) = &mut body.stmts[0].kind else { panic!() };
                        expr.id = ast::CRATE_NODE_ID;
                    }
                }
                assert!(inspect(&krate, Limits::default()).is_err());
            });
        }
    }

    #[test]
    fn ids_placeholders_and_budgets() {
        with_crate("fn f(x: u32) -> u32 { x + 1 }", |mut krate| {
            assert!(inspect(&krate, Limits::default()).is_ok());
            for limits in [
                Limits { max_items: 0, ..Limits::default() },
                Limits { max_nodes: 1, ..Limits::default() },
                Limits { max_depth: 1, ..Limits::default() },
                Limits { max_estimated_bytes: 0, ..Limits::default() },
            ] {
                assert_eq!(inspect(&krate, limits), Err(Error::BudgetExceeded));
            }
            krate.id = ast::CRATE_NODE_ID;
            assert!(inspect(&krate, Limits::default()).is_ok());
            krate.items[0].id = ast::CRATE_NODE_ID;
            assert_eq!(inspect(&krate, Limits::default()), Err(Error::NonDummyId));
            krate.items[0].id = ast::DUMMY_NODE_ID;
            let ast::ItemKind::Fn(fun) = &mut krate.items[0].kind else { panic!() };
            fun.sig.decl.inputs[0].is_placeholder = true;
            assert_eq!(inspect(&krate, Limits::default()), Err(Error::Unsupported));
        });
    }
}
