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
            }
            | Expr::Index {
                site: Some(site), ..
            }
            | Expr::DynamicIndex {
                site: Some(site), ..
            } => site,
            _ => return Ok(()),
        };
        if !matches!(
            self.exprs[id.0].kind,
            ir::ExprKind::Binary { .. }
                | ir::ExprKind::Unary { .. }
                | ir::ExprKind::Index { .. }
                | ir::ExprKind::DynamicIndex { .. }
                | ir::ExprKind::SliceIndex { .. }
                | ir::ExprKind::BoundsCheck { .. }
        ) {
            return Ok(());
        }
        self.record_runtime_span(*site, id)
    }

    pub(super) fn record_runtime_span(
        &mut self,
        site: super::super::expansion_span::SpanId,
        id: ir::ExprId,
    ) -> Result<(), String> {
        let names = self.names()?;
        validate_source_site(site, names)?;
        let original = names
            .session()
            .original(
                panic_callsite(names.session(), site).map_err(|error| {
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
    fn bounds_publication_rejects_reused_indices_moves_and_swapped_origins() {
        use crate::compiler::cfg;
        let program = parse("fn f(a:&mut [i32],b:&mut [i32],i:usize){a[i]=9;}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let PublishedCallableBody::Source(publication) = plan
            .publish_callable_body(plan.function_decl(0).unwrap(), &budget)
            .unwrap()
        else {
            panic!("expected canonical source");
        };
        let source = publication.source();
        let probe = source
            .exprs
            .iter()
            .position(|value| matches!(value.kind, ir::ExprKind::BoundsCheck { .. }))
            .unwrap();
        let ir::ExprKind::BoundsCheck {
            array,
            index: probe_index,
        } = source.exprs[probe].kind.clone()
        else {
            unreachable!()
        };
        let final_index = source
            .body
            .statements
            .iter()
            .find_map(|statement| match statement {
                ir::Stmt::AssignIndex { index, .. } => Some(*index),
                _ => None,
            })
            .unwrap();
        assert_ne!(
            probe_index, final_index,
            "a probe and final write must have independent local-read identities"
        );
        assert_eq!(source.exprs[probe].ty, ir::Ty::Unit);
        assert!(source.runtime_sites.get(ir::ExprId(probe)).is_some());
        assert!(source.runtime_sites.get(final_index).is_none());
        for change in 0..6 {
            let mut forged = source.as_ref().clone();
            match change {
                0 => forged.runtime_sites = Default::default(),
                1 => {
                    let ir::ExprKind::BoundsCheck { array, .. } = &mut forged.exprs[probe].kind
                    else {
                        unreachable!()
                    };
                    array.local = ir::LocalId(1);
                }
                2 => {
                    let ir::ExprKind::BoundsCheck { index, .. } = &mut forged.exprs[probe].kind
                    else {
                        unreachable!()
                    };
                    *index = ir::ExprId(probe);
                }
                3 => {
                    for statement in &mut forged.body.statements {
                        if let ir::Stmt::AssignIndex { index, .. } = statement {
                            *index = probe_index;
                        }
                    }
                }
                4 => {
                    forged.exprs[probe].kind = ir::ExprKind::Borrow {
                        kind: ir::BorrowKind::Mut,
                        place: array.clone(),
                        timing: ir::BorrowTiming::Immediate,
                    }
                }
                _ => forged.exprs[probe].ty = ir::Ty::Int(ir::IntTy::Usize),
            }
            assert!(
                publication
                    .validate_source(&Arc::new(forged), publication.facts(), &budget)
                    .is_err(),
                "forgery {change}"
            );
        }
        let function = cfg::lower_source_closed(&publication, &budget).unwrap();
        let (block, statement) = function
            .blocks
            .iter()
            .enumerate()
            .find_map(|(block, body)| {
                body.statements
                    .iter()
                    .enumerate()
                    .find_map(|(statement, value)| {
                        matches!(value.value, cfg::Statement::IndexInspect { .. })
                            .then_some((block, statement))
                    })
            })
            .unwrap();
        assert!(
            function
                .runtime_sites
                .get(cfg::BlockId(block), statement)
                .is_some()
        );
        for origin in [None, Some(final_index), Some(probe_index)] {
            let mut forged = function.clone();
            forged.blocks[block].statements[statement].origin = origin;
            assert!(
                forged
                    .validate_provenance(publication.facts(), &budget)
                    .is_err()
            );
        }
        let mut erased = function.clone();
        erased.runtime_sites = Default::default();
        assert!(
            erased
                .validate_provenance(publication.facts(), &budget)
                .is_err()
        );
    }

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
