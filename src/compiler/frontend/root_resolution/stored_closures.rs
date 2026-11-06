//! Resolve a stored closure's syntax in its parent's lexical environment.
use super::*;

impl Resolver<'_> {
    /// A sealed suspended body is admitted source syntax, not a resolved
    /// executable body. Preserve its exact authored tree until the mandatory
    /// per-callable context check selects E0728 or explicit Unsupported.
    /// Anonymous declarations named by that tree still need real admission.
    pub(super) fn source_body(
        &mut self,
        body: &mut Block,
        suspended: bool,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<(), String> {
        if suspended {
            self.admit_suspended_closures(body, 0)
        } else {
            self.block(body, module, locals)
        }
    }

    fn admit_suspended_closures(&mut self, body: &Block, depth: usize) -> Result<(), String> {
        self.alias_budget.check_depth(depth)?;
        let mut paths = Vec::new();
        walk_depth_inspecting(
            DepthNode::Block(body),
            Some(&self.alias_budget),
            None,
            Some(&mut |node, _| {
                if let DepthNode::Expr(Expr::Var(path)) = node {
                    self.alias_budget
                        .retain_bytes(2 * std::mem::size_of::<AstPath>())?;
                    paths
                        .try_reserve_exact(1)
                        .map_err(|_| "ResourceLimit: suspended closure source allocation failed")?;
                    let authored: &AstPath = path;
                    paths.push(authored.clone());
                }
                Ok(())
            }),
        )?;
        for path in paths {
            self.alias_budget.charge_work(1)?;
            if !closures::anonymous_path(&path, self.names())? {
                continue;
            }
            let key = self.key(&path)?;
            if self.resolved_closures.contains_key(&key) {
                continue;
            }
            let original = self
                .program
                .closure_sources
                .get(&key)
                .ok_or("contract: suspended closure lacks original declaration")?;
            if self.key(&original.name)? != key {
                return Err("contract: suspended closure declaration changed original path".into());
            }
            if let Some(parent) = &original.parent {
                source_clone::clone_parameters(&parent.params, &self.alias_budget)?;
            }
            if let Some(syntax) = &original.stored {
                if let Some(receipt) = &syntax.suspended_source {
                    receipt.replay_admission(&syntax.body, self.names())?;
                }
                source_clone::clone_vector(&syntax.params, &self.alias_budget)?;
                for (pattern, ty) in &syntax.params {
                    walk_depth(DepthNode::Pattern(pattern), Some(&self.alias_budget))?;
                    walk_depth(DepthNode::Type(ty), Some(&self.alias_budget))?;
                }
                walk_depth(DepthNode::Type(&syntax.result), Some(&self.alias_budget))?;
                walk_depth(DepthNode::Block(&syntax.body), Some(&self.alias_budget))?;
            }
            let mut source = original.clone();
            if source.parent.is_none() {
                source.owner_source = self.current_callable;
            }
            let syntax = source
                .stored
                .as_ref()
                .ok_or("unsupported: suspended immediate closure source is unsupported")?;
            self.admit_suspended_closures(&syntax.body, depth + 1)?;
            self.charge_entries::<closures::ClosureSource>(1)?;
            self.resolved_closures
                .try_reserve(1)
                .map_err(|_| "ResourceLimit: suspended closure declaration allocation failed")?;
            self.resolved_closures.insert(key, source);
        }
        Ok(())
    }

    pub(super) fn stored_closure_value(
        &mut self,
        name: &AstPath,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<bool, String> {
        if !closures::anonymous_path(name, self.names())? {
            return Ok(false);
        }
        let key = self.key(name)?;
        let Some(mut source) = self.program.closure_sources.get(&key).cloned() else {
            return Ok(false);
        };
        if source.parent.is_none() {
            source.owner_source = self.current_callable;
        }
        let Some(syntax) = &mut source.stored else {
            return Ok(false);
        };
        let previous_scope = self.lexical_function;
        let previous_anonymous = self.anonymous_closure_body;
        let previous_callable = self.current_callable.take();
        self.lexical_function = syntax.lexical_scope;
        self.anonymous_closure_body = false;
        let previous_depth = locals.len();
        let result = (|| {
            let mut parameters = std::collections::HashSet::new();
            for (pattern, ty) in &mut syntax.params {
                if !matches!(ty, Type::Infer) {
                    self.ty(ty, module)?;
                }
                self.pattern(pattern, module, &mut parameters)?;
            }
            if !matches!(syntax.result, Type::Infer) {
                self.ty(&mut syntax.result, module)?;
            }
            locals.push(parameters);
            self.block(&mut syntax.body, module, locals)
        })();
        locals.truncate(previous_depth);
        self.lexical_function = previous_scope;
        self.anonymous_closure_body = previous_anonymous;
        self.current_callable = previous_callable;
        result?;
        self.resolved_closures.insert(key, source);
        Ok(true)
    }
}
