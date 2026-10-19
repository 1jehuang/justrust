//! One source guard/body stays one checked expression across or alternatives.
//! Each alternative writes the same canonical pattern locals before evaluating it.
use super::*;

pub(super) struct SharedOrArm {
    pub(super) bindings: HashMap<NameKey, ir::LocalId>,
    pub(super) guard: Option<ir::ExprId>,
    pub(super) body: ir::ExprId,
}

impl Checker<'_> {
    pub(super) fn shared_pattern_bindings(
        &self,
        scope: usize,
    ) -> Result<HashMap<NameKey, ir::LocalId>, String> {
        let mut bindings = HashMap::new();
        for (key, local) in &self.scopes[scope] {
            let name = match key {
                LocalKey::Named(name) => *name,
                LocalKey::Anchored(binding) => {
                    let arena = self
                        .signatures
                        .plan()?
                        .program
                        .scopes
                        .as_ref()
                        .ok_or("missing canonical pattern scope")?;
                    let declaration = arena
                        .binding(*binding)
                        .map_err(scope_diagnostic)?
                        .declaration;
                    self.names()?
                        .key(declaration, LookupClass::Local)
                        .map_err(native_adapter::name_diagnostic)?
                }
                LocalKey::Receiver => return Err("receiver in pattern scope".into()),
            };
            bindings.insert(name, *local);
        }
        Ok(bindings)
    }
    pub(super) fn group_or_arms(
        &self,
        arms: Vec<ir::MatchArm>,
        groups: &[usize],
        domain: &ir::Ty,
    ) -> Result<Vec<ir::MatchArm>, String> {
        let mut output = Vec::with_capacity(arms.len());
        let mut pending = arms.into_iter().zip(groups.iter().copied()).peekable();
        while let Some((first, group)) = pending.next() {
            if first.guard.is_none() || pending.peek().is_none_or(|(_, next)| *next != group) {
                output.push(first);
                continue;
            }
            let mut rows = vec![first.pattern];
            while pending.peek().is_some_and(|(_, next)| *next == group) {
                let (row, _) = pending.next().unwrap();
                if row.guard != first.guard || row.body != first.body {
                    return Err("grouped or-pattern source expression mismatch".into());
                }
                rows.push(row.pattern);
            }
            output.push(ir::MatchArm {
                pattern: ir::MatchPattern::Or {
                    ty: domain.clone(),
                    rows,
                },
                guard: first.guard,
                body: first.body,
            });
        }
        Ok(output)
    }
    pub(super) fn pattern_local(
        &mut self,
        name: AstIdent,
        ty: ir::Ty,
        mutable: bool,
        previous: Option<&HashMap<NameKey, ir::LocalId>>,
    ) -> Result<ir::LocalId, String> {
        let Some(previous) = previous else {
            return self.local(name, ty, mutable);
        };
        let key = self
            .local_key(&name, true)?
            .ok_or("or-pattern binding has no canonical local identity")?;
        let semantic_name = self
            .names()?
            .key(name, LookupClass::Local)
            .map_err(native_adapter::name_diagnostic)?;
        let local = *previous
            .get(&semantic_name)
            .ok_or("or-pattern alternatives bind different variables")?;
        let declaration = self
            .locals
            .get(local.0)
            .ok_or("or-pattern binding outside checked locals")?;
        if declaration.ty != ty || declaration.mutable != mutable {
            return Err("or-pattern alternatives bind different types or mutability".into());
        }
        self.reserve_scoped_local(self.scopes.len() - 1)?;
        self.scopes.last_mut().unwrap().insert(key, local);
        Ok(local)
    }
}
