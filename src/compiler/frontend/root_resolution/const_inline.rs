//! General constant and static items by use-site inlining.
//!
//! Scalar constants with a pure arithmetic initializer keep the graph CTFE
//! path (CONST_VALUE facts). Every other constant (aggregates, references,
//! strings, floats, `const fn` calls) is expanded at each use as
//! `{ let NAME: TY = INIT; NAME }`, resolved in the declaring module. Rust
//! constants are values copied into each use, so this is semantically exact
//! for the pure initializers admitted here.
use super::*;

pub(super) fn needs_inline(declaration: &ConstDecl) -> bool {
    let scalar = matches!(declaration.ty, Type::Bool)
        || matches!(declaration.ty, Type::Int(ty) if ty != ir::IntTy::Isize);
    !(scalar
        && (simple(&declaration.initializer)
            || (declaration.assertion && assertion_operand(&declaration.initializer))))
}

/// Constants the scalar evaluator does not compute are inlined at each use and
/// type checked, but never evaluated. Stock rustc evaluates every constant
/// (E0080 on panic or overflow) even when unused. An initializer that reaches
/// `panic!` unconditionally is that error. One with any other operation that
/// can fail during evaluation gets an explicit unsupported diagnostic, never
/// an unevaluated acceptance. Calls keep their E0015/const-fn qualification.
pub(super) fn validate_unevaluated(declaration: &ConstDecl) -> Result<(), String> {
    if !needs_inline(declaration) {
        return Ok(());
    }
    let float = matches!(declaration.ty, Type::Float(_));
    match fallible(&declaration.initializer, float, 0) {
        Fallible::No => Ok(()),
        Fallible::Panics => Err("E0080: evaluation of constant value failed: evaluation panicked".into()),
        Fallible::Maybe => Err("unsupported: constant initializers with operations that may panic or overflow during evaluation (arithmetic, indexing, assertions, conditional panics) are not evaluated by this compiler".into()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fallible {
    No,
    Maybe,
    Panics,
}

fn fallible(expr: &Expr, float: bool, depth: usize) -> Fallible {
    if depth > 256 {
        return Fallible::Maybe;
    }
    let combine = |items: &mut dyn Iterator<Item = &Expr>| {
        let mut result = Fallible::No;
        for item in items {
            match fallible(item, float, depth + 1) {
                Fallible::No => {}
                other => {
                    if result == Fallible::No {
                        result = other;
                    }
                    if other == Fallible::Panics {
                        // Evaluation order reaches this only if no earlier
                        // operand might fail first; keep the first verdict.
                    }
                }
            }
            if result != Fallible::No {
                break;
            }
        }
        result
    };
    match expr {
        Expr::Panic(_) => Fallible::Panics,
        Expr::Binary {
            op, left, right, ..
        } => {
            let arithmetic = matches!(
                op,
                BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem
                    | BinaryOp::Shl
                    | BinaryOp::Shr
            );
            if arithmetic && !float {
                return Fallible::Maybe;
            }
            combine(&mut [left.as_ref(), right.as_ref()].into_iter())
        }
        Expr::Unary {
            op: UnaryOp::Neg,
            value,
            ..
        } if !float => {
            if matches!(value.as_ref(), Expr::Integer(_)) {
                Fallible::No
            } else {
                Fallible::Maybe
            }
        }
        Expr::Unary { value, .. } => fallible(value, float, depth + 1),
        Expr::If { .. }
        | Expr::IfLet { .. }
        | Expr::Match { .. }
        | Expr::Loop { .. }
        | Expr::LabeledBlock { .. }
        | Expr::Index { .. }
        | Expr::DynamicIndex { .. }
        | Expr::RangeIndex { .. }
        | Expr::Try(_)
        | Expr::Deref { .. } => Fallible::Maybe,
        Expr::Block(block) | Expr::Unsafe(block) => {
            let mut parts = Vec::new();
            for statement in &block.statements {
                match statement {
                    Stmt::Expr(value) | Stmt::ImplicitExpr(value) => parts.push(value),
                    Stmt::Let { value, .. } => parts.push(value),
                    _ => return Fallible::Maybe,
                }
            }
            if let Some(tail) = block.tail.as_deref() {
                parts.push(tail);
            }
            combine(&mut parts.into_iter())
        }
        Expr::Array(items) | Expr::Tuple(items) => combine(&mut items.iter()),
        Expr::TupleStruct { fields, .. } => combine(&mut fields.iter()),
        Expr::Call { args, .. } => combine(&mut args.iter()),
        Expr::MethodCall { receiver, args, .. } => {
            combine(&mut std::iter::once(receiver.as_ref()).chain(args.iter()))
        }
        Expr::AssociatedCall { args, .. } | Expr::TraitCall { args, .. } => {
            combine(&mut args.iter())
        }
        Expr::Struct { fields, .. } => combine(&mut fields.iter().map(|(_, value)| value)),
        Expr::EnumVariant { fields, .. } => combine(&mut fields.iter().map(|(_, value)| value)),
        Expr::Repeat { value, .. }
        | Expr::ParamRepeat { value, .. }
        | Expr::Apply { value, .. }
        | Expr::BorrowValue { value, .. }
        | Expr::Cast { value, .. } => fallible(value, float, depth + 1),
        Expr::Field { base, .. } => fallible(base, float, depth + 1),
        _ => Fallible::No,
    }
}

/// A `const _` assertion condition the scalar constant evaluator computes:
/// scalar operations over constants and argument-free layout calls such as
/// `align_of::<u32>()`. The callee and queried type are checked later.
pub(in crate::compiler::frontend) fn scalar_const_assertion(expr: &Expr) -> bool {
    assertion_operand(expr)
}

fn assertion_operand(expr: &Expr) -> bool {
    match expr {
        Expr::Apply { value, .. } => {
            matches!(value.as_ref(), Expr::Call { args, .. } if args.is_empty())
        }
        Expr::Unary { value, .. } => assertion_operand(value),
        Expr::Binary { left, right, .. } => assertion_operand(left) && assertion_operand(right),
        other => simple(other),
    }
}

/// An initializer the scalar graph CTFE path computes directly.
pub(in crate::compiler::frontend) fn scalar_simple(expr: &Expr) -> bool {
    simple(expr)
}

fn simple(expr: &Expr) -> bool {
    match expr {
        Expr::Integer(_) | Expr::Bool(_) | Expr::Var(_) | Expr::ConstItem(_) => true,
        Expr::Unary { value, .. } => simple(value),
        Expr::Binary { left, right, .. } => simple(left) && simple(right),
        _ => false,
    }
}

/// Elided references in constant types are `'static`.
fn static_elided(ty: Type) -> Type {
    match ty {
        Type::Ref {
            kind,
            region,
            referent,
        } => Type::Ref {
            kind,
            region: Some(region.unwrap_or(RegionName::Static)),
            referent: Box::new(static_elided(*referent)),
        },
        Type::DependentArray(inner, parameter) => {
            Type::DependentArray(Box::new(static_elided(*inner)), parameter)
        }
        Type::Array(inner, n) => Type::Array(Box::new(static_elided(*inner)), n),
        Type::Slice(inner) => Type::Slice(Box::new(static_elided(*inner))),
        Type::Tuple(fields) => Type::Tuple(fields.into_iter().map(static_elided).collect()),
        other => other,
    }
}

/// Unresolved hidden checker body; resolved with the ordinary functions.
pub(super) fn checked_body(declaration: &ConstDecl) -> Function {
    Function {
        const_params: Vec::new(),
        target_feature: None,
        embedded_source: None,
        direct_return_source: None,
        suspended_source: None,
        lexical_scope: None,
        module: declaration.module,
        constness: Constness::Const,
        unsafety: false,
        static_mut: false,
        visibility: declaration.visibility.clone(),
        bounds: Vec::new(),
        name: declaration.name.clone(),
        regions: RegionDecls::default(),
        type_params: Vec::new(),
        params: Vec::new(),
        param_mut: Vec::new(),
        result: static_elided(declaration.ty.clone()),
        impl_params: 0,
        body: Block {
            statements: Vec::new(),
            tail: Some(Box::new(declaration.initializer.clone())),
        },
    }
}

impl Resolver<'_> {
    /// Replace `expr` (a use of constant `target`) by its inlined initializer.
    pub(super) fn inline_constant(
        &mut self,
        expr: &mut Expr,
        target: &AstPath,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<bool, String> {
        let key = self.key(target)?;
        let mut found = None;
        for declaration in &self.program.constants {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? == key {
                found = Some(declaration);
                break;
            }
        }
        let Some(declaration) = found else {
            return Ok(false);
        };
        if !needs_inline(declaration) {
            return Ok(false);
        }
        let name = *declaration
            .name
            .segments()
            .last()
            .ok_or("empty constant path")?;
        let local = self.single_path(name)?;
        let module = declaration.module;
        if locals.len() > 256 {
            return Err("constant inlining depth exceeds limit 256 (cyclic constant?)".into());
        }
        let mut ty = declaration.ty.clone();
        let mut value = declaration.initializer.clone();
        let static_storage = declaration.storage == ConstantStorage::Static;
        let previous_params = std::mem::take(&mut self.type_params);
        locals.push(std::collections::HashSet::new());
        let result = self
            .ty(&mut ty, module)
            .and_then(|()| self.expr(&mut value, module, locals));
        locals.pop();
        self.type_params = previous_params;
        result?;
        if static_storage
            && (matches!(ty, Type::Ref { .. })
                || static_literal_storage(&value, &self.alias_budget, 0)?
                || static_initializer_has_reference(&value, &self.alias_budget, 0)?)
        {
            // Copy the value from its canonical allocation. Re-inlining a
            // reference-bearing initializer would allocate a different target
            // from the one observed by `&S` or by an autoref receiver.
            *expr = Expr::Deref {
                value: Box::new(Expr::StaticRef(target.clone())),
            };
            return Ok(true);
        }
        // The hidden const checker body is qualified with every other const
        // declaration before signature publication, including unused items.
        // Built after resolution: the binding is a fresh local whose name
        // must not re-resolve to the constant item it implements.
        *expr = Expr::Block(Block {
            statements: vec![Stmt::Let {
                name: BindingName::Named(name),
                mutable: false,
                ty,
                value,
            }],
            tail: Some(Box::new(Expr::Var(local))),
        });
        Ok(true)
    }
}

fn static_literal_storage(
    expr: &Expr,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match expr {
        Expr::Integer(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) => Ok(true),
        Expr::Unary {
            op: UnaryOp::Neg,
            value,
            ..
        } => Ok(matches!(value.as_ref(), Expr::Integer(_))),
        Expr::Tuple(values) | Expr::Array(values) | Expr::TupleStruct { fields: values, .. } => {
            for value in values {
                if !static_literal_storage(value, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::Struct { fields, .. } => {
            for (_, value) in fields {
                if !static_literal_storage(value, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::Repeat { value, .. } => static_literal_storage(value, budget, depth + 1),
        Expr::BorrowValue {
            kind: ir::BorrowKind::Shared,
            value,
        } if matches!(value.as_ref(), Expr::Array(_) | Expr::Repeat { .. }) => {
            static_literal_storage(value, budget, depth + 1)
        }
        _ => Ok(false),
    }
}

fn static_initializer_has_reference(
    expr: &Expr,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match expr {
        Expr::Str(_)
        | Expr::ByteStr(_)
        | Expr::Borrow { .. }
        | Expr::BorrowValue { .. }
        | Expr::StaticRef(_) => Ok(true),
        Expr::Array(values) | Expr::Tuple(values) | Expr::TupleStruct { fields: values, .. } => {
            for value in values {
                if static_initializer_has_reference(value, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Expr::Repeat { value, .. } => static_initializer_has_reference(value, budget, depth + 1),
        Expr::Struct { fields, .. } => {
            for (_, value) in fields {
                if static_initializer_has_reference(value, budget, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Expr::Block(block) => {
            for statement in &block.statements {
                if let Stmt::Let { value, .. } | Stmt::LetPattern { value, .. } | Stmt::Expr(value) =
                    statement
                    && static_initializer_has_reference(value, budget, depth + 1)?
                {
                    return Ok(true);
                }
            }
            match block.tail.as_deref() {
                Some(value) => static_initializer_has_reference(value, budget, depth + 1),
                None => Ok(false),
            }
        }
        _ => Ok(false),
    }
}

pub(in crate::compiler::frontend) fn static_elided_type(ty: Type) -> Type {
    static_elided(ty)
}

/// Lowered (non-scalar) associated constants are never evaluated, so any
/// initializer that could panic or overflow is rejected like free constants.
pub(in crate::compiler::frontend) fn validate_lowered_constant(
    initializer: &Expr,
    ty: &Type,
) -> Result<(), String> {
    match fallible(initializer, matches!(ty, Type::Float(_)), 0) {
        Fallible::No => Ok(()),
        Fallible::Panics => Err("E0080: evaluation of constant value failed: evaluation panicked".into()),
        Fallible::Maybe => Err("unsupported: constant initializers with operations that may panic or overflow during evaluation (arithmetic, indexing, assertions, conditional panics) are not evaluated by this compiler".into()),
    }
}
