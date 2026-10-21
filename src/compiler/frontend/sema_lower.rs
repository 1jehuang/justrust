//! Lowering of resolved source bodies into `sema::hir` (SEMANTIC-CORE.md).
//!
//! Runs inside one body-check computation. Locals are resolved by lexical
//! scope (name keys), items through the registry's per-path maps. Written
//! types become `sema` types over the callable's ordered generic list.
//! Anything not yet modeled returns an explicit `Unsupported`, never an
//! approximation.

use super::*;
use crate::compiler::sema::adapter::{TyMap, def, mutbl};
use crate::compiler::sema::hir as h;
use crate::compiler::sema::ty as st;
use crate::compiler::semantic_name::{LookupClass, NameKey};

pub struct SemaLowered {
    pub body: h::HirBody,
}

#[derive(Debug)]
pub struct Unsupported(pub String);

type R<T> = Result<T, Unsupported>;

fn un<T>(m: impl Into<String>) -> R<T> {
    Err(Unsupported(m.into()))
}

pub struct Lower<'a> {
    plan: &'a PlanCtx,
    /// Type parameter names in env order, for `Type::Param`.
    tparams: Vec<Option<NameKey>>,
    self_ty: Option<st::Ty>,
    exprs: Vec<h::HirExpr>,
    locals: u32,
    scopes: Vec<Vec<(NameKey, h::LocalId)>>,
    receiver: Option<h::LocalId>,
    labels: Vec<(Option<NameKey>, u32)>,
    next_label: u32,
    /// Inside a later or-pattern alternative: bindings reuse earlier locals.
    rebinding: bool,
}

impl PlanCtx {
    /// Lower one source callable's body for the semantic core.
    pub fn sema_lower_body(&self, decl: ir::DeclId) -> Result<SemaLowered, Unsupported> {
        let info = self.declaration(decl).map_err(Unsupported)?;
        let schema = self
            .sema_schema(decl)
            .ok_or_else(|| Unsupported("no validated schema".into()))?;
        let function = self
            .registry
            .source_callable(decl)
            .ok_or_else(|| Unsupported("no source body".into()))?;
        let names = &self.program.names;
        let mut tparams = Vec::new();
        // Parent (impl/trait) parameters first, then the function's own.
        if let Some(owner) = &info.path.associated {
            let owner_info = self.declaration(owner.impl_decl).map_err(Unsupported)?;
            if owner.trait_decl.is_some() && info.kind == ir::DeclKind::TraitItem {
                // Trait default bodies: Self is parameter 0.
                tparams.push(None);
            }
            for p in owner_info.type_params.iter() {
                tparams.push(names.key(*p, LookupClass::TypeParameter).ok());
            }
        }
        for p in function.type_params.iter() {
            tparams.push(names.key(*p, LookupClass::TypeParameter).ok());
        }
        let env = &schema.env;
        let map = TyMap { env, extra: &[] };
        let self_ty = info.path.associated.as_ref().map(|a| {
            let mut u = None;
            map.ty(&a.self_pattern, &mut u)
        });
        let mut l = Lower {
            plan: self,
            tparams,
            self_ty,
            exprs: Vec::new(),
            locals: 0,
            scopes: vec![Vec::new()],
            receiver: None,
            labels: Vec::new(),
            next_label: 0,
            rebinding: false,
        };
        let mut u = None;
        let sig_params: Vec<st::Ty> = schema
            .signature
            .params
            .iter()
            .map(|t| map.ty(t, &mut u))
            .collect();
        let ret = map.ty(&schema.signature.result, &mut u);
        if let Some(m) = u {
            return un(m);
        }
        let mut params = Vec::new();
        let has_receiver = !matches!(info.receiver, ReceiverKind::None);
        let mut sig_iter = sig_params.into_iter();
        if has_receiver {
            let t = sig_iter
                .next()
                .ok_or_else(|| Unsupported("receiver without parameter".into()))?;
            let id = l.fresh_local();
            l.receiver = Some(id);
            params.push((
                h::HirPat::Binding {
                    local: id,
                    by_ref: None,
                },
                t,
            ));
        }
        for ((name, _), t) in function.params.iter().zip(sig_iter) {
            let pat = match name {
                BindingName::Named(n) => {
                    let id = l.bind(*n)?;
                    h::HirPat::Binding {
                        local: id,
                        by_ref: None,
                    }
                }
                BindingName::Wildcard => h::HirPat::Wild,
            };
            params.push((pat, t));
        }
        let root = l.block(&function.body)?;
        Ok(SemaLowered {
            body: h::HirBody {
                exprs: l.exprs,
                locals: l.locals,
                params,
                ret,
                root,
            },
        })
    }
}

impl<'a> Lower<'a> {
    fn names(&self) -> &NameStore {
        &self.plan.program.names
    }
    fn key(&self, n: AstIdent, c: LookupClass) -> R<NameKey> {
        self.names()
            .key(n, c)
            .map_err(|e| Unsupported(format!("name: {e:?}")))
    }
    fn fresh_local(&mut self) -> h::LocalId {
        self.locals += 1;
        self.locals - 1
    }
    fn bind(&mut self, n: AstIdent) -> R<h::LocalId> {
        if self.rebinding {
            return self
                .lookup(n)?
                .ok_or_else(|| Unsupported("or-pattern alternatives bind different names".into()));
        }
        let k = self.key(n, LookupClass::Local)?;
        let id = self.fresh_local();
        self.scopes.last_mut().unwrap().push((k, id));
        Ok(id)
    }
    fn lookup(&self, n: AstIdent) -> R<Option<h::LocalId>> {
        let k = self.key(n, LookupClass::Local)?;
        for s in self.scopes.iter().rev() {
            if let Some((_, id)) = s.iter().rev().find(|(x, _)| *x == k) {
                return Ok(Some(*id));
            }
        }
        Ok(None)
    }
    fn push(&mut self, e: h::HirExpr) -> h::ExprId {
        self.exprs.push(e);
        (self.exprs.len() - 1) as h::ExprId
    }
    fn scoped<T>(&mut self, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        self.scopes.push(Vec::new());
        let r = f(self);
        self.scopes.pop();
        r
    }

    // ---------------------------------------------------------------- types

    fn ty(&mut self, t: &Type) -> R<st::Ty> {
        Ok(match t {
            Type::Infer => st::hole(),
            Type::Bool => st::bool_ty(),
            Type::Char => st::mk(st::TyKind::Char),
            Type::Str => st::mk(st::TyKind::Str),
            Type::Unit => st::unit(),
            Type::Never => st::mk(st::TyKind::Never),
            Type::Int(i) => st::int(*i),
            Type::Float(f) => st::mk(st::TyKind::Float(*f)),
            Type::Tuple(v) => st::mk(st::TyKind::Tuple(
                v.iter().map(|x| self.ty(x)).collect::<R<_>>()?,
            )),
            Type::Ref { kind, referent, .. } => st::reference(mutbl(*kind), self.ty(referent)?),
            Type::RawPtr { kind, pointee } => {
                st::mk(st::TyKind::RawPtr(mutbl(*kind), self.ty(pointee)?))
            }
            Type::Array(e, n) => st::mk(st::TyKind::Array(self.ty(e)?, *n as u64)),
            Type::Slice(e) => st::mk(st::TyKind::Slice(self.ty(e)?)),
            Type::SelfType => self
                .self_ty
                .clone()
                .ok_or_else(|| Unsupported("Self outside impl".into()))?,
            Type::Param(p) => {
                let k = Some(self.key(*p, LookupClass::TypeParameter)?);
                match self.tparams.iter().position(|x| *x == k) {
                    Some(i) => st::param(i as u32),
                    None => return un("unknown type parameter"),
                }
            }
            Type::Named(path) => self.nominal(path, &[])?,
            Type::Apply(gp) => {
                let path = gp.path(self.names()).map_err(Unsupported)?;
                let args: Vec<Type> = gp
                    .segments
                    .iter()
                    .flat_map(|s| s.args.iter().cloned())
                    .collect();
                self.nominal(&path, &args)?
            }
            Type::Projection(_) => return un("written projection type"),
            Type::Opaque(_) => return un("impl Trait type"),
            Type::Dyn(_) => return un("dyn Trait type"),
        })
    }

    fn nominal(&mut self, path: &AstPath, args: &[Type]) -> R<st::Ty> {
        if path.anchor() == PathAnchor::SelfType && path.is_empty() {
            return self
                .self_ty
                .clone()
                .ok_or_else(|| Unsupported("Self outside impl".into()));
        }
        // A single-segment path may name a type parameter.
        if let Some(n) = path.single()
            && let Ok(k) = self.key(n, LookupClass::TypeParameter)
            && let Some(i) = self.tparams.iter().position(|x| *x == Some(k))
        {
            return Ok(st::param(i as u32));
        }
        let key = path.key(Namespace::Type);
        let decl = *self
            .plan
            .registry
            .nominals
            .get(&key)
            .ok_or_else(|| Unsupported("unresolved nominal type".into()))?;
        let arity = self
            .plan
            .declaration(decl)
            .map_err(Unsupported)?
            .param_env
            .arity();
        let mut a: Vec<st::Ty> = args.iter().map(|t| self.ty(t)).collect::<R<_>>()?;
        if a.is_empty() {
            a = (0..arity).map(|_| st::hole()).collect();
        }
        if a.len() != arity {
            return un("wrong number of type arguments");
        }
        Ok(st::adt(def(decl), a))
    }

    // ---------------------------------------------------------------- blocks

    fn block(&mut self, b: &Block) -> R<h::ExprId> {
        self.scoped(|l| {
            let mut stmts = Vec::new();
            for s in &b.statements {
                l.stmt(s, &mut stmts)?;
            }
            let tail = match &b.tail {
                Some(t) => Some(l.expr(t)?),
                None => None,
            };
            Ok(l.push(h::HirExpr::Block { stmts, tail }))
        })
    }

    fn local_place(&mut self, n: &LocalName) -> R<h::ExprId> {
        let id = match n {
            LocalName::Named(n) => self
                .lookup(*n)?
                .ok_or_else(|| Unsupported("unresolved local".into()))?,
            LocalName::Receiver | LocalName::ReceiverOccurrence(_) => self
                .receiver
                .ok_or_else(|| Unsupported("self outside method".into()))?,
        };
        Ok(self.push(h::HirExpr::Local(id)))
    }

    fn place(&mut self, p: &Place) -> R<h::ExprId> {
        let mut e = self.local_place(&p.local)?;
        for f in &p.fields {
            e = self.project(e, f)?;
        }
        Ok(e)
    }

    fn project(&mut self, base: h::ExprId, f: &FieldSelector) -> R<h::ExprId> {
        Ok(match f {
            FieldSelector::Deref => self.push(h::HirExpr::Unary(h::UnOp::Deref, base)),
            FieldSelector::Tuple(i) => self.push(h::HirExpr::TupleField {
                base,
                index: *i as u32,
            }),
            FieldSelector::ConstantIndex(i) => {
                let idx = self.push(h::HirExpr::Lit(h::HirLit::Int(
                    *i as u128,
                    Some(ir::IntTy::Usize),
                )));
                self.push(h::HirExpr::Index { base, index: idx })
            }
            FieldSelector::Named(n) => {
                let name = self
                    .names()
                    .spelling(*n)
                    .map_err(|e| Unsupported(format!("{e:?}")))?
                    .to_owned();
                self.push(h::HirExpr::Field { base, name })
            }
        })
    }

    fn stmt(&mut self, s: &Stmt, out: &mut Vec<h::HirStmt>) -> R<()> {
        match s {
            Stmt::Declare { name, ty, .. } => {
                let t = self.ty(ty)?;
                let pat = self.binding_pat(name)?;
                out.push(h::HirStmt::Let {
                    pat,
                    ty: t,
                    init: None,
                    else_block: None,
                });
            }
            Stmt::Let {
                name, ty, value, ..
            } => {
                let t = self.ty(ty)?;
                let init = self.expr(value)?;
                let pat = self.binding_pat(name)?;
                out.push(h::HirStmt::Let {
                    pat,
                    ty: t,
                    init: Some(init),
                    else_block: None,
                });
            }
            Stmt::LetPattern { pattern, ty, value } => {
                let t = self.ty(ty)?;
                let init = self.expr(value)?;
                let pat = self.pat(pattern)?;
                out.push(h::HirStmt::Let {
                    pat,
                    ty: t,
                    init: Some(init),
                    else_block: None,
                });
            }
            Stmt::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                let t = self.ty(ty)?;
                let init = self.expr(value)?;
                let eb = self.block(else_block)?;
                let pat = self.pat(pattern)?;
                out.push(h::HirStmt::Let {
                    pat,
                    ty: t,
                    init: Some(init),
                    else_block: Some(eb),
                });
            }
            Stmt::Assign { name, value } => {
                let v = self.expr(value)?;
                let l = self.local_place(name)?;
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::Assign(l, v))));
            }
            Stmt::AssignPlace { place, value, .. } => {
                let v = self.expr(value)?;
                let p = self.place(place)?;
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::Assign(p, v))));
            }
            Stmt::AssignOp {
                place, op, value, ..
            } => {
                let v = self.expr(value)?;
                let p = self.place(place)?;
                let op = binop(*op)?;
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::AssignOp(op, p, v))));
            }
            Stmt::AssignIndex {
                array,
                index,
                value,
                ..
            } => {
                let v = self.expr(value)?;
                let base = self.place(array)?;
                let i = self.expr(index)?;
                let p = self.push(h::HirExpr::Index { base, index: i });
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::Assign(p, v))));
            }
            Stmt::AssignIndexOp {
                array,
                index,
                op,
                value,
                ..
            } => {
                let v = self.expr(value)?;
                let base = self.place(array)?;
                let i = self.expr(index)?;
                let p = self.push(h::HirExpr::Index { base, index: i });
                let op = binop(*op)?;
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::AssignOp(op, p, v))));
            }
            Stmt::AssignNestedIndex { target, op, value } => {
                let v = self.expr(value)?;
                let p = self.expr(target)?;
                let e = match op {
                    Some(op) => h::HirExpr::AssignOp(binop(*op)?, p, v),
                    None => h::HirExpr::Assign(p, v),
                };
                out.push(h::HirStmt::Expr(self.push(e)));
            }
            Stmt::AssignDeref {
                carrier,
                fields,
                index,
                op,
                value,
                ..
            } => {
                let v = self.expr(value)?;
                let mut p = self.expr(carrier)?;
                for f in fields {
                    p = self.project(p, f)?;
                }
                if let Some(i) = index {
                    let i = self.expr(i)?;
                    p = self.push(h::HirExpr::Index { base: p, index: i });
                }
                let e = match op {
                    Some(op) => h::HirExpr::AssignOp(binop(*op)?, p, v),
                    None => h::HirExpr::Assign(p, v),
                };
                out.push(h::HirStmt::Expr(self.push(e)));
            }
            Stmt::Expr(e) | Stmt::ImplicitExpr(e) => {
                let x = self.expr(e)?;
                out.push(h::HirStmt::Expr(x));
            }
            Stmt::Return(v) => {
                let v = match v {
                    Some(v) => Some(self.expr(v)?),
                    None => None,
                };
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::Return(v))));
            }
            Stmt::While {
                label,
                condition,
                body,
            } => {
                let lab = self.enter_label(label.as_ref())?;
                let cond = self.expr(condition)?;
                let b = self.block(body)?;
                self.labels.pop();
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::While {
                    cond,
                    body: b,
                    label: lab,
                })));
            }
            Stmt::WhileLet {
                label,
                pattern,
                scrutinee,
                body,
                ..
            } => {
                // while let P = E { B }  ==>  loop { match E { P => B, _ => break } }
                let lab = self.enter_label(label.as_ref())?;
                let s = self.expr(scrutinee)?;
                let (pat, b) = self.scoped(|l| Ok((l.pat(pattern)?, l.block(body)?)))?;
                let brk = self.push(h::HirExpr::Break {
                    label: lab,
                    value: None,
                });
                let m = self.push(h::HirExpr::Match {
                    scrutinee: s,
                    arms: vec![
                        h::HirArm {
                            pat,
                            guard: None,
                            body: b,
                        },
                        h::HirArm {
                            pat: h::HirPat::Wild,
                            guard: None,
                            body: brk,
                        },
                    ],
                });
                self.labels.pop();
                let lb = self.push(h::HirExpr::Block {
                    stmts: vec![h::HirStmt::Expr(m)],
                    tail: None,
                });
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::Loop {
                    body: lb,
                    label: lab,
                })));
            }
            Stmt::For {
                label,
                pattern,
                source,
                body,
                ..
            } => {
                let mut it = match &source.iterable {
                    ForIterable::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        let s = self.expr(start)?;
                        let e = self.expr(end)?;
                        self.push(h::HirExpr::Range {
                            start: Some(s),
                            end: Some(e),
                            inclusive: *inclusive,
                        })
                    }
                    ForIterable::Value(v) => self.expr(v)?,
                };
                for a in &source.adapters {
                    let name = match a {
                        ForAdapter::Iter => "iter",
                        ForAdapter::IntoIter => "into_iter",
                        ForAdapter::Rev => "rev",
                        ForAdapter::Enumerate => "enumerate",
                    };
                    it = self.push(h::HirExpr::MethodCall {
                        receiver: it,
                        name: name.into(),
                        turbofish: vec![],
                        args: vec![],
                    });
                }
                let lab = self.enter_label(label.as_ref())?;
                let (pat, b) = self.scoped(|l| Ok((l.pat(pattern)?, l.block(body)?)))?;
                self.labels.pop();
                out.push(h::HirStmt::Expr(self.push(h::HirExpr::For {
                    pat,
                    iter: it,
                    body: b,
                    label: lab,
                })));
            }
        }
        Ok(())
    }

    fn enter_label(&mut self, label: Option<&AstIdent>) -> R<u32> {
        let k = match label {
            Some(n) => Some(self.key(*n, LookupClass::Label)?),
            None => None,
        };
        let id = self.next_label;
        self.next_label += 1;
        self.labels.push((k, id));
        Ok(id)
    }
    fn find_label(&self, label: Option<&AstIdent>) -> R<u32> {
        match label {
            None => self
                .labels
                .last()
                .map(|(_, i)| *i)
                .ok_or_else(|| Unsupported("break outside loop".into())),
            Some(n) => {
                let k = self.key(*n, LookupClass::Label)?;
                self.labels
                    .iter()
                    .rev()
                    .find(|(x, _)| *x == Some(k))
                    .map(|(_, i)| *i)
                    .ok_or_else(|| Unsupported("unknown label".into()))
            }
        }
    }

    fn binding_pat(&mut self, n: &BindingName) -> R<h::HirPat> {
        Ok(match n {
            BindingName::Named(n) => h::HirPat::Binding {
                local: self.bind(*n)?,
                by_ref: None,
            },
            BindingName::Wildcard => h::HirPat::Wild,
        })
    }

    // ---------------------------------------------------------------- patterns

    fn pat_rebind(&mut self, p: &Pattern) -> R<h::HirPat> {
        let saved = std::mem::replace(&mut self.rebinding, true);
        let r = self.pat(p);
        self.rebinding = saved;
        r
    }

    fn pat(&mut self, p: &Pattern) -> R<h::HirPat> {
        Ok(match p {
            Pattern::Wildcard => h::HirPat::Wild,
            Pattern::Binding { name, by_ref, .. } => h::HirPat::Binding {
                local: self.bind(*name)?,
                by_ref: by_ref.map(mutbl),
            },
            Pattern::Tuple(v) => h::HirPat::Tuple(v.iter().map(|x| self.pat(x)).collect::<R<_>>()?),
            Pattern::Ref { kind, pattern } => {
                h::HirPat::Ref(mutbl(*kind), Box::new(self.pat(pattern)?))
            }
            Pattern::Bool(_) => h::HirPat::Lit(h::HirLit::Bool),
            Pattern::Char(_) => h::HirPat::Lit(h::HirLit::Char),
            Pattern::Int { literal, .. } => {
                h::HirPat::Lit(h::HirLit::Int(literal.magnitude, literal.suffix))
            }
            Pattern::Variant { name, fields, .. } => {
                let (adt, variant) = self.variant(name)?;
                let mut out = Vec::new();
                for (sel, fp) in fields {
                    let idx = self.field_index(adt, variant, sel)?;
                    out.push((idx, self.pat(fp)?));
                }
                h::HirPat::Adt {
                    adt,
                    variant,
                    fields: out,
                }
            }
            Pattern::Path(path) => {
                let (adt, variant) = self.variant(path)?;
                h::HirPat::Adt {
                    adt,
                    variant,
                    fields: vec![],
                }
            }
            Pattern::Struct { name, fields, .. } => {
                let key = name.key(Namespace::Type);
                let (adt, variant) = match self.plan.registry.nominals.get(&key) {
                    Some(d) => (def(*d), 0),
                    None => self.variant(name)?,
                };
                let mut out = Vec::new();
                for (n, fp) in fields {
                    let idx = self.field_index(adt, variant, &FieldSelector::Named(*n))?;
                    out.push((idx, self.pat(fp)?));
                }
                h::HirPat::Adt {
                    adt,
                    variant,
                    fields: out,
                }
            }
            Pattern::Alternatives(alts) => {
                // Alternatives bind the same names. Bind them once from the
                // first alternative, then resolve later alternatives to the
                // same locals by looking names up rather than rebinding.
                let mut out = Vec::new();
                for (i, a) in alts.iter().enumerate() {
                    if i == 0 {
                        out.push(self.pat(a)?);
                    } else {
                        out.push(self.pat_rebind(a)?);
                    }
                }
                h::HirPat::Or(out)
            }
            Pattern::At { .. } => return un("@ pattern"),
            Pattern::Range { lo, hi, .. } => {
                let b = lo.as_ref().or(hi.as_ref());
                h::HirPat::Range(match b {
                    Some(RangeBound::Char(_)) => h::HirLit::Char,
                    Some(RangeBound::Int { literal, .. }) => {
                        h::HirLit::Int(literal.magnitude, literal.suffix)
                    }
                    Some(RangeBound::IntLimit { ty, .. }) => h::HirLit::Int(0, Some(*ty)),
                    None => return un("empty range pattern"),
                })
            }
            Pattern::Array(_) | Pattern::ArrayRest(_) => return un("array pattern"),
            Pattern::TupleRest(_) | Pattern::VariantRest { .. } => return un("rest pattern"),
            Pattern::Apply { .. } => return un("generic pattern path"),
        })
    }

    /// Resolve `Enum::Variant` to (adt, variant index).
    fn variant(&self, path: &AstPath) -> R<(st::DefId, u32)> {
        let names = self.names();
        let member = *path
            .segments()
            .last()
            .ok_or_else(|| Unsupported("empty variant path".into()))?;
        let owner = path
            .semantic_parent(names)
            .map_err(|e| Unsupported(format!("{e:?}")))?;
        let decl = *self
            .plan
            .registry
            .nominals
            .get(&owner.key(Namespace::Type))
            .ok_or_else(|| Unsupported("unresolved enum".into()))?;
        let mk = self.key(member, LookupClass::Member)?;
        let info = self.plan.declaration(decl).map_err(Unsupported)?;
        let e = self
            .plan
            .program
            .enums
            .get(info.source_index)
            .ok_or_else(|| Unsupported("enum source".into()))?;
        for (i, v) in e.variants.iter().enumerate() {
            if self.key(v.name, LookupClass::Member)? == mk {
                return Ok((def(decl), i as u32));
            }
        }
        un("unknown variant")
    }

    fn field_index(&self, adt: st::DefId, variant: u32, sel: &FieldSelector) -> R<u32> {
        match sel {
            FieldSelector::Tuple(i) => Ok(*i as u32),
            FieldSelector::Named(n) => {
                let k = self.key(*n, LookupClass::Member)?;
                let fields = self.source_fields(adt, variant)?;
                for (i, f) in fields.iter().enumerate() {
                    if let FieldName::Named(fname) = f.name
                        && self.key(fname, LookupClass::Member)? == k
                    {
                        return Ok(i as u32);
                    }
                }
                un("unknown field")
            }
            _ => un("field selector in pattern"),
        }
    }

    fn source_fields(&self, adt: st::DefId, variant: u32) -> R<&'a [FieldDecl]> {
        let plan = self.plan;
        let info = plan
            .registry
            .declarations
            .get(adt as usize)
            .ok_or_else(|| Unsupported("unknown nominal".into()))?;
        Ok(match info.kind {
            ir::DeclKind::Struct => &plan.program.structs[info.source_index].fields,
            ir::DeclKind::Enum => {
                &plan.program.enums[info.source_index].variants[variant as usize].fields
            }
            _ => return un("not a nominal"),
        })
    }

    // ---------------------------------------------------------------- exprs

    fn exprs(&mut self, v: &[Expr]) -> R<Vec<h::ExprId>> {
        v.iter().map(|e| self.expr(e)).collect()
    }

    fn expr(&mut self, e: &Expr) -> R<h::ExprId> {
        Ok(match e {
            Expr::Integer(l) => self.push(h::HirExpr::Lit(h::HirLit::Int(l.magnitude, l.suffix))),
            Expr::Float(f) => self.push(h::HirExpr::Lit(h::HirLit::Float(f.suffix))),
            Expr::Bool(_) => self.push(h::HirExpr::Lit(h::HirLit::Bool)),
            Expr::Char(_) => self.push(h::HirExpr::Lit(h::HirLit::Char)),
            Expr::Str(_) => self.push(h::HirExpr::Lit(h::HirLit::Str)),
            Expr::Var(path) => self.var(path)?,
            Expr::Tuple(v) => {
                let v = self.exprs(v)?;
                self.push(h::HirExpr::Tuple(v))
            }
            Expr::Array(v) => {
                let v = self.exprs(v)?;
                self.push(h::HirExpr::Array(v))
            }
            Expr::Repeat { value, len } => {
                let v = self.expr(value)?;
                self.push(h::HirExpr::Repeat {
                    value: v,
                    count: *len as u64,
                })
            }
            Expr::Unary { op, value, .. } => {
                let v = self.expr(value)?;
                let op = match op {
                    UnaryOp::Neg => h::UnOp::Neg,
                    UnaryOp::Not => h::UnOp::Not,
                };
                self.push(h::HirExpr::Unary(op, v))
            }
            Expr::Deref { value } => {
                let v = self.expr(value)?;
                self.push(h::HirExpr::Unary(h::UnOp::Deref, v))
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let l = self.expr(left)?;
                let r = self.expr(right)?;
                let op = binop(*op)?;
                self.push(h::HirExpr::Binary(op, l, r))
            }
            Expr::Cast { value, target } => {
                let v = self.expr(value)?;
                let t = self.ty(target)?;
                self.push(h::HirExpr::Cast(v, t))
            }
            Expr::Borrow { kind, place } => {
                let p = self.place(place)?;
                self.push(h::HirExpr::Ref(mutbl(*kind), p))
            }
            Expr::BorrowValue { kind, value } => {
                let v = self.expr(value)?;
                self.push(h::HirExpr::Ref(mutbl(*kind), v))
            }
            Expr::Field { base, field } => {
                let b = self.expr(base)?;
                self.project(b, field)?
            }
            Expr::Index { base, index, .. } => {
                let b = self.expr(base)?;
                self.project(b, &FieldSelector::ConstantIndex(*index))?
            }
            Expr::DynamicIndex { base, index, .. } => {
                let b = self.expr(base)?;
                let i = self.expr(index)?;
                self.push(h::HirExpr::Index { base: b, index: i })
            }
            Expr::RangeIndex {
                base,
                start,
                end,
                inclusive,
            } => {
                let b = self.expr(base)?;
                let s = match start {
                    Some(s) => Some(self.expr(s)?),
                    None => None,
                };
                let en = match end {
                    Some(x) => Some(self.expr(x)?),
                    None => None,
                };
                let r = self.push(h::HirExpr::Range {
                    start: s,
                    end: en,
                    inclusive: *inclusive,
                });
                self.push(h::HirExpr::Index { base: b, index: r })
            }
            Expr::Block(b) | Expr::Unsafe(b) => self.block(b)?,
            Expr::LabeledBlock { label, body } => {
                let lab = self.enter_label(Some(label))?;
                let b = self.block(body)?;
                self.labels.pop();
                let brk = self.push(h::HirExpr::Break {
                    label: lab,
                    value: Some(b),
                });
                let lb = self.push(h::HirExpr::Block {
                    stmts: vec![],
                    tail: Some(brk),
                });
                self.push(h::HirExpr::Loop {
                    body: lb,
                    label: lab,
                })
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                let c = self.expr(condition)?;
                let t = self.block(then_block)?;
                let has_else = !else_block.statements.is_empty() || else_block.tail.is_some();
                let e = if has_else {
                    Some(self.block(else_block)?)
                } else {
                    None
                };
                self.push(h::HirExpr::If {
                    cond: c,
                    then: t,
                    els: e,
                })
            }
            Expr::IfLet {
                pattern,
                scrutinee,
                then_block,
                else_branch,
            } => {
                let s = self.expr(scrutinee)?;
                let (pat, t) = self.scoped(|l| Ok((l.pat(pattern)?, l.block(then_block)?)))?;
                let e = match else_branch {
                    Some(e) => self.expr(e)?,
                    None => self.push(h::HirExpr::Block {
                        stmts: vec![],
                        tail: None,
                    }),
                };
                self.push(h::HirExpr::Match {
                    scrutinee: s,
                    arms: vec![
                        h::HirArm {
                            pat,
                            guard: None,
                            body: t,
                        },
                        h::HirArm {
                            pat: h::HirPat::Wild,
                            guard: None,
                            body: e,
                        },
                    ],
                })
            }
            Expr::Match { scrutinee, arms } => {
                let s = self.expr(scrutinee)?;
                let mut out = Vec::new();
                for a in arms {
                    let arm = self.scoped(|l| {
                        let pat = l.pat(&a.pattern)?;
                        let guard = match &a.guard {
                            Some(g) => Some(l.expr(g)?),
                            None => None,
                        };
                        let body = l.expr(&a.body)?;
                        Ok(h::HirArm { pat, guard, body })
                    })?;
                    out.push(arm);
                }
                self.push(h::HirExpr::Match {
                    scrutinee: s,
                    arms: out,
                })
            }
            Expr::Loop { label, body } => {
                let lab = self.enter_label(label.as_ref())?;
                let b = self.block(body)?;
                self.labels.pop();
                self.push(h::HirExpr::Loop {
                    body: b,
                    label: lab,
                })
            }
            Expr::Break { label, value } => {
                let lab = self.find_label(label.as_ref())?;
                let v = match value {
                    Some(v) => Some(self.expr(v)?),
                    None => None,
                };
                self.push(h::HirExpr::Break {
                    label: lab,
                    value: v,
                })
            }
            Expr::Continue { .. } => self.push(h::HirExpr::Continue),
            Expr::MethodCall {
                receiver,
                name,
                type_args,
                args,
            } => {
                let r = self.expr(receiver)?;
                let tf = type_args.iter().map(|t| self.ty(t)).collect::<R<_>>()?;
                let a = self.exprs(args)?;
                let name = self
                    .names()
                    .spelling(*name)
                    .map_err(|e| Unsupported(format!("{e:?}")))?
                    .to_owned();
                self.push(h::HirExpr::MethodCall {
                    receiver: r,
                    name,
                    turbofish: tf,
                    args: a,
                })
            }
            Expr::AssociatedCall {
                owner,
                name,
                type_args,
                args,
            } => {
                let self_ty = self.ty(owner)?;
                let name = self
                    .names()
                    .spelling(*name)
                    .map_err(|e| Unsupported(format!("{e:?}")))?
                    .to_owned();
                if !type_args.is_empty() {
                    return un("turbofish on associated call");
                }
                let callee = self.push(h::HirExpr::AssocFn { self_ty, name });
                let a = self.exprs(args)?;
                self.push(h::HirExpr::Call { callee, args: a })
            }
            Expr::Call { name, args } => {
                let callee = self.callee(name)?;
                let a = self.exprs(args)?;
                self.push(h::HirExpr::Call { callee, args: a })
            }
            Expr::Apply { path, value } => {
                // Explicit generic arguments on a call or constructor.
                let Expr::Call { name, args } = value.as_ref() else {
                    return un("generic application on non-call");
                };
                let callee = self.callee(name)?;
                let written: Vec<st::Ty> = path
                    .segments
                    .iter()
                    .flat_map(|s| s.args.iter())
                    .map(|t| self.ty(t))
                    .collect::<R<_>>()?;
                if let h::HirExpr::Fn { args: fa, .. } = &mut self.exprs[callee as usize] {
                    *fa = written;
                }
                let a = self.exprs(args)?;
                self.push(h::HirExpr::Call { callee, args: a })
            }
            Expr::Struct { name, fields } => {
                let (adt, variant) = self.struct_target(name)?;
                let mut out = Vec::new();
                for (n, v) in fields {
                    let idx = self.field_index(adt, variant, &FieldSelector::Named(*n))?;
                    out.push((idx, self.expr(v)?));
                }
                self.push(h::HirExpr::Struct {
                    adt,
                    variant,
                    fields: out,
                    base: None,
                })
            }
            Expr::TupleStruct { name, fields } => {
                let (adt, variant) = self.struct_target(name)?;
                let mut out = Vec::new();
                for (i, v) in fields.iter().enumerate() {
                    out.push((i as u32, self.expr(v)?));
                }
                self.push(h::HirExpr::Struct {
                    adt,
                    variant,
                    fields: out,
                    base: None,
                })
            }
            Expr::EnumVariant { name, fields, .. } => {
                let (adt, variant) = self.variant(name)?;
                let mut out = Vec::new();
                for (sel, v) in fields {
                    let idx = self.field_index(adt, variant, sel)?;
                    out.push((idx, self.expr(v)?));
                }
                self.push(h::HirExpr::Struct {
                    adt,
                    variant,
                    fields: out,
                    base: None,
                })
            }
            Expr::Println(_) => self.push(h::HirExpr::Block {
                stmts: vec![],
                tail: None,
            }),
            Expr::Format { arguments, .. } => {
                // Arguments are checked as expressions; formatting trait
                // obligations are not modeled yet.
                let mut stmts = Vec::new();
                for a in arguments {
                    let x = self.expr(a)?;
                    stmts.push(h::HirStmt::Expr(x));
                }
                self.push(h::HirExpr::Block { stmts, tail: None })
            }
            Expr::Panic(_) => {
                // Diverges. A label-free empty loop has type `!`.
                let body = self.push(h::HirExpr::Block {
                    stmts: vec![],
                    tail: None,
                });
                self.push(h::HirExpr::Loop {
                    body,
                    label: u32::MAX,
                })
            }
            Expr::Try(v) => {
                let x = self.expr(v)?;
                self.push(h::HirExpr::Try(x))
            }
            // Literal-only output has no operands and type `()`.
            Expr::Output(_) => self.push(h::HirExpr::Block {
                stmts: vec![],
                tail: None,
            }),
            // Buffered format!/write! placeholder calls (format_macros.rs)
            // have type `()` inside the parser-built buffer block. As for
            // `Format`, operands are checked as expressions; the
            // Display/Debug obligation is not modeled yet. The fabricated
            // `rest` arguments only name that block's buffer and constants.
            Expr::FormatInto(source) => {
                let mut stmts = Vec::new();
                for a in &source.arguments {
                    let x = self.expr(a)?;
                    stmts.push(h::HirStmt::Expr(x));
                }
                self.push(h::HirExpr::Block { stmts, tail: None })
            }
            Expr::ConstItem(path) => {
                let decl = *self
                    .plan
                    .registry
                    .constants
                    .get(&path.key(Namespace::Value))
                    .ok_or_else(|| Unsupported("unresolved constant".into()))?;
                let info = self.plan.declaration(decl).map_err(Unsupported)?;
                let ty = self
                    .plan
                    .program
                    .constants
                    .get(info.source_index)
                    .map(|c| c.ty.clone())
                    .ok_or_else(|| Unsupported("constant source".into()))?;
                let t = self.ty(&ty)?;
                self.push(h::HirExpr::Const(t))
            }
            // Inherent `Type::NAME`. Trait-qualified forms stay unsupported.
            Expr::AssociatedConst(value) if value.trait_path.is_none() => {
                let self_ty = self.ty(&value.owner)?;
                let name = self
                    .names()
                    .spelling(value.name)
                    .map_err(|e| Unsupported(format!("{e:?}")))?
                    .to_owned();
                self.push(h::HirExpr::AssocConst { self_ty, name })
            }
            Expr::AssociatedConst(_) => return un("trait associated constant"),
            Expr::TraitCall {
                owner,
                trait_path,
                name,
                type_args,
                args,
            } => {
                if !type_args.is_empty() {
                    return un("turbofish on qualified trait call");
                }
                let trait_decl = self
                    .plan
                    .resolve_trait_path(trait_path)
                    .map_err(Unsupported)?;
                let self_ty = self.ty(owner)?;
                let name = self
                    .names()
                    .spelling(*name)
                    .map_err(|e| Unsupported(format!("{e:?}")))?
                    .to_owned();
                let a = self.exprs(args)?;
                self.push(h::HirExpr::TraitCall {
                    trait_id: def(trait_decl),
                    self_ty,
                    name,
                    args: a,
                })
            }
            Expr::Invoke { .. } => return un("callee expression invocation"),
            Expr::ByteStr(_) => return un("byte string"),
            Expr::TypeAlignment(_) | Expr::NativeSyscall(_) | Expr::RawSlice { .. } => {
                return un("bootstrap intrinsic");
            }
            Expr::UnexpandedMacro(_) => return un("unexpanded macro"),
        })
    }

    /// Inline a stored closure: its body is checked in the parent's scopes,
    /// so captured locals resolve to the parent's bindings.
    fn closure(&mut self, s: &closures::stored::StoredClosureSyntax) -> R<h::ExprId> {
        let saved_labels = std::mem::take(&mut self.labels);
        let r = self.scoped(|l| {
            let mut params = Vec::new();
            for (p, t) in &s.params {
                let t = l.ty(t)?;
                params.push((l.pat(p)?, t));
            }
            let ret = l.ty(&s.result)?;
            let body = l.block(&s.body)?;
            Ok(h::HirExpr::Closure {
                params,
                ret,
                body,
                id: s.site,
            })
        });
        self.labels = saved_labels;
        let e = r?;
        Ok(self.push(e))
    }

    fn struct_target(&self, name: &AstPath) -> R<(st::DefId, u32)> {
        if name.anchor() == PathAnchor::SelfType
            && name.is_empty()
            && let Some(t) = &self.self_ty
            && let st::TyKind::Adt(d, _) = t.as_ref()
        {
            return Ok((*d, 0));
        }
        match self.plan.registry.nominals.get(&name.key(Namespace::Type)) {
            Some(d) => Ok((def(*d), 0)),
            None => self.variant(name),
        }
    }

    fn var(&mut self, path: &AstPath) -> R<h::ExprId> {
        if let Ok(Some(stored)) = self.plan.stored_closure_syntax(path) {
            return self.closure(stored);
        }
        if path.anchor() == PathAnchor::SelfModule && path.is_empty() {
            let id = self
                .receiver
                .ok_or_else(|| Unsupported("self outside method".into()))?;
            return Ok(self.push(h::HirExpr::Local(id)));
        }
        if path.anchor() == PathAnchor::Relative
            && let Some(n) = path.single()
            && let Some(id) = self.lookup(n)?
        {
            return Ok(self.push(h::HirExpr::Local(id)));
        }
        if let Some(d) = self
            .plan
            .registry
            .functions
            .get(&path.key(Namespace::Value))
        {
            return Ok(self.push(h::HirExpr::Fn {
                def: def(*d),
                self_ty: None,
                args: vec![],
            }));
        }
        // Unit variant or unit struct.
        if let Ok((adt, variant)) = self.variant(path) {
            return Ok(self.push(h::HirExpr::Ctor { adt, variant }));
        }
        un("unresolved value path")
    }

    fn callee(&mut self, name: &AstPath) -> R<h::ExprId> {
        if name.anchor() == PathAnchor::Relative
            && let Some(n) = name.single()
            && let Some(id) = self.lookup(n)?
        {
            return Ok(self.push(h::HirExpr::Local(id)));
        }
        if let Some(d) = self
            .plan
            .registry
            .functions
            .get(&name.key(Namespace::Value))
        {
            return Ok(self.push(h::HirExpr::Fn {
                def: def(*d),
                self_ty: None,
                args: vec![],
            }));
        }
        // Tuple-struct / tuple-variant constructor.
        let (adt, variant) = self.struct_target(name)?;
        Ok(self.push(h::HirExpr::Ctor { adt, variant }))
    }
}

fn binop(op: BinaryOp) -> R<h::BinOp> {
    use h::BinOp as B;
    Ok(match op {
        BinaryOp::Add => B::Add,
        BinaryOp::Sub => B::Sub,
        BinaryOp::Mul => B::Mul,
        BinaryOp::Div => B::Div,
        BinaryOp::Rem => B::Rem,
        BinaryOp::Eq => B::Eq,
        BinaryOp::Ne => B::Ne,
        BinaryOp::Lt => B::Lt,
        BinaryOp::Le => B::Le,
        BinaryOp::Gt => B::Gt,
        BinaryOp::Ge => B::Ge,
        BinaryOp::And => B::And,
        BinaryOp::Or => B::Or,
        BinaryOp::BitAnd => B::BitAnd,
        BinaryOp::BitOr => B::BitOr,
        BinaryOp::BitXor => B::BitXor,
        BinaryOp::Shl => B::Shl,
        BinaryOp::Shr => B::Shr,
    })
}
