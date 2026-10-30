//! What several lints ask of the code, as clippy's `clippy_utils` answers it.

use hir::PathResolution;
use ide_db::famous_defs::FamousDefs;
use syntax::{
    AstNode, SyntaxNode, TextRange, TextSize,
    ast::{self, HasArgList as _},
};

use crate::check::Cx;

/// A method call: `receiver.name(args)`.
pub(crate) struct MethodCall {
    pub(crate) name: String,
    /// The method's name, where it is written.
    pub(crate) name_ref: ast::NameRef,
    pub(crate) receiver: ast::Expr,
    pub(crate) args: Vec<ast::Expr>,
}

/// clippy's `method_call`: `expr` as a method call whose receiver and arguments are all
/// written in the file, none of them made by a macro.
pub(crate) fn method_call(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> Option<MethodCall> {
    let ast::Expr::MethodCallExpr(call) = expr else { return None };
    let receiver = call.receiver()?;
    let args: Vec<ast::Expr> = call.arg_list()?.args().collect();
    if !cx.is_written(receiver.syntax()) || !args.iter().all(|it| cx.is_written(it.syntax())) {
        return None;
    }
    let name_ref = call.name_ref()?;
    Some(MethodCall { name: name_ref.text().to_owned(), name_ref, receiver, args })
}

/// `Option` or `Result`, when `ty`, its references peeled, is one of them.
pub(crate) fn option_or_result(cx: &Cx<'_, '_, '_>, ty: &hir::Type<'_>) -> Option<&'static str> {
    let adt = ty.strip_references().as_adt()?;
    let famous = FamousDefs(cx.sema(), cx.krate());
    let option = famous.core_option_Option().map(hir::Adt::Enum);
    let result = famous.core_result_Result().map(hir::Adt::Enum);
    match () {
        _ if option == Some(adt) => Some("Option"),
        _ if result == Some(adt) => Some("Result"),
        _ => None,
    }
}

/// clippy's `is_none_expr`: `expr` is `None`, `Option`'s.
pub(crate) fn is_none(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> bool {
    let ast::Expr::PathExpr(path) = expr else { return false };
    let Some(path) = path.path() else { return false };
    let Some(PathResolution::Def(hir::ModuleDef::EnumVariant(variant))) =
        cx.sema().resolve_path(&path)
    else {
        return false;
    };
    let famous = FamousDefs(cx.sema(), cx.krate());
    famous.core_option_Option() == Some(variant.parent_enum(cx.db()))
        && variant.name(cx.db()).as_str() == "None"
}

/// `expr` is the literal `value`.
pub(crate) fn is_bool_literal(expr: &ast::Expr, value: bool) -> bool {
    let ast::Expr::Literal(literal) = expr else { return false };
    matches!(literal.kind(), ast::LiteralKind::Bool(it) if it == value)
}

/// The locals the paths of `node` name.
pub(crate) fn locals_in<'db>(cx: &Cx<'_, '_, 'db>, node: &SyntaxNode) -> Vec<hir::Local<'db>> {
    let mut found = Vec::new();
    for path in node.descendants().filter_map(ast::PathExpr::cast) {
        if let Some(local) = local_of(cx, &path)
            && !found.contains(&local)
        {
            found.push(local);
        }
    }
    found
}

pub(crate) fn local_of<'db>(cx: &Cx<'_, '_, 'db>, path: &ast::PathExpr) -> Option<hir::Local<'db>> {
    match cx.sema().resolve_path(&path.path()?)? {
        PathResolution::Local(local) => Some(local),
        _ => None,
    }
}

/// The body `node` is in, as rustc's `hir_enclosing_body_owner` finds it: its closure, its
/// function, or the initializer of its constant or static.
pub(crate) fn enclosing_body(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.ancestors().skip(1).find_map(|it| {
        if let Some(closure) = ast::ClosureExpr::cast(it.clone()) {
            return closure.body().map(|it| it.syntax().clone());
        }
        if let Some(function) = ast::Fn::cast(it.clone()) {
            return function.body().map(|it| it.syntax().clone());
        }
        if let Some(konst) = ast::Const::cast(it.clone()) {
            return konst.body().map(|it| it.syntax().clone());
        }
        ast::Static::cast(it).and_then(|it| it.body()).map(|it| it.syntax().clone())
    })
}

/// `expr`, or what initialised the binding it names when that binding is not `mut`, as
/// clippy's `expr_or_init` follows it.
pub(crate) fn expr_or_init(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> ast::Expr {
    let mut expr = expr.clone();
    for _ in 0..16 {
        let ast::Expr::PathExpr(path) = &expr else { break };
        let Some(local) = local_of(cx, path) else { break };
        let Some(init) = binding_init(cx, local) else { break };
        expr = init;
    }
    expr
}

/// What `let` initialised `local` with, if it is bound by a plain `let` and not `mut`.
fn binding_init(cx: &Cx<'_, '_, '_>, local: hir::Local<'_>) -> Option<ast::Expr> {
    let source = local.primary_source(cx.db());
    let file = source.source.file_id.file_id()?;
    let range = source.as_ident_pat()?.syntax().text_range();
    // The tree semantics knows, so that what is found in it can be asked of.
    let root = cx.sema().parse(file);
    let pat = root.syntax().covering_element(range).ancestors().find_map(ast::IdentPat::cast)?;
    if pat.mut_token().is_some() || pat.ref_token().is_some() {
        return None;
    }
    let let_stmt = ast::LetStmt::cast(pat.syntax().parent()?)?;
    let_stmt.initializer()
}

/// clippy's `Sugg::hir_with_context` for `expr`, written in the file.
pub(crate) fn sugg(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> crate::sugg::Sugg {
    let snippet = |node: &SyntaxNode| cx.source_of(node);
    let same_type = |prefix: &ast::PrefixExpr| {
        let inner = prefix.expr();
        let outer = cx.ty(&ast::Expr::PrefixExpr(prefix.clone()));
        inner.and_then(|it| cx.ty(&it)).zip(outer).is_some_and(|(a, b)| a == b)
    };
    crate::sugg::Sugg::of(expr, &snippet, &same_type)
}

/// clippy's `peel_blocks`: `expr` with the blocks around it that only hold an expression
/// taken off.
pub(crate) fn peel_blocks(expr: &ast::Expr) -> ast::Expr {
    let mut expr = expr.clone();
    while let ast::Expr::BlockExpr(block) = &expr
        && block.modifier().is_none()
        && block.label().is_none()
        && block.statements().next().is_none()
        && let Some(tail) = block.tail_expr()
    {
        expr = tail;
    }
    expr
}

/// clippy's `peel_blocks_with_stmt`: as [`peel_blocks`], a block holding one expression
/// statement and nothing else taken off too.
pub(crate) fn peel_blocks_with_stmt(expr: &ast::Expr) -> ast::Expr {
    let mut expr = expr.clone();
    loop {
        let ast::Expr::BlockExpr(block) = &expr else { return expr };
        if block.modifier().is_some() || block.label().is_some() {
            return expr;
        }
        let statements: Vec<ast::Stmt> = block.statements().collect();
        expr = match (&statements[..], block.tail_expr()) {
            ([], Some(tail)) => tail,
            ([ast::Stmt::ExprStmt(stmt)], None) => match stmt.expr() {
                Some(inner) => inner,
                None => return expr,
            },
            _ => return expr,
        };
    }
}

/// clippy's `span_contains_comment`: whether a comment is written in `range` of `node`.
pub(crate) fn contains_comment(node: &SyntaxNode, range: TextRange) -> bool {
    node.descendants_with_tokens().filter_map(|it| it.into_token()).any(|it| {
        matches!(
            it.kind(),
            syntax::SyntaxKind::COMMENT
                | syntax::SyntaxKind::INNER_DOC_COMMENT
                | syntax::SyntaxKind::OUTER_DOC_COMMENT
        ) && range.contains_range(it.text_range())
    })
}

/// Calls `f` on each path expression of `root` that starts at or after `after`, and of what
/// the macro calls there expand to.
pub(crate) fn each_path(
    cx: &Cx<'_, '_, '_>,
    root: &SyntaxNode,
    after: Option<TextSize>,
    f: &mut dyn FnMut(&ast::PathExpr),
) {
    each_path_at(cx, root, after, 0, f);
}

fn each_path_at(
    cx: &Cx<'_, '_, '_>,
    root: &SyntaxNode,
    after: Option<TextSize>,
    depth: u32,
    f: &mut dyn FnMut(&ast::PathExpr),
) {
    const DEEPEST: u32 = 8;
    for node in root.descendants() {
        if after.is_some_and(|it| node.text_range().start() < it) {
            continue;
        }
        if let Some(path) = ast::PathExpr::cast(node.clone()) {
            f(&path);
        } else if let Some(call) = ast::MacroCall::cast(node)
            && depth < DEEPEST
            && let Some(expansion) = cx.sema().expand_macro_call(&call)
        {
            each_path_at(cx, &expansion.value, None, depth + 1, f);
        }
    }
}
