//! Immutable `static` items as places.
//!
//! Scalar and aggregate statics are still read by value through the const
//! path (each read copies the value, exactly like stock). A shared borrow of
//! the bare item instead names its one interned storage (`Expr::StaticRef`),
//! so every `&S` has the same address. Mutable borrows and assignments are
//! rejected exactly as stock does for immutable statics.
use super::*;

impl Resolver<'_> {
    /// The canonical declaration path when `ident` names an immutable source
    /// `static` item rather than a local.
    pub(super) fn static_item(
        &mut self,
        ident: AstIdent,
        module: ModuleId,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<Option<AstPath>, String> {
        let path = self.single_path(ident)?;
        self.static_path(&path, module, locals)
    }

    fn static_path(
        &mut self,
        path: &AstPath,
        module: ModuleId,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<Option<AstPath>, String> {
        let path = path.clone();
        if self.local_contains(&path, locals)? {
            return Ok(None);
        }
        let Some(item) = self.path(&path, module, false, false)? else {
            return Ok(None);
        };
        if item.kind != ResolvedKind::Const {
            return Ok(None);
        }
        let target = item.source()?.clone();
        let key = self.key(&target)?;
        let mut storage = None;
        for declaration in &self.program.constants {
            self.alias_budget.charge_work(1)?;
            if self.key(&declaration.name)? == key {
                storage = Some(declaration.storage);
                break;
            }
        }
        if storage != Some(ConstantStorage::Static) {
            return Ok(None);
        }
        Ok(Some(self.resolved_use(&target, &path)?))
    }

    /// `&S` / `&mut S` / `&S.f` rooted at an immutable static item, and
    /// `&S[i]...` value borrows projected through its one interned storage.
    pub(super) fn static_borrow(
        &mut self,
        expr: &mut Expr,
        module: ModuleId,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<bool, String> {
        if let Expr::BorrowValue { kind, value } = expr {
            let kind = *kind;
            let mut root: &mut Expr = value;
            let mut projected = false;
            loop {
                match root {
                    Expr::Index { base, .. }
                    | Expr::DynamicIndex { base, .. }
                    | Expr::Field { base, .. } => {
                        projected = true;
                        root = base;
                    }
                    _ => break,
                }
            }
            let Expr::Var(name) = root else {
                return Ok(false);
            };
            if name.anchor() == PathAnchor::SelfModule && name.is_empty() {
                return Ok(false);
            }
            let name = name.clone();
            let Some(path) = self.static_path(&name, module, locals)? else {
                return Ok(false);
            };
            if kind == ir::BorrowKind::Mut {
                return Err("E0596: cannot borrow immutable static item as mutable".into());
            }
            if !projected {
                *expr = Expr::StaticRef(path);
                return Ok(true);
            }
            *root = Expr::Deref {
                value: Box::new(Expr::StaticRef(path)),
            };
            // Resolve the remaining index operands; the static root is final.
            return Ok(false);
        }
        let Expr::Borrow { kind, place } = expr else {
            return Ok(false);
        };
        let LocalName::Named(ident) = place.local else {
            return Ok(false);
        };
        let Some(path) = self.static_item(ident, module, locals)? else {
            return Ok(false);
        };
        if *kind == ir::BorrowKind::Mut {
            return Err("E0596: cannot borrow immutable static item as mutable".into());
        }
        if !place.fields.is_empty() {
            return Err(Self::unsupported(
                "borrows of projections of a static item are unsupported",
            ));
        }
        *expr = Expr::StaticRef(path);
        Ok(true)
    }

    /// `S.m()`, `S.f.m()`, `S[i].m()`: a static is a place, so an autoref
    /// receiver borrows its one interned storage rather than a value copy
    /// (stock: `S.0.get(i)` returns `Option<&'static T>`). By-value receivers
    /// read `*&S`, which copies exactly as a direct read does.
    pub(super) fn static_receiver(
        &mut self,
        expr: &mut Expr,
        module: ModuleId,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<(), String> {
        let Expr::MethodCall { receiver, .. } = expr else {
            return Ok(());
        };
        let mut root: &mut Expr = receiver;
        loop {
            match root {
                Expr::Index { base, .. }
                | Expr::DynamicIndex { base, .. }
                | Expr::Field { base, .. } => root = base,
                _ => break,
            }
        }
        let Expr::Var(name) = root else {
            return Ok(());
        };
        if name.anchor() == PathAnchor::SelfModule && name.is_empty() {
            return Ok(());
        }
        let name = name.clone();
        let Some(path) = self.static_path(&name, module, locals)? else {
            return Ok(());
        };
        *root = Expr::Deref {
            value: Box::new(Expr::StaticRef(path)),
        };
        Ok(())
    }

    /// `S = v`, `S OP= v`, `S.f = v` on an immutable static item.
    pub(super) fn static_assignment(
        &mut self,
        stmt: &Stmt,
        module: ModuleId,
        locals: &[std::collections::HashSet<LocalKey>],
    ) -> Result<(), String> {
        let local = match stmt {
            Stmt::Assign { name, .. } => *name,
            Stmt::AssignOp { place, .. } | Stmt::AssignPlace { place, .. } => place.local,
            _ => return Ok(()),
        };
        let LocalName::Named(ident) = local else {
            return Ok(());
        };
        if self.static_item(ident, module, locals)?.is_some() {
            return Err("E0594: cannot assign to immutable static item".into());
        }
        Ok(())
    }
}
