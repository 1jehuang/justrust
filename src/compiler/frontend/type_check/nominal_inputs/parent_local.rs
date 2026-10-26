//! Parent-local realization of nominal applications whose ordered actuals hold
//! closure types (`W<{closure}>`, adapter structs).
//!
//! A closure type is final only inside the parent body that defines it, so a
//! graph realization keyed by layout-free shapes cannot name it, and reading the
//! parent's published closure facts from a separate node would wait on the
//! parent itself. Instead the parent realizes such an application from its own
//! finalized `ClosureTy` actuals:
//!
//! * identity is `(parent declaration, application ordinal)`; the ordinal is
//!   assigned in deterministic first-use order inside this checking node, and
//!   each ordinal records the exact ordered actuals it was minted for. The
//!   closure actuals themselves carry `(owner, site, provider, owner_args)`, so
//!   distinct closures (or one closure under different parent arguments) never
//!   share an ordinal;
//! * fields are the canonical source schema with the parent's actuals
//!   substituted structurally. No interner, lock or other node is consulted.
//!
//! The ordinal table is local `RefCell` scratch of one checking computation,
//! never part of a published fact or reachable from another node.
use super::*;

/// Whether any ordered actual structurally holds a closure type.
pub(in crate::compiler::frontend) fn contains_closure(
    types: &[ir::Ty],
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    for ty in types {
        if closure_in(ty, budget, depth)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn closure_in(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    Ok(match ty {
        ir::Ty::Closure(_) => true,
        ir::Ty::Struct(n) if !n.is_back_edge() => {
            contains_closure(&n.def().type_args, budget, depth + 1)?
        }
        ir::Ty::Enum(n) if !n.is_back_edge() => {
            contains_closure(&n.def().type_args, budget, depth + 1)?
        }
        ir::Ty::Tuple(types) => contains_closure(types, budget, depth + 1)?,
        ir::Ty::Ref { referent: t, .. }
        | ir::Ty::RawPtr { pointee: t, .. }
        | ir::Ty::Array { element: t, .. }
        | ir::Ty::Slice { element: t } => closure_in(t, budget, depth + 1)?,
        _ => false,
    })
}

/// One minted application: exact declaration, ordered actuals and the shared
/// definition handed to every later use of the same application.
pub(in crate::compiler::frontend) struct ParentLocalApplication {
    declaration: ir::DeclId,
    actuals: Arc<[ir::Ty]>,
    definition: ir::Ty,
}

impl Checker<'_> {
    pub(in crate::compiler::frontend) fn mint_parent_local_nominal(
        &self,
        declaration: ir::DeclId,
        actuals: &[ir::Ty],
    ) -> Result<ir::Ty, String> {
        let plan = self.signatures.plan()?;
        let budget = &plan.budget;
        let owner = self
            .declaration
            .ok_or("unsupported: closure-bearing nominal outside a parent callable")?;
        {
            let table = self.parent_local_nominals.borrow();
            for entry in table.iter() {
                self.infer_work(0)?;
                if entry.declaration == declaration
                    && entry.actuals.len() == actuals.len()
                    && same_actuals(&entry.actuals, actuals, budget)?
                {
                    return Ok(entry.definition.clone());
                }
            }
        }
        for actual in actuals {
            self.facts()
                .validate_ty_metadata_with_budget(actual, budget)?;
        }
        let (env, template) = plan.nominal_schema(declaration, budget)?;
        if env.arity() != actuals.len() {
            return Err("invalid metadata: parent-local nominal actual arity".into());
        }
        let ordinal = u32::try_from(self.parent_local_nominals.borrow().len())
            .map_err(|_| "parent-local nominal ordinal overflow")?;
        let id = ir::NominalAppId {
            snapshot: match &template {
                ir::Ty::Struct(n) => n.id().snapshot,
                ir::Ty::Enum(n) => n.id().snapshot,
                _ => return Err("invalid metadata: nominal schema is not nominal".into()),
            },
            index: ir::NominalIndex::ParentLocal { owner, ordinal },
        };
        let actuals: Arc<[ir::Ty]> = actuals.into();
        let subst = |ty: &ir::Ty| substitute(ty, &env, &actuals, budget, 0);
        let definition = match &template {
            ir::Ty::Struct(n) => {
                let mut def = n.def().clone();
                def.id = id;
                def.type_args = actuals.clone();
                def.fields = substitute_fields(&def.fields.read(budget)?, &subst)?.into();
                ir::Ty::Struct(ir::StructTy::new_parent_local(def))
            }
            ir::Ty::Enum(n) => {
                let mut def = n.def().clone();
                def.id = id;
                def.type_args = actuals.clone();
                let mut variants = Vec::with_capacity(def.variants.len());
                for variant in def.variants.iter() {
                    let mut variant = variant.clone();
                    variant.fields =
                        substitute_fields(&variant.fields.read(budget)?, &subst)?.into();
                    variants.push(variant);
                }
                def.variants = variants.into();
                ir::Ty::Enum(ir::EnumTy::new_parent_local(def))
            }
            _ => return Err("invalid metadata: nominal schema is not nominal".into()),
        };
        self.infer_storage(1)?;
        self.parent_local_nominals
            .borrow_mut()
            .push(ParentLocalApplication {
                declaration,
                actuals,
                definition: definition.clone(),
            });
        Ok(definition)
    }
}

fn substitute_fields(
    fields: &[ir::FieldDef],
    subst: &dyn Fn(&ir::Ty) -> Result<ir::Ty, String>,
) -> Result<Arc<[ir::FieldDef]>, String> {
    fields
        .iter()
        .map(|field| {
            let mut field = field.clone();
            field.ty = subst(&field.ty)?;
            Ok(field)
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Arc::from)
}

fn same_actuals(a: &[ir::Ty], b: &[ir::Ty], budget: &ir::PlanBudget) -> Result<bool, String> {
    for (a, b) in a.iter().zip(b) {
        budget.charge_work(1)?;
        // Structural Ty equality: closures compare by owner, site, provider,
        // owner arguments and every type; nominals by application identity.
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Structural substitution of the declaration's own parameters. Nested generic
/// nominals in field types need their own realization and are rejected
/// explicitly rather than approximated.
fn substitute(
    ty: &ir::Ty,
    env: &ir::ParamEnv,
    actuals: &[ir::Ty],
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<ir::Ty, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let child = |t: &ir::Ty| substitute(t, env, actuals, budget, depth + 1).map(Arc::new);
    Ok(match ty {
        ir::Ty::Param(p) => {
            if !env.contains(*p) {
                return Err("invalid metadata: foreign parameter in nominal field".into());
            }
            actuals
                .get(usize::from(p.index))
                .cloned()
                .ok_or("invalid metadata: parameter outside nominal actuals")?
        }
        ir::Ty::Ref { kind, referent } => ir::Ty::Ref {
            kind: *kind,
            referent: child(referent)?,
        },
        ir::Ty::RawPtr { kind, pointee } => ir::Ty::RawPtr {
            kind: *kind,
            pointee: child(pointee)?,
        },
        ir::Ty::Array { element, len } => ir::Ty::Array {
            element: child(element)?,
            len: *len,
        },
        ir::Ty::Slice { element } => ir::Ty::Slice {
            element: child(element)?,
        },
        ir::Ty::Tuple(types) => ir::Ty::Tuple(
            types
                .iter()
                .map(|t| substitute(t, env, actuals, budget, depth + 1))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        ),
        ir::Ty::Struct(_)
        | ir::Ty::Enum(_)
        | ir::Ty::AssociatedType(_)
        | ir::Ty::Closure(_)
        | ir::Ty::Opaque(_) => {
            if mentions_param(ty, budget, depth)? {
                return Err(
                    "unsupported: closure-bearing nominal field with a nested generic application"
                        .into(),
                );
            }
            ty.clone()
        }
        other => other.clone(),
    })
}

fn mentions_param(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let any = |types: &[ir::Ty]| -> Result<bool, String> {
        for t in types {
            if mentions_param(t, budget, depth + 1)? {
                return Ok(true);
            }
        }
        Ok(false)
    };
    Ok(match ty {
        ir::Ty::Param(_) | ir::Ty::AssociatedType(_) | ir::Ty::Opaque(_) => true,
        ir::Ty::Closure(c) => any(&c.types().cloned().collect::<Vec<_>>())?,
        ir::Ty::Struct(n) if !n.is_back_edge() => any(&n.def().type_args)?,
        ir::Ty::Enum(n) if !n.is_back_edge() => any(&n.def().type_args)?,
        ir::Ty::Tuple(types) => any(types)?,
        ir::Ty::Ref { referent: t, .. }
        | ir::Ty::RawPtr { pointee: t, .. }
        | ir::Ty::Array { element: t, .. }
        | ir::Ty::Slice { element: t } => mentions_param(t, budget, depth + 1)?,
        _ => false,
    })
}
