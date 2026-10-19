//! Source-authenticated immutable initializer projections for scalar CTFE.
//!
//! This module never publishes parsed AST, name stores, planning contexts or
//! mutable budget handles. Its factory is reachable only through the original
//! source/registry identity check in `frontend::PlanCtx`.
use super::{const_eval as scalar, frontend, ir, source_replay};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) enum SyntaxKind {
    Integer {
        magnitude: u128,
        suffix: Option<ir::IntTy>,
    },
    Bool(bool),
    Reference(ir::DeclId),
    Unary {
        op: scalar::Unary,
        value: usize,
    },
    Binary {
        op: scalar::Binary,
        left: usize,
        right: usize,
    },
    Call {
        declaration: ir::DeclId,
        arguments: std::ops::Range<usize>,
    },
}
#[derive(Clone, Debug)]
pub(super) struct SyntaxExpr {
    pub(super) source: source_replay::SourceNode,
    pub(super) kind: SyntaxKind,
}
#[derive(Debug)]
pub(super) struct DeclarationFact {
    pub(super) declaration: ir::DeclId,
    pub(super) module: super::module_identity::ModuleId,
    pub(super) visibility: ir::Visibility,
    pub(super) ty: scalar::ScalarTy,
    pub(super) expressions: Arc<[SyntaxExpr]>,
    pub(super) arguments: Arc<[usize]>,
    pub(super) root: usize,
    authority: DeclarationAuthority,
    _storage: ir::RetainedStorage,
}
#[derive(Debug)]
struct DeclarationAuthority {
    declaration: ir::DeclId,
    module: super::module_identity::ModuleId,
    visibility: ir::Visibility,
    ty: scalar::ScalarTy,
    expressions: Arc<[SyntaxExpr]>,
    arguments: Arc<[usize]>,
    root: usize,
}
impl DeclarationFact {
    pub(super) fn validate_authority(&self) -> Result<(), String> {
        let original = &self.authority;
        if self.declaration != original.declaration
            || self.module != original.module
            || self.visibility != original.visibility
            || self.ty != original.ty
            || self.root != original.root
            || !Arc::ptr_eq(&self.expressions, &original.expressions)
            || !Arc::ptr_eq(&self.arguments, &original.arguments)
        {
            return Err("contract: frozen constant source publication changed".into());
        }
        Ok(())
    }
}

pub(super) fn scalar_type(ty: &frontend::Type) -> Result<scalar::ScalarTy, String> {
    match ty {
        frontend::Type::Bool => Ok(scalar::ScalarTy::Bool),
        frontend::Type::Int(ir::IntTy::Isize | ir::IntTy::Usize) => {
            Err("unsupported: target-sized source constants".into())
        }
        frontend::Type::Int(ty) => Ok(scalar::ScalarTy::Int(*ty)),
        _ => Err("unsupported: source constants require fixed-width integer or bool types".into()),
    }
}

fn shape(
    expr: &frontend::Expr,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(usize, usize), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let mut nodes = 1usize;
    let mut edges = 0usize;
    let mut child = |expr| -> Result<(), String> {
        let (n, e) = shape(expr, budget, depth + 1)?;
        nodes = nodes
            .checked_add(n)
            .ok_or("constant source node count overflow")?;
        edges = edges
            .checked_add(e)
            .ok_or("constant source edge count overflow")?;
        Ok(())
    };
    match expr {
        frontend::Expr::Integer(_)
        | frontend::Expr::Bool(_)
        | frontend::Expr::ConstItem(_)
        | frontend::Expr::Var(_) => {}
        frontend::Expr::Unary { value, .. } => child(value)?,
        frontend::Expr::Binary { left, right, .. } => {
            child(left)?;
            child(right)?;
        }
        frontend::Expr::Call { args, .. } => {
            for arg in args {
                child(arg)?;
            }
            edges = edges
                .checked_add(args.len())
                .ok_or("constant call edge count overflow")?;
        }
        _ => return Err("unsupported: expression in scalar constant initializer".into()),
    }
    Ok((nodes, edges))
}

pub(super) fn project(
    plan: &frontend::PlanCtx,
    declaration: ir::DeclId,
    source: &frontend::ConstDecl,
    budget: &ir::PlanBudget,
) -> Result<DeclarationFact, String> {
    if !std::ptr::eq(plan.constant_source(declaration)?, source) {
        return Err(
            "contract: constant initializer is not its authoritative source declaration".into(),
        );
    }
    let ty = scalar_type(&source.ty)?;
    let (node_count, edge_count) = shape(&source.initializer, budget, 0)?;
    let bytes = node_count
        .checked_mul(std::mem::size_of::<SyntaxExpr>())
        .and_then(|n| {
            edge_count
                .checked_mul(std::mem::size_of::<usize>())
                .and_then(|e| n.checked_add(e))
        })
        .ok_or("constant source allocation overflow")?;
    // Both incoming vectors and overlapping frozen Arc allocations are paid.
    let _scratch = budget.reserve_bytes(bytes)?;
    let storage = budget.reserve_bytes(
        bytes
            // Two frozen arena Arc headers, the empty root-path Arc header,
            // and the engine's published DeclarationFact Arc header.
            .checked_add(8 * std::mem::size_of::<usize>())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<DeclarationFact>()))
            .ok_or("constant source allocation overflow")?,
    )?;
    let mut expressions = Vec::new();
    expressions
        .try_reserve_exact(node_count)
        .map_err(|_| "constant source arena allocation")?;
    let mut arguments = Vec::new();
    arguments
        .try_reserve_exact(edge_count)
        .map_err(|_| "constant source edge allocation")?;
    let owner = source_replay::OriginalBodyId::new(budget).map_err(|e| e.to_string())?;
    let root_source = source_replay::SourceNode {
        owner,
        path: Arc::from([]),
    };
    let mut builder = ConstProjectionScratch {
        plan,
        budget,
        expressions: &mut expressions,
        arguments: &mut arguments,
    };
    let root = builder.expr(&source.initializer, root_source, 0)?;
    if expressions.len() != node_count || arguments.len() != edge_count {
        return Err("contract: constant projection shape changed".into());
    }
    let expressions: Arc<[SyntaxExpr]> = expressions.into();
    let arguments: Arc<[usize]> = arguments.into();
    let authority = DeclarationAuthority {
        declaration,
        module: source.module,
        visibility: source.visibility.clone(),
        ty,
        expressions: expressions.clone(),
        arguments: arguments.clone(),
        root,
    };
    Ok(DeclarationFact {
        declaration,
        module: source.module,
        visibility: source.visibility.clone(),
        ty,
        expressions,
        arguments,
        root,
        authority,
        _storage: storage.retain()?,
    })
}
struct ConstProjectionScratch<'a> {
    plan: &'a frontend::PlanCtx,
    budget: &'a ir::PlanBudget,
    expressions: &'a mut Vec<SyntaxExpr>,
    arguments: &'a mut Vec<usize>,
}
impl ConstProjectionScratch<'_> {
    fn expr(
        &mut self,
        expr: &frontend::Expr,
        source: source_replay::SourceNode,
        depth: usize,
    ) -> Result<usize, String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        let kind = match expr {
            frontend::Expr::Integer(value) => SyntaxKind::Integer {
                magnitude: value.magnitude,
                suffix: value.suffix,
            },
            frontend::Expr::Bool(value) => SyntaxKind::Bool(*value),
            frontend::Expr::ConstItem(path) => {
                SyntaxKind::Reference(self.plan.constant_path(path)?)
            }
            frontend::Expr::Var(path) => return Err(self.plan.constant_unresolved_value(path)?),
            frontend::Expr::Unary { op, value } => SyntaxKind::Unary {
                op: match op {
                    frontend::UnaryOp::Neg => scalar::Unary::Neg,
                    frontend::UnaryOp::Not => scalar::Unary::Not,
                },
                value: self.expr(
                    value,
                    source.child(1, 0, self.budget).map_err(|e| e.to_string())?,
                    depth + 1,
                )?,
            },
            frontend::Expr::Binary { op, left, right } => {
                let left = self.expr(
                    left,
                    source.child(2, 0, self.budget).map_err(|e| e.to_string())?,
                    depth + 1,
                )?;
                let right = self.expr(
                    right,
                    source.child(2, 1, self.budget).map_err(|e| e.to_string())?,
                    depth + 1,
                )?;
                SyntaxKind::Binary {
                    op: binary(*op),
                    left,
                    right,
                }
            }
            frontend::Expr::Call { name, args } => {
                // Child calls append their own argument rows first. Record this
                // call's roots after all children, without a second scratch Vec.
                let begin = self.expressions.len();
                for (index, arg) in args.iter().enumerate() {
                    self.expr(
                        arg,
                        source
                            .child(3, index, self.budget)
                            .map_err(|e| e.to_string())?,
                        depth + 1,
                    )?;
                }
                let start = self.arguments.len();
                self.budget.charge_work(self.expressions.len() - begin)?;
                for index in begin..self.expressions.len() {
                    let node = &self.expressions[index].source;
                    if node.path.len() == source.path.len() + 2
                        && node.path.starts_with(&source.path)
                    {
                        self.arguments.push(index);
                    }
                }
                if self.arguments.len() - start != args.len() {
                    return Err("contract: constant call child coverage".into());
                }
                SyntaxKind::Call {
                    declaration: self.plan.constant_callee(name)?,
                    arguments: start..self.arguments.len(),
                }
            }
            _ => return Err("unsupported: expression in scalar constant initializer".into()),
        };
        let index = self.expressions.len();
        self.expressions.push(SyntaxExpr { source, kind });
        Ok(index)
    }
}
fn binary(op: frontend::BinaryOp) -> scalar::Binary {
    use frontend::BinaryOp as F;
    use scalar::Binary as S;
    match op {
        F::Add => S::Add,
        F::Sub => S::Sub,
        F::Mul => S::Mul,
        F::Div => S::Div,
        F::Rem => S::Rem,
        F::Eq => S::Eq,
        F::Ne => S::Ne,
        F::Lt => S::Lt,
        F::Le => S::Le,
        F::Gt => S::Gt,
        F::Ge => S::Ge,
        F::And => S::And,
        F::Or => S::Or,
        F::BitAnd => S::BitAnd,
        F::BitOr => S::BitOr,
        F::BitXor => S::BitXor,
        F::Shl => S::Shl,
        F::Shr => S::Shr,
    }
}
