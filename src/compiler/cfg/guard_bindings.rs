//! By-value non-Copy bindings under a match guard (stock's guard model).
//!
//! The guard must not move out of the scrutinee: a failed guard leaves it
//! whole for later arms. Each such binding first gets a hidden `&T` local
//! borrowing its selected place. While the guard is lowered, every use of the
//! binding local denotes `*hidden`, so the guard reads through a shared
//! reference. Moving or assigning the binding inside the guard is then a
//! move or write through `&`, which the existing analyses already reject.
//! The real by-value move happens only on the guard's success edge.
use super::*;

/// One binding whose by-value initialization waits for the guard.
pub(super) struct Deferred {
    pub binding: ir::PatternBinding,
    pub selected: Place,
}

impl Lowerer<'_> {
    /// True when a by-value binding of `selected` moves a non-Copy value
    /// under a guard and so must be deferred.
    pub(super) fn guard_defers(&self, selected: &Place, by_value: bool) -> Result<bool, String> {
        if !by_value || !self.guard_deferring {
            return Ok(false);
        }
        Ok(!self.facts.must_copy_scratch(
            &self.result.place_ty_with_budget(selected, self.budget)?,
            &mut self.scratch.borrow_mut(),
        )?)
    }

    /// Bind a hidden shared reference to `selected` and redirect the binding
    /// local to its referent until `end_guard_view`.
    pub(super) fn defer_guard_binding(
        &mut self,
        binding: &ir::PatternBinding,
        selected: Place,
        origin: ExprId,
    ) -> Result<(), String> {
        // A later row of one grouped or-arm shares the first row's view
        // local: the single guard reads whichever row selected.
        let shared = self
            .guard_views
            .iter()
            .find(|(bound, _)| *bound == binding.local)
            .map(|(_, hidden)| *hidden);
        let hidden = match shared {
            Some(hidden) => {
                self.live(hidden, Some(origin));
                hidden
            }
            None => {
                let referent = self.result.place_ty_with_budget(&selected, self.budget)?;
                let hidden = self
                    .temporary(
                        Ty::Ref {
                            kind: ir::BorrowKind::Shared,
                            referent: std::sync::Arc::new(referent),
                        },
                        Some(origin),
                    )
                    .local;
                self.guard_views.push((binding.local, hidden));
                hidden
            }
        };
        self.check_resource()?;
        let view = ir::PatternBinding {
            local: hidden,
            projection: Vec::new(),
            borrow: Some(ir::BorrowKind::Shared),
        };
        self.bind_selected(&view, selected.clone(), origin)?;
        self.guard_deferred.push(Deferred {
            binding: binding.clone(),
            selected,
        });
        Ok(())
    }

    /// The place a source local denotes right now: its guard view, if any.
    pub(super) fn local_place(&self, local: LocalId) -> Place {
        match self
            .guard_views
            .iter()
            .rev()
            .find(|(bound, _)| *bound == local)
        {
            Some((_, hidden)) => Place {
                local: *hidden,
                projection: vec![Projection::Deref],
            },
            None => place(local),
        }
    }

    /// `source` with a guard-viewed root replaced by `*hidden`.
    pub(super) fn guard_place(&self, source: &Place) -> Place {
        let mut viewed = self.local_place(source.local);
        viewed.projection.extend_from_slice(&source.projection);
        viewed
    }

    /// A borrow lowered while a guard view is active was given a loan site
    /// naming the binding local before lowering began. Point that one loan
    /// at the guard view. Outside a guard this changes nothing.
    pub(super) fn retarget_guard_loan(&mut self, loan: LoanId, borrowed: &Place) -> Place {
        if self.guard_views.is_empty() {
            return borrowed.clone();
        }
        let viewed = self.guard_place(borrowed);
        self.result.loan_sites[loan.0].place = viewed.clone();
        viewed
    }

    /// Leave the guard view. Returns the bindings to move on the success edge.
    pub(super) fn end_guard_view(&mut self, views: usize, deferred: usize) -> Vec<Deferred> {
        self.guard_views.truncate(views);
        self.guard_deferred.split_off(deferred)
    }

    /// A grouped or-arm's guard passed: move from the row that selected.
    /// `attempt` holds that row's index and `scratch` is a free `bool`.
    /// The rows joined before the guard, which forgot which variant was
    /// selected, so the chosen row's test runs again to prove its places.
    /// The guard only had shared views, so that test cannot fail.
    pub(super) fn bind_row_moves(
        &mut self,
        alternatives: &super::grouped_or::Alternatives<'_>,
        mut rows: Vec<Vec<Deferred>>,
        (attempt, scratch): (&Place, &Place),
        origin: ExprId,
    ) -> Result<(), String> {
        if rows.iter().all(Vec::is_empty) {
            return Ok(());
        }
        let bound = self.block();
        let last = rows.len() - 1;
        for (index, moves) in rows.drain(..).enumerate() {
            let next = if index == last {
                None
            } else {
                let row = self.block();
                let next = self.block();
                self.check_resource()?;
                self.assign(
                    scratch.clone(),
                    Rvalue::Binary {
                        op: BinaryOp::Eq,
                        left: Operand::Copy(attempt.clone()),
                        right: Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::Usize,
                            bits: index as u128,
                        })),
                    },
                    Some(origin),
                );
                self.terminate(
                    Terminator::Switch {
                        condition: Operand::Copy(scratch.clone()),
                        then_block: row,
                        else_block: next,
                    },
                    Some(origin),
                )?;
                self.current = row;
                Some(next)
            };
            if !moves.is_empty() {
                let tested = self.pattern_test(
                    origin,
                    alternatives.source,
                    &alternatives.rows[index],
                    true,
                )?;
                self.current = tested.rejected;
                self.terminate(Terminator::Unreachable, Some(origin))?;
                self.current = tested.selected;
                for local in tested.temporaries.iter().rev() {
                    self.end_storage(*local, Some(origin));
                }
                self.bind_deferred(moves, origin)?;
            }
            self.terminate(Terminator::Goto(bound), Some(origin))?;
            if let Some(next) = next {
                self.current = next;
            }
        }
        self.current = bound;
        Ok(())
    }

    /// The guard passed: perform the deferred by-value moves.
    pub(super) fn bind_deferred(
        &mut self,
        deferred: Vec<Deferred>,
        origin: ExprId,
    ) -> Result<(), String> {
        for Deferred { binding, selected } in deferred {
            self.bind_selected(&binding, selected, origin)?;
        }
        Ok(())
    }
}
