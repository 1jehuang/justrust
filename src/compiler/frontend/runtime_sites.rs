//! Source occurrence to immutable runtime diagnostic facts. Coordinates come
//! from the original token session, never from reparsing source during codegen.
use super::*;

impl Checker<'_> {
    pub(super) fn record_runtime_site(
        &mut self,
        expression: &Expr,
        id: ir::ExprId,
    ) -> Result<(), String> {
        let site = match expression {
            Expr::Binary {
                site: Some(site), ..
            }
            | Expr::Unary {
                site: Some(site), ..
            } => site,
            _ => return Ok(()),
        };
        if !matches!(
            self.exprs[id.0].kind,
            ir::ExprKind::Binary { .. } | ir::ExprKind::Unary { .. }
        ) {
            return Ok(());
        }
        let names = self.names()?;
        validate_source_site(*site, names)?;
        let original = names
            .session()
            .original(
                panic_callsite(names.session(), *site).map_err(|error| {
                    super::super::source::SourceError::from_token(error).to_string()
                })?,
                512,
            )
            .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        let (path, line, column) = names
            .session()
            .sources()
            .location(original)
            .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        let location = ir::RuntimeLocation {
            path: path.display().to_string().into(),
            line,
            column,
        };
        self.charge_node(
            std::mem::size_of::<ir::RuntimeLocation>()
                + location.path.len()
                + 2 * std::mem::size_of::<(ir::ExprId, Arc<ir::RuntimeLocation>)>(),
        )?;
        let location = Arc::new(location);
        if let Some(cap) = self.canonical {
            cap.runtime_sites.borrow_mut().push((id, location.clone()));
        }
        self.runtime_sites.push((id, location));
        Ok(())
    }
}

impl CanonicalSourceCapability {
    pub(in crate::compiler) fn validate_runtime_sites(
        &self,
        source: &ir::CheckedFunction,
    ) -> Result<(), FactsError> {
        let expected = ir::RuntimeSites::from_source(self.runtime_sites.borrow().clone())
            .map_err(|message| FactsError::invalid(&self.plan.budget, &message))?;
        if source.runtime_sites != expected {
            return Err(FactsError::invalid(
                &self.plan.budget,
                "source runtime panic locations changed original occurrences",
            ));
        }
        source
            .runtime_sites
            .validate(source.exprs.len())
            .map_err(|message| FactsError::invalid(&self.plan.budget, &message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_locations_cannot_replace_sealed_source_or_cfg_origins() {
        let program = parse("fn add(a:i32,b:i32)->i32{a+b}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected source publication");
        };
        let checked = publication.source();
        let expression = ir::ExprId(
            checked
                .exprs
                .iter()
                .position(|e| matches!(e.kind, ir::ExprKind::Binary { .. }))
                .unwrap(),
        );
        let original = checked.runtime_sites.get(expression).unwrap();
        assert!(original.line > 0 && original.column > 0);
        for location in [
            None,
            Some(ir::RuntimeLocation {
                path: "foreign.rs".into(),
                line: 99,
                column: 17,
            }),
        ] {
            let mut forged = checked.as_ref().clone();
            forged.runtime_sites = match location {
                None => Default::default(),
                Some(location) => {
                    ir::RuntimeSites::from_source(vec![(expression, Arc::new(location))]).unwrap()
                }
            };
            assert!(
                publication
                    .validate_source(&Arc::new(forged), publication.facts(), &budget)
                    .is_err()
            );
        }
        let cfg = crate::compiler::cfg::lower_source_closed(&publication, &budget).unwrap();
        cfg.validate_provenance(publication.facts(), &budget)
            .unwrap();
        let mut erased = cfg.clone();
        erased.runtime_sites = Default::default();
        assert!(
            erased
                .validate_provenance(publication.facts(), &budget)
                .is_err()
        );
        let mut forged = cfg.clone();
        let operation = forged
            .blocks
            .iter_mut()
            .flat_map(|b| &mut b.statements)
            .find(|s| s.origin == Some(expression))
            .unwrap();
        operation.origin = None;
        assert!(
            forged
                .validate_provenance(publication.facts(), &budget)
                .is_err()
        );
    }
}
