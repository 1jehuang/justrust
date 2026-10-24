//! Canonical, authority-bound normal-path destruction for the bounded scalar-owner slice.
use super::*;

#[derive(Clone, Debug)]
pub enum NormalDropPlan {
    Trivial,
    /// No negative evidence: requires concrete admission before executable lowering.
    Unresolved,
    Destructor(Arc<ValidatedDestructor>),
}

/// Declaration-ordered fields with nontrivial drop glue.
pub type DropGlueFields = Vec<(usize, Arc<ValidatedDestructor>)>;

/// Structural drop glue run after the owner's own `Drop::drop` (if any).
/// Field indices follow declaration order; drops run in that order, matching
/// Rust's field, tuple element and array element destruction order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropGlue {
    None,
    /// Struct or tuple fields that need dropping.
    Fields(DropGlueFields),
    /// Enum variants whose payload fields need dropping, keyed by variant index.
    Variants(Vec<(usize, DropGlueFields)>),
    /// Every element of a fixed-length array.
    Array {
        len: usize,
        element: Arc<ValidatedDestructor>,
    },
    /// An owning `Box<dyn P>`: run the vtable's drop slot on the data
    /// pointer, then release the allocation by the vtable's size.
    BoxedObject,
}

/// Only canonical trait facts may construct this capability. Display names never participate.
#[derive(Clone)]
pub struct ValidatedDestructor {
    owner: Ty,
    target: Option<CallTarget>,
    signature: Option<Signature>,
    glue: DropGlue,
    world: Arc<ValidatedTraitWorld>,
    budget: PlanBudget,
}
impl std::fmt::Debug for ValidatedDestructor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedDestructor")
            .field("owner", &self.owner)
            .field("target", &self.target)
            .field("glue", &self.glue)
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
            && self.glue == other.glue
    }
}
impl Eq for ValidatedDestructor {}
impl ValidatedDestructor {
    pub fn owner(&self) -> &Ty {
        &self.owner
    }
    /// The owner's own canonical `Drop::drop`, if it implements Drop.
    pub fn own_target(&self) -> Option<&CallTarget> {
        self.target.as_ref()
    }
    pub fn glue(&self) -> &DropGlue {
        &self.glue
    }
    /// Every user destructor reachable through this glue, deduplicated.
    pub fn targets(&self) -> Vec<CallTarget> {
        let mut out = Vec::new();
        self.collect_targets(&mut out);
        out
    }
    fn collect_targets(&self, out: &mut Vec<CallTarget>) {
        if let Some(t) = &self.target
            && !out.contains(t)
        {
            out.push(t.clone());
        }
        match &self.glue {
            DropGlue::None => {}
            DropGlue::Fields(fields) => fields.iter().for_each(|(_, d)| d.collect_targets(out)),
            DropGlue::Variants(variants) => variants
                .iter()
                .for_each(|(_, f)| f.iter().for_each(|(_, d)| d.collect_targets(out))),
            DropGlue::Array { element, .. } => element.collect_targets(out),
            // Object destructors are demanded at each unsizing site.
            DropGlue::BoxedObject => {}
        }
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
        if canonical.glue != self.glue {
            return Err(FactsError::invalid(
                facts.budget(),
                "cleanup changed canonical drop glue",
            ));
        }
        if let Some(target) = &self.target {
            let proof = self.world.discharge_direct(target, facts)?;
            if Some(proof.signature()) != self.signature.as_ref() {
                return Err(FactsError::invalid(
                    facts.budget(),
                    "cleanup changed canonical signature",
                ));
            }
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
        // References and raw pointers never own their referent.
        if matches!(
            ty,
            Ty::Ref { .. }
                | Ty::RawPtr { .. }
                | Ty::Int(_)
                | Ty::Float(_)
                | Ty::Bool
                | Ty::Char
                | Ty::Unit
                | Ty::Never
                | Ty::Str
        ) {
            return Ok(NormalDropPlan::Trivial);
        }
        if matches!(ty, Ty::Param(_) | Ty::AssociatedType(_)) {
            return Ok(NormalDropPlan::Unresolved);
        }
        if ty.owned_box_object().is_some() {
            // The prelude `impl<T> Drop for Box<T>` is Sized-only; the boxed
            // object's concrete destructor and size come from its vtable.
            charged(
                self.budget(),
                self.budget()
                    .retain_bytes(std::mem::size_of::<ValidatedDestructor>()),
            )?;
            return Ok(NormalDropPlan::Destructor(Arc::new(ValidatedDestructor {
                owner: ty.clone(),
                target: None,
                signature: None,
                glue: DropGlue::BoxedObject,
                world: self.world.clone(),
                budget: self.budget().clone(),
            })));
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
        let mut unresolved = false;
        let mut own = None;
        if implementation.is_some() {
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
            match resolved.callee() {
                Callee::Direct(target) => {
                    own = Some((target.clone(), resolved.signature().clone()));
                }
                _ if self.phase() == FactsPhase::Universal => unresolved = true,
                _ => {
                    return Err(FactsError::unsupported(
                        self.budget(),
                        "unsupported: unresolved normal Drop",
                    ));
                }
            }
        }
        let mut child = |child: &Ty| -> Result<Option<Arc<ValidatedDestructor>>> {
            match self.normal_drop_plan_inner(child, depth + 1)? {
                NormalDropPlan::Destructor(d) => Ok(Some(d)),
                NormalDropPlan::Unresolved => {
                    unresolved = true;
                    Ok(None)
                }
                NormalDropPlan::Trivial => Ok(None),
            }
        };
        let glue = match ty {
            Ty::Tuple(parts) => {
                let mut fields = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    if let Some(d) = child(part)? {
                        fields.push((i, d));
                    }
                }
                if fields.is_empty() {
                    DropGlue::None
                } else {
                    DropGlue::Fields(fields)
                }
            }
            Ty::Array { element, len } => match child(element)? {
                Some(d) if *len > 0 => DropGlue::Array {
                    len: *len,
                    element: d,
                },
                _ => DropGlue::None,
            },
            Ty::Slice { element } => {
                if child(element)?.is_some() {
                    return Err(FactsError::unsupported(
                        self.budget(),
                        "unsupported: drop glue for an unsized slice",
                    ));
                }
                DropGlue::None
            }
            Ty::Struct(s) => {
                let mut fields = Vec::new();
                for (i, f) in s.def().fields.iter().enumerate() {
                    if let Some(d) = child(&f.ty)? {
                        fields.push((i, d));
                    }
                }
                if fields.is_empty() {
                    DropGlue::None
                } else {
                    DropGlue::Fields(fields)
                }
            }
            Ty::Enum(e) => {
                let mut variants = Vec::new();
                for (vi, v) in e.def().variants.iter().enumerate() {
                    let mut fields = Vec::new();
                    for (i, f) in v.fields.iter().enumerate() {
                        if let Some(d) = child(&f.ty)? {
                            fields.push((i, d));
                        }
                    }
                    if !fields.is_empty() {
                        variants.push((vi, fields));
                    }
                }
                if variants.is_empty() {
                    DropGlue::None
                } else {
                    DropGlue::Variants(variants)
                }
            }
            Ty::Dyn(_) => DropGlue::None,
            _ => DropGlue::None,
        };
        if unresolved {
            return Ok(NormalDropPlan::Unresolved);
        }
        if own.is_none() && glue == DropGlue::None {
            return Ok(NormalDropPlan::Trivial);
        }
        charged(
            self.budget(),
            self.budget().retain_bytes(
                std::mem::size_of::<ValidatedDestructor>() + 2 * std::mem::size_of::<usize>(),
            ),
        )?;
        let (target, signature) = match own {
            Some((t, s)) => (Some(t), Some(s)),
            None => (None, None),
        };
        Ok(NormalDropPlan::Destructor(Arc::new(ValidatedDestructor {
            owner: ty.clone(),
            target,
            signature,
            glue,
            world: self.world.clone(),
            budget: self.budget().clone(),
        })))
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
            Callee::Virtual(_) | Callee::Untupled(_) => false,
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
