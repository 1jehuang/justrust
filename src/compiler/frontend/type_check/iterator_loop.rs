//! For-loop desugaring uses canonical IntoIterator and Iterator declarations.
//! The iterable is evaluated once. No adapter is peeled and no source names
//! or source patterns are invented for the generated calls and Option match.
use super::*;

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests;

impl Checker<'_> {
    pub(super) fn for_iterator_statement(
        &mut self,
        label: Option<AstIdent>,
        pattern: &Pattern,
        source: &ForSource,
        value: &Expr,
        body: &Block,
    ) -> Result<(ir::Stmt, bool), String> {
        let loop_depth = self.loops.len();
        let scope_depth = self.scopes.len();
        let outer_reachable = self.reachable;
        let result = (|| {
            let plan = self.signatures.plan()?.clone();
            let value = self.expr(value)?;
            let flow = self.flow(value);
            let item_ty = plan
                .iterator_item_type(&flow.ty, self.facts())?
                .ok_or("E0277: for-loop value does not implement IntoIterator")?;
            let iterator_ty = plan
                .iterator_into_iter_type(&flow.ty, self.facts())?
                .ok_or("IntoIterator has no authoritative IntoIter type")?;
            let (into_trait, into_name, _) = plan
                .into_iterator_method()?
                .ok_or("canonical IntoIterator::into_iter declaration is unavailable")?;
            let into_goal = ir::TraitRef {
                declaration: into_trait,
                self_ty: flow.ty.clone(),
                args: Arc::from([]),
            };
            let (target, signature, _) = plan
                .select_trait(
                    &into_goal,
                    &into_name,
                    &[],
                    self.facts(),
                    false,
                    &plan.budget,
                )?
                .ok_or("E0277: for-loop IntoIterator selection failed")?;
            let iterator_value = self.finish_checked_call(
                target,
                into_name,
                signature,
                &[],
                Arc::from([]),
                Some((value, None)),
            )?;
            self.require(&self.flow(iterator_value), &iterator_ty)?;
            if let Some(cap) = self.canonical {
                cap.record_for_iterator_call(
                    source,
                    false,
                    iterator_value,
                    &self.exprs[iterator_value.0].kind,
                )?;
            }
            let iterator = self.temporary("$for_iterator", iterator_ty.clone(), true)?;
            let (next_trait, next_name, _) = plan
                .iterator_next_method()?
                .ok_or("canonical Iterator::next declaration is unavailable")?;
            let next_goal = ir::TraitRef {
                declaration: next_trait,
                self_ty: iterator_ty,
                args: Arc::from([]),
            };
            let (target, signature, _) = plan
                .select_trait(
                    &next_goal,
                    &next_name,
                    &[],
                    self.facts(),
                    false,
                    &plan.budget,
                )?
                .ok_or("E0277: IntoIterator::IntoIter does not implement Iterator")?;
            let ir::Ty::Enum(option) = &signature.result else {
                return Err("canonical Iterator::next must return Option<Item>".into());
            };
            let option = option.clone();
            let variants = &option.def().variants;
            if variants.len() != 2 {
                return Err("canonical Iterator::next result changed Option shape".into());
            }
            let mut some = None;
            let mut none = None;
            for (index, variant) in variants.iter().enumerate() {
                plan.budget.charge_work(1)?;
                match (variant.name.as_str(), variant.shape, variant.fields.len()) {
                    ("Some", ir::VariantShape::Tuple, 1) => {
                        self.require(
                            &Flow {
                                ty: variant.fields[0].ty.clone(),
                                completes: true,
                            },
                            &item_ty,
                        )?;
                        some = Some(index);
                    }
                    ("None", ir::VariantShape::Unit, 0) => none = Some(index),
                    _ => {
                        return Err("canonical Iterator::next result changed Option variants".into());
                    }
                }
            }
            let some = some.ok_or("canonical Option::Some is missing")?;
            let none = none.ok_or("canonical Option::None is missing")?;
            let iterator_borrow = self.borrow_expression(
                ir::BorrowKind::Mut,
                ir::Place {
                    local: iterator,
                    projection: vec![],
                },
            )?;
            let next = self.finish_checked_call(
                target,
                next_name,
                signature,
                &[],
                Arc::from([]),
                Some((iterator_borrow, None)),
            )?;
            if let Some(cap) = self.canonical {
                cap.record_for_iterator_call(source, true, next, &self.exprs[next.0].kind)?;
            }
            let id = self.enter_loop(false);
            let key = self.label_key(label)?;
            let frame = self.loops.last_mut().ok_or("missing iterator loop frame")?;
            frame.label = key;
            frame.checking_condition = false;
            frame.can_exit = true;
            self.reachable = true;
            let item = self.temporary("$for_item", item_ty.clone(), false)?;
            let item_value = self.for_expr(item_ty, ir::ExprKind::Local(item))?;
            let (body_kind, body_flow) = self.lower_match_value(
                item_value,
                [BorrowedMatchArm {
                    pattern: Some(pattern),
                    guarded: false,
                    guard: None,
                    body: MatchBody::Block(body),
                }]
                .into_iter(),
                Some(&ir::Ty::Unit),
            )?;
            self.require(&body_flow, &ir::Ty::Unit)?;
            let matched_body = self.for_expr(body_flow.ty, body_kind)?;
            self.exprs[matched_body.0].completion = if body_flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            };
            let exit = self.for_expr(
                ir::Ty::Never,
                ir::ExprKind::Break {
                    target: id,
                    value: None,
                },
            )?;
            self.exprs[exit.0].completion = ir::Completion::Never;
            let matched = self.for_expr(
                ir::Ty::Unit,
                ir::ExprKind::Match {
                    scrutinee: next,
                    arms: vec![
                        ir::MatchArm {
                            pattern: ir::MatchPattern::EnumVariant {
                                ty: option.clone(),
                                variant: some,
                                bindings: vec![ir::PatternBinding {
                                    local: item,
                                    projection: vec![
                                        ir::Projection::Downcast(ir::VariantId {
                                            owner: option.id(),
                                            index: some,
                                        }),
                                        ir::Projection::Field(0),
                                    ],
                                    borrow: None,
                                }],
                            },
                            guard: None,
                            body: matched_body,
                        },
                        ir::MatchArm {
                            pattern: ir::MatchPattern::EnumVariant {
                                ty: option,
                                variant: none,
                                bindings: vec![],
                            },
                            guard: None,
                            body: exit,
                        },
                    ],
                },
            )?;
            let loop_value = self.for_expr(
                ir::Ty::Unit,
                ir::ExprKind::Loop {
                    id,
                    body: ir::Block {
                        statements: vec![ir::Stmt::Expr(matched)],
                        tail: None,
                    },
                },
            )?;
            let block = self.for_expr(
                ir::Ty::Unit,
                ir::ExprKind::Block(ir::Block {
                    statements: vec![
                        ir::Stmt::Let {
                            local: iterator,
                            value: iterator_value,
                        },
                        ir::Stmt::Expr(loop_value),
                    ],
                    tail: None,
                }),
            )?;
            self.exprs[block.0].completion = if flow.completes {
                ir::Completion::Returns
            } else {
                ir::Completion::Never
            };
            Ok((ir::Stmt::Expr(block), flow.completes))
        })();
        self.loops.truncate(loop_depth);
        self.scopes.truncate(scope_depth);
        self.reachable = outer_reachable;
        result
    }
}
