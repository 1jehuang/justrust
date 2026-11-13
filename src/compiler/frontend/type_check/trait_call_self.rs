//! `Trait::f(args)` for an associated function without `self`: stock Rust
//! infers `Self` from the call's expected type, not from the first argument
//! (`fn decode<T: AsRef<[u8]>>(d: T) -> Result<Vec<u8>, E> { FromHex::from_hex(d) }`).
use super::*;

impl Checker<'_> {
    /// Self read from the expected result when the trait item has no
    /// receiver and its declared result mentions `Self` structurally.
    /// This is only a hint: checked selection still proves the application.
    pub(super) fn trait_call_self_from_expected(
        &self,
        trait_path: &TraitPath,
        name: &AstIdent,
        expected: Option<&ir::Ty>,
    ) -> Result<Option<ir::Ty>, String> {
        let Some(expected) = expected else {
            return Ok(None);
        };
        let plan = self.signatures.plan()?;
        let Ok(trait_id) = plan.resolve_trait_path(trait_path) else {
            return Ok(None);
        };
        let Some(item) = plan.trait_item(trait_id, name, &plan.budget)? else {
            return Ok(None);
        };
        if plan.declaration(item)?.receiver != ReceiverKind::None {
            return Ok(None);
        }
        let self_param = ir::ParamId {
            owner: plan.param_env(trait_id)?.binder(),
            index: 0,
        };
        let schema = plan.raw_schema(item)?;
        let mut found = None;
        if !self_from(
            &schema.signature.result,
            expected,
            self_param,
            &mut found,
            0,
        )? {
            return Ok(None);
        }
        Ok(found)
    }
}

/// Matches `formal` against `actual`, recording Self. Returns false on a
/// structural disagreement. Unknown formal parts (other parameters and
/// projections) match anything.
fn self_from(
    formal: &ir::Ty,
    actual: &ir::Ty,
    self_param: ir::ParamId,
    found: &mut Option<ir::Ty>,
    depth: usize,
) -> Result<bool, String> {
    if depth > 64 {
        return Ok(false);
    }
    match (formal, actual) {
        (ir::Ty::Param(p), _) if *p == self_param => {
            if found.as_ref().is_some_and(|f| f != actual) {
                return Ok(false);
            }
            *found = Some(actual.clone());
            Ok(true)
        }
        (ir::Ty::Param(_) | ir::Ty::AssociatedType(_), _) => Ok(true),
        (
            ir::Ty::Ref {
                kind: a,
                referent: x,
            },
            ir::Ty::Ref {
                kind: b,
                referent: y,
            },
        ) if a == b => self_from(x, y, self_param, found, depth + 1),
        (ir::Ty::Tuple(xs), ir::Ty::Tuple(ys)) if xs.len() == ys.len() => {
            for (x, y) in xs.iter().zip(ys.iter()) {
                if !self_from(x, y, self_param, found, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ir::Ty::Slice { element: x }, ir::Ty::Slice { element: y }) => {
            self_from(x, y, self_param, found, depth + 1)
        }
        _ => match (nominal_parts(formal), nominal_parts(actual)) {
            (Some((a, xs)), Some((b, ys))) if a == b && xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys.iter()) {
                    if !self_from(x, y, self_param, found, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Some(_), _) | (_, Some(_)) => Ok(false),
            _ => Ok(true),
        },
    }
}
