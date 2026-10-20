//! Resolve a stored closure's syntax in its parent's lexical environment.
use super::*;

impl Resolver<'_> {
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
