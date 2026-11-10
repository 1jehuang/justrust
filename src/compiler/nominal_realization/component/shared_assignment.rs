use super::*;

struct SharedAssignmentInventory<'a, F> {
    read: &'a ReadComponent,
    budget: &'a ir::PlanBudget,
    queued: Vec<bool>,
    pending: Vec<usize>,
    external: &'a mut F,
}

impl<F> SharedAssignmentInventory<'_, F>
where
    F: FnMut(&ir::Ty, usize) -> Result<bool, Error>,
{
    fn term(&mut self, term: &Term, depth: usize) -> Result<bool, Error> {
        let b = self.budget;
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        match term {
            Term::Ref(ir::BorrowKind::Shared, child) | Term::Array(child, _) => {
                self.term(child, depth + 1)
            }
            Term::Ref(ir::BorrowKind::Mut, _)
            | Term::Slice(_)
            | Term::Dyn(..)
            | Term::Closure(..) => Ok(false),
            Term::Tuple(children) | Term::Nominal(_, children) => {
                for child in children {
                    if !self.term(child, depth + 1)? {
                        return Ok(false);
                    }
                }
                if let Term::Nominal(declaration, actuals) = term {
                    let key = producer::key(self.read.world_word(), *declaration, actuals, b)?;
                    let mut target = None;
                    for (index, member) in self.read.fact.members.iter().enumerate() {
                        b.charge_work(1).map_err(Error::new)?;
                        if member.key == key {
                            target = Some(index);
                            break;
                        }
                    }
                    if let Some(index) = target {
                        if !self.queued[index] {
                            self.queued[index] = true;
                            self.pending.push(index);
                        }
                    } else {
                        b.charge_work(self.read.fact.external.len())
                            .map_err(Error::new)?;
                        let read = self.read.external(&key).ok_or_else(|| {
                            Error::new("shared assignment inventory omitted nominal dependency")
                        })?;
                        let ty = read.definition()?;
                        read.check_definition(&ty)?;
                        return (self.external)(&ty, depth);
                    }
                }
                Ok(true)
            }
            // Match the ordinary predicate: raw pointees and sealed scalar
            // wrappers have no nominal type_arguments or by-value payload walk.
            Term::RawPtr(..)
            | Term::Cell(..)
            | Term::ManuallyDrop(..)
            | Term::PhantomData(..)
            | Term::FnPointer(..)
            | Term::FnItem(..)
            | Term::Str
            | Term::Int(_)
            | Term::Float(_)
            | Term::Bool
            | Term::Unit
            | Term::Never
            | Term::Const(_)
            | Term::Char => Ok(true),
        }
    }
}

impl ReadComponent {
    pub(in crate::compiler::nominal_realization) fn shared_assignment_payload(
        &self,
        member: u32,
        b: &ir::PlanBudget,
        depth: usize,
        external: &mut impl FnMut(&ir::Ty, usize) -> Result<bool, Error>,
    ) -> Result<bool, Error> {
        b.check_depth(depth).map_err(Error::new)?;
        let root =
            usize::try_from(member).map_err(|_| Error::new("component member ordinal overflow"))?;
        if root >= self.fact.members.len() {
            return Err(Error::new("component member ordinal out of bounds"));
        }
        let mut queued = topology::storage(self.fact.members.len(), b)?;
        queued.resize(self.fact.members.len(), false);
        queued[root] = true;
        let mut pending = topology::storage(self.fact.members.len(), b)?;
        pending.push(root);
        let mut inventory = SharedAssignmentInventory {
            read: self,
            budget: b,
            queued,
            pending,
            external,
        };
        // The property is a conjunction over the finite reachable equation
        // graph. A queued back edge contributes no provisional answer: all
        // reached fields and ordered actuals must finish before true escapes.
        while let Some(index) = inventory.pending.pop() {
            b.charge_work(1).map_err(Error::new)?;
            let member = &self.fact.members[index];
            for term in member
                .actuals
                .iter()
                .chain(member.fields.iter().map(|(_, term)| term))
            {
                if !inventory.term(term, depth + 1)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
