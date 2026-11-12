//! Preserve exact closure capture coordinates through ordinary typed aggregates.
use super::*;

fn child(
    opaque: &OpaqueTy,
    value: &Ty,
    step: CarrierStep,
    locations: &[CarrierLocation],
    next: &mut usize,
    budget: &PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let sources = pack_capture_sources_at(opaque, value, budget, depth)?;
    for source in sources.iter() {
        budget.charge_work(
            source
                .path
                .0
                .len()
                .checked_add(1)
                .ok_or("opaque aggregate path extent overflow")?,
        )?;
        let actual = locations
            .get(*next)
            .ok_or("opaque aggregate omitted a captured carrier")?;
        let Some((actual_step, tail)) = actual.path.0.split_first() else {
            return Err("opaque aggregate changed its captured field path".into());
        };
        if actual_step != &step
            || tail != source.path.0.as_ref()
            || actual.channel != source.channel
        {
            return Err("opaque aggregate changed its exact captured carrier coordinate".into());
        }
        *next = next
            .checked_add(1)
            .ok_or("opaque aggregate carrier count overflow")?;
    }
    Ok(())
}

pub(super) fn validate(
    opaque: &OpaqueTy,
    hidden: &Ty,
    locations: &[CarrierLocation],
    budget: &PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    let mut next = 0;
    let child_depth = depth
        .checked_add(1)
        .ok_or("opaque aggregate depth overflow")?;
    let mut field = |step, value: &Ty| {
        child(
            opaque,
            value,
            step,
            locations,
            &mut next,
            budget,
            child_depth,
        )
    };
    match hidden {
        Ty::Tuple(fields) => {
            for (index, value) in fields.iter().enumerate() {
                field(CarrierStep::Field(index), value)?;
            }
        }
        Ty::Struct(value) => {
            if let Some(element) = hidden.owned_sequence_element() {
                field(CarrierStep::SliceElement, element)?;
            }
            for (index, value) in value.def().fields.read(budget)?.iter().enumerate() {
                field(CarrierStep::Field(index), &value.ty)?;
            }
        }
        Ty::Enum(value) => {
            for (variant, definition) in value.def().variants.iter().enumerate() {
                for (index, value_field) in definition.fields.read(budget)?.iter().enumerate() {
                    field(
                        CarrierStep::VariantField {
                            variant: VariantId {
                                owner: value.def().id,
                                index: variant,
                            },
                            field: index,
                        },
                        &value_field.ty,
                    )?;
                }
            }
        }
        // Scalar elements carry no reference positions.
        Ty::Array { element, .. }
            if matches!(
                element.as_ref(),
                Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit
            ) => {}
        Ty::Array { element, len } => {
            if *len > MAX_ARRAY_LEN {
                return Err("array carrier resource limit exceeded".into());
            }
            for index in 0..*len {
                field(CarrierStep::ConstantIndex(index), element)?;
            }
        }
        _ => return Ok(false),
    }
    if next != locations.len() {
        return Err("opaque aggregate retained an unauthenticated carrier".into());
    }
    Ok(true)
}
