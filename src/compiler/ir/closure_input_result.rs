//! Exact value reads from a closure's checked argument tuple.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClosureInputValueOrigin {
    pub(in crate::compiler) argument: usize,
    pub(in crate::compiler) tuple: LocalId,
    pub(in crate::compiler) local: LocalId,
    /// A checked read, not a borrow of storage owned by the closure.
    pub(in crate::compiler) value: ExprId,
    /// Projection from the original argument, after cancelling a pattern
    /// binding's shared borrow against its explicit dereference in the read.
    pub(in crate::compiler) projection: Arc<[Projection]>,
}

impl ClosureInputValueOrigin {
    pub(in crate::compiler) fn projected_type(
        &self,
        parameters: &[Ty],
        budget: &PlanBudget,
    ) -> Result<Ty, String> {
        let mut ty = parameters
            .get(self.argument)
            .ok_or("closure input result names a foreign argument")?
            .clone();
        for step in self.projection.iter() {
            budget.charge_work(1)?;
            ty = match (step, &ty) {
                (
                    Projection::Deref,
                    Ty::Ref {
                        kind: BorrowKind::Shared,
                        referent,
                    },
                ) => referent.as_ref().clone(),
                (Projection::Field(index), _) => ty
                    .field_ty(*index, budget)?
                    .ok_or("closure input result has an invalid field")?,
                (Projection::ConstantIndex(index), Ty::Array { element, len }) if index < len => {
                    element.as_ref().clone()
                }
                _ => return Err("unsupported: closure input result value projection".into()),
            };
        }
        Ok(ty)
    }

    pub(in crate::compiler) fn value_carriers(
        &self,
        parameters: &[Ty],
        budget: &PlanBudget,
    ) -> Result<Arc<[CarrierReturn]>, String> {
        let result = self.projected_type(parameters, budget)?;
        let inputs = carrier_locations(&parameters[self.argument], budget)?;
        let locations = carrier_locations(&result, budget)?;
        budget.retain_bytes(locations.len() * std::mem::size_of::<CarrierReturn>())?;
        let mut returned = Vec::new();
        for location in locations.iter() {
            let mut source = location.clone();
            for step in self.projection.iter().rev() {
                budget.charge_work(1)?;
                match step {
                    Projection::Field(index) => {
                        source.path = prepend(CarrierStep::Field(*index), &source.path, budget)?;
                    }
                    Projection::ConstantIndex(index) => {
                        source.path =
                            prepend(CarrierStep::ConstantIndex(*index), &source.path, budget)?;
                    }
                    Projection::Deref if !source.path.0.is_empty() => {
                        source.path = prepend(CarrierStep::Referent, &source.path, budget)?;
                    }
                    Projection::Deref => {
                        let next = source
                            .channel
                            .prefix()
                            .checked_add(1)
                            .ok_or("closure input result reference depth overflow")?;
                        match &mut source.channel {
                            CarrierChannel::Fixed(position) => position.depth = next,
                            CarrierChannel::ParamTail { prefix, .. }
                            | CarrierChannel::ProjectionTail { prefix, .. }
                            | CarrierChannel::OpaqueTail { prefix, .. } => *prefix = next,
                        }
                    }
                    _ => return Err("unsupported: closure input result carrier projection".into()),
                }
            }
            budget.charge_work(inputs.len())?;
            if !inputs.contains(&source) {
                return Err(
                    "closure input result is outside its original argument carrier domain".into(),
                );
            }
            source.path = prepend(CarrierStep::Field(self.argument), &source.path, budget)?;
            budget.retain_bytes(
                std::mem::size_of::<InputCarrier>() + 4 * std::mem::size_of::<usize>(),
            )?;
            returned.push(CarrierReturn {
                location: location.clone(),
                sources: Arc::from([InputCarrier {
                    param: 1,
                    location: source,
                }]),
            });
        }
        Ok(returned.into())
    }
}

fn prepend(
    step: CarrierStep,
    path: &CarrierPath,
    budget: &PlanBudget,
) -> Result<CarrierPath, String> {
    budget.charge_work(path.0.len())?;
    budget.retain_bytes((path.0.len() + 1) * std::mem::size_of::<CarrierStep>())?;
    let mut parts = Vec::with_capacity(path.0.len() + 1);
    parts.push(step);
    parts.extend_from_slice(&path.0);
    Ok(CarrierPath(parts.into()))
}
