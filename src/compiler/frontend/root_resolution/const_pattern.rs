//! Constant patterns (`match x { LIMIT => .. }`): a path naming a scalar
//! `const` item is evaluated during resolution and replaced by the equivalent
//! literal pattern. Only integer, bool and char constants with pure
//! initializers (literals, unary/binary arithmetic, casts between integers and
//! other such constants) are admitted. Anything else keeps the explicit
//! unsupported diagnostic.
use super::*;

#[derive(Clone, Copy)]
enum Value {
    Int(i128, Option<ir::IntTy>),
    Bool(bool),
    Char(char),
}

fn fit(v: i128, ty: ir::IntTy) -> Result<i128, String> {
    let bits = ty.bits();
    let (lo, hi) = if ty.is_signed() {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (
            0,
            if bits >= 127 {
                i128::MAX
            } else {
                (1i128 << bits) - 1
            },
        )
    };
    if v < lo || v > hi {
        return Err("E0080: evaluation of constant value failed (overflow)".into());
    }
    Ok(v)
}

impl Resolver<'_> {
    /// `Some(pattern)` when `path` names a scalar constant.
    pub(super) fn const_pattern(
        &mut self,
        path: &AstPath,
        module: ModuleId,
    ) -> Result<Option<Pattern>, String> {
        let Some(item) = self.path(path, module, false, false)? else {
            return Ok(None);
        };
        if item.kind != ResolvedKind::Const {
            return Ok(None);
        }
        let source = item.source()?.clone();
        let value = self.const_value(&source, 0)?;
        Ok(Some(match value {
            Value::Bool(b) => Pattern::Bool(b),
            Value::Char(c) => Pattern::Char(c),
            Value::Int(v, ty) => Pattern::Int {
                literal: IntegerLiteral {
                    magnitude: v.unsigned_abs(),
                    suffix: ty,
                },
                negative: v < 0,
            },
        }))
    }

    fn const_value(&mut self, path: &AstPath, depth: usize) -> Result<Value, String> {
        if depth > 64 {
            return Err("constant pattern evaluation depth exceeds limit 64".into());
        }
        let key = self.key(path)?;
        let mut found = None;
        for declaration in &self.program.constants {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? == key {
                found = Some((
                    declaration.module,
                    declaration.ty.clone(),
                    declaration.initializer.clone(),
                ));
                break;
            }
        }
        let (module, ty, init) =
            found.ok_or("unsupported: constant patterns require a scalar const item")?;
        let ty_int = match ty {
            Type::Int(t) => Some(t),
            Type::Bool | Type::Char => None,
            _ => {
                return Err(
                    "unsupported: constant patterns require integer, bool or char constants".into(),
                );
            }
        };
        let v = self.const_expr(&init, module, ty_int, depth + 1)?;
        Ok(match (v, ty_int) {
            (Value::Int(n, _), Some(t)) => Value::Int(fit(n, t)?, Some(t)),
            (Value::Bool(b), None) if matches!(ty, Type::Bool) => Value::Bool(b),
            (Value::Char(c), None) if matches!(ty, Type::Char) => Value::Char(c),
            _ => return Err("E0308: mismatched types in constant initializer".into()),
        })
    }

    fn const_expr(
        &mut self,
        e: &Expr,
        module: ModuleId,
        hint: Option<ir::IntTy>,
        depth: usize,
    ) -> Result<Value, String> {
        if depth > 256 {
            return Err("constant pattern evaluation depth exceeds limit".into());
        }
        let unsupported = || -> String {
            "unsupported: constant pattern initializer is not a pure scalar expression".into()
        };
        Ok(match e {
            Expr::Integer(l) => {
                let ty = l.suffix.or(hint);
                Value::Int(i128::try_from(l.magnitude).map_err(|_| unsupported())?, ty)
            }
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Char(c) => Value::Char(*c),
            Expr::Block(b) if b.statements.is_empty() && b.tail.is_some() => {
                self.const_expr(b.tail.as_ref().unwrap(), module, hint, depth + 1)?
            }
            Expr::Var(p) | Expr::ConstItem(p) => {
                let item = self
                    .path(p, module, false, false)?
                    .ok_or_else(unsupported)?;
                if item.kind != ResolvedKind::Const {
                    return Err(unsupported());
                }
                let source = item.source()?.clone();
                self.const_value(&source, depth + 1)?
            }
            Expr::Cast { value, target } => {
                let Type::Int(t) = target else {
                    return Err(unsupported());
                };
                let v = self.const_expr(value, module, None, depth + 1)?;
                let n = match v {
                    Value::Int(n, _) => n,
                    Value::Bool(b) => i128::from(b),
                    Value::Char(c) => i128::from(u32::from(c)),
                };
                // Truncating integer cast.
                let bits = t.bits();
                let mask: u128 = if bits >= 128 {
                    u128::MAX
                } else {
                    (1u128 << bits) - 1
                };
                let raw = (n as u128) & mask;
                let n = if t.is_signed() && bits < 128 && raw >> (bits - 1) & 1 == 1 {
                    (raw as i128) - (1i128 << bits)
                } else {
                    raw as i128
                };
                Value::Int(n, Some(*t))
            }
            Expr::Unary { op, value } => {
                match (op, self.const_expr(value, module, hint, depth + 1)?) {
                    (UnaryOp::Neg, Value::Int(n, t)) => Value::Int(-n, t),
                    (UnaryOp::Not, Value::Bool(b)) => Value::Bool(!b),
                    (UnaryOp::Not, Value::Int(n, Some(t))) => {
                        if t.is_signed() {
                            Value::Int(!n, Some(t))
                        } else {
                            Value::Int(fit(n, t)? ^ ((1i128 << t.bits().min(126)) - 1), Some(t))
                        }
                    }
                    _ => return Err(unsupported()),
                }
            }
            Expr::Binary { op, left, right } => {
                let l = self.const_expr(left, module, hint, depth + 1)?;
                let r_hint = match l {
                    Value::Int(_, t) => t.or(hint),
                    _ => hint,
                };
                let r = self.const_expr(right, module, r_hint, depth + 1)?;
                match (l, r) {
                    (Value::Int(a, ta), Value::Int(b, tb)) => {
                        let t = ta.or(tb);
                        let check = |v: Option<i128>| -> Result<i128, String> {
                            let v = v.ok_or("E0080: evaluation of constant value failed")?;
                            match t {
                                Some(t) => fit(v, t),
                                None => Ok(v),
                            }
                        };
                        match op {
                            BinaryOp::Add => Value::Int(check(a.checked_add(b))?, t),
                            BinaryOp::Sub => Value::Int(check(a.checked_sub(b))?, t),
                            BinaryOp::Mul => Value::Int(check(a.checked_mul(b))?, t),
                            BinaryOp::Div => Value::Int(check(a.checked_div(b))?, t),
                            BinaryOp::Rem => Value::Int(check(a.checked_rem(b))?, t),
                            BinaryOp::BitAnd => Value::Int(a & b, t),
                            BinaryOp::BitOr => Value::Int(a | b, t),
                            BinaryOp::BitXor => Value::Int(a ^ b, t),
                            BinaryOp::Shl => Value::Int(
                                check(u32::try_from(b).ok().and_then(|s| a.checked_shl(s)))?,
                                t,
                            ),
                            BinaryOp::Shr => Value::Int(a >> b.clamp(0, 127), t),
                            BinaryOp::Eq => Value::Bool(a == b),
                            BinaryOp::Ne => Value::Bool(a != b),
                            BinaryOp::Lt => Value::Bool(a < b),
                            BinaryOp::Le => Value::Bool(a <= b),
                            BinaryOp::Gt => Value::Bool(a > b),
                            BinaryOp::Ge => Value::Bool(a >= b),
                            _ => return Err(unsupported()),
                        }
                    }
                    (Value::Bool(a), Value::Bool(b)) => Value::Bool(match op {
                        BinaryOp::And | BinaryOp::BitAnd => a && b,
                        BinaryOp::Or | BinaryOp::BitOr => a || b,
                        BinaryOp::BitXor | BinaryOp::Ne => a != b,
                        BinaryOp::Eq => a == b,
                        _ => return Err(unsupported()),
                    }),
                    _ => return Err(unsupported()),
                }
            }
            _ => return Err(unsupported()),
        })
    }
}
