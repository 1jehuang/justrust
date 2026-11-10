//! Type outlives checks for metadata which has no runtime borrow carrier.
use super::*;

/// Runtime origin checks cannot prove lifetimes hidden by raw pointers,
/// PhantomData, or function signatures. Admit those only when their metadata
/// is independently borrow-free (or a declaration-proven static parameter).
/// Reference lifetimes erased from this metadata require an explicit diagnostic.
pub(in crate::compiler) fn check_static_bound_metadata(
    root: &Ty,
    env: &GenericEnv,
    budget: &PlanBudget,
) -> Result<(), String> {
    fn visit(
        ty: &Ty,
        tracked: bool,
        env: &GenericEnv,
        budget: &PlanBudget,
        depth: usize,
        seen: &mut Vec<(u8, usize, bool)>,
    ) -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        let mut child = |ty: &Ty, tracked| visit(ty, tracked, env, budget, depth + 1, seen);
        match ty {
            Ty::Ref { referent, .. } => {
                if !tracked {
                    return Err("unsupported: proving `'static` for erased reference metadata requires source lifetime transport".into());
                }
                child(referent, true)?;
            }
            Ty::Param(parameter) if !tracked && !env.outlives_static(*parameter) => {
                return Err("type parameter in erased metadata does not satisfy its declared `'static` bound".into());
            }
            Ty::AssociatedType(_) | Ty::Opaque(_) if !tracked => {
                return Err(
                    "unsupported: proving `'static` for erased projection or opaque metadata"
                        .into(),
                );
            }
            Ty::RawPtr { pointee, .. } => child(pointee, false)?,
            Ty::PhantomData(value) => child(value.argument(), false)?,
            Ty::Cell(value) => child(value.argument(), false)?,
            Ty::ManuallyDrop(value) => child(value.argument(), tracked)?,
            Ty::FnPointer(value) => {
                for ty in value.types() {
                    child(ty, false)?;
                }
            }
            Ty::FnItem(value) => {
                for ty in value.type_args.iter() {
                    child(ty, false)?;
                }
            }
            Ty::Dyn(value) => {
                for ty in value.types() {
                    child(ty, false)?;
                }
            }
            Ty::Closure(value) => {
                for ty in value.captures() {
                    child(ty, tracked)?;
                }
            }
            Ty::Tuple(fields) => {
                for ty in fields.iter() {
                    child(ty, tracked)?;
                }
            }
            Ty::Array { element, .. }
            | Ty::DependentArray { element, .. }
            | Ty::Slice { element } => child(element, tracked)?,
            Ty::Struct(value) => {
                let key = (0, std::ptr::from_ref(value.def()) as usize, tracked);
                budget.charge_work(seen.len())?;
                if seen.contains(&key) {
                    return Ok(());
                }
                budget.retain_bytes(std::mem::size_of_val(&key))?;
                seen.push(key);
                if value.is_back_edge() {
                    return Err(
                        "unsupported: proving `'static` through recursive erased metadata".into(),
                    );
                }
                for field in value.def().fields.read(budget)?.iter() {
                    visit(&field.ty, tracked, env, budget, depth + 1, seen)?;
                }
            }
            Ty::Enum(value) => {
                let key = (1, std::ptr::from_ref(value.def()) as usize, tracked);
                budget.charge_work(seen.len())?;
                if seen.contains(&key) {
                    return Ok(());
                }
                budget.retain_bytes(std::mem::size_of_val(&key))?;
                seen.push(key);
                if value.is_back_edge() {
                    return Err(
                        "unsupported: proving `'static` through recursive erased metadata".into(),
                    );
                }
                for variant in value.def().variants.iter() {
                    for field in variant.fields.read(budget)?.iter() {
                        visit(&field.ty, tracked, env, budget, depth + 1, seen)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(root, true, env, budget, 0, &mut Vec::new())
}
