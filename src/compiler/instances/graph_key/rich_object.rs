//! Canonical complete trait-object coordinates for instance graph keys.
use super::*;

pub(super) fn encode(
    words: &mut Vec<u64>,
    object: &ir::DynTy,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    declaration(words, object.principal);
    types(words, &object.args, budget, depth + 1)?;
    budget.charge_work(object.associated_types.len())?;
    words.push(object.associated_types.len() as u64);
    for (item, value) in object.associated_types.iter() {
        declaration(words, *item);
        ty(words, value, budget, depth + 1)?;
    }
    // DynTy currently admits only the default/static object lifetime and no
    // additional auto traits. Reserve explicit coordinates, never omit them.
    words.extend([0, 0]);
    Ok(())
}

fn markers(reader: &mut Reader<'_>) -> Result<(), String> {
    if reader.word()? != 0 || reader.word()? != 0 {
        return Err("unsupported: instance graph key object lifetime or auto traits".into());
    }
    Ok(())
}

pub(super) fn children(
    reader: &mut Reader<'_>,
    depth: usize,
    children: &mut Vec<(usize, usize)>,
) -> Result<(), String> {
    reader.skip_declaration()?;
    reader.child_list(depth + 1, children)?;
    let count = reader.count()?;
    reader.budget.charge_work(count)?;
    for _ in 0..count {
        reader.skip_declaration()?;
        reader.child_frame(depth + 1, children)?;
    }
    markers(reader)
}

pub(super) fn decode(
    reader: &mut Reader<'_>,
    facts: ir::FactsCtx<'_>,
    depth: usize,
) -> Result<Arc<ir::DynTy>, String> {
    reader.budget.check_depth(depth)?;
    let principal = reader.declaration()?;
    let args = reader.types(facts, depth + 1)?;
    let count = reader.count()?;
    reader.budget.charge_work(count)?;
    reader.budget.retain_bytes(
        count
            .checked_mul(2 * std::mem::size_of::<(ir::DeclId, ir::Ty)>())
            .ok_or("instance graph object bindings storage overflow")?,
    )?;
    let mut bindings = Vec::new();
    bindings
        .try_reserve_exact(count)
        .map_err(|_| "instance graph object bindings allocation failed")?;
    for _ in 0..count {
        bindings.push((reader.declaration()?, reader.ty(facts, depth + 1)?));
    }
    markers(reader)?;
    // Reader::ty applies the existing facts metadata validation to this node:
    // principal arity, associated item ownership and unique canonical order.
    Ok(Arc::new(ir::DynTy {
        principal,
        args: args.into(),
        associated_types: bindings.into(),
    }))
}
