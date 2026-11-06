//! Source-only suspended syntax. No future value or executable is synthesized.
use super::*;

/// Parser-issued original occurrence and its lexical async boundary. Private
/// fields cannot be supplied by typed-body clients or reconstructed from names.
#[derive(Clone, Debug)]
pub struct SourceAsync {
    site: super::super::expansion_span::SpanId,
    parent: Option<super::super::expansion_span::SpanId>,
    owner: AstIdent,
    block: bool,
    by_move: bool,
}
impl SourceAsync {
    pub(super) fn syntax(
        &self,
    ) -> (
        super::super::expansion_span::SpanId,
        Option<super::super::expansion_span::SpanId>,
        bool,
        bool,
    ) {
        (self.site, self.parent, self.block, self.by_move)
    }
    pub(super) fn validate_kind(&self, names: &impl NameAccess, block: bool) -> Result<(), String> {
        if self.block != block {
            return Err("contract: async source certificate changed AST kind".into());
        }
        self.validate(names)
    }
    pub(super) fn validate(&self, names: &impl NameAccess) -> Result<(), String> {
        names
            .check(self.owner)
            .map_err(native_adapter::name_diagnostic)?;
        let session = names.session();
        let data = session
            .span(self.site)
            .map_err(|e| super::super::source::SourceError::from_token(e).to_string())?;
        let super::super::expansion_span::SpanOrigin::Original(original) = data.origin else {
            return Err("contract: async syntax requires original provenance".into());
        };
        if data.expansion.is_some() || data.context != session.root_context() {
            return Err("contract: async syntax requires original context".into());
        }
        let sources = session.sources();
        let range = sources
            .render_span(original)
            .map_err(|e| super::super::source::SourceError::from_token(e).to_string())?;
        let text = sources
            .text(original.file())
            .map_err(|e| super::super::source::SourceError::from_token(e).to_string())?;
        if text.get(range.lo as usize..range.hi as usize)
            != Some(if self.block { "async" } else { "await" })
        {
            return Err("contract: async occurrence changed original keyword".into());
        }
        if let Some(parent) = self.parent {
            session
                .span(parent)
                .map_err(|e| super::super::source::SourceError::from_token(e).to_string())?;
        }
        Ok(())
    }
}
impl Parser<'_, '_> {
    pub(super) fn async_occurrence(&mut self, block: bool) -> Result<SourceAsync, ParseError> {
        if self.token().provenance != TokenProvenance::Prepared {
            return Err(self.unsupported("expanded async syntax is unsupported"));
        }
        let source = SourceAsync {
            site: self.token().span,
            parent: self.lexical.async_boundary,
            owner: self.lexical.async_owner.ok_or_else(|| {
                self.unsupported("async syntax outside callable bodies is unsupported")
            })?,
            block,
            by_move: false,
        };
        source.validate(self.names).map_err(|e| self.error(&e))?;
        self.lexical.async_serial = self
            .lexical
            .async_serial
            .checked_add(1)
            .ok_or_else(|| self.limit("async source occurrence identity exhausted"))?;
        Ok(source)
    }
    pub(super) fn async_block(&mut self) -> Result<Expr, ParseError> {
        let mut source = self.async_occurrence(true)?;
        self.expect("async")?;
        source.by_move = self.eat("move");
        if !self.is("{") {
            return Err(self.unsupported("async closures are unsupported"));
        }
        let previous = self.lexical.async_boundary.replace(source.site);
        let body = self.block();
        self.lexical.async_boundary = previous;
        Ok(Expr::AsyncBlock {
            source,
            body: body?,
        })
    }
}

/// Visit the entire admitted body before choosing Unsupported. In particular a
/// suspended block must not hide a wrong-context await in an ordinary closure.
/// Stored anonymous bodies are reached through their exact source value path,
/// never by scanning declarations or reading other graph nodes.
pub(super) fn validate_body(
    program: &Program,
    owner: &AstPath,
    body: &Block,
    budget: &ir::PlanBudget,
    recursion: usize,
    original: Option<&function_source::AsyncBodySource>,
) -> Result<bool, String> {
    if recursion > 16 {
        return Err(
            "ResourceLimit: async source closure recursion exceeds parser admission".into(),
        );
    }
    if let Some(original) = original {
        original.replay(body, &program.names, budget)?;
    }
    let owner = *owner
        .segments()
        .last()
        .ok_or("contract: async callable lacks identity")?;
    let mut boundaries = Vec::new();
    let mut suspended = false;
    let mut invalid = false;
    walk_depth_inspecting(
        DepthNode::Block(body),
        Some(budget),
        Some(&program.names),
        Some(&mut |node, depth| {
            budget.charge_work(1)?;
            let inherited = if depth == 0 {
                None
            } else {
                boundaries.get(depth - 1).copied().flatten()
            };
            boundaries.truncate(depth);
            if boundaries.len() == boundaries.capacity() {
                let extra = boundaries.capacity().max(1);
                budget.retain_bytes(
                    extra * std::mem::size_of::<Option<super::super::expansion_span::SpanId>>(),
                )?;
                boundaries
                    .try_reserve_exact(extra)
                    .map_err(|_| "ResourceLimit: async context allocation failed")?;
            }
            boundaries.push(inherited);
            match node {
                DepthNode::Expr(Expr::AsyncBlock { source, .. } | Expr::Await { source, .. }) => {
                    let block = matches!(node, DepthNode::Expr(Expr::AsyncBlock { .. }));
                    source.validate_kind(program.names.as_ref(), block)?;
                    if original.is_none() {
                        return Err(
                            "unsupported: async original body syntax exceeds admitted recipe"
                                .into(),
                        );
                    }
                    if program
                        .names
                        .span(source.owner)
                        .map_err(native_adapter::name_diagnostic)?
                        != program
                            .names
                            .span(owner)
                            .map_err(native_adapter::name_diagnostic)?
                        || source.parent != inherited
                    {
                        return Err(
                            "contract: async occurrence changed callable or enclosing boundary"
                                .into(),
                        );
                    }
                    suspended = true;
                    if source.block {
                        boundaries[depth] = Some(source.site);
                    } else if inherited.is_none() {
                        invalid = true;
                    }
                }
                DepthNode::Expr(Expr::Var(path)) => {
                    let key = name_scope::path_key(program.names.as_ref(), path, Namespace::Type)?;
                    if let Some(source) = program.closure_sources.get(&key)
                        && let Some(stored) = &source.stored
                    {
                        let nested = validate_body(
                            program,
                            &source.name,
                            &stored.body,
                            budget,
                            recursion + 1,
                            stored.suspended_source.as_deref(),
                        );
                        match nested {
                            Ok(false) => {}
                            Ok(true) if original.is_some() => suspended = true,
                            Err(error) if original.is_some() || !error.starts_with("E0728:") => {
                                return Err(error);
                            }
                            _ => return Err(
                                "unsupported: async enclosing closure source recipe is unavailable"
                                    .into(),
                            ),
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }),
    )?;
    if invalid {
        return Err("E0728: await is only allowed inside async functions and blocks".into());
    }
    Ok(suspended)
}

pub(super) fn check_callable(
    program: &Program,
    function: &Function,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    if validate_body(
        program,
        &function.name,
        &function.body,
        budget,
        0,
        function.suspended_source.as_deref(),
    )? {
        return Err("unsupported: async future typing and execution are unsupported".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(program: &mut Program, index: usize) -> &mut Expr {
        match &mut program.functions[0].body.statements[index] {
            Stmt::Let { value, .. } => value,
            _ => panic!("authored let"),
        }
    }
    fn typed_error(original: &Program, changed: &Program) -> String {
        let signatures = signature_table(original).unwrap();
        type_function_with_signatures(changed, 0, &signatures).unwrap_err()
    }
    fn assert_contract(original: &Program, changed: &Program) {
        let error = typed_error(original, changed);
        assert!(error.contains("contract:"), "{error}");
        assert!(
            !error.contains("E0728"),
            "forged source became semantic evidence: {error}"
        );
    }

    #[test]
    fn async_source_replay_rejects_same_owner_foreign_and_body_replacements() {
        let source_text =
            "fn unused(){let a=async {7i32}.await;let b=async move {8i32}.await;}fn main(){}";
        let original = parse(source_text).unwrap();
        assert!(typed_error(&original, &original).contains("E0728"));
        let mut changed = original.clone();
        let second = value(&mut changed, 1).clone();
        *value(&mut changed, 0) = second;
        assert_contract(&original, &changed);

        let mut changed = original.clone();
        let Expr::Await { source, .. } = value(&mut changed, 1) else {
            panic!("await")
        };
        let replacement = source.clone();
        let Expr::Await { source, .. } = value(&mut changed, 0) else {
            panic!("await")
        };
        *source = replacement;
        assert_contract(&original, &changed);

        let mut foreign = parse(source_text).unwrap();
        let replacement = value(&mut foreign, 0).clone();
        let mut changed = original.clone();
        *value(&mut changed, 0) = replacement;
        assert!(
            typed_error(&original, &changed).contains("contract:")
                || name_validation::validate_program_names(&changed).is_err()
        );

        let mut changed = original.clone();
        let Expr::Await { operand, .. } = value(&mut changed, 0) else {
            panic!("await")
        };
        let Expr::AsyncBlock { body, .. } = operand.as_mut() else {
            panic!("async")
        };
        body.tail = Some(Box::new(Expr::Integer(IntegerLiteral {
            magnitude: 9,
            suffix: Some(ir::IntTy::I32),
        })));
        assert_contract(&original, &changed);
    }

    #[test]
    fn async_source_replay_rejects_capture_kind_variant_and_context_replacement() {
        let original = parse("fn unused(){let a=async {7i32}.await;}fn main(){}").unwrap();
        let mut changed = original.clone();
        let Expr::Await { operand, .. } = value(&mut changed, 0) else {
            panic!("await")
        };
        let Expr::AsyncBlock { source, .. } = operand.as_mut() else {
            panic!("async")
        };
        source.by_move = true;
        assert_contract(&original, &changed);

        let mut changed = original.clone();
        let Expr::Await { source, operand } = value(&mut changed, 0) else {
            panic!("await")
        };
        let Expr::AsyncBlock {
            source: block_source,
            ..
        } = operand.as_mut()
        else {
            panic!("async")
        };
        *source = block_source.clone();
        assert!(
            name_validation::validate_program_names(&changed)
                .unwrap_err()
                .contains("AST kind")
        );
        assert_contract(&original, &changed);

        let original = parse("fn unused(){let a=async {async {7i32}.await};}fn main(){}").unwrap();
        let mut changed = original.clone();
        let Expr::AsyncBlock { body, .. } = value(&mut changed, 0) else {
            panic!("async")
        };
        let replacement = *body.tail.clone().unwrap();
        *value(&mut changed, 0) = replacement;
        assert_contract(&original, &changed);

        let mut changed = original.clone();
        changed.functions[0].suspended_source = None;
        assert_contract(&original, &changed);
    }

    #[test]
    fn async_source_replay_keeps_authored_callee_and_stored_closure_identity() {
        let original = parse("fn unused(){let a=async {one()}.await;let b=async {two()}.await;}fn one()->i32{1i32}fn two()->i32{2i32}fn main(){}").unwrap();
        let mut changed = original.clone();
        let Expr::Await { operand, .. } = value(&mut changed, 1) else {
            panic!("await")
        };
        let Expr::AsyncBlock { body, .. } = operand.as_mut() else {
            panic!("async")
        };
        let replacement = body.tail.clone();
        let Expr::Await { operand, .. } = value(&mut changed, 0) else {
            panic!("await")
        };
        let Expr::AsyncBlock { body, .. } = operand.as_mut() else {
            panic!("async")
        };
        body.tail = replacement;
        assert_contract(&original, &changed);

        let original =
            parse("fn unused(){let a=|| {async {7i32}};let b=|| {async {8i32}.await};}fn main(){}")
                .unwrap();
        let mut changed = original.clone();
        let replacement = value(&mut changed, 1).clone();
        *value(&mut changed, 0) = replacement;
        assert_contract(&original, &changed);

        let mut changed = original.clone();
        let Expr::Var(path) = value(&mut changed, 0).clone() else {
            panic!("closure")
        };
        let key = name_scope::path_key(changed.names.as_ref(), &path, Namespace::Type).unwrap();
        Arc::make_mut(&mut changed.closure_sources)
            .get_mut(&key)
            .unwrap()
            .stored
            .as_mut()
            .unwrap()
            .body
            .tail = Some(Box::new(Expr::Integer(IntegerLiteral {
            magnitude: 1,
            suffix: Some(ir::IntTy::I32),
        })));
        assert_contract(&original, &changed);
    }
}
