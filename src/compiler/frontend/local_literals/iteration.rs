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
                    // A nominal receiver (for example `Vec<T>`) has an
                    // ordinary declared `iter` method: solve it as a method
                    // call and project the result's `Iterator::Item`.
                    let owner = self.peel_refs(receiver);
                    if let Some(candidate) = self.candidate_for_receiver(owner, name)? {
                        let iterator = self.callable(candidate, &[], Some(owner), None, &[])?;
                        return self.iterator_item(iterator);
                    }
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
        if let Some(item) = self.resolved_iterator_item(source)? {
            return self.fixed(item);
        }
        let result = self.fresh()?;
        self.delayed_results.push(result);
        self.iterator_items.push(Pending::Ordinary(source, result));
        Ok(result)
    }

    fn resolved_iterator_item(&self, source: Term) -> Result<Option<ir::Ty>, String> {
        let Some(ty) = self.resolve(source, 0)? else {
            return Ok(None);
        };
        let plan = self.checker.signatures.plan()?;
        let item = plan.iterator_item_type(&ty, self.checker.facts())?;
        // A resolved, non-closure source with finalized numeric choices has
        // all its graph inputs. The canonical lookup's None is then an exact
        // failed IntoIterator obligation, not an inference dependency.
        // Unknown terms, provisional defaults, closure-owned metadata and an
        // absent native trait remain pending rather than being reclassified.
        if item.is_none()
            && !self.contains_callable(source, 0)?
            && self.manifest_ready(&[source], 0)?
            && plan.into_iterator_trait_id()?.is_some()
        {
            return Err(self.user_error(
                "E0277: loop source does not satisfy the IntoIterator implementation bounds",
            ));
        }
        Ok(item)
    }

    pub(super) fn solve_iterator_items(&mut self) -> Result<bool, String> {
        let pending = std::mem::take(&mut self.iterator_items);
        let mut progress = false;
        for pending in pending {
            self.checker.infer_work(0)?;
            let (item, result) = match &pending {
                Pending::Ordinary(source, result) => {
                    let item = self.resolved_iterator_item(*source)?;
                    (item, *result)
                }
                Pending::ArrayMethod {
                    receiver,
                    name,
                    result,
                } => {
                    let Some(mut ty) = self.resolve(*receiver, 0)? else {
                        self.iterator_items.push(pending);
                        continue;
                    };
                    let plan = self.checker.signatures.plan()?;
                    // Method autoderef through overloaded Deref (`guard.iter()`
                    // on `Ref<Vec<T>>`): step to the Deref target while the
                    // receiver nominal itself has no such method.
                    for _ in 0..8 {
                        let owner = match &ty {
                            ir::Ty::Ref { referent, .. } => referent.as_ref().clone(),
                            other => other.clone(),
                        };
                        if !matches!(owner, ir::Ty::Struct(_) | ir::Ty::Enum(_))
                            || owner.owned_box_pointee().is_some()
                        {
                            break;
                        }
                        if plan
                            .select_method_with_arguments(
                                &owner,
                                name,
                                self.checker.facts(),
                                (&self.checker.module, &plan.budget),
                                |_, _, _| {
                                    Ok(super::super::method_argument_selection::MethodArguments::Ready(
                                        std::borrow::Cow::Borrowed(&[]),
                                    ))
                                },
                                |_, _| Ok(None),
                            )
                            .ok()
                            .flatten()
                            .is_some()
                        {
                            break;
                        }
                        let Some(target) = self.checker.deref_target_hint(&owner)? else {
                            break;
                        };
                        ty = ir::Ty::Ref {
                            kind: ir::BorrowKind::Shared,
                            referent: Arc::new(target),
                        };
                    }
                    let selected = match plan.select_method_with_arguments(
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
                        // Realize graph nominals exactly as ordinary method
                        // hints do, so `v.iter()` on a realized `Vec<T>`
                        // compares against the same receiver representation.
                        |target, signature| {
                            let checker = self.checker;
                            if checker.nominal_inputs.is_none() {
                                return Ok(None);
                            }
                            match checker.nominal_call_signature(target, signature.clone())? {
                                super::super::nominal_calls::NominalCallSignature::Ready {
                                    signature,
                                    ..
                                } => Ok(Some(signature)),
                                super::super::nominal_calls::NominalCallSignature::Blocked(
                                    blocked,
                                ) => {
                                    *checker.nominal_blocked.borrow_mut() = Some(blocked);
                                    Err("missing graph input: canonical method signature realization"
                                        .into())
                                }
                            }
                        },
                    ) {
                        // Without nominal inputs this solver cannot realize a
                        // graph nominal receiver's declaration signature. It
                        // only supplies inference evidence, so it contributes
                        // none here and the ordinary loop checker decides.
                        Err(error)
                            if self.checker.nominal_inputs.is_none()
                                && error
                                    .starts_with("canonical method receiver formal mismatch") =>
                        {
                            self.iterator_items.push(pending);
                            continue;
                        }
                        other => other?,
                    };
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
