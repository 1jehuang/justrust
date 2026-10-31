//! Local annotations with nested `_` placeholders (`let v: Vec<_> = ...`).
//! The initializer is checked without an expectation, then the written shape
//! is matched structurally against the inferred type. `_` matches any type.
use super::super::{Namespace, Type, name_scope};
use super::{TypeEnv, checked_local_ty};
use crate::compiler::ir;

/// True when `ty` mentions `_` anywhere below the root.
pub(in crate::compiler::frontend) fn has_nested_infer(ty: &Type) -> bool {
    match ty {
        Type::PhantomData(source) | Type::ManuallyDrop(source) => source
            .argument
            .as_ref()
            .is_some_and(|argument| has_nested_infer(argument)),
        Type::Infer => true,
        Type::Apply(path) => path
            .segments
            .iter()
            .any(|segment| segment.args.iter().any(has_nested_infer)),
        Type::Ref { referent, .. } => has_nested_infer(referent),
        Type::Array(element, _) | Type::Slice(element) => has_nested_infer(element),
        Type::RawPtr { pointee, .. } => has_nested_infer(pointee),
        Type::FnPointer { inputs, output } => {
            inputs.iter().any(has_nested_infer) || has_nested_infer(output)
        }
        Type::Tuple(fields) => fields.iter().any(has_nested_infer),
        _ => false,
    }
}

/// Structural match of a partially written annotation against an inferred
/// type. Nominal heads are compared by resolved declaration identity and arity.
pub(in crate::compiler::frontend) fn matches_partial(
    ty: &Type,
    got: &ir::Ty,
    env: &TypeEnv,
) -> Result<(), String> {
    let mismatch = || Err("E0308: mismatched types: annotation does not match initializer".into());
    if !has_nested_infer(ty) {
        return if checked_local_ty(ty, env)? == *got {
            Ok(())
        } else {
            mismatch()
        };
    }
    match (ty, got) {
        (Type::Infer, _) => Ok(()),
        (Type::PhantomData(source), ir::Ty::PhantomData(marker)) => {
            let plan = env.plan()?;
            marker.validate(&plan.catalog, &plan.budget)?;
            match &source.argument {
                Some(argument) => matches_partial(argument, marker.argument(), env),
                None => mismatch(),
            }
        }
        (Type::ManuallyDrop(source), ir::Ty::ManuallyDrop(marker)) => {
            let plan = env.plan()?;
            marker.validate(&plan.catalog, &plan.budget)?;
            match &source.argument {
                Some(argument) => matches_partial(argument, marker.argument(), env),
                None => mismatch(),
            }
        }
        (
            Type::Ref { kind, referent, .. },
            ir::Ty::Ref {
                kind: k,
                referent: r,
            },
        ) if kind == k => matches_partial(referent, r, env),
        (Type::Array(e, n), ir::Ty::Array { element, len }) if n == len => {
            matches_partial(e, element, env)
        }
        (Type::Slice(e), ir::Ty::Slice { element }) => matches_partial(e, element, env),
        (
            Type::RawPtr { kind, pointee },
            ir::Ty::RawPtr {
                kind: k,
                pointee: p,
            },
        ) if kind == k => matches_partial(pointee, p, env),
        (Type::FnPointer { .. }, ir::Ty::FnPointer(_)) => {
            Err("unsupported: inferred function pointer annotations are not implemented".into())
        }
        (Type::Tuple(fields), ir::Ty::Tuple(got)) if fields.len() == got.len() => {
            for (field, got) in fields.iter().zip(got.iter()) {
                matches_partial(field, got, env)?;
            }
            Ok(())
        }
        (Type::Apply(path), ir::Ty::Struct(_) | ir::Ty::Enum(_)) => {
            let (decl, args) = match got {
                ir::Ty::Struct(s) => (s.def().source_decl, &s.def().type_args),
                ir::Ty::Enum(e) => (e.def().source_decl, &e.def().type_args),
                _ => unreachable!(),
            };
            let Some(last) = path.segments.last() else {
                return mismatch();
            };
            let plan = env.plan()?;
            let registry = &plan.registry;
            let key = name_scope::path_key(
                registry.names.as_ref(),
                &path.path(registry.names.as_ref())?,
                Namespace::Type,
            )?;
            if registry.nominals.get(&key).copied() != Some(decl) || last.args.len() != args.len() {
                return mismatch();
            }
            for (arg, got) in last.args.iter().zip(args.iter()) {
                matches_partial(arg, got, env)?;
            }
            Ok(())
        }
        _ => mismatch(),
    }
}
