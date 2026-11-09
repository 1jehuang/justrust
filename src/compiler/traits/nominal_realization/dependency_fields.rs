//! Candidate dependency metadata, never executable fields or graph read receipts.
use super::*;

impl ValidatedTraitWorld {
    /// Close one authoritative source field without awaiting physical nominal
    /// children. This lets the producer discover the complete closed dependency
    /// graph before it can create a wait cycle. Physical fields are still built
    /// separately from successful typed child reads.
    pub(in crate::compiler) fn close_nominal_dependency_field(
        &self,
        input: &ClosedNominalInput,
        ty: &Ty,
    ) -> Result<Ty> {
        let mut replays = NominalReplays::default();
        self.replay_closed_nominal_input_with(&mut replays, input)?;
        self.close_dependency_field_in(input, ty, &mut replays, 0)
    }

    fn close_dependency_field_in(
        &self,
        input: &ClosedNominalInput,
        ty: &Ty,
        replays: &mut NominalReplays,
        depth: usize,
    ) -> Result<Ty> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let child = |ty: &Ty, replays: &mut NominalReplays| {
            self.close_dependency_field_in(input, ty, replays, depth + 1)
        };
        let mut children = |types: &[Ty]| -> Result<Vec<Ty>> {
            retain::<Ty>(&self.budget, types.len())?;
            let mut result = Vec::new();
            result.try_reserve_exact(types.len()).map_err(|_| {
                FactsError::resource(
                    &self.budget,
                    "nominal dependency metadata allocation failed",
                )
            })?;
            for ty in types {
                result.push(child(ty, replays)?);
            }
            Ok(result)
        };
        let result = match ty {
            Ty::Param(parameter) => {
                let ordinal = input
                    .source
                    .schema()
                    .env
                    .ordered_params()
                    .position(|declared| declared == *parameter)
                    .ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "nominal dependency references a foreign source binder",
                        )
                    })?;
                input.actuals.get(ordinal).cloned().ok_or_else(|| {
                    FactsError::invalid(&self.budget, "nominal dependency actual arity changed")
                })?
            }
            Ty::AssociatedType(projection) => {
                self.normalize_nominal_field_projection_with(replays, input, projection)?
                    .0
            }
            Ty::Struct(nominal) => {
                let actuals = children(&nominal.def().type_args)?;
                self.schema.reapply_nominal(
                    ty,
                    &actuals,
                    &input.source.schema().env,
                    &self.budget,
                )?
            }
            Ty::Enum(nominal) => {
                let actuals = children(&nominal.def().type_args)?;
                self.schema.reapply_nominal(
                    ty,
                    &actuals,
                    &input.source.schema().env,
                    &self.budget,
                )?
            }
            Ty::Tuple(types) => Ty::Tuple(children(types)?.into()),
            Ty::Ref { kind, referent } => Ty::Ref {
                kind: *kind,
                referent: Arc::new(child(referent, replays)?),
            },
            Ty::RawPtr { kind, pointee } => Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(child(pointee, replays)?),
            },
            Ty::Array { element, len } => Ty::Array {
                element: Arc::new(child(element, replays)?),
                len: *len,
            },
            Ty::Slice { element } => Ty::Slice {
                element: Arc::new(child(element, replays)?),
            },
            Ty::Cell(marker) => Ty::Cell(marker.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |ty| child(ty, replays),
            )?),
            Ty::ManuallyDrop(marker) => Ty::ManuallyDrop(marker.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |ty| child(ty, replays),
            )?),
            Ty::PhantomData(marker) => Ty::PhantomData(marker.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |ty| child(ty, replays),
            )?),
            Ty::FnPointer(pointer) => Ty::FnPointer(pointer.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |ty| child(ty, replays),
            )?),
            _ => ty.clone(),
        };
        self.validate_ty_metadata_with(&result, &self.empty.env, depth, replays)?;
        Ok(result)
    }
}
