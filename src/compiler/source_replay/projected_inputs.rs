use super::*;

type PendingInput = (InferTerm, InferTerm);

impl CallInferenceState {
    /// Establish only injective source-input equations. Projection inputs are
    /// retained, never inverted to infer their owning generic arguments.
    pub(in crate::compiler) fn constrain_input_frontier(
        &mut self,
        parameter: &ir::Ty,
        input: &ir::Ty,
    ) -> Result<Vec<PendingInput>, FactsError> {
        let schema = self.type_term(parameter, self.env.as_ref().map(|env| env.binder()), 0)?;
        let evidence = self.type_term(input, None, 0)?;
        let (schema, evidence, depth) = call_input_equation(&schema, &evidence);
        let mut pending = Vec::new();
        self.split_input_projection(schema, evidence, &mut pending, depth)?;
        Ok(pending)
    }

    fn split_input_projection(
        &mut self,
        schema: &InferTerm,
        evidence: &InferTerm,
        pending: &mut Vec<PendingInput>,
        depth: usize,
    ) -> Result<(), FactsError> {
        self.work(depth)?;
        match (schema.as_ref(), evidence.as_ref()) {
            (InferKind::Projection(..), InferKind::Unknown(_)) => Ok(()),
            (InferKind::Projection(..), _) => {
                push(pending, (schema.clone(), evidence.clone()), &self.budget)
            }
            (InferKind::Tuple(a), InferKind::Tuple(b)) if a.len() == b.len() => {
                for (a, b) in a.iter().zip(b.iter()) {
                    self.split_input_projection(a, b, pending, depth + 1)?;
                }
                Ok(())
            }
            (InferKind::Nominal(d, a), InferKind::Nominal(e, b))
                if d == e && a.len() == b.len() =>
            {
                for (a, b) in a.iter().zip(b.iter()) {
                    self.split_input_projection(a, b, pending, depth + 1)?;
                }
                Ok(())
            }
            (InferKind::Opaque(d, a), InferKind::Opaque(e, b))
                if a.len() == b.len() && opaque_shape_equal(d, e, &self.budget, depth + 1)? =>
            {
                for (a, b) in a.iter().zip(b.iter()) {
                    self.split_input_projection(a, b, pending, depth + 1)?;
                }
                Ok(())
            }
            (InferKind::FnPointer(a, x), InferKind::FnPointer(b, y)) if a.len() == b.len() => {
                for (a, b) in a.iter().zip(b.iter()) {
                    self.split_input_projection(a, b, pending, depth + 1)?;
                }
                self.split_input_projection(x, y, pending, depth + 1)
            }
            (InferKind::Ref(k, a), InferKind::Ref(l, b))
            | (InferKind::RawPtr(k, a), InferKind::RawPtr(l, b))
                if k == l =>
            {
                self.split_input_projection(a, b, pending, depth + 1)
            }
            (InferKind::Cell(d, a), InferKind::Cell(e, b))
            | (InferKind::ManuallyDrop(d, a), InferKind::ManuallyDrop(e, b))
            | (InferKind::PhantomData(d, a), InferKind::PhantomData(e, b))
                if d == e =>
            {
                self.split_input_projection(a, b, pending, depth + 1)
            }
            (InferKind::Array(a, n), InferKind::Array(b, m)) if n == m => {
                self.split_input_projection(a, b, pending, depth + 1)
            }
            (InferKind::DependentArray(a, n), InferKind::Array(b, m)) => {
                let extent = self.node(InferKind::Rigid(ir::Ty::Const(
                    ir::ConstArgument::Integer {
                        ty: ir::IntTy::Usize,
                        bits: *m as u128,
                    },
                )))?;
                self.constrain(n, &extent, depth + 1)?;
                self.split_input_projection(a, b, pending, depth + 1)
            }
            (InferKind::DependentArray(a, n), InferKind::DependentArray(b, m)) => {
                self.split_input_projection(a, b, pending, depth + 1)?;
                self.split_input_projection(n, m, pending, depth + 1)
            }
            (InferKind::Slice(a), InferKind::Slice(b)) => {
                self.split_input_projection(a, b, pending, depth + 1)
            }
            _ => self.constrain(schema, evidence, depth + 1),
        }
    }

    pub(super) fn validate_input_projections(
        &mut self,
        pending: Vec<PendingInput>,
        materialize: &mut impl FnMut(ir::DeclId, &[ir::Ty]) -> Result<ir::Ty, FactsError>,
        normalize: &mut dyn FnMut(&ir::Ty) -> Result<ir::Ty, FactsError>,
    ) -> Result<Vec<ir::Ty>, FactsError> {
        self.storage(pending.len())?;
        let mut normals = Vec::new();
        for (schema, evidence) in pending {
            let bound = self
                .resolve_bound_schema(&schema, 0, materialize)?
                .ok_or_else(|| {
                    FactsError::unsupported(
                        &self.budget,
                        "source projection inputs remain unresolved",
                    )
                })?;
            let normal = normalize(&bound)?;
            let term = self.type_term(&normal, None, 0)?;
            self.constrain(&term, &evidence, 0)?;
            push(&mut normals, normal, &self.budget)?;
        }
        Ok(normals)
    }
}
