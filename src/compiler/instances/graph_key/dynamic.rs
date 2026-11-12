//! Full supported trait-object coordinates. Source object construction currently
//! permits only the static/default-static lifetime and no additional auto traits.
//! Keep those restrictions explicit on the wire instead of silently discarding
//! metadata if the supported source domain is extended later.
use super::*;

const STATIC_LIFETIME: u64 = 0;
const EMPTY_AUTO_TRAITS: u64 = 0;

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
    words.extend([STATIC_LIFETIME, EMPTY_AUTO_TRAITS]);
    Ok(())
}

fn supported_markers(reader: &mut Reader<'_>) -> Result<(), String> {
    if reader.word()? != STATIC_LIFETIME {
        return Err("unsupported: dynamic instance key object lifetime transport".into());
    }
    if reader.word()? != EMPTY_AUTO_TRAITS {
        return Err("unsupported: dynamic instance key auto-trait set".into());
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
    supported_markers(reader)
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
            .checked_mul(std::mem::size_of::<(ir::DeclId, ir::Ty)>() * 2)
            .ok_or("dynamic instance key binding allocation overflow")?,
    )?;
    let mut associated_types = Vec::new();
    associated_types
        .try_reserve_exact(count)
        .map_err(|_| "dynamic instance key binding allocation failed")?;
    for _ in 0..count {
        associated_types.push((reader.declaration()?, reader.ty(facts, depth + 1)?));
    }
    supported_markers(reader)?;
    // Reader::ty validates each completed node through callable facts. This
    // authenticates principal arity, associated declaring owners, required
    // equalities, and strictly ordered unique binding identities before use.
    Ok(Arc::new(ir::DynTy {
        principal,
        args: args.into(),
        associated_types: associated_types.into(),
    }))
}
