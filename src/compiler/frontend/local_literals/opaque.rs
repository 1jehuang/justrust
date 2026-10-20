//! Deferred opaque metadata substitution for body-local inference. The retained
//! descriptor supplies identity only. Its children remain ordinary solver terms
//! until caller arguments resolve, then normal checking authenticates the result.
use super::*;

impl Solver<'_, '_, '_> {
    pub(super) fn opaque_identity_matches(
        &self,
        left: &ir::OpaqueTy,
        right: &ir::OpaqueTy,
        depth: usize,
    ) -> Result<bool, String> {
        for _ in left
            .captured_regions()
            .iter()
            .chain(right.captured_regions())
        {
            self.checker.infer_work(depth)?;
        }
        for bound in left.bounds().iter().chain(right.bounds()) {
            self.checker.infer_work(depth)?;
            for _ in bound.args().iter() {
                self.checker.infer_work(depth)?;
            }
            for _ in bound.equalities() {
                self.checker.infer_work(depth)?;
            }
        }
        left.try_eq(right, |_, _| {
            self.checker.infer_work(depth)?;
            Ok(true)
        })
    }

    pub(super) fn opaque_schema(
        &mut self,
        descriptor: &Arc<ir::OpaqueTy>,
        params: &HashMap<ir::ParamId, Term>,
        depth: usize,
    ) -> Result<Term, String> {
        self.checker.infer_storage(descriptor.types().count())?;
        let children = descriptor
            .types()
            .map(|ty| self.schema(ty, params, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        self.node(Domain::Opaque(descriptor.clone(), children))
    }

    pub(super) fn resolve_opaque(
        &self,
        descriptor: &Arc<ir::OpaqueTy>,
        children: &[Term],
        depth: usize,
    ) -> Result<Option<ir::Ty>, String> {
        self.checker.infer_storage(children.len())?;
        let mut actuals = Vec::with_capacity(children.len());
        for child in children {
            let Some(actual) = self.resolve(*child, depth + 1)? else {
                return Ok(None);
            };
            actuals.push(actual);
        }
        let mut actuals = actuals.into_iter();
        let mapped = descriptor.try_map(|_| {
            self.checker.infer_work(depth + 1)?;
            actuals
                .next()
                .ok_or_else(|| "opaque inference child count changed".to_string())
        })?;
        if actuals.next().is_some() {
            return Err("opaque inference retained extra children".into());
        }
        Ok(Some(ir::Ty::Opaque(Arc::new(mapped))))
    }
}
