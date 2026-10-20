//! Loop binding constraints follow the source iteration mode. This only supplies
//! inference evidence. The ordinary loop checker still proves iteration support.
use super::*;

pub(super) enum Pending {
    Ordinary(Term, Term),
    ArrayMethod {
        receiver: Term,
        name: AstIdent,
        result: Term,
    },
}

impl<'s> Solver<'_, '_, 's> {
    pub(super) fn iteration_item(
        &mut self,
        source: &'s ForSource,
        depth: usize,
    ) -> Result<Term, String> {
        let (raw_item, borrowed) = match &source.iterable {
            ForIterable::Range { start, end, .. } => {
                let start = self.expr(start, depth + 1)?;
                let end = self.expr(end, depth + 1)?;
                self.unify(start, end)?;
                (start, None)
            }
            ForIterable::Value(value) => {
                if source.adapters.is_empty()
                    && let Some((receiver, name)) =
                        iterator_source::array_iter_candidate(value, self.checker.names()?)?
                {
                    let receiver = self.expr(receiver, depth + 1)?;
                    let result = self.fresh()?;
                    self.delayed_results.push(result);
                    self.iterator_items.push(Pending::ArrayMethod {
                        receiver,
                        name,
                        result,
                    });
                    return Ok(result);
                }
                let value = self.expr(value, depth + 1)?;
                let (base, borrowed) = self.pattern_referent(value);
                let item = match self.domain(base) {
                    Domain::Array(item, _) | Domain::Slice(item) => item,
                    _ if source.adapters.is_empty() => return self.iterator_item(value),
                    // Non-array adapters must be reconstructed as ordinary
                    // method expressions before projecting their actual Item.
                    _ => return self.fresh(),
                };
                (item, borrowed)
            }
        };
        let mut item = match borrowed {
            Some(kind) => self.node(Domain::Ref(kind, raw_item))?,
            None => raw_item,
        };
        for adapter in &source.adapters {
            self.checker.infer_work(depth)?;
            match adapter {
                ForAdapter::Iter => {
                    item = self.node(Domain::Ref(ir::BorrowKind::Shared, raw_item))?;
                }
                ForAdapter::Enumerate => {
                    let index = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                    item = self.node(Domain::Tuple(vec![index, item]))?;
                }
                ForAdapter::IntoIter | ForAdapter::Rev => {}
            }
        }
        Ok(item)
    }

    fn iterator_item(&mut self, source: Term) -> Result<Term, String> {
        if let Some(ty) = self.resolve(source, 0)? {
            let plan = self.checker.signatures.plan()?;
            if let Some(item) = plan.iterator_item_type(&ty, self.checker.facts())? {
                return self.fixed(item);
            }
        }
        let result = self.fresh()?;
        self.delayed_results.push(result);
        self.iterator_items.push(Pending::Ordinary(source, result));
        Ok(result)
    }

    pub(super) fn solve_iterator_items(&mut self) -> Result<bool, String> {
        let pending = std::mem::take(&mut self.iterator_items);
        let mut progress = false;
        for pending in pending {
            self.checker.infer_work(0)?;
            let (item, result) = match &pending {
                Pending::Ordinary(source, result) => {
                    let item = match self.resolve(*source, 0)? {
                        Some(ty) => self
                            .checker
                            .signatures
                            .plan()?
                            .iterator_item_type(&ty, self.checker.facts())?,
                        None => None,
                    };
                    (item, *result)
                }
                Pending::ArrayMethod {
                    receiver,
                    name,
                    result,
                } => {
                    let Some(ty) = self.resolve(*receiver, 0)? else {
                        self.iterator_items.push(pending);
                        continue;
                    };
                    let plan = self.checker.signatures.plan()?;
                    let selected = plan.select_method_with_arguments(
                        &ty,
                        name,
                        self.checker.facts(),
                        (&self.checker.module, &plan.budget),
                        |_, _, _| {
                            Ok(
                                super::super::method_argument_selection::MethodArguments::Ready(
                                    std::borrow::Cow::Borrowed(&[]),
                                ),
                            )
                        },
                        |_, _| Ok(None),
                    )?;
                    if let Some(selected) = selected {
                        if selected.signature.params.len() != 1 {
                            return Err(self.user_error("E0061: wrong number of method arguments"));
                        }
                        let item = match &selected.signature.result {
                            ir::Ty::Array { element, .. } => Some(element.as_ref().clone()),
                            ir::Ty::Ref { kind, referent } => match referent.as_ref() {
                                ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                                    Some(ir::Ty::Ref {
                                        kind: *kind,
                                        referent: element.clone(),
                                    })
                                }
                                _ => plan.iterator_item_type(
                                    &selected.signature.result,
                                    self.checker.facts(),
                                )?,
                            },
                            ty => plan.iterator_item_type(ty, self.checker.facts())?,
                        };
                        (item, *result)
                    } else {
                        let array = match &ty {
                            ir::Ty::Ref { referent, .. } => referent.as_ref(),
                            ty => ty,
                        };
                        let ir::Ty::Array { element, .. } = array else {
                            return Err(self.user_error("no applicable method iter"));
                        };
                        (
                            Some(ir::Ty::Ref {
                                kind: ir::BorrowKind::Shared,
                                referent: element.clone(),
                            }),
                            *result,
                        )
                    }
                }
            };
            match item {
                Some(item) => {
                    let item = self.fixed(item)?;
                    self.unify(result, item)?;
                    progress = true;
                }
                None => self.iterator_items.push(pending),
            }
        }
        Ok(progress)
    }
}
