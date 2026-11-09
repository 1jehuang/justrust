//! Nominal field types may only name lifetimes declared by their struct or
//! enum (E0261), independent of whether the type is ever used.
use super::*;

pub(super) fn check_fields(
    names: &NameStore,
    declared: &[AstIdent],
    fields: &[FieldDecl],
    mut work: impl FnMut(usize) -> Result<(), String>,
) -> Result<(), String> {
    let mut keys = Vec::with_capacity(declared.len());
    for name in declared {
        keys.push(
            names
                .key(*name, LookupClass::LifetimeParameter)
                .map_err(native_adapter::name_diagnostic)?,
        );
    }
    let mut stack: Vec<&Type> = fields.iter().map(|field| &field.ty).collect();
    let region = |value: &RegionName| -> Result<(), String> {
        if let RegionName::Named(name) = value {
            let key = names
                .key(*name, LookupClass::LifetimeParameter)
                .map_err(native_adapter::name_diagnostic)?;
            if !keys.contains(&key) {
                return Err("E0261: use of undeclared lifetime name".into());
            }
        }
        Ok(())
    };
    while let Some(ty) = stack.pop() {
        work(1)?;
        match ty {
            Type::Ref {
                region: lifetime,
                referent,
                ..
            } => {
                if let Some(lifetime) = lifetime {
                    region(lifetime)?;
                }
                stack.push(referent);
            }
            Type::Apply(path) => {
                for segment in &path.segments {
                    for lifetime in &segment.region_args {
                        region(lifetime)?;
                    }
                    for argument in &segment.args {
                        stack.push(argument);
                    }
                }
            }
            Type::Array(element, _)
            | Type::Slice(element)
            | Type::DependentArray(element, _)
            | Type::RawPtr {
                pointee: element, ..
            } => stack.push(element),
            Type::Tuple(elements) => {
                for element in elements {
                    stack.push(element);
                }
            }
            // Binder-introducing or catalog types are checked elsewhere.
            _ => {}
        }
    }
    Ok(())
}
