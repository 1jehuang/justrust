//! Canonical, authority-bound normal-path destruction for the bounded scalar-owner slice.
use super::*;

#[derive(Clone, Debug)]
pub enum NormalDropPlan {
    Trivial,
    /// No negative evidence: requires concrete admission before executable lowering.
    Unresolved,
    Destructor(Arc<ValidatedDestructor>),
}

/// Only canonical trait facts may construct this capability. Display names never participate.
#[derive(Clone)]
pub struct ValidatedDestructor {
    owner: Ty,
    target: CallTarget,
    signature: Signature,
    world: Arc<ValidatedTraitWorld>,
    budget: PlanBudget,
}
impl std::fmt::Debug for ValidatedDestructor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedDestructor")
            .field("owner", &self.owner)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}
impl PartialEq for ValidatedDestructor {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.world, &other.world)
            && self.budget.same_authority(&other.budget)
            && self.owner == other.owner
            && self.target == other.target
            && self.signature == other.signature
    }
}
impl Eq for ValidatedDestructor {}
impl ValidatedDestructor {
    pub fn owner(&self) -> &Ty {
        &self.owner
    }
    pub fn target(&self) -> &CallTarget {
        &self.target
    }
    pub fn signature(&self) -> &Signature {
        &self.signature
    }
    pub fn validate(&self, owner: &Ty, facts: &ValidatedFacts) -> Result<()> {
        if !Arc::ptr_eq(&self.world, facts.world()) || !self.budget.same_authority(facts.budget()) {
            return Err(FactsError::invalid(
                facts.budget(),
                "cleanup has foreign world or budget",
            ));
        }
        facts.validate_ty(owner)?;
        if !ir::identical_type(owner, &self.owner, facts.budget())
            .map_err(|e| FactsError::from_message(facts.budget(), &e))?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "cleanup changed canonical owner",
            ));
        }
        let NormalDropPlan::Destructor(canonical) = facts.normal_drop_plan(owner)? else {
            return Err(FactsError::invalid(
                facts.budget(),
                "cleanup owner has no canonical destructor",
            ));
        };
        if canonical.target != self.target {
            return Err(FactsError::invalid(
                facts.budget(),
                "cleanup target is not the owner's canonical Drop item",
            ));
        }
        let proof = self.world.discharge_direct(&self.target, facts)?;
        if proof.signature() != &self.signature {
            return Err(FactsError::invalid(
                facts.budget(),
                "cleanup changed canonical signature",
            ));
        }
        Ok(())
    }
}
impl ValidatedFacts {
    /// Feature-presence optimization, not a proof that an arbitrary type is Copy.
    pub fn has_normal_drop_impls(&self) -> bool {
        self.world.has_normal_drop
    }
    pub fn normal_drop_plan(&self, ty: &Ty) -> Result<NormalDropPlan> {
        self.validate_ty(ty)?;
        self.normal_drop_plan_inner(ty, 0)
    }
    fn normal_drop_plan_inner(&self, ty: &Ty, depth: usize) -> Result<NormalDropPlan> {
        charged(self.budget(), self.budget().check_depth(depth))?;
        charged(self.budget(), self.budget().charge_work(1))?;
        // References never own their referent. Admission separately rejects unsupported
        // source reference positions without mistaking a destructor receiver for an owner.
        if matches!(ty, Ty::Ref { .. }) {
            return Ok(NormalDropPlan::Trivial);
        }
        let nominal = match ty {
            Ty::Struct(t) => Some(t.def().source_decl),
            Ty::Enum(t) => Some(t.def().source_decl),
            _ => None,
        };
        let mut implementation = None;
        if let Some(nominal) = nominal {
            for h in self.world.headers.impls.iter() {
                charged(self.budget(), self.budget().charge_work(1))?;
                if h.trait_ref.declaration != self.world.catalog.id(CatalogItem::Drop) {
                    continue;
                }
                let owner = match &h.trait_ref.self_ty {
                    Ty::Struct(t) => Some(t.def().source_decl),
                    Ty::Enum(t) => Some(t.def().source_decl),
                    _ => None,
                };
                if owner == Some(nominal) {
                    implementation = Some(h);
                    break;
                }
            }
        }
        if implementation.is_some() {
            let Ty::Struct(s) = ty else {
                return Err(FactsError::unsupported(
                    self.budget(),
                    "unsupported: normal Drop requires a scalar-field struct",
                ));
            };
            if s.def().fields.iter().any(|f| {
                !matches!(
                    f.ty,
                    Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char | Ty::Unit | Ty::RawPtr { .. }
                )
            }) {
                return Err(FactsError::unsupported(
                    self.budget(),
                    "unsupported: normal Drop requires a scalar-field struct",
                ));
            }
            let call = TraitCall {
                trait_ref: TraitRef {
                    declaration: self.world.catalog.id(CatalogItem::Drop),
                    self_ty: ty.clone(),
                    args: Arc::from([]),
                },
                item: self.world.catalog.id(CatalogItem::DropItem),
                method_args: Arc::from([]),
            };
            let resolved = self.world.resolve_trait_call(&call, self)?;
            let Callee::Direct(target) = resolved.callee() else {
                if self.phase() == FactsPhase::Universal {
                    // Canonical trait resolution succeeded, but the generic
                    // owner has no concrete destructor target yet. Every closed
                    // instance must resolve and demand it before executable CFG.
                    return Ok(NormalDropPlan::Unresolved);
                }
                return Err(FactsError::unsupported(
                    self.budget(),
                    "unsupported: unresolved normal Drop",
                ));
            };
            charged(
                self.budget(),
                self.budget().retain_bytes(
                    std::mem::size_of::<ValidatedDestructor>()
                        + 2 * std::mem::size_of::<usize>()
                        + resolved.signature().params.len() * std::mem::size_of::<Ty>(),
                ),
            )?;
            return Ok(NormalDropPlan::Destructor(Arc::new(ValidatedDestructor {
                owner: ty.clone(),
                target: target.clone(),
                signature: resolved.signature().clone(),
                world: self.world.clone(),
                budget: self.budget().clone(),
            })));
        }
        let mut unresolved = false;
        let mut child = |child: &Ty| -> Result<()> {
            match self.normal_drop_plan_inner(child, depth + 1)? {
                NormalDropPlan::Destructor(_) => {
                    return Err(FactsError::unsupported(
                        self.budget(),
                        "unsupported: aggregate containing a normal Drop owner",
                    ));
                }
                NormalDropPlan::Unresolved => unresolved = true,
                NormalDropPlan::Trivial => {}
            }
            Ok(())
        };
        match ty {
            Ty::Tuple(parts) => {
                for part in parts.iter() {
                    child(part)?;
                }
            }
            Ty::Array { element, .. } | Ty::Slice { element } => child(element)?,
            Ty::Struct(s) => {
                for f in s.def().fields.iter() {
                    child(&f.ty)?;
                }
            }
            Ty::Enum(e) => {
                for v in e.def().variants.iter() {
                    for f in v.fields.iter() {
                        child(&f.ty)?;
                    }
                }
            }
            Ty::Param(_) | Ty::AssociatedType(_) => return Ok(NormalDropPlan::Unresolved),
            _ => {}
        }
        Ok(if unresolved {
            NormalDropPlan::Unresolved
        } else {
            NormalDropPlan::Trivial
        })
    }
    pub fn is_canonical_drop_call(&self, callee: &Callee) -> Result<bool> {
        if let Callee::TraitItem(call) = callee {
            return Ok(call.item == self.world.catalog.id(CatalogItem::DropItem));
        }
        if !self.has_normal_drop_impls() {
            return Ok(false);
        }
        self.world.check_facts(self, true)?;
        for h in self.world.headers.impls.iter() {
            charged(
                self.budget(),
                self.budget().charge_work(h.items.len().saturating_add(1)),
            )?;
        }
        Ok(match callee {
            Callee::TraitItem(call) => call.item == self.world.catalog.id(CatalogItem::DropItem),
            Callee::Virtual(_) => false,
            Callee::Direct(target) => self.world.headers.impls.iter().any(|h| {
                h.trait_ref.declaration == self.world.catalog.id(CatalogItem::Drop)
                    && h.items.iter().any(|(item, body)| {
                        *item == self.world.catalog.id(CatalogItem::DropItem)
                            && *body == target.declaration
                    })
            }),
        })
    }
    pub fn is_destructor_body(&self) -> Result<bool> {
        self.is_canonical_drop_call(&Callee::Direct(CallTarget {
            declaration: self.declaration(),
            type_args: Arc::from([]),
        }))
    }
}

#[cfg(test)]
mod tests;
