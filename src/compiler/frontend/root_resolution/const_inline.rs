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
    !(scalar && simple(&declaration.initializer))
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
        Type::Array(inner, n) => Type::Array(Box::new(static_elided(*inner)), n),
        Type::Slice(inner) => Type::Slice(Box::new(static_elided(*inner))),
        Type::Tuple(fields) => Type::Tuple(fields.into_iter().map(static_elided).collect()),
        other => other,
    }
}

/// Unresolved hidden checker body; resolved with the ordinary functions.
pub(super) fn checked_body(declaration: &ConstDecl) -> Function {
    Function {
        embedded_source: None,
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
        let previous_params = std::mem::take(&mut self.type_params);
        locals.push(std::collections::HashSet::new());
        let result = self
            .ty(&mut ty, module)
            .and_then(|()| self.expr(&mut value, module, locals));
        locals.pop();
        self.type_params = previous_params;
        result?;
        self.const_calls(&value, 0)?;
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

    /// E0015: constant initializers may only call `const fn`s.
    fn const_calls(&self, expr: &Expr, depth: usize) -> Result<(), String> {
        if depth > 256 {
            return Err("constant initializer nesting exceeds limit 256".into());
        }
        let mut children: Vec<&Expr> = Vec::new();
        match expr {
            Expr::Call { name, args } => {
                let key = name_scope::path_key(self.names(), name, Namespace::Value)?;
                for function in &self.program.functions {
                    if name_scope::path_key(self.names(), &function.name, Namespace::Value)? == key
                        && function.constness == Constness::Runtime
                    {
                        return Err(
                            "E0015: cannot call non-const function in constant initializer".into(),
                        );
                    }
                }
                children.extend(args);
            }
            Expr::MethodCall { .. } | Expr::AssociatedCall { .. } | Expr::TraitCall { .. } => {
                return Err(
                    "unsupported: method calls in constant initializers are not yet evaluated"
                        .into(),
                );
            }
            Expr::Unary { value, .. }
            | Expr::Cast { value, .. }
            | Expr::Deref { value }
            | Expr::BorrowValue { value, .. }
            | Expr::Repeat { value, .. } => children.push(value),
            Expr::Binary { left, right, .. } => {
                children.push(left);
                children.push(right);
            }
            Expr::Array(values)
            | Expr::Tuple(values)
            | Expr::TupleStruct { fields: values, .. } => children.extend(values),
            Expr::Struct { fields, .. } => children.extend(fields.iter().map(|(_, v)| v)),
            Expr::Field { base, .. } => children.push(base),
            Expr::Block(block) => {
                if let Some(tail) = &block.tail {
                    children.push(tail);
                }
            }
            _ => {}
        }
        for child in children {
            self.const_calls(child, depth + 1)?;
        }
        Ok(())
    }
}
